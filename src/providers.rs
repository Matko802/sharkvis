use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::crypto::{base64_decode, hex_lower, hmac_sha256};
use crate::lyrics::{
    distribute_plain, normalize_name, parse_lrc, parse_ttml, url_encode, FetchOpts, LyricLine,
    LyricWord,
};
use crate::mpris::cmd_out;

pub(crate) struct ProviderOutcome {
    pub lines: Option<Vec<LyricLine>>,
    pub contacted: bool,
    pub plain: bool,
}

fn none(contacted: bool) -> ProviderOutcome {
    ProviderOutcome {
        lines: None,
        contacted,
        plain: false,
    }
}

fn curl(args: &[String], timeout_ms: u64) -> Option<(u16, String)> {
    let mut argv: Vec<String> = vec![
        "-sS".to_string(),
        "-L".to_string(),
        "-m".to_string(),
        "15".to_string(),
        "-w".to_string(),
        "\n__SV_STATUS__%{http_code}".to_string(),
    ];
    argv.extend(args.iter().cloned());
    let refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    let out = cmd_out("curl", &refs, timeout_ms)?;
    let idx = out.rfind("__SV_STATUS__")?;
    let status = out[idx + 13..].trim().parse::<u16>().ok()?;
    Some((status, out[..idx].to_string()))
}

fn get(url: &str, headers: &[(&str, &str)]) -> Option<(u16, String)> {
    let mut args = Vec::new();
    for (k, v) in headers {
        args.push("-H".to_string());
        args.push(format!("{}: {}", k, v));
    }
    args.push(url.to_string());
    curl(&args, 20000)
}

fn post_json(url: &str, body: &str, headers: &[(&str, &str)]) -> Option<(u16, String)> {
    let mut args = vec![
        "-X".to_string(),
        "POST".to_string(),
        "-H".to_string(),
        "Content-Type: application/json".to_string(),
        "--data-binary".to_string(),
        body.to_string(),
    ];
    for (k, v) in headers {
        args.push("-H".to_string());
        args.push(format!("{}: {}", k, v));
    }
    args.push(url.to_string());
    curl(&args, 20000)
}

fn tag_time(s: &str) -> Option<f64> {
    let s = s.trim();
    let (min, rest) = s.split_once(':')?;
    let sec: f64 = rest.replace(',', ".").parse().ok()?;
    Some(min.parse::<f64>().ok()? * 60.0 + sec)
}

fn parse_word_tags(body: &str, base: f64) -> (String, Vec<LyricWord>) {
    let mut chunks: Vec<(f64, String)> = Vec::new();
    let mut pending = base;
    let mut rest = body;
    let mut saw = false;
    loop {
        let Some(open) = rest.find('<') else {
            break;
        };
        let Some(close_rel) = rest[open..].find('>') else {
            break;
        };
        let close = open + close_rel;
        let text = &rest[..open];
        if !text.is_empty() {
            chunks.push((pending, text.to_string()));
        }
        let tag = &rest[open + 1..close];
        if let Some(t) = tag_time(tag) {
            saw = true;
            pending = t;
        } else {
            let parts: Vec<&str> = tag.split(':').collect();
            let packed = if parts.len() == 3 {
                parts[1]
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .map(|t| (t, parts[0].to_string()))
            } else {
                None
            };
            match packed {
                Some((t, word)) => {
                    saw = true;
                    pending = t;
                    chunks.push((t, word));
                }
                None => chunks.push((pending, format!("<{}>", tag))),
            }
        }
        rest = &rest[close + 1..];
    }
    if !rest.is_empty() {
        chunks.push((pending, rest.to_string()));
    }
    let display = chunks
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let words = if !saw {
        Vec::new()
    } else {
        chunks
            .into_iter()
            .filter_map(|(t, text)| {
                let w = text.trim().to_string();
                if w.is_empty() {
                    None
                } else {
                    Some(LyricWord { t, text: w })
                }
            })
            .collect::<Vec<_>>()
    };
    (display, words)
}

fn parse_elrc(text: &str) -> Vec<LyricLine> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if !line.starts_with('[') {
            continue;
        }
        let Some(close) = line.find(']') else {
            continue;
        };
        let Some(t) = tag_time(&line[1..close]) else {
            continue;
        };
        let body = line[close + 1..].trim_start();
        let (display, words) = parse_word_tags(body, t);
        if !display.is_empty() {
            out.push(LyricLine { t, text: display, words });
        }
    }
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn parse_mlx(text: &str) -> Vec<LyricLine> {
    let mut out = Vec::new();
    let mut in_lyrics = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line == "[lyrics]" {
            in_lyrics = true;
            continue;
        }
        if line == "[romanLyrics]" || line == "[translatedLyrics]" {
            in_lyrics = false;
            continue;
        }
        if !in_lyrics || !line.starts_with('[') {
            continue;
        }
        let Some(close) = line.find(']') else {
            continue;
        };
        let tag = &line[1..close];
        if tag.starts_with("part") {
            continue;
        }
        let start = tag.split('~').next().unwrap_or(tag);
        let Some(t) = tag_time(start) else {
            continue;
        };
        let mut body = &line[close + 1..];
        if let Some(pos) = body.find("[bg:") {
            body = &body[..pos];
        }
        let (display, words) = parse_word_tags(body, t);
        if !display.is_empty() {
            out.push(LyricLine { t, text: display, words });
        }
    }
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn parse_timed_or_plain(text: &str, duration: f64) -> (Vec<LyricLine>, bool) {
    let lrc = parse_lrc(text);
    if lrc.iter().any(|l| l.t > 0.0) {
        return (lrc, false);
    }
    let plain = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('['))
        .map(|l| l.to_string())
        .collect::<Vec<_>>();
    (distribute_plain(plain, duration).unwrap_or_default(), true)
}

fn parse_any(format: &str, text: &str, duration: f64) -> (Vec<LyricLine>, bool) {
    match format.to_ascii_lowercase().as_str() {
        "ttml" => {
            let lines = parse_ttml(text);
            if lines.is_empty() {
                parse_timed_or_plain(text, duration)
            } else {
                (lines, false)
            }
        }
        "mlx" => {
            let lines = parse_mlx(text);
            if lines.is_empty() {
                parse_timed_or_plain(text, duration)
            } else {
                (lines, false)
            }
        }
        "lrc" => {
            let lines = parse_lrc(text);
            if lines.is_empty() {
                parse_timed_or_plain(text, duration)
            } else {
                (lines, false)
            }
        }
        "elrc" => {
            let lines = parse_elrc(text);
            if lines.is_empty() {
                parse_timed_or_plain(text, duration)
            } else {
                (lines, false)
            }
        }
        _ => parse_timed_or_plain(text, duration),
    }
}

fn sync_quality(kind: &str) -> i32 {
    match kind.to_ascii_lowercase().replace('_', "-").as_str() {
        "word" | "words" | "word-synced" | "word-synchronized" => 2,
        "line" | "lines" | "line-synced" | "line-synchronized" => 1,
        _ => 0,
    }
}

fn fetch_lyrio(artist: &str, title: &str, duration: f64, opts: &FetchOpts) -> ProviderOutcome {
    let base = opts.lyrio_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return none(false);
    }
    let Some(key) = base64_decode(opts.lyrio_key.trim()) else {
        return none(false);
    };
    if key.len() != 32 {
        return none(false);
    }
    let body = format!(
        "{{\"title\":\"{}\",\"artist\":\"{}\",\"album\":null,\"duration\":{},\"videoId\":null,\"allSources\":false}}",
        json_escape(title),
        json_escape(artist),
        if duration > 0.0 { duration as i64 } else { 0 }
    );
    let mut best: Option<(i32, Vec<LyricLine>, bool)> = None;
    let mut contacted = false;
    for attempt in 0..3 {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let sig = hex_lower(&hmac_sha256(&key, format!("{}\n{}", ts, body).as_bytes()));
        let ts_s = ts.to_string();
        let res = post_json(
            &format!("{}/v2/lyrics/search", base),
            &body,
            &[
                ("X-Lyprio-Timestamp", ts_s.as_str()),
                ("X-Lyprio-Signature", sig.as_str()),
                ("Accept", "application/json"),
            ],
        );
        let Some((status, text)) = res else {
            continue;
        };
        contacted = true;
        if status == 404 {
            break;
        }
        if !(200..300).contains(&status) {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            break;
        };
        let Some(results) = v.get("results").and_then(|r| r.as_array()) else {
            break;
        };
        for r in results {
            let format = r.get("format").and_then(|f| f.as_str()).unwrap_or("");
            let kind = r.get("syncType").and_then(|f| f.as_str()).unwrap_or("");
            let Some(lyrics) = r.get("lyrics").and_then(|f| f.as_str()) else {
                continue;
            };
            let (lines, plain) = parse_any(format, lyrics, duration);
            if lines.is_empty() {
                continue;
            }
            let q = sync_quality(kind);
            if best.as_ref().is_none_or(|(bq, _, _)| q > *bq) {
                best = Some((q, lines, plain));
            }
        }
        let complete = v.get("complete").and_then(|c| c.as_bool()).unwrap_or(true);
        if best.is_some() || complete {
            break;
        }
        let retry = v.get("retryAfterMs").and_then(|c| c.as_u64()).unwrap_or(2000);
        let wait = retry.clamp(250, 3000);
        if attempt < 2 {
            std::thread::sleep(std::time::Duration::from_millis(wait));
        }
    }
    ProviderOutcome {
        lines: best.as_ref().map(|(_, l, _)| l.clone()),
        contacted,
        plain: best.as_ref().is_some_and(|(_, _, p)| *p),
    }
}

fn fetch_betterlyrics(artist: &str, title: &str, duration: f64) -> ProviderOutcome {
    let mut url = format!(
        "https://lyrics-api.boidu.dev/getLyrics?s={}&a={}",
        url_encode(title),
        url_encode(artist)
    );
    if duration > 0.0 {
        url.push_str(&format!("&d={}", duration as i64));
    }
    let Some((status, body)) = get(&url, &[("Accept", "application/json")]) else {
        return none(false);
    };
    if !(200..300).contains(&status) {
        return none(true);
    }
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        return none(true);
    };
    let Some(ttml) = v.get("ttml").and_then(|t| t.as_str()) else {
        return none(true);
    };
    let lines = parse_ttml(ttml);
    ProviderOutcome {
        lines: if lines.is_empty() { None } else { Some(lines) },
        contacted: true,
        plain: false,
    }
}

fn fetch_lrcred(artist: &str, title: &str, duration: f64) -> ProviderOutcome {
    let url = format!(
        "https://lrc.red/api/v1?q={}",
        url_encode(format!("{} {}", title, artist).trim())
    );
    let Some((status, body)) = get(&url, &[("Accept", "application/json")]) else {
        return none(false);
    };
    if !(200..300).contains(&status) {
        return none(true);
    }
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        return none(true);
    };
    let Some(results) = v.get("results").and_then(|r| r.as_array()) else {
        return none(true);
    };
    let want_title = normalize_name(title);
    let want_artist = normalize_name(artist);
    let mut best: Option<(&Value, f64)> = None;
    for r in results {
        let d = r.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0);
        if duration > 0.0 && d > 0.0 && (d - duration).abs() > 3.0 {
            continue;
        }
        let name = normalize_name(r.get("track_name").and_then(|x| x.as_str()).unwrap_or(""));
        let art = normalize_name(r.get("artist_name").and_then(|x| x.as_str()).unwrap_or(""));
        let mut score = 0.0;
        if !want_title.is_empty() && name == want_title {
            score += 50.0;
        } else if !name.is_empty() && (name.contains(&want_title) || want_title.contains(&name)) {
            score += 20.0;
        }
        if !want_artist.is_empty() && art == want_artist {
            score += 40.0;
        }
        if duration > 0.0 && d > 0.0 {
            score -= (d - duration).abs();
        }
        if best.as_ref().is_none_or(|(_, bs)| score > *bs) {
            best = Some((r, score));
        }
    }
    let Some((hit, _)) = best else {
        return none(true);
    };
    let Some(url) = hit.get("lyricsUrl").and_then(|x| x.as_str()) else {
        return none(true);
    };
    let full = if url.starts_with('/') {
        format!("https://lrc.red{}", url)
    } else {
        url.to_string()
    };
    let Some((status, ttml)) = get(&full, &[("Accept", "text/plain")]) else {
        return none(true);
    };
    if !(200..300).contains(&status) {
        return none(true);
    }
    let lines = parse_ttml(&ttml);
    let (lines, plain) = if lines.is_empty() {
        parse_timed_or_plain(&ttml, duration)
    } else {
        (lines, false)
    };
    ProviderOutcome {
        lines: if lines.is_empty() { None } else { Some(lines) },
        contacted: true,
        plain,
    }
}

fn fetch_unison(artist: &str, title: &str, duration: f64) -> ProviderOutcome {
    let mut url = format!(
        "https://unison.boidu.dev/lyrics?song={}",
        url_encode(title)
    );
    if !artist.trim().is_empty() {
        url.push_str(&format!("&artist={}", url_encode(artist)));
    }
    if duration > 0.0 {
        url.push_str(&format!("&duration={}", duration as i64));
    }
    let Some((status, body)) = get(&url, &[("Accept", "application/json")]) else {
        return none(false);
    };
    if !(200..300).contains(&status) {
        return none(true);
    }
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        return none(true);
    };
    if !v.get("success").and_then(|s| s.as_bool()).unwrap_or(false) {
        return none(true);
    }
    let Some(data) = v.get("data") else {
        return none(true);
    };
    let Some(lyrics) = data.get("lyrics").and_then(|l| l.as_str()) else {
        return none(true);
    };
    let format = data.get("format").and_then(|f| f.as_str()).unwrap_or("");
    let (lines, plain) = parse_any(format, lyrics, duration);
    ProviderOutcome {
        lines: if lines.is_empty() { None } else { Some(lines) },
        contacted: true,
        plain,
    }
}

fn fetch_lrclib(artist: &str, title: &str, duration: f64) -> ProviderOutcome {
    let mut url = format!(
        "https://lrclib.net/api/search?track_name={}",
        url_encode(title)
    );
    if !artist.trim().is_empty() {
        url.push_str(&format!("&artist_name={}", url_encode(artist)));
    }
    if duration > 0.0 {
        url.push_str(&format!("&duration={}", duration as i64));
    }
    let Some((status, body)) = get(&url, &[("Accept", "application/json")]) else {
        return none(false);
    };
    if !(200..300).contains(&status) {
        return none(true);
    }
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        return none(true);
    };
    let Some(results) = v.as_array() else {
        return none(true);
    };
    let want_title = normalize_name(title);
    let want_artist = normalize_name(artist);
    let mut best: Option<(&Value, f64)> = None;
    for r in results {
        if r.get("instrumental").and_then(|x| x.as_bool()).unwrap_or(false) {
            continue;
        }
        let d = r.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0);
        if duration > 0.0 && d > 0.0 && (d - duration).abs() > 10.0 {
            continue;
        }
        let name = normalize_name(r.get("name").and_then(|x| x.as_str()).unwrap_or(""));
        let art = normalize_name(
            r.get("artistName")
                .and_then(|x| x.as_str())
                .unwrap_or(""),
        );
        if !want_title.is_empty() && name != want_title && !name.contains(&want_title) {
            continue;
        }
        if !want_artist.is_empty() && !art.is_empty() && art != want_artist {
            continue;
        }
        let mut score = 0.0;
        if name == want_title {
            score += 20.0;
        }
        if art == want_artist {
            score += 10.0;
        }
        if r.get("syncedLyrics")
            .and_then(|x| x.as_str())
            .is_some_and(|s| !s.trim().is_empty())
        {
            score += 5.0;
        }
        if duration > 0.0 && d > 0.0 {
            score -= (d - duration).abs();
        }
        if best.as_ref().is_none_or(|(_, bs)| score > *bs) {
            best = Some((r, score));
        }
    }
    let Some((hit, _)) = best else {
        return none(true);
    };
    if let Some(synced) = hit
        .get("syncedLyrics")
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        let lines = parse_lrc(synced);
        return ProviderOutcome {
            lines: if lines.is_empty() { None } else { Some(lines) },
            contacted: true,
            plain: false,
        };
    }
    if let Some(plain) = hit
        .get("plainLyrics")
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        let texts = plain
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect::<Vec<_>>();
        let lines = distribute_plain(texts, duration).unwrap_or_default();
        return ProviderOutcome {
            lines: if lines.is_empty() { None } else { Some(lines) },
            contacted: true,
            plain: true,
        };
    }
    none(true)
}

fn kugou_title(title: &str) -> String {
    let mut s = title.to_string();
    for (open, close) in [
        ("(", ")"),
        ("（", "）"),
        ("「", "」"),
        ("『", "』"),
        ("<", ">"),
        ("《", "》"),
        ("〈", "〉"),
        ("＜", "＞"),
    ] {
        while let Some(a) = s.find(open) {
            let Some(b) = s[a..].find(close) else {
                break;
            };
            s.replace_range(a..a + b + close.len(), "");
        }
    }
    s.trim().to_string()
}

fn fetch_kugou(artist: &str, title: &str, duration: f64) -> ProviderOutcome {
    let keyword = format!("{} - {}", kugou_title(title), artist.replace(", ", "、"))
        .trim()
        .to_string();
    if keyword == "-" || keyword.is_empty() {
        return none(false);
    }
    let mut url = format!(
        "https://lyrics.kugou.com/search?ver=1&man=yes&client=pc&keyword={}",
        url_encode(&keyword)
    );
    if duration > 0.0 {
        url.push_str(&format!("&duration={}", (duration * 1000.0) as i64));
    }
    let Some((status, body)) = get(&url, &[("Accept", "application/json")]) else {
        return none(false);
    };
    if !(200..300).contains(&status) {
        return none(true);
    }
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        return none(true);
    };
    let Some(cands) = v.get("candidates").and_then(|c| c.as_array()) else {
        return none(true);
    };
    let mut best: Option<(&Value, f64)> = None;
    for c in cands {
        let d = c.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let diff = if d > 10000.0 {
            (d / 1000.0 - duration).abs()
        } else {
            (d - duration).abs()
        };
        if duration > 0.0 && diff > 8.0 {
            continue;
        }
        if best.as_ref().is_none_or(|(_, bd)| diff < *bd) {
            best = Some((c, diff));
        }
    }
    let Some((cand, _)) = best.or_else(|| cands.first().map(|c| (c, 0.0))) else {
        return none(true);
    };
    let Some(id) = cand.get("id").and_then(|x| x.as_i64()) else {
        return none(true);
    };
    let Some(key) = cand.get("accesskey").and_then(|x| x.as_str()) else {
        return none(true);
    };
    let url = format!(
        "https://lyrics.kugou.com/download?fmt=lrc&charset=utf8&client=pc&ver=1&id={}&accesskey={}",
        id,
        url_encode(key)
    );
    let Some((status, body)) = get(&url, &[("Accept", "application/json")]) else {
        return none(true);
    };
    if !(200..300).contains(&status) {
        return none(true);
    }
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        return none(true);
    };
    let Some(content) = v.get("content").and_then(|c| c.as_str()) else {
        return none(true);
    };
    let Some(raw) = base64_decode(content) else {
        return none(true);
    };
    let text = String::from_utf8_lossy(&raw);
    let lines = parse_lrc(&text);
    ProviderOutcome {
        lines: if lines.is_empty() { None } else { Some(lines) },
        contacted: true,
        plain: false,
    }
}

static APPLE_TOKEN: Mutex<Option<String>> = Mutex::new(None);

fn apple_token() -> Option<String> {
    if let Ok(guard) = APPLE_TOKEN.lock() {
        if let Some(t) = guard.as_ref() {
            return Some(t.clone());
        }
    }
    let (status, page) = get("https://beta.music.apple.com", &[])?;
    if !(200..300).contains(&status) {
        return None;
    }
    let marker = "/assets/index~";
    let start = page.find(marker)?;
    let rest = &page[start..];
    let end = rest.find(".js")? + 3;
    let path = &rest[..end];
    let (status, js) = get(&format!("https://beta.music.apple.com{}", path), &[])?;
    if !(200..300).contains(&status) {
        return None;
    }
    let token = find_jwt(&js)?;
    if let Ok(mut guard) = APPLE_TOKEN.lock() {
        *guard = Some(token.clone());
    }
    Some(token)
}

fn find_jwt(s: &str) -> Option<String> {
    let mut from = 0;
    while let Some(rel) = s[from..].find("eyJ") {
        let start = from + rel;
        let bytes = s.as_bytes();
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric()
                || bytes[end] == b'-'
                || bytes[end] == b'_'
                || bytes[end] == b'.'
                || bytes[end] == b'=')
        {
            end += 1;
        }
        let cand = &s[start..end];
        if cand.matches('.').count() == 2 && cand.len() > 60 {
            return Some(cand.to_string());
        }
        from = end;
    }
    None
}

fn fetch_paxsenix(artist: &str, title: &str, duration: f64) -> ProviderOutcome {
    let Some(token) = apple_token() else {
        return none(false);
    };
    let query = format!("{} {}", title, artist).trim().to_string();
    let url = format!(
        "https://amp-api.music.apple.com/v1/catalog/us/search?term={}&types=songs&limit=25&l=en-US&platform=web&format%5Bresources%5D=map&include%5Bsongs%5D=artists&extend=artistUrl",
        url_encode(&query)
    );
    let auth = format!("Bearer {}", token);
    let headers = [
        ("Authorization", auth.as_str()),
        ("Origin", "https://music.apple.com"),
        ("Referer", "https://music.apple.com/"),
        (
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:95.0) Gecko/20100101 Firefox/95.0",
        ),
        ("Accept", "application/json"),
        ("x-apple-renewal", "true"),
    ];
    let Some((status, body)) = get(&url, &headers) else {
        return none(false);
    };
    if status == 401 {
        if let Ok(mut guard) = APPLE_TOKEN.lock() {
            *guard = None;
        }
        return none(true);
    }
    if !(200..300).contains(&status) {
        return none(true);
    }
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        return none(true);
    };
    let Some(data) = v
        .get("results")
        .and_then(|r| r.get("songs"))
        .and_then(|s| s.get("data"))
        .and_then(|d| d.as_array())
    else {
        return none(true);
    };
    let resources = v
        .get("resources")
        .and_then(|r| r.get("songs"))
        .and_then(|s| s.as_object());
    let want_title = normalize_name(title);
    let mut cands: Vec<(String, f64)> = Vec::new();
    for song in data {
        let Some(id) = song.get("id").and_then(|x| x.as_str()) else {
            continue;
        };
        let Some(attr) = resources
            .and_then(|m| m.get(id))
            .and_then(|r| r.get("attributes"))
        else {
            continue;
        };
        let name = normalize_name(attr.get("name").and_then(|x| x.as_str()).unwrap_or(""));
        let art = normalize_name(
            attr.get("artistName")
                .and_then(|x| x.as_str())
                .unwrap_or(""),
        );
        let d = attr
            .get("durationInMillis")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let mut score = 0.0;
        if name == want_title {
            score += 90.0;
        } else if !name.is_empty() && (name.contains(&want_title) || want_title.contains(&name)) {
            score += 50.0;
        }
        let want_artist = normalize_name(artist);
        if !want_artist.is_empty() && art == want_artist {
            score += 40.0;
        }
        if duration > 0.0 && d > 0.0 {
            let delta = (d / 1000.0 - duration).abs();
            score += if delta <= 2.0 {
                30.0
            } else if delta <= 5.0 {
                15.0
            } else if delta <= 10.0 {
                5.0
            } else {
                -30.0
            };
        }
        if score >= 40.0 {
            cands.push((id.to_string(), score));
        }
    }
    cands.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut best: Option<(Vec<LyricLine>, bool)> = None;
    let contacted = true;
    for (id, _) in cands.iter().take(5) {
        let url = format!("https://lyrics.paxsenix.org/apple-music/lyrics?id={}", url_encode(id));
        let Some((status, body)) = get(&url, &[("User-Agent", "Metrolist")]) else {
            continue;
        };
        if !(200..300).contains(&status) {
            continue;
        }
        let (lines, plain) = parse_paxsenix(&body, duration);
        if !lines.is_empty() {
            best = Some((lines, plain));
            break;
        }
    }
    ProviderOutcome {
        lines: best.as_ref().map(|(l, _)| l.clone()),
        contacted,
        plain: best.as_ref().is_some_and(|(_, p)| *p),
    }
}

fn parse_paxsenix(body: &str, duration: f64) -> (Vec<LyricLine>, bool) {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return (Vec::new(), false);
    };
    if let Some(ttml) = v.get("ttmlContent").and_then(|x| x.as_str()) {
        if !ttml.trim().is_empty() {
            let lines = parse_ttml(ttml);
            if !lines.is_empty() {
                return (lines, false);
            }
        }
    }
    for key in ["elrcMultiPerson", "elrc"] {
        if let Some(text) = v.get(key).and_then(|x| x.as_str()) {
            if !text.trim().is_empty() {
                let lines = parse_elrc(text);
                if !lines.is_empty() {
                    return (lines, false);
                }
            }
        }
    }
    if let Some(plain) = v.get("plain").and_then(|x| x.as_str()) {
        if !plain.trim().is_empty() {
            let texts = plain
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect::<Vec<_>>();
            if let Some(lines) = distribute_plain(texts, duration) {
                if !lines.is_empty() {
                    return (lines, true);
                }
            }
        }
    }
    let Some(content) = v.get("content").and_then(|x| x.as_array()) else {
        return (Vec::new(), false);
    };
    let mut out = Vec::new();
    for line in content {
        if line.get("background").and_then(|x| x.as_bool()).unwrap_or(false) {
            continue;
        }
        let start = line.get("timestamp").and_then(|x| x.as_i64()).unwrap_or(0);
        let end = line.get("endtime").and_then(|x| x.as_i64()).unwrap_or(0);
        let Some(words_json) = line.get("text").and_then(|x| x.as_array()) else {
            continue;
        };
        let mut words = Vec::new();
        for w in words_json {
            let Some(text) = w.get("text").and_then(|x| x.as_str()) else {
                continue;
            };
            let word = text.trim();
            if word.is_empty() {
                continue;
            }
            let t = w.get("timestamp").and_then(|x| x.as_i64()).unwrap_or(0) as f64 / 1000.0;
            words.push(LyricWord {
                t,
                text: word.to_string(),
            });
        }
        if words.is_empty() {
            continue;
        }
        let display = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let t = start as f64 / 1000.0;
        if display.is_empty() || end <= start {
            continue;
        }
        out.push(LyricLine {
            t,
            text: display,
            words,
        });
    }
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    (out, false)
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn fetch_provider(
    name: &str,
    artist: &str,
    title: &str,
    duration: f64,
    opts: &FetchOpts,
) -> ProviderOutcome {
    match name {
        "lyrio" => fetch_lyrio(artist, title, duration, opts),
        "betterlyrics" => fetch_betterlyrics(artist, title, duration),
        "lrcred" => fetch_lrcred(artist, title, duration),
        "unison" => fetch_unison(artist, title, duration),
        "lrclib" => fetch_lrclib(artist, title, duration),
        "kugou" => fetch_kugou(artist, title, duration),
        "paxsenix" => fetch_paxsenix(artist, title, duration),
        _ => none(false),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_elrc, parse_mlx, tag_time};

    #[test]
    fn time_tags_parse() {
        assert!((tag_time("01:02.500").unwrap() - 62.5).abs() < 1e-6);
        assert!((tag_time("00:05.25").unwrap() - 5.25).abs() < 1e-6);
        assert!(tag_time("nope").is_none());
    }

    #[test]
    fn mlx_words_split() {
        let text = "[meta]\nmlx_version=1.0\n\n[lyrics]\n[00:01.000~00:03.000]<00:01.000>hello <00:02.000>world<00:03.000>\n";
        let lines = parse_mlx(text);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "hello world");
        assert_eq!(lines[0].words.len(), 2);
        assert!((lines[0].words[1].t - 2.0).abs() < 1e-6);
    }

    #[test]
    fn elrc_words_split() {
        let lines = parse_elrc("[00:10.00]<00:10.00>one <00:11.00>two\n");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "one two");
        assert_eq!(lines[0].words.len(), 2);
    }
}
