use std::time::Duration;

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

fn fetch_subs(url: &str, cache_path: &str) -> Vec<LyricLine> {
    if !url.contains("youtube.com/watch") && !url.contains("youtu.be/") {
        return Vec::new();
    }
    let dir = match std::path::Path::new(cache_path).parent() {
        Some(p) => p.to_string_lossy().into_owned(),
        None => return Vec::new(),
    };
    let stem = format!("subs_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0));
    let out_tpl = format!("{}/{}.%(ext)s", dir, stem);
    let _ = cmd_out(
        "yt-dlp",
        &[
            "--skip-download", "--no-playlist", "--write-auto-subs", "--write-subs",
            "--sub-langs", "en*", "--sub-format", "vtt/best", "-o", &out_tpl, url,
        ],
        90000,
    );
    let mut best = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(&stem) && name.ends_with(".vtt") {
                let p = format!("{}/{}", dir, name);
                if let Ok(text) = std::fs::read_to_string(&p) {
                    let lines = parse_vtt(&text);
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

fn fetch_lyrics(artist: &str, title: &str, url: &str, cache_path: &str) -> Vec<LyricLine> {
    let synced = fetch_synced(artist, title).unwrap_or_default();
    if !synced.is_empty() {
        return synced;
    }
    if !url.is_empty() {
        let subs = fetch_subs(url, cache_path);
        if !subs.is_empty() {
            return subs;
        }
    }
    Vec::new()
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
    fetching_for: String,
}

impl LyricWorker {
    pub fn new() -> Self {
        LyricWorker {
            key: String::new(),
            lines: Vec::new(),
            rx: None,
            fetching_for: String::new(),
        }
    }

    pub fn update(&mut self, track: &Track) {
        while let Some(Ok((k, lines))) = self.rx.as_ref().map(|r| r.try_recv()) {
            self.rx = None;
            self.fetching_for = String::new();
            if k == self.key {
                self.lines = lines;
            }
        }
        let key = track.key();
        if key == self.key {
            return;
        }
        self.key = key.clone();
        self.lines.clear();
        self.rx = None;
        self.fetching_for = String::new();
        if key.is_empty() {
            return;
        }
        if let Some(path) = cache_path(&key) {
            if let Ok(meta) = std::fs::metadata(&path) {
                let fresh_empty = meta.len() == 0
                    && meta
                        .modified()
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age < Duration::from_secs(7 * 86400));
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if !text.is_empty() {
                        self.lines = parse_lrc(&text);
                        return;
                    } else if fresh_empty {
                        return;
                    }
                }
            }
            if self.fetching_for != key {
                self.fetching_for = key.clone();
                let artist = track.artist.clone();
                let title = track.title.clone();
                let url = track.url.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                self.rx = Some(rx);
                std::thread::spawn(move || {
                    let lines = fetch_lyrics(&artist, &title, &url, &path);
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
                    } else if std::fs::metadata(&path).is_err() {
                        if let Some(parent) = std::path::Path::new(&path).parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(&path, "");
                    }
                    let _ = tx.send((key, lines));
                });
            }
        }
    }

    pub fn display(&self, track: &Track, static_text: &str) -> String {
        if self.lines.is_empty() {
            if track.present && (!track.artist.is_empty() || !track.title.is_empty()) {
                let a = track.artist.trim();
                let t = track.title.trim();
                if !a.is_empty() && !t.is_empty() {
                    return format!("{} - {}", a, t);
                }
                return format!("{}{}", a, t);
            }
            return static_text.to_string();
        }
        let pos = track.position;
        let mut idx = None;
        for (i, l) in self.lines.iter().enumerate() {
            if l.t <= pos {
                idx = Some(i);
            } else {
                break;
            }
        }
        let Some(i) = idx else {
            return String::new();
        };
        let start = self.lines[i].t;
        let end = self
            .lines
            .get(i + 1)
            .map(|l| l.t)
            .unwrap_or(start + 8.0);
        let words: Vec<&str> = self.lines[i].text.split_whitespace().collect();
        if words.is_empty() {
            return String::new();
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
        if k > words.len() {
            k = words.len();
        }
        words[..k].join(" ")
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
            fetching_for: String::new(),
        }
    }

    fn track_at(pos: f64) -> Track {
        Track {
            present: true,
            artist: "a".to_string(),
            title: "b".to_string(),
            position: pos,
            duration: 30.0,
            url: String::new(),
        }
    }

    #[test]
    fn picks_line_and_reveals_words() {
        let w = worker_with(vec![
            LyricLine { t: 10.0, text: "one two three four".to_string() },
            LyricLine { t: 20.0, text: "next line".to_string() },
        ]);
        assert_eq!(w.display(&track_at(5.0), "STATIC"), "");
        assert_eq!(w.display(&track_at(10.0), "STATIC"), "one");
        assert_eq!(w.display(&track_at(15.0), "STATIC"), "one two");
        assert_eq!(w.display(&track_at(19.9), "STATIC"), "one two three four");
        assert_eq!(w.display(&track_at(20.0), "STATIC"), "next");
    }

    #[test]
    fn falls_back_without_lines() {
        let w = worker_with(vec![]);
        assert_eq!(w.display(&track_at(3.0), "STATIC"), "a - b");
        let mut no_track = track_at(0.0);
        no_track.present = false;
        assert_eq!(w.display(&no_track, "STATIC"), "STATIC");
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
