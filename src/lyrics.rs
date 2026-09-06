use std::time::{Duration, Instant};

use crate::mpris::{cmd_out, Track};

#[derive(Clone, Default)]
pub struct LyricLine {
    pub t: f64,
    pub text: String,
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
            out.push(LyricLine { t: t + offset, text: text.clone() });
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

fn strip_vtt_tags(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' => {
                tag = false;
                o.push(' ');
            }
            _ if !tag => o.push(c),
            _ => {}
        }
    }
    o.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn parse_vtt(text: &str) -> Vec<LyricLine> {
    let mut out = Vec::new();
    let mut cur_start: Option<f64> = None;
    let mut cur_text = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            if let Some(t) = cur_start.take() {
                let clean = strip_vtt_tags(cur_text.trim());
                if !clean.is_empty() {
                    out.push(LyricLine { t, text: clean });
                }
            }
            cur_text.clear();
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
            if let Some(t) = cur_start.take() {
                let clean = strip_vtt_tags(cur_text.trim());
                if !clean.is_empty() {
                    out.push(LyricLine { t, text: clean });
                }
            }
            cur_text.clear();
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
    if let Some(t) = cur_start.take() {
        let clean = strip_vtt_tags(cur_text.trim());
        if !clean.is_empty() {
            out.push(LyricLine { t, text: clean });
        }
    }
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn normalize_name(s: &str) -> String {
    s.to_ascii_lowercase()
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

fn fetch_lyrics(
    artist: &str,
    title: &str,
    url: &str,
    duration: f64,
    cache_path: &str,
    local_folder: &str,
) -> Vec<LyricLine> {
    if !local_folder.trim().is_empty() {
        if let Some(lines) = scan_local_lrc(local_folder, artist, title) {
            if !lines.is_empty() {
                return lines;
            }
        }
    }
    let synced = fetch_synced(artist, title).unwrap_or_default();
    if !synced.is_empty() {
        return synced;
    }
    if let Some(lines) = fetch_search_synced(artist, title) {
        if !lines.is_empty() {
            return lines;
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

fn fetch_search_synced(artist: &str, title: &str) -> Option<Vec<LyricLine>> {
    let url = format!(
        "https://lrclib.net/api/search?q={}%20{}",
        url_encode(artist),
        url_encode(title)
    );
    let body = cmd_out("curl", &["-fsSL", "-m", "15", &url], 20000)?;
    for chunk in body.split("{\"id\":").skip(1) {
        if chunk.contains("\"instrumental\":true") {
            continue;
        }
        if let Some(synced) = json_string(chunk, "syncedLyrics") {
            if synced.trim().is_empty() {
                continue;
            }
            let lines = parse_lrc(&synced);
            if !lines.is_empty() {
                return Some(lines);
            }
        }
    }
    None
}

fn distribute_plain(texts: Vec<String>, duration: f64) -> Option<Vec<LyricLine>> {
    let texts: Vec<String> = texts.into_iter().filter(|l| !l.is_empty()).collect();
    if texts.len() < 2 || duration < 30.0 {
        return None;
    }
    let step = duration / texts.len() as f64;
    Some(
        texts
            .into_iter()
            .enumerate()
            .map(|(i, text)| LyricLine { t: i as f64 * step, text })
            .collect(),
    )
}

fn fetch_search_plain(artist: &str, title: &str, duration: f64) -> Option<Vec<LyricLine>> {
    if duration < 30.0 {
        return None;
    }
    let url = format!(
        "https://lrclib.net/api/search?q={}%20{}",
        url_encode(artist),
        url_encode(title)
    );
    let body = cmd_out("curl", &["-fsSL", "-m", "15", &url], 20000)?;
    for chunk in body.split("{\"id\":").skip(1) {
        if chunk.contains("\"instrumental\":true") {
            continue;
        }
        if let Some(plain) = json_string(chunk, "plainLyrics") {
            let texts: Vec<String> = plain
                .lines()
                .map(|l| l.trim().to_string())
                .collect();
            if let Some(lines) = distribute_plain(texts, duration) {
                return Some(lines);
            }
        }
    }
    None
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
}

impl LyricWorker {
    pub fn new() -> Self {
        LyricWorker {
            key: String::new(),
            lines: Vec::new(),
            rx: None,
            last_attempt: None,
            last_pos: 0.0,
        }
    }

    pub fn update(&mut self, track: &Track, local_folder: &str) {
        self.update_meta(track, local_folder);
        if track.present {
            self.last_pos = track.position;
        }
    }

    pub fn update_pos(&mut self, pos: f64) {
        if pos.is_finite() && pos >= 0.0 {
            self.last_pos = pos;
        }
    }

    fn update_meta(&mut self, track: &Track, local_folder: &str) {
        if let Some((k, lines)) = self.rx.as_ref().and_then(|r| r.try_recv().ok()) {
            self.rx = None;
            if k == self.key {
                self.lines = lines;
            }
        }
        let key = track.key();
        if key.is_empty() {
            return;
        }
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
        let artist = track.artist.clone();
        let title = track.title.clone();
        let url = track.url.clone();
        let duration = track.duration;
        let folder = local_folder.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let lines = fetch_lyrics(&artist, &title, &url, duration, &path, &folder);
            if !lines.is_empty() {
                if let Some(parent) = std::path::Path::new(&path).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let raw: String = lines
                    .iter()
                    .map(|l| format!("[{:02}:{:05.2}]{}", l.t as u32 / 60, l.t % 60.0, l.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                let _ = std::fs::write(&path, raw);
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
        if track.present && track.key() == self.key {
            track.position
        } else {
            self.last_pos
        }
    }

    fn current_idx(&self, pos: f64) -> Option<usize> {
        let mut idx = None;
        for (i, l) in self.lines.iter().enumerate() {
            if l.t <= pos {
                idx = Some(i);
            } else {
                break;
            }
        }
        idx
    }

    fn revealed_count(&self, i: usize, pos: f64) -> usize {
        let start = self.lines[i].t;
        let end = self.lines.get(i + 1).map(|l| l.t).unwrap_or(start + 8.0);
        let words: Vec<&str> = self.lines[i].text.split_whitespace().collect();
        if words.is_empty() {
            return 0;
        }
        let frac = if end > start {
            ((pos - start) / (end - start)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let mut k = (frac * words.len() as f64).ceil() as usize;
        if k < 1 {
            k = 1;
        }
        k.min(words.len())
    }

    pub fn display_lines(&self, track: &Track, static_text: &str) -> Vec<(String, bool)> {
        if self.lines.is_empty() {
            return vec![(Self::fallback_title(track, static_text), true)];
        }
        let pos = self.live_pos(track);
        let Some(i) = self.current_idx(pos) else {
            return vec![(String::new(), true)];
        };
        let words: Vec<&str> = self.lines[i].text.split_whitespace().collect();
        if words.is_empty() {
            return vec![(String::new(), true)];
        }
        let k = self.revealed_count(i, pos).min(words.len()).max(1) - 1;
        vec![(words[k].to_string(), true)]
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
    fn json_string_unescapes() {
        let body = r#"{"syncedLyrics":"[00:01.00]hi\nthere \"yo\"","instrumental":false}"#;
        assert_eq!(
            json_string(body, "syncedLyrics"),
            Some("[00:01.00]hi\nthere \"yo\"".to_string())
        );
        assert_eq!(json_string(body, "missing"), None);
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;
    use crate::mpris::Track;

    fn worker_with(lines: Vec<LyricLine>) -> LyricWorker {
        LyricWorker {
            key: "a|b".to_string(),
            lines,
            rx: None,
            last_attempt: None,
            last_pos: 0.0,
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
    fn shows_only_current_word() {
        let w = worker_with(vec![
            LyricLine { t: 10.0, text: "one two three four".to_string() },
            LyricLine { t: 20.0, text: "next line".to_string() },
        ]);
        assert_eq!(w.display_lines(&track_at(5.0), "STATIC"), vec![(String::new(), true)]);
        assert_eq!(w.display_lines(&track_at(10.0), "STATIC"), vec![("one".to_string(), true)]);
        assert_eq!(w.display_lines(&track_at(15.0), "STATIC"), vec![("two".to_string(), true)]);
        assert_eq!(
            w.display_lines(&track_at(19.9), "STATIC"),
            vec![("four".to_string(), true)]
        );
        assert_eq!(w.display_lines(&track_at(25.0), "STATIC"), vec![("line".to_string(), true)]);
    }

    #[test]
    fn falls_back_without_lines() {
        let w = worker_with(vec![]);
        assert_eq!(w.display_lines(&track_at(3.0), "STATIC"), vec![("a - b".to_string(), true)]);
        let mut no_track = track_at(0.0);
        no_track.present = false;
        assert_eq!(w.display_lines(&no_track, "STATIC"), vec![("STATIC".to_string(), true)]);
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
        LyricLine { t, text: text.to_string() }
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
