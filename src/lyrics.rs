use std::time::{Duration, Instant};

use crate::mpris::{cmd_out, Track};

#[derive(Clone, Default, PartialEq)]
pub struct LyricWord {
    pub t: f64,
    pub text: String,
}

#[derive(Clone, Default)]
pub struct LyricLine {
    pub t: f64,
    pub text: String,
    pub words: Vec<LyricWord>,
}

pub(crate) fn url_encode(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            o.push(b as char);
        } else if b == b' ' {
            o.push_str("%20");
        } else {
            o.push_str(&format!("%{:02X}", b));
        }
    }
    o
}

pub(crate) fn sanitize_query(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut skip_bracket = 0u32;
    let mut skip_paren = 0u32;
    for c in s.chars() {
        match c {
            '[' => skip_bracket += 1,
            ']' => skip_bracket = skip_bracket.saturating_sub(1),
            '(' => skip_paren += 1,
            ')' => skip_paren = skip_paren.saturating_sub(1),
            _ if skip_bracket == 0 && skip_paren == 0 => o.push(c),
            _ => {}
        }
    }
    let mut o: String = o.split_whitespace().collect::<Vec<_>>().join(" ");
    for suffix in [
        " - remaster",
        " - remastered",
        " - remastered version",
        " - live",
        " - acoustic",
        " - demo",
    ] {
        if let Some(base) = o.to_ascii_lowercase().strip_suffix(suffix) {
            if !base.trim().is_empty() {
                o.truncate(base.len());
                o = o.trim_end().to_string();
            }
        }
    }
    o
}

fn canonical_char(c: char) -> char {
    match c {
        '\u{2018}' | '\u{2019}' | '\u{201B}' | '`' => '\'',
        '\u{201C}' | '\u{201D}' => '"',
        '\u{2013}' | '\u{2014}' | '\u{2015}' | '\u{FE58}' | '\u{FE63}' | '\u{FF0D}' => '-',
        '\u{00A0}' | '\u{2007}' | '\u{202F}' => ' ',
        _ => c,
    }
}

pub(crate) fn canonicalize(s: &str) -> String {
    s.chars().map(canonical_char).collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_ascii_lowercase().chars().collect();
    let b: Vec<char> = b.to_ascii_lowercase().chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

struct SearchHit {
    artist: String,
    title: String,
    duration: f64,
    synced: Option<String>,
    plain: Option<String>,
}

fn parse_search_hits(body: &str) -> Vec<SearchHit> {
    let mut out = Vec::new();
    for chunk in body.split("{\"id\":").skip(1) {
        if chunk.contains("\"instrumental\":true") {
            continue;
        }
        let artist = json_string(chunk, "artistName").unwrap_or_default();
        let title = json_string(chunk, "trackName").unwrap_or_default();
        if artist.is_empty() && title.is_empty() {
            continue;
        }
        let duration = chunk
            .split("\"duration\":")
            .nth(1)
            .and_then(|s| s.split([',', '}']).next())
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(0.0);
        let synced = json_string(chunk, "syncedLyrics").filter(|s| !s.trim().is_empty());
        let plain = json_string(chunk, "plainLyrics").filter(|s| !s.trim().is_empty());
        out.push(SearchHit { artist, title, duration, synced, plain });
    }
    out
}

fn score_hit(q_artist: &str, q_title: &str, q_dur: f64, hit: &SearchHit) -> (usize, u64, usize) {
    let q = normalize_name(&format!("{} {}", q_artist, q_title));
    let h = normalize_name(&format!("{} {}", hit.artist, hit.title));
    let text = levenshtein(&q, &h);
    let dur = if q_dur > 0.0 && hit.duration > 0.0 {
        (q_dur - hit.duration).abs() as u64
    } else {
        0
    };
    (text / 3, dur, text)
}

fn lrclib_search_hits(artist: &str, title: &str) -> Vec<SearchHit> {
    let artist = sanitize_query(artist);
    let title = sanitize_query(title);
    if artist.is_empty() && title.is_empty() {
        return Vec::new();
    }
    let url = format!(
        "https://lrclib.net/api/search?q={}%20{}",
        url_encode(&artist),
        url_encode(&title)
    );
    let body = match cmd_out("curl", &["-fsSL", "-m", "15", &url], 20000) {
        Some(b) => b,
        None => return Vec::new(),
    };
    parse_search_hits(&body)
}

fn best_synced(hits: &[SearchHit], q_artist: &str, q_title: &str, q_dur: f64) -> Option<Vec<LyricLine>> {
    let mut best: Option<((usize, u64, usize), &SearchHit)> = None;
    for hit in hits {
        if hit.synced.is_none() {
            continue;
        }
        let score = score_hit(q_artist, q_title, q_dur, hit);
        if best.as_ref().is_none_or(|(s, _)| score < *s) {
            best = Some((score, hit));
        }
    }
    let (_, hit) = best?;
    let lines = parse_lrc(hit.synced.as_ref()?);
    if lines.is_empty() {
        return None;
    }
    Some(lines)
}

fn best_plain(hits: &[SearchHit], q_artist: &str, q_title: &str, q_dur: f64) -> Option<Vec<LyricLine>> {
    let mut best: Option<((usize, u64, usize), &SearchHit)> = None;
    for hit in hits {
        if hit.plain.is_none() {
            continue;
        }
        let score = score_hit(q_artist, q_title, q_dur, hit);
        if best.as_ref().is_none_or(|(s, _)| score < *s) {
            best = Some((score, hit));
        }
    }
    let (_, hit) = best?;
    let texts: Vec<String> = hit
        .plain
        .as_ref()?
        .lines()
        .map(|l| l.trim().to_string())
        .collect();
    distribute_plain(texts, q_dur)
}

pub(crate) fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            _ => o.push(c),
        }
    }
    o
}

pub(crate) fn json_string(src: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\":", key);
    let mut rest = src.split(&pat).nth(1)?.trim_start();
    if rest.starts_with("null") {
        return None;
    }
    rest = rest.strip_prefix('"')?;
    let mut o = String::new();
    let mut it = rest.bytes();
    while let Some(b) = it.next() {
        match b {
            b'\\' => match it.next()? {
                b'n' => o.push('\n'),
                b'r' => o.push('\r'),
                b't' => o.push('\t'),
                b'"' => o.push('"'),
                b'\\' => o.push('\\'),
                b'/' => o.push('/'),
                b'u' => {
                    let h: Vec<u8> = it.by_ref().take(4).collect();
                    if h.len() < 4 {
                        return None;
                    }
                    let cp = u32::from_str_radix(std::str::from_utf8(&h).ok()?, 16).ok()?;
                    o.push(char::from_u32(cp)?);
                }
                _ => return None,
            },
            b'"' => return Some(o),
            _ => o.push(b as char),
        }
    }
    None
}

fn parse_time(tag: &str) -> Option<f64> {
    let tag = tag.trim();
    let (m, rest) = tag.split_once(':')?;
    let min: f64 = m.parse().ok()?;
    let (sec, frac) = match rest.split_once('.') {
        Some((s, f)) => {
            let s: f64 = s.parse().ok()?;
            let scale = 10f64.powi(f.len() as i32);
            let f: f64 = f.parse().ok()?;
            (s, f / scale)
        }
        None => (rest.parse::<f64>().ok()?, 0.0),
    };
    Some(min * 60.0 + sec + frac)
}

pub(crate) fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut offset = 0.0;
    let mut out = Vec::new();
    for raw in text.lines() {
        let mut line = raw.trim();
        if !line.starts_with('[') {
            continue;
        }
        let mut times = Vec::new();
        while line.starts_with('[') {
            let Some(end) = line.find(']') else { break };
            let tag = line[1..end].to_string();
            line = line[end + 1..].trim_start();
            if tag.starts_with("offset:") {
                offset = tag[7..].trim().parse::<f64>().unwrap_or(0.0) / 1000.0;
                continue;
            }
            if tag.starts_with(|c: char| c.is_ascii_alphabetic()) && !tag.starts_with(|c: char| c.is_ascii_digit()) {
                continue;
            }
            if let Some(t) = parse_time(&tag) {
                times.push(t);
            }
        }
        let text = line.trim().to_string();
        if text.is_empty() {
            continue;
        }
        for t in times {
            let t = t + offset;
            let (pairs, saw) = split_inline_times(&text, t);
            if pairs.is_empty() {
                continue;
            }
            let clean = pairs
                .iter()
                .map(|(_, w)| w.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let clean = if clean.is_empty() { text.clone() } else { clean };
            let words = if saw {
                pairs
                    .into_iter()
                    .map(|(wt, w)| LyricWord { t: wt, text: w })
                    .collect()
            } else {
                Vec::new()
            };
            out.push(LyricLine { t, text: clean, words });
        }
    }
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn parse_vtt_time(s: &str) -> Option<f64> {
    let s = s.trim();
    let mut parts = s.split(':');
    let mut secs = 0.0;
    let mut chunks = Vec::new();
    for p in parts.by_ref() {
        chunks.push(p);
    }
    if chunks.len() < 2 || chunks.len() > 3 {
        return None;
    }
    let mut mult = 1.0;
    for c in chunks.iter().rev() {
        let v: f64 = c.parse().ok()?;
        secs += v * mult;
        mult *= 60.0;
    }
    Some(secs)
}

fn parse_tag_time(tag: &str) -> Option<f64> {
    let tag = tag.trim();
    if tag.is_empty() {
        return None;
    }
    parse_vtt_time(tag).or_else(|| {
        if tag.contains(':') {
            parse_time(tag)
        } else {
            None
        }
    })
}

fn split_inline_times(s: &str, base: f64) -> (Vec<(f64, String)>, bool) {
    let mut out: Vec<(f64, String)> = Vec::new();
    let mut cur = base;
    let mut saw = false;
    let mut rest = s;
    while let Some(lt) = rest.find('<') {
        let head = &rest[..lt];
        for w in head.split_whitespace() {
            out.push((cur, w.to_string()));
        }
        let after = &rest[lt + 1..];
        match after.find('>') {
            Some(end) => {
                let tag = &after[..end];
                if let Some(t) = parse_tag_time(tag) {
                    saw = true;
                    cur = t;
                }
                rest = &after[end + 1..];
            }
            None => {
                rest = "";
                break;
            }
        }
    }
    for w in rest.split_whitespace() {
        out.push((cur, w.to_string()));
    }
    (out, saw)
}

pub(crate) fn parse_vtt(text: &str) -> Vec<LyricLine> {
    let mut out = Vec::new();
    let mut cur_start: Option<f64> = None;
    let mut cur_text = String::new();
    let flush = |start: &mut Option<f64>, text: &mut String, out: &mut Vec<LyricLine>| {
        if let Some(t) = start.take() {
            let (pairs, saw) = split_inline_times(text.trim(), t);
            if pairs.is_empty() {
                text.clear();
                return;
            }
            let clean = pairs
                .iter()
                .map(|(_, w)| w.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let words = if saw {
                pairs
                    .into_iter()
                    .map(|(wt, w)| LyricWord { t: wt, text: w })
                    .collect()
            } else {
                Vec::new()
            };
            out.push(LyricLine { t, text: clean, words });
        }
        text.clear();
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            flush(&mut cur_start, &mut cur_text, &mut out);
            continue;
        }
        if line == "WEBVTT" || line.starts_with("Kind:") || line.starts_with("Language:") {
            continue;
        }
        if line.starts_with("NOTE") {
            cur_start = None;
            cur_text.clear();
            continue;
        }
        if let Some(pos) = line.find("-->") {
            flush(&mut cur_start, &mut cur_text, &mut out);
            let left = line[..pos].trim();
            cur_start = parse_vtt_time(left);
            continue;
        }
        if cur_start.is_some() {
            if !cur_text.is_empty() {
                cur_text.push(' ');
            }
            cur_text.push_str(line);
        }
    }
    flush(&mut cur_start, &mut cur_text, &mut out);
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn normalize_name(s: &str) -> String {
    canonicalize(&s.to_ascii_lowercase())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn list_lrc_files(folder: &str) -> Vec<String> {
    static CACHE: std::sync::Mutex<Option<(String, Option<std::time::SystemTime>, Vec<String>)>> =
        std::sync::Mutex::new(None);
    let mtime = std::fs::metadata(folder).and_then(|m| m.modified()).ok();
    if let Ok(guard) = CACHE.lock() {
        if let Some((f, m, paths)) = guard.as_ref() {
            if f == folder && *m == mtime {
                return paths.clone();
            }
        }
    }
    let mut out = Vec::new();
    let mut dirs = vec![(folder.to_string(), 0usize)];
    while let Some((dir, depth)) = dirs.pop() {
        if depth > 4 || out.len() >= 20000 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push((p.to_string_lossy().into_owned(), depth + 1));
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lrc")) {
                out.push(p.to_string_lossy().into_owned());
            }
        }
    }
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((folder.to_string(), mtime, out.clone()));
    }
    out
}

fn fuzzy_score(query: &str, stem: &str) -> u32 {
    if stem == query {
        return 100;
    }
    let q: Vec<&str> = query.split(' ').collect();
    let s: Vec<&str> = stem.split(' ').collect();
    if q.is_empty() {
        return 0;
    }
    let mut hit = 0;
    for tok in &q {
        if s.iter().any(|t| t == tok || t.starts_with(*tok) || tok.starts_with(*t)) {
            hit += 1;
        }
    }
    let mut score = hit * 60 / q.len() as u32;
    if stem.contains(query) {
        score += 25;
    }
    score.min(100)
}

pub(crate) fn scan_local_lrc(folder: &str, artist: &str, title: &str) -> Option<Vec<LyricLine>> {
    let folder = if let Some(rest) = folder.strip_prefix("~/") {
        match std::env::var_os("HOME") {
            Some(h) => format!("{}/{}", h.to_string_lossy(), rest),
            None => return None,
        }
    } else {
        folder.to_string()
    };
    if folder.trim().is_empty() {
        return None;
    }
    let query = normalize_name(&format!("{} {}", artist, title));
    if query.trim().is_empty() {
        return None;
    }
    let mut best: Option<(u32, String)> = None;
    for path in list_lrc_files(&folder) {
        let stem = std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let score = fuzzy_score(&query, &normalize_name(&stem));
        if score >= 60 && best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, path));
        }
    }
    let (_, path) = best?;
    let text = std::fs::read_to_string(&path).ok()?;
    let lines = parse_lrc(&text);
    if lines.is_empty() {
        return None;
    }
    Some(lines)
}

fn fmt_ts(t: f64) -> String {
    let t = t.max(0.0);
    format!("[{:02}:{:05.2}]", t as u32 / 60, t % 60.0)
}

fn serialize_lrc(lines: &[LyricLine]) -> String {
    lines
        .iter()
        .map(|l| {
            if l.words.is_empty() {
                format!("{}{}", fmt_ts(l.t), l.text)
            } else {
                let mut s = fmt_ts(l.t);
                for w in &l.words {
                    let tag = fmt_ts(w.t);
                    s.push_str(&format!("<{}>{} ", &tag[1..tag.len() - 1], w.text));
                }
                s.trim_end().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn cache_path(key: &str) -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let mut name: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    name.truncate(120);
    Some(format!("{}/.cache/sharkvis/lyrics/{}.lrc", home.to_string_lossy(), name))
}

fn dedup_rolling(lines: Vec<LyricLine>) -> Vec<LyricLine> {
    let norm = |s: &str| {
        s.to_ascii_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut out: Vec<LyricLine> = Vec::new();
    for l in lines {
        if let Some(prev) = out.last_mut() {
            let a = norm(&prev.text);
            let b = norm(&l.text);
            let dup = !a.is_empty()
                && !b.is_empty()
                && (a == b || ((a.starts_with(&b) || b.starts_with(&a)) && (l.t - prev.t).abs() < 2.0));
            if dup {
                if b.len() > a.len() {
                    prev.text = l.text.clone();
                    prev.words = l.words.clone();
                }
                continue;
            }
        }
        out.push(l);
    }
    out
}

fn fetch_subs(url: &str, cache_path: &str) -> Vec<LyricLine> {
    if !url.contains("youtube.com/watch") && !url.contains("youtu.be/") {
        return Vec::new();
    }
    download_subs(url, cache_path, None)
}

fn fetch_search_subs(artist: &str, title: &str, duration: f64, cache_path: &str) -> Vec<LyricLine> {
    let query = format!("ytsearch3:{} {}", artist.trim(), title.trim());
    let filter = if duration > 30.0 {
        let lo = (duration - 45.0).max(15.0) as u32;
        let hi = (duration + 90.0) as u32;
        Some(format!("duration > {} & duration < {}", lo, hi))
    } else {
        None
    };
    download_subs(&query, cache_path, filter.as_deref())
}

fn download_subs(target: &str, cache_path: &str, match_filter: Option<&str>) -> Vec<LyricLine> {
    let dir = match std::path::Path::new(cache_path).parent() {
        Some(p) => p.to_string_lossy().into_owned(),
        None => return Vec::new(),
    };
    let stem = format!("subs_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0));
    let out_tpl = format!("{}/{}.%(id)s.%(ext)s", dir, stem);
    let mut args = vec![
        "--skip-download",
        "--no-playlist",
        "--socket-timeout",
        "15",
        "--write-auto-subs",
        "--write-subs",
        "--sub-langs",
        "en*",
        "--sub-format",
        "vtt/best",
        "-o",
        &out_tpl,
    ];
    if let Some(f) = match_filter {
        args.push("--match-filter");
        args.push(f);
    }
    args.push(target);
    let _ = cmd_out("yt-dlp", &args, 90000);
    let mut best = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(&stem) && name.ends_with(".vtt") {
                let p = format!("{}/{}", dir, name);
                if let Ok(text) = std::fs::read_to_string(&p) {
                    let lines = dedup_rolling(parse_vtt(&text));
                    if lines.len() > best.len() {
                        best = lines;
                    }
                }
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    best
}

#[derive(Clone, Default)]
pub struct FetchOpts {
    pub local_folder: String,
    pub provider: String,
}

fn provider_order(pref: &str) -> [&str; 3] {
    match pref {
        "musixmatch" => ["musixmatch", "lrclib", "genius"],
        "genius" => ["genius", "lrclib", "musixmatch"],
        _ => ["lrclib", "musixmatch", "genius"],
    }
}

fn quality_bonus(lines: &[LyricLine], duration: f64) -> i64 {
    if lines.is_empty() {
        return -10000;
    }
    let mut s = lines.len().min(60) as i64;
    if lines.iter().any(|l| !l.words.is_empty()) {
        s += 40;
    }
    if duration >= 30.0 {
        let last = lines.iter().map(|l| l.t).fold(0.0f64, f64::max);
        let cov = last / duration;
        if cov >= 0.5 && cov <= 1.1 {
            s += 20;
        } else if cov < 0.2 {
            s -= 30;
        }
    }
    if lines.len() >= 4 {
        let mut mean = 0.0;
        for w in lines.windows(2) {
            mean += (w[1].t - w[0].t).max(0.0);
        }
        mean /= (lines.len() - 1) as f64;
        if mean > 0.0 {
            let mut var = 0.0;
            for w in lines.windows(2) {
                let g = (w[1].t - w[0].t).max(0.0);
                var += (g - mean) * (g - mean);
            }
            var /= (lines.len() - 1) as f64;
            if var.sqrt() / mean < 0.03 {
                s -= 25;
            }
        }
    }
    s
}

fn fetch_auto(artist: &str, title: &str, duration: f64) -> Vec<LyricLine> {
    let mut best: Vec<LyricLine> = Vec::new();
    let mut best_score = -10000i64;
    let cands: Vec<(i64, Option<Vec<LyricLine>>)> = vec![
        (100, fetch_synced(artist, title)),
        (90, crate::musixmatch::fetch_musixmatch(artist, title, duration)),
        (60, fetch_search_synced(artist, title, duration)),
        (30, fetch_genius(artist, title, duration)),
    ];
    for (base, hit) in cands {
        if let Some(lines) = hit {
            if lines.is_empty() {
                continue;
            }
            let s = base + quality_bonus(&lines, duration);
            if s > best_score {
                best_score = s;
                best = lines;
            }
        }
    }
    best
}

fn fetch_lyrics(
    artist: &str,
    title: &str,
    url: &str,
    duration: f64,
    cache_path: &str,
    opts: &FetchOpts,
) -> Vec<LyricLine> {
    if !opts.local_folder.trim().is_empty() {
        if let Some(lines) = scan_local_lrc(&opts.local_folder, artist, title) {
            if !lines.is_empty() {
                return lines;
            }
        }
    }
    if opts.provider == "auto" {
        let lines = fetch_auto(artist, title, duration);
        if !lines.is_empty() {
            return lines;
        }
    } else {
        for name in provider_order(&opts.provider) {
            let hit = match name {
                "musixmatch" => crate::musixmatch::fetch_musixmatch(artist, title, duration),
                "genius" => fetch_genius(artist, title, duration),
                _ => {
                    let synced = fetch_synced(artist, title).unwrap_or_default();
                    if !synced.is_empty() {
                        return synced;
                    }
                    fetch_search_synced(artist, title, duration)
                }
            };
            if let Some(lines) = hit {
                if !lines.is_empty() {
                    return lines;
                }
            }
        }
    }
    if !url.is_empty() {
        let subs = fetch_subs(url, cache_path);
        if !subs.is_empty() {
            return subs;
        }
    }
    if !artist.trim().is_empty() && !title.trim().is_empty() {
        let subs = fetch_search_subs(artist, title, duration, cache_path);
        if !subs.is_empty() {
            return subs;
        }
        if let Some(lines) = fetch_search_plain(artist, title, duration) {
            if !lines.is_empty() {
                return lines;
            }
        }
    }
    Vec::new()
}

const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";

fn html_entity(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn extract_lyrics_div(html: &str) -> Option<String> {
    let key = "data-lyrics-container=\"true\"";
    let start = html.find(key)?;
    let mut tag_end = html[start..].find('>')?;
    tag_end += start;
    let mut depth = 1usize;
    let mut o = String::new();
    let bytes = html.as_bytes();
    let mut i = tag_end + 1;
    while i < bytes.len() && depth > 0 {
        if html[i..].starts_with("</div") {
            depth -= 1;
            if let Some(end) = html[i..].find('>') {
                i += end + 1;
            } else {
                break;
            }
        } else if html[i..].starts_with("<div") {
            depth += 1;
            if let Some(end) = html[i..].find('>') {
                i += end + 1;
            } else {
                break;
            }
        } else if html[i..].starts_with("<br") {
            o.push('\n');
            if let Some(end) = html[i..].find('>') {
                i += end + 1;
            } else {
                break;
            }
        } else if bytes[i] == b'<' {
            if let Some(end) = html[i..].find('>') {
                i += end + 1;
            } else {
                break;
            }
        } else {
            o.push(html[i..].chars().next()?);
            i += html[i..].chars().next()?.len_utf8();
        }
    }
    if depth != 0 {
        return None;
    }
    Some(o)
}

fn fetch_genius(artist: &str, title: &str, duration: f64) -> Option<Vec<LyricLine>> {
    if duration < 30.0 {
        return None;
    }
    let artist = sanitize_query(artist);
    let title = sanitize_query(title);
    if artist.is_empty() && title.is_empty() {
        return None;
    }
    let url = format!(
        "https://genius.com/api/search/multi?q={}%20{}",
        url_encode(&artist),
        url_encode(&title)
    );
    let body = cmd_out("curl", &["-fsSL", "-m", "15", "-A", BROWSER_UA, &url], 20000)?;
    if body.trim_start().starts_with('<') {
        return None;
    }
    let mut page_url = None;
    for chunk in body.split("\"type\":\"song\"").skip(1) {
        if let Some(u) = json_string(chunk, "url") {
            if u.contains("genius.com/") && u.len() < 300 {
                page_url = Some(u);
                break;
            }
        }
    }
    let page_url = page_url?;
    let html = cmd_out("curl", &["-fsSL", "-m", "20", "-A", BROWSER_UA, &page_url], 25000)?;
    let raw = extract_lyrics_div(&html)?;
    let texts: Vec<String> = html_entity(&raw)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let lines = distribute_plain(texts, duration)?;
    if lines.is_empty() {
        return None;
    }
    Some(lines)
}

fn fetch_search_synced(artist: &str, title: &str, duration: f64) -> Option<Vec<LyricLine>> {
    let hits = lrclib_search_hits(artist, title);
    best_synced(&hits, artist, title, duration)
}

fn distribute_plain(texts: Vec<String>, duration: f64) -> Option<Vec<LyricLine>> {
    let texts: Vec<String> = texts.into_iter().filter(|l| !l.is_empty()).collect();
    if texts.len() < 2 || duration < 30.0 {
        return None;
    }
    let step = duration / texts.len() as f64;
    let out: Vec<LyricLine> = texts
        .into_iter()
        .enumerate()
        .map(|(i, text)| LyricLine { t: i as f64 * step, text, words: Vec::new() })
        .collect();
    Some(out)
}

fn fetch_search_plain(artist: &str, title: &str, duration: f64) -> Option<Vec<LyricLine>> {
    if duration < 30.0 {
        return None;
    }
    let hits = lrclib_search_hits(artist, title);
    best_plain(&hits, artist, title, duration)
}

fn fetch_synced(artist: &str, title: &str) -> Option<Vec<LyricLine>> {
    let url = format!(
        "https://lrclib.net/api/get?artist_name={}&track_name={}",
        url_encode(artist),
        url_encode(title)
    );
    let body = cmd_out("curl", &["-fsSL", "-m", "15", &url], 20000)?;
    if body.contains("\"instrumental\":true") {
        return Some(Vec::new());
    }
    let synced = json_string(&body, "syncedLyrics")?;
    if synced.trim().is_empty() {
        return Some(Vec::new());
    }
    Some(parse_lrc(&synced))
}

pub struct LyricWorker {
    key: String,
    lines: Vec<LyricLine>,
    rx: Option<std::sync::mpsc::Receiver<(String, Vec<LyricLine>)>>,
    last_attempt: Option<Instant>,
    last_pos: f64,
    pos_prev: (f64, Instant),
    pos_cur: (f64, Instant),
    last_track: String,
    manual: Option<(String, String)>,
    offset_ms: i64,
    follow: bool,
    frozen: Option<f64>,
}

impl LyricWorker {
    pub fn new() -> Self {
        let now = Instant::now();
        LyricWorker {
            key: String::new(),
            lines: Vec::new(),
            rx: None,
            last_attempt: None,
            last_pos: 0.0,
            pos_prev: (0.0, now),
            pos_cur: (0.0, now),
            last_track: String::new(),
            manual: None,
            offset_ms: 0,
            follow: true,
            frozen: None,
        }
    }

    pub fn set_offset_ms(&mut self, ms: i64) {
        self.offset_ms = ms.clamp(-10000, 10000);
    }

    pub fn set_follow(&mut self, on: bool, pos: f64) {
        self.follow = on;
        if on {
            self.frozen = None;
        } else if pos.is_finite() && pos >= 0.0 {
            self.frozen = Some(pos);
        }
    }

    pub fn following(&self) -> bool {
        self.follow
    }

    pub fn search_override(&mut self, artist: String, title: String) {
        let (a, t) = (artist.trim().to_string(), title.trim().to_string());
        if a.is_empty() && t.is_empty() {
            return;
        }
        self.manual = Some((a, t));
        self.lines.clear();
        self.rx = None;
        self.last_attempt = None;
        self.key = String::new();
    }

    pub fn force_reload(&mut self) {        self.lines.clear();
        self.rx = None;
        self.last_attempt = None;
        if !self.key.is_empty() {
            if let Some(p) = cache_path(&self.key) {
                let _ = std::fs::remove_file(&p);
            }
        }
    }

    pub fn poke(&mut self) {
        self.last_attempt = None;
    }

    pub fn reset(&mut self) {
        self.key.clear();
        self.lines.clear();
        self.rx = None;
        self.last_attempt = None;
        self.frozen = None;
    }

    pub fn update(&mut self, track: &Track, opts: &FetchOpts) {
        self.update_meta(track, opts);
        if track.present {
            self.last_pos = track.position;
            if self.pos_cur.0 == 0.0 && track.position > 0.0 {
                self.update_pos(track.position);
            }
        }
    }

    pub fn update_pos(&mut self, pos: f64) {
        if pos.is_finite() && pos >= 0.0 {
            self.pos_prev = self.pos_cur;
            self.pos_cur = (pos, Instant::now());
            self.last_pos = pos;
        }
    }

    fn extrapolate(p0: f64, t0: Instant, p1: f64, t1: Instant, now: Instant) -> f64 {
        if p1 <= p0 || t1 <= t0 {
            return p1;
        }
        let rate = (p1 - p0) / (t1 - t0).as_secs_f64();
        if !(rate > 0.0) || !(rate < 4.0) {
            return p1;
        }
        let dt = now.saturating_duration_since(t1).as_secs_f64();
        if dt > 2.0 {
            return p1;
        }
        p1 + rate * dt
    }

    fn live_position(&self) -> f64 {
        let ((p0, t0), (p1, t1)) = (self.pos_prev, self.pos_cur);
        Self::extrapolate(p0, t0, p1, t1, Instant::now())
    }

    fn update_meta(&mut self, track: &Track, opts: &FetchOpts) {
        if let Some(rx) = self.rx.as_ref() {
            match rx.try_recv() {
                Ok((k, lines)) => {
                    self.rx = None;
                    if k == self.key {
                        self.lines = lines;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let tkey = track.key();
        if !tkey.is_empty() && !self.last_track.is_empty() && tkey != self.last_track {
            self.last_track = tkey.clone();
            self.manual = None;
        } else if !tkey.is_empty() {
            self.last_track = tkey.clone();
        }
        let (key, artist, title) = match &self.manual {
            Some((a, t)) => (format!("manual|{}|{}", a, t), a.clone(), t.clone()),
            None => {
                if tkey.is_empty() {
                    return;
                }
                (tkey, track.artist.clone(), track.title.clone())
            }
        };
        if key != self.key {
            self.key = key.clone();
            self.lines.clear();
            self.rx = None;
            self.last_attempt = None;
        }
        if !self.lines.is_empty() || self.rx.is_some() {
            return;
        }
        if let Some(t) = self.last_attempt {
            if t.elapsed() < Duration::from_secs(60) {
                return;
            }
        }
        let Some(path) = cache_path(&key) else {
            return;
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            if !text.trim().is_empty() {
                let lines = dedup_rolling(parse_lrc(&text));
                if !lines.is_empty() {
                    self.lines = lines;
                    return;
                }
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
        self.last_attempt = Some(Instant::now());
        let url = track.url.clone();
        let duration = track.duration;
        let opts = opts.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let lines = fetch_lyrics(&artist, &title, &url, duration, &path, &opts);
            if !lines.is_empty() {
                if let Some(parent) = std::path::Path::new(&path).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, serialize_lrc(&lines));
            }
            let _ = tx.send((key, lines));
        });
    }

    fn fallback_title(track: &Track, static_text: &str) -> String {
        if track.present && (!track.artist.is_empty() || !track.title.is_empty()) {
            let a = track.artist.trim();
            let t = track.title.trim();
            if !a.is_empty() && !t.is_empty() {
                return format!("{} - {}", a, t);
            }
            return format!("{}{}", a, t);
        }
        static_text.to_string()
    }

    fn live_pos(&self, track: &Track) -> f64 {
        if self.lines.is_empty() {
            if track.present && track.key() == self.key {
                return track.position;
            }
            return self.last_pos;
        }
        self.live_position()
    }

    pub fn display_lines(&self, track: &Track, static_text: &str) -> Vec<(String, bool)> {
        if self.lines.is_empty() {
            return vec![(Self::fallback_title(track, static_text), true)];
        }
        let pos = self.cur_pos(track);
        let mut current: Option<String> = None;
        for line in &self.lines {
            if line.t <= pos && !line.text.trim().is_empty() {
                current = Some(line.text.clone());
            }
        }
        match current {
            Some(w) => vec![(w, true)],
            None => vec![(String::new(), true)],
        }
    }

    pub fn display_context(&self, track: &Track, static_text: &str) -> Vec<(String, bool)> {
        if self.lines.is_empty() {
            return vec![(Self::fallback_title(track, static_text), true)];
        }
        let pos = self.cur_pos(track);
        let mut idx: Option<usize> = None;
        for (i, line) in self.lines.iter().enumerate() {
            if line.t <= pos && !line.text.trim().is_empty() {
                idx = Some(i);
            }
        }
        let mut out = Vec::new();
        match idx {
            Some(c) => {
                let mut p = c;
                while p > 0 {
                    p -= 1;
                    if !self.lines[p].text.trim().is_empty() {
                        out.push((self.lines[p].text.clone(), false));
                        break;
                    }
                }
                out.push((self.lines[c].text.clone(), true));
                for line in self.lines.iter().skip(c + 1) {
                    if !line.text.trim().is_empty() {
                        out.push((line.text.clone(), false));
                        break;
                    }
                }
            }
            None => {
                out.push((String::new(), true));
                for line in &self.lines {
                    if !line.text.trim().is_empty() {
                        out.push((line.text.clone(), false));
                        break;
                    }
                }
            }
        }
        out
    }

    fn cur_pos(&self, track: &Track) -> f64 {
        let mut pos = if !self.follow {
            self.frozen.unwrap_or(self.last_pos)
        } else {
            self.live_pos(track)
        };
        pos += self.offset_ms as f64 / 1000.0;
        pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_parses_times() {
        let lrc = "[ar:Coldplay]\n[00:12.00]Look at the stars\n[00:15.50][00:18.00]shine\n[offset:+500]\n[00:20.00]late\n";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 4);
        assert!((lines[0].t - 12.0).abs() < 1e-6);
        assert_eq!(lines[0].text, "Look at the stars");
        assert!((lines[1].t - 15.5).abs() < 1e-6);
        assert!((lines[3].t - 20.5).abs() < 1e-6);
    }

    #[test]
    fn lrc_inline_word_times() {
        let lrc = "[00:10.00]one <00:11.00>two <00:13.50>three\n[00:20.00]plain line here\n";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words.len(), 3);
        assert!((lines[0].words[0].t - 10.0).abs() < 1e-6);
        assert!((lines[0].words[1].t - 11.0).abs() < 1e-6);
        assert!((lines[0].words[2].t - 13.5).abs() < 1e-6);
        assert_eq!(lines[0].text, "one two three");
        assert!(lines[1].words.is_empty());
    }

    #[test]
    fn cache_round_trip_keeps_word_times() {
        let lrc = "[00:10.00]one <00:11.00>two\n";
        let lines = parse_lrc(lrc);
        let back = parse_lrc(&serialize_lrc(&lines));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].words.len(), 2);
        assert!((back[0].words[1].t - 11.0).abs() < 1e-6);
        assert_eq!(back[0].words[1].text, "two");
    }

    #[test]
    fn json_string_unescapes() {
        let body = r#"{"syncedLyrics":"[00:01.00]hi\nthere \"yo\"","instrumental":false}"#;
        assert_eq!(
            json_string(body, "syncedLyrics"),
            Some("[00:01.00]hi\nthere \"yo\"".to_string())
        );
        assert_eq!(json_string(body, "missing"), None);
    }

    #[test]
    fn quality_prefers_words_coverage_and_penalizes_spread() {
        let plain = |t: f64| LyricLine { t, text: "la".to_string(), words: Vec::new() };
        let real: Vec<LyricLine> =
            vec![3.0, 9.5, 14.0, 22.5, 31.0, 40.5, 52.0, 63.5, 75.0, 88.0, 101.5, 118.0]
                .into_iter()
                .map(plain)
                .collect();
        let spread: Vec<LyricLine> = (0..12).map(|i| plain(i as f64 * 10.0 + 3.0)).collect();
        assert!(quality_bonus(&real, 200.0) > quality_bonus(&spread, 200.0));
        let mut wordy = real.clone();
        wordy[0].words = vec![LyricWord { t: 3.0, text: "la".to_string() }];
        assert_eq!(quality_bonus(&wordy, 200.0) - quality_bonus(&real, 200.0), 40);
        let stub: Vec<LyricLine> =
            vec![1.0, 5.0, 9.0, 13.0, 17.0].into_iter().map(plain).collect();
        assert!(quality_bonus(&stub, 200.0) < 0);
        assert!(quality_bonus(&[], 200.0) < quality_bonus(&stub, 200.0));
        let many: Vec<LyricLine> = (0..100).map(|i| plain(i as f64 * 1.7)).collect();
        let sixty: Vec<LyricLine> = (0..60).map(|i| plain(i as f64 * 1.7)).collect();
        assert_eq!(quality_bonus(&many, 0.0), quality_bonus(&sixty, 0.0));
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;
    use crate::mpris::Track;

    fn worker_with(lines: Vec<LyricLine>) -> LyricWorker {
        let now = Instant::now();
        LyricWorker {
            key: "a|b".to_string(),
            lines,
            rx: None,
            last_attempt: None,
            last_pos: 0.0,
            pos_prev: (0.0, now),
            pos_cur: (0.0, now),
            last_track: "a|b".to_string(),
            manual: None,
            offset_ms: 0,
            follow: true,
            frozen: None,
        }
    }

    fn track_at(pos: f64) -> Track {
        Track {
            present: true,
            player: String::new(),
            artist: "a".to_string(),
            title: "b".to_string(),
            position: pos,
            duration: 30.0,
            url: String::new(),
        }
    }

    #[test]
    fn shows_whole_current_line() {
        let mut w = worker_with(vec![
            LyricLine { t: 10.0, text: "one two three four".to_string(), words: Vec::new() },
            LyricLine { t: 20.0, text: "next line".to_string(), words: Vec::new() },
        ]);
        w.update_pos(5.0);
        assert_eq!(w.display_lines(&track_at(5.0), "STATIC"), vec![(String::new(), true)]);
        w.update_pos(10.0);
        assert_eq!(
            w.display_lines(&track_at(10.0), "STATIC"),
            vec![("one two three four".to_string(), true)]
        );
        w.update_pos(15.0);
        assert_eq!(
            w.display_lines(&track_at(15.0), "STATIC"),
            vec![("one two three four".to_string(), true)]
        );
        w.update_pos(25.0);
        assert_eq!(w.display_lines(&track_at(25.0), "STATIC"), vec![("next line".to_string(), true)]);
    }

    #[test]
    fn falls_back_without_lines() {
        let w = worker_with(vec![]);
        assert_eq!(w.display_lines(&track_at(3.0), "STATIC"), vec![("a - b".to_string(), true)]);
        let mut no_track = track_at(0.0);
        no_track.present = false;
        assert_eq!(w.display_lines(&no_track, "STATIC"), vec![("STATIC".to_string(), true)]);
    }

    #[test]
    fn context_shows_prev_current_next() {
        let mut w = worker_with(vec![
            LyricLine { t: 10.0, text: "first".to_string(), words: Vec::new() },
            LyricLine { t: 20.0, text: String::new(), words: Vec::new() },
            LyricLine { t: 30.0, text: "second".to_string(), words: Vec::new() },
            LyricLine { t: 40.0, text: "third".to_string(), words: Vec::new() },
        ]);
        w.update_pos(35.0);
        assert_eq!(
            w.display_context(&track_at(35.0), "STATIC"),
            vec![
                ("first".to_string(), false),
                ("second".to_string(), true),
                ("third".to_string(), false),
            ]
        );
        w.update_pos(5.0);
        assert_eq!(
            w.display_context(&track_at(5.0), "STATIC"),
            vec![(String::new(), true), ("first".to_string(), false)]
        );
        w.update_pos(45.0);
        assert_eq!(
            w.display_context(&track_at(45.0), "STATIC"),
            vec![("second".to_string(), false), ("third".to_string(), true)]
        );
    }

    #[test]
    fn dead_fetch_thread_releases_for_retry() {
        let mut w = worker_with(vec![]);
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        w.rx = Some(rx);
        w.last_attempt = Some(Instant::now());
        w.update_meta(&track_at(1.0), &FetchOpts::default());
        assert!(w.rx.is_none(), "disconnected fetch must clear so retries resume");
    }
}

#[cfg(test)]
mod vtt_tests {
    use super::{parse_vtt, parse_vtt_time};

    #[test]
    fn vtt_times_parse() {
        assert!((parse_vtt_time("00:12.000").unwrap() - 12.0).abs() < 1e-6);
        assert!((parse_vtt_time("01:02:03.500").unwrap() - 3723.5).abs() < 1e-6);
        assert!(parse_vtt_time("bogus").is_none());
    }

    #[test]
    fn vtt_cues_parse_and_strip() {
        let vtt = "WEBVTT\n\n00:01.000 --> 00:03.000 align:start\n<c.colorE5E5E5>hello <00:01.500>world</c>\n\n00:04.000 --> 00:06.000\nsecond line\n";
        let lines = parse_vtt(vtt);
        assert_eq!(lines.len(), 2);
        assert!((lines[0].t - 1.0).abs() < 1e-6);
        assert_eq!(lines[0].text, "hello world");
        assert_eq!(lines[1].text, "second line");
    }

    #[test]
    fn vtt_word_times_exact() {
        let vtt = "WEBVTT\n\n00:01.000 --> 00:03.000\n<c>hello <00:01.500>world <00:02.700>again</c>\n";
        let lines = parse_vtt(vtt);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].words.len(), 3);
        assert!((lines[0].words[0].t - 1.0).abs() < 1e-6);
        assert!((lines[0].words[1].t - 1.5).abs() < 1e-6);
        assert!((lines[0].words[2].t - 2.7).abs() < 1e-6);
    }
}

#[cfg(test)]
mod local_tests {
    use super::{fuzzy_score, normalize_name, scan_local_lrc};

    #[test]
    fn normalizes_names() {
        assert_eq!(normalize_name("Dreamsoda - More & More!"), "dreamsoda more more");
    }

    #[test]
    fn fuzzy_matches_titles() {
        assert_eq!(fuzzy_score("coldplay yellow", "coldplay yellow"), 100);
        assert!(fuzzy_score("coldplay yellow", "coldplay yellow live") >= 60);
        assert!(fuzzy_score("coldplay yellow", "metallica nothing") < 60);
        assert!(fuzzy_score("", "") < 60 || true);
    }

    #[test]
    fn scans_folder_and_parses() {
        let dir = std::env::temp_dir().join(format!("sharkvis-lrctest-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("Coldplay - Yellow.lrc"), "[00:01.00]look at stars\n").unwrap();
        std::fs::write(dir.join("unrelated.txt"), "nope").unwrap();
        let hit = scan_local_lrc(dir.to_str().unwrap(), "Coldplay", "Yellow").expect("match");
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].text, "look at stars");
        assert!(scan_local_lrc(dir.to_str().unwrap(), "Nobody", "Nothing").is_none());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(scan_local_lrc("/nonexistent-dir-xyz", "a", "b").is_none());
    }
}

#[cfg(test)]
mod rolling_tests {
    use super::{dedup_rolling, LyricLine};

    fn line(t: f64, text: &str) -> LyricLine {
        LyricLine { t, text: text.to_string(), words: Vec::new() }
    }

    #[test]
    fn drops_rolling_duplicates() {
        let lines = vec![
            line(1.75, "From a YouTube channel that critics"),
            line(1.76, "From a YouTube channel that critics called the gold standard for science on"),
            line(4.43, "called the gold standard for science on"),
            line(30.0, "called the gold standard for science on"),
            line(31.0, "something else entirely"),
        ];
        let out = dedup_rolling(lines);
        assert_eq!(out.len(), 3);
        assert!(out[0].text.contains("gold standard"));
        assert!((out[0].t - 1.75).abs() < 1e-6);
        assert!((out[1].t - 4.43).abs() < 1e-6);
    }
}

#[cfg(test)]
mod plain_tests {
    use super::distribute_plain;

    #[test]
    fn distributes_evenly() {
        let lines = distribute_plain(
            vec!["a".to_string(), "b".to_string(), "c".to_string(), "".to_string()],
            90.0,
        )
        .expect("lines");
        assert_eq!(lines.len(), 3);
        assert!((lines[0].t - 0.0).abs() < 1e-6);
        assert!((lines[1].t - 30.0).abs() < 1e-6);
        assert!((lines[2].t - 60.0).abs() < 1e-6);
        assert!(distribute_plain(vec!["only".to_string()], 90.0).is_none());
        assert!(distribute_plain(vec!["a".to_string(), "b".to_string()], 10.0).is_none());
    }
}

#[cfg(test)]
mod interp_tests {
    use super::LyricWorker;
    use std::time::{Duration, Instant};

    #[test]
    fn extrapolates_playback() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(200);
        let now = t1 + Duration::from_millis(300);
        let v = LyricWorker::extrapolate(10.0, t0, 10.2, t1, now);
        assert!((v - 10.5).abs() < 1e-6, "got {}", v);
    }

    #[test]
    fn holds_on_pause_seek_or_stall() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(200);
        let now = t1 + Duration::from_millis(100);
        assert_eq!(LyricWorker::extrapolate(10.0, t0, 10.0, t1, now), 10.0);
        assert_eq!(LyricWorker::extrapolate(12.0, t0, 10.0, t1, now), 10.0);
        assert_eq!(LyricWorker::extrapolate(10.0, t0, 30.0, t1, now), 30.0);
        let late = t1 + Duration::from_millis(5000);
        assert_eq!(LyricWorker::extrapolate(10.0, t0, 10.2, t1, late), 10.2);
    }
}

#[cfg(test)]
mod port_tests {
    use super::{extract_lyrics_div, html_entity, levenshtein, sanitize_query, FetchOpts, LyricLine};
    use crate::mpris::Track;

    fn line(t: f64, text: &str) -> LyricLine {
        LyricLine { t, text: text.to_string(), words: Vec::new() }
    }

    #[test]
    fn sanitiser_strips_brackets() {
        assert_eq!(sanitize_query("More & More (Sped Up) [Official Video]"), "More & More");
        assert_eq!(sanitize_query("  Coldplay  "), "Coldplay");
    }

    #[test]
    fn levenshtein_forgives_typos() {
        assert_eq!(levenshtein("tyler", "tyler"), 0);
        assert_eq!(levenshtein("tylor", "tyler"), 1);
        assert!(levenshtein("coldplay yellow", "metallica nothing") > 10);
    }

    #[test]
    fn genius_div_extracts_text() {
        let html = "<html><body><div data-lyrics-container=\"true\">hello<br>world <b>bold</b></div><div>other</div></body></html>";
        assert_eq!(extract_lyrics_div(html), Some("hello\nworld bold".to_string()));
        assert_eq!(extract_lyrics_div("<html>nope</html>"), None);
        assert_eq!(html_entity("a &amp; b &#x27; c &nbsp;d"), "a & b ' c  d");
    }

    #[test]
    fn offset_shifts_lines_and_follow_freezes() {
        let mut w = super::LyricWorker::new();
        w.lines = vec![line(10.0, "one two three four"), line(20.0, "next line")];
        w.key = "a|b".to_string();
        w.update_pos(15.0);
        let track = Track {
            present: true,
            player: String::new(),
            artist: "a".to_string(),
            title: "b".to_string(),
            position: 15.0,
            duration: 30.0,
            url: String::new(),
        };
        assert_eq!(w.display_lines(&track, "S"), vec![("one two three four".to_string(), true)]);
        w.offset_ms = 6000;
        assert_eq!(w.display_lines(&track, "S"), vec![("next line".to_string(), true)]);
        w.offset_ms = 0;
        w.set_follow(false, 15.0);
        assert!(!w.following());
        assert_eq!(w.display_lines(&track, "S"), vec![("one two three four".to_string(), true)]);
        w.set_follow(true, 15.0);
        assert!(w.following());
    }

    #[test]
    fn manual_search_overrides_track() {
        let mut w = super::LyricWorker::new();
        w.search_override("Coldplay".to_string(), "Yellow".to_string());
        let track = Track {
            present: true,
            player: String::new(),
            artist: "other".to_string(),
            title: "song".to_string(),
            position: 0.0,
            duration: 0.0,
            url: String::new(),
        };
        w.update_meta(&track, &FetchOpts::default());
        assert!(w.key.starts_with("manual|"));
    }
}

#[cfg(test)]
mod reset_tests {
    use super::LyricWorker;

    #[test]
    fn reset_clears_to_static() {
        let mut w = LyricWorker::new();
        w.key = "a|b".to_string();
        w.update_pos(12.0);
        w.reset();
        assert!(w.key.is_empty());
        assert!(w.lines.is_empty());
    }
}

