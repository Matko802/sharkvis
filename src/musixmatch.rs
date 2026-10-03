use crate::mpris::cmd_out;

const UA: &str = "Musixmatch/2025120901 CFNetwork/3860.300.31 Darwin/25.2.0";
const APP_ID: &str = "mac-ios-v2.0";

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        if let Json::Obj(m) = self {
            m.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        } else {
            None
        }
    }

    pub(crate) fn pointer(&self, path: &str) -> Option<&Json> {
        let mut cur = self;
        for part in path.split('/').filter(|s| !s.is_empty()) {
            if let Ok(i) = part.parse::<usize>() {
                if let Json::Arr(a) = cur {
                    cur = a.get(i)?;
                    continue;
                }
                return None;
            }
            cur = cur.get(part)?;
        }
        Some(cur)
    }

    fn as_str(&self) -> Option<&str> {
        if let Json::Str(s) = self {
            Some(s)
        } else {
            None
        }
    }

    fn as_f64(&self) -> Option<f64> {
        if let Json::Num(n) = self {
            Some(*n)
        } else {
            None
        }
    }

    pub(crate) fn as_i64(&self) -> Option<i64> {
        self.as_f64().map(|n| n as i64)
    }
}

struct Parser<'a> {
    b: &'a [u8],
    pos: usize,
}

fn skip_ws(p: &mut Parser) {
    while p.pos < p.b.len() && matches!(p.b[p.pos], b' ' | b'\t' | b'\n' | b'\r') {
        p.pos += 1;
    }
}

fn parse_value(p: &mut Parser, depth: usize) -> Option<Json> {
    if depth > 64 {
        return None;
    }
    skip_ws(p);
    let c = *p.b.get(p.pos)?;
    match c {
        b'n' if p.b.get(p.pos..p.pos + 4) == Some(b"null".as_slice()) => {
            p.pos += 4;
            Some(Json::Null)
        }
        b't' if p.b.get(p.pos..p.pos + 4) == Some(b"true".as_slice()) => {
            p.pos += 4;
            Some(Json::Bool(true))
        }
        b'f' if p.b.get(p.pos..p.pos + 5) == Some(b"false".as_slice()) => {
            p.pos += 5;
            Some(Json::Bool(false))
        }
        b'"' => parse_string(p).map(Json::Str),
        b'[' => {
            p.pos += 1;
            let mut out = Vec::new();
            if out.len() > 4096 {
                return None;
            }
            loop {
                skip_ws(p);
                if p.b.get(p.pos) == Some(&b']') {
                    p.pos += 1;
                    break;
                }
                out.push(parse_value(p, depth + 1)?);
                if out.len() > 4096 {
                    return None;
                }
                skip_ws(p);
                match p.b.get(p.pos) {
                    Some(b',') => p.pos += 1,
                    Some(b']') => continue,
                    _ => return None,
                }
            }
            Some(Json::Arr(out))
        }
        b'{' => {
            p.pos += 1;
            let mut out = Vec::new();
            loop {
                skip_ws(p);
                if p.b.get(p.pos) == Some(&b'}') {
                    p.pos += 1;
                    break;
                }
                let Json::Str(k) = parse_string(p).map(Json::Str)? else {
                    return None;
                };
                skip_ws(p);
                if p.b.get(p.pos) != Some(&b':') {
                    return None;
                }
                p.pos += 1;
                let v = parse_value(p, depth + 1)?;
                out.push((k, v));
                if out.len() > 1024 {
                    return None;
                }
                skip_ws(p);
                match p.b.get(p.pos) {
                    Some(b',') => p.pos += 1,
                    Some(b'}') => continue,
                    _ => return None,
                }
            }
            Some(Json::Obj(out))
        }
        b'-' | b'0'..=b'9' => parse_number(p).map(Json::Num),
        _ => None,
    }
}

fn parse_string(p: &mut Parser) -> Option<String> {
    if p.b.get(p.pos) != Some(&b'"') {
        return None;
    }
    p.pos += 1;

    let mut o: Vec<u8> = Vec::new();
    while let Some(&c) = p.b.get(p.pos) {
        match c {
            b'"' => {
                p.pos += 1;
                return Some(String::from_utf8_lossy(&o).into_owned());
            }
            b'\\' => {
                p.pos += 1;
                match p.b.get(p.pos) {
                    Some(b'n') => o.push(b'\n'),
                    Some(b'r') => o.push(b'\r'),
                    Some(b't') => o.push(b'\t'),
                    Some(b'b') => o.push(0x08),
                    Some(b'f') => o.push(0x0c),
                    Some(b'"') => o.push(b'"'),
                    Some(b'\\') => o.push(b'\\'),
                    Some(b'/') => o.push(b'/'),
                    Some(b'u') => {
                        if p.pos + 4 >= p.b.len() {
                            return None;
                        }
                        let h = std::str::from_utf8(&p.b[p.pos + 1..p.pos + 5]).ok()?;
                        let mut cp = u32::from_str_radix(h, 16).ok()?;
                        p.pos += 4;

                        if (0xD800..0xDC00).contains(&cp)
                            && p.pos + 7 <= p.b.len()
                            && p.b[p.pos + 1] == b'\\'
                            && p.b[p.pos + 2] == b'u'
                        {
                            if let Ok(lo_h) =
                                std::str::from_utf8(&p.b[p.pos + 3..p.pos + 7])
                            {
                                if let Ok(lo) = u32::from_str_radix(lo_h, 16) {
                                    if (0xDC00..0xE000).contains(&lo) {
                                        cp = 0x10000
                                            + ((cp - 0xD800) << 10)
                                            + (lo - 0xDC00);
                                        p.pos += 6;
                                    }
                                }
                            }
                        }
                        let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                        let mut enc = [0u8; 4];
                        o.extend_from_slice(ch.encode_utf8(&mut enc).as_bytes());
                    }
                    _ => return None,
                }
                p.pos += 1;
            }
            _ => {
                o.push(c);
                p.pos += 1;
            }
        }
        if o.len() > 1_000_000 {
            return None;
        }
    }
    None
}

fn parse_number(p: &mut Parser) -> Option<f64> {
    let start = p.pos;
    if p.b.get(p.pos) == Some(&b'-') {
        p.pos += 1;
    }
    let mut any = false;
    while p.b.get(p.pos).is_some_and(|c| c.is_ascii_digit()) {
        p.pos += 1;
        any = true;
    }
    if p.b.get(p.pos) == Some(&b'.') {
        p.pos += 1;
        while p.b.get(p.pos).is_some_and(|c| c.is_ascii_digit()) {
            p.pos += 1;
            any = true;
        }
    }
    if matches!(p.b.get(p.pos), Some(b'e') | Some(b'E')) {
        p.pos += 1;
        if matches!(p.b.get(p.pos), Some(b'+') | Some(b'-')) {
            p.pos += 1;
        }
        while p.b.get(p.pos).is_some_and(|c| c.is_ascii_digit()) {
            p.pos += 1;
            any = true;
        }
    }
    if !any {
        return None;
    }
    std::str::from_utf8(&p.b[start..p.pos]).ok()?.parse::<f64>().ok()
}

pub(crate) fn parse_json(text: &str) -> Option<Json> {
    let mut p = Parser { b: text.as_bytes(), pos: 0 };
    let v = parse_value(&mut p, 0)?;
    skip_ws(&mut p);
    if p.pos != p.b.len() {
        return None;
    }
    Some(v)
}

fn api_get(url: &str, guid: Option<&str>, timeout_ms: u64) -> Option<String> {
    let cookie;
    let xcookie;
    let mut args: Vec<&str> = vec!["-fsSL", "-m", "20", "-A", UA];
    if let Some(g) = guid {
        cookie = format!("x-mxm-token-guid={}", g);
        xcookie = format!("X-Cookie: {}", cookie);
        args.push("--cookie");
        args.push(&cookie);
        args.push("-H");
        args.push(&xcookie);
    }
    args.push(url);
    let _ = timeout_ms;
    cmd_out("curl", &args, 25000)
}

fn token_path() -> Option<String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(|d| d.to_string_lossy().into_owned())
        .filter(|d| !d.trim().is_empty())
        .or_else(|| {
            std::env::var_os("HOME").map(|h| format!("{}/.cache", h.to_string_lossy()))
        })?;
    Some(format!("{}/sharkvis/musixmatch_token.json", base))
}

pub(crate) fn is_valid_token(token: &str) -> bool {
    let t = token.trim();
    if t.is_empty() || t.contains("UpgradeOnly") {
        return false;
    }
    if t.chars().all(|c| c == '0' || c == '-') {
        return false;
    }
    true
}

fn load_disk_token() -> Option<String> {
    let path = token_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let json = parse_json(&content)?;
    let token = json.get("user_token")?.as_str()?.trim().to_string();
    if is_valid_token(&token) {
        Some(token)
    } else {
        None
    }
}

fn save_disk_token(token: &str) {
    let Some(path) = token_path() else {
        return;
    };
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!("{{\"user_token\":\"{}\"}}", crate::lyrics::json_escape(token)));
}

fn clear_disk_token() {
    if let Some(path) = token_path() {
        let _ = std::fs::remove_file(path);
    }
}

fn new_guid() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = (&f).take(16).read(&mut bytes);
    }
    if bytes == [0u8; 16] {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes[0..8].copy_from_slice(&t.to_le_bytes()[0..8.min(16)]);
        bytes[8..16].copy_from_slice(&std::process::id().to_le_bytes().repeat(2)[0..8]);
    }
    let h = |b: &[u8]| b.iter().map(|x| format!("{:02x}", x)).collect::<String>();
    format!("{}-{}-{}-{}-{}", h(&bytes[0..4]), h(&bytes[4..6]), h(&bytes[6..8]), h(&bytes[8..10]), h(&bytes[10..16]))
}

fn fetch_fresh_token() -> Option<String> {
    let guid = new_guid();
    let url = format!(
        "https://apic-appmobile.musixmatch.com/ws/1.1/token.get?app_id={}&guid={}",
        APP_ID, guid
    );
    let body = api_get(&url, Some(&guid), 20000)?;
    let json = parse_json(&body)?;
    if json.pointer("/message/header/status_code").and_then(|v| v.as_i64()) != Some(200) {
        return None;
    }
    let token = json
        .pointer("/message/body/user_token")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| is_valid_token(s))?;
    save_disk_token(&token);
    Some(token)
}

fn env_tokens() -> Vec<String> {
    std::env::var("MUSIXMATCH_USERTOKEN")
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| is_valid_token(t))
                .collect()
        })
        .unwrap_or_default()
}

fn query_encode(s: &str) -> String {
    crate::lyrics::url_encode(s)
}

fn macro_url(token: &str, artist: &str, title: &str, album: &str, duration: f64) -> String {
    let mut url = format!(
        "https://apic-appmobile.musixmatch.com/ws/1.1/macro.subtitles.get?format=json&namespace=lyrics_richsynched&subtitle_format=mxm&optional_calls=track.richsync&app_id={}&richsync_compact_type=words",
        APP_ID
    );
    url.push_str(&format!("&q_artist={}", query_encode(artist)));
    url.push_str(&format!("&q_artists={}", query_encode(artist)));
    url.push_str(&format!("&q_track={}", query_encode(title)));
    if !album.trim().is_empty() {
        url.push_str(&format!("&q_album={}", query_encode(album)));
    }
    if duration > 0.0 {
        url.push_str(&format!("&q_duration={}", duration.round() as i64));
    }
    url.push_str(&format!("&usertoken={}", query_encode(token)));
    url
}

fn is_token_error(code: Option<i64>) -> (bool, bool) {
    match code {
        Some(401) => (true, true),
        Some(402) | Some(403) | Some(429) => (true, false),
        _ => (false, false),
    }
}

fn parse_richsync_words(line: &Json, line_start: f64, line_end: f64) -> Option<Vec<crate::lyrics::LyricWord>> {
    if let Some(arr) = line.get("words").and_then(|v| {
        if let Json::Arr(a) = v {
            Some(a)
        } else {
            None
        }
    }) {
        let mut out = Vec::new();
        for w in arr.iter().take(64) {
            let start = w.get("start").and_then(|v| v.as_f64()).unwrap_or(line_start);
            let mut end = w.get("end").and_then(|v| v.as_f64()).unwrap_or(start);
            if end <= start {
                end = line_end;
            }
            let text = w
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            if end - start < 0.039 {
                end = start + 0.039;
            }
            out.push(crate::lyrics::LyricWord { t: start, text });
            let _ = end;
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    if let Some(arr) = line.get("l").and_then(|v| {
        if let Json::Arr(a) = v {
            Some(a)
        } else {
            None
        }
    }) {
        let mut out = Vec::new();
        let mut t = line_start;
        let step = if arr.is_empty() {
            0.0
        } else {
            (line_end - line_start) / arr.len() as f64
        };
        for elem in arr.iter().take(128) {
            let text = elem.get("c").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if text.trim().is_empty() {
                continue;
            }
            out.push(crate::lyrics::LyricWord { t, text });
            t += step;
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

pub(crate) fn richsync_lines(calls: &Json) -> Option<Vec<crate::lyrics::LyricLine>> {
    let ok = calls
        .pointer("/track.richsync.get/message/header/status_code")
        .and_then(|v| v.as_i64())
        == Some(200);
    if !ok {
        return None;
    }
    let body = calls.pointer("/track.richsync.get/message/body/richsync/richsync_body")?;
    let text = body.as_str()?;
    let json = parse_json(text)?;
    let Json::Arr(arr) = json else {
        return None;
    };
    let mut out = Vec::new();
    for line in arr.iter().take(400) {
        let start = line.pointer("/ts").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let end = line
            .pointer("/te")
            .and_then(|v| v.as_f64())
            .unwrap_or(start + 3.0);
        let text = line
            .get("x")
            .or_else(|| line.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let words = parse_richsync_words(line, start, end).unwrap_or_default();
        out.push(crate::lyrics::LyricLine { t: start, text, words });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn subtitles_lines(calls: &Json) -> Option<Vec<crate::lyrics::LyricLine>> {
    let ok = calls
        .pointer("/track.subtitles.get/message/header/status_code")
        .and_then(|v| v.as_i64())
        == Some(200);
    if !ok {
        return None;
    }
    let body = calls.pointer("/track.subtitles.get/message/body/subtitle_list/0/subtitle/subtitle_body")?;
    let text = body.as_str()?;
    let json = parse_json(text)?;
    let Json::Arr(arr) = json else {
        return None;
    };
    let mut out = Vec::new();
    for line in arr.iter().take(400) {
        let t = line.pointer("/time/total").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let text = line
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        out.push(crate::lyrics::LyricLine {
            t,
            text,
            words: Vec::new(),
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn is_instrumental(calls: &Json) -> bool {
    calls
        .pointer("/matcher.track.get/message/body/track/instrumental")
        .map(|v| v.as_i64() == Some(1))
        .unwrap_or(false)
}

fn try_macro(token: &str, artist: &str, title: &str, duration: f64) -> MacroOutcome {
    let url = macro_url(token, artist, title, "", duration);
    let body = match api_get(&url, Some(""), 25000) {
        Some(b) => b,
        None => return MacroOutcome::Fail,
    };
    let json = match parse_json(&body) {
        Some(j) => j,
        None => return MacroOutcome::Fail,
    };
    let code = json.pointer("/message/header/status_code").and_then(|v| v.as_i64());
    let (token_err, renewable) = is_token_error(code);
    if token_err {
        if renewable {
            clear_disk_token();
        }
        return MacroOutcome::Token;
    }
    if code != Some(200) {
        return MacroOutcome::Fail;
    }
    let Some(calls) = json.pointer("/message/body/macro_calls") else {
        return MacroOutcome::Fail;
    };
    if is_instrumental(calls) {
        return MacroOutcome::Lines(vec![crate::lyrics::LyricLine {
            t: 0.0,
            text: "♪ Instrumental ♪".to_string(),
            words: Vec::new(),
        }]);
    }
    if let Some(lines) = richsync_lines(calls) {
        return MacroOutcome::Lines(lines);
    }
    if let Some(lines) = subtitles_lines(calls) {
        return MacroOutcome::Lines(lines);
    }
    MacroOutcome::Fail
}

enum MacroOutcome {
    Lines(Vec<crate::lyrics::LyricLine>),
    Token,
    Fail,
}

pub fn fetch_musixmatch(
    artist: &str,
    title: &str,
    duration: f64,
) -> Option<Vec<crate::lyrics::LyricLine>> {
    let mut tokens = env_tokens();
    if tokens.is_empty() {
        if let Some(t) = load_disk_token() {
            tokens.push(t);
        }
    }
    let mut tried_fresh = false;
    loop {
        if tokens.is_empty() {
            if tried_fresh {
                return None;
            }
            tried_fresh = true;
            if let Some(t) = fetch_fresh_token() {
                tokens.push(t);
                continue;
            }
            return None;
        }
        let token = tokens.remove(0);
        match try_macro(&token, artist, title, duration) {
            MacroOutcome::Lines(lines) => {
                if !tokens.is_empty() || load_disk_token().is_none() {
                    save_disk_token(&token);
                }
                return Some(lines);
            }
            MacroOutcome::Token => {
                clear_disk_token();
                continue;
            }
            MacroOutcome::Fail => {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_validation() {
        assert!(is_valid_token("abc123"));
        assert!(!is_valid_token(""));
        assert!(!is_valid_token("0000-0000"));
        assert!(!is_valid_token("UpgradeOnly-enabled"));
    }

    #[test]
    fn json_mini_parser() {
        let v = parse_json(r#"{"a":1,"b":[true,null,"x\ny"],"c":{"d":-2.5e2}}"#).expect("parse");
        assert_eq!(v.pointer("/a").and_then(|x| x.as_i64()), Some(1));
        assert_eq!(v.pointer("/b/2").and_then(|x| x.as_str()), Some("x\ny"));
        assert_eq!(v.pointer("/c/d").and_then(|x| x.as_f64()), Some(-250.0));
        assert!(parse_json("{bad").is_none());
        assert!(parse_json(&"[".repeat(9000)).is_none());
    }

    #[test]
    fn json_strings_keep_multibyte_utf8() {

        let v = parse_json("\"caf\u{00e9} \\u4e2d\\u6587\"").expect("parse");
        assert_eq!(v.as_str(), Some("café 中文"));
    }

    #[test]
    fn json_string_escapes_and_surrogates() {
        let v = parse_json("\"a\\bb\\fc\"").expect("parse");
        assert_eq!(v.as_str(), Some("a\x08b\x0cc"));

        let v = parse_json("\"\\uD83C\\uDFB5\"").expect("pair");
        assert_eq!(v.as_str(), Some("\u{1F3B5}"));

        let v = parse_json("\"x\\uD83Cy\"").expect("lone");
        assert_eq!(v.as_str(), Some("x\u{FFFD}y"));
    }

    #[test]
    fn disk_token_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sharkvis-tok-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let prev = std::env::var_os("XDG_CACHE_HOME");
        std::env::set_var("XDG_CACHE_HOME", &dir);
        save_disk_token("abc123");
        assert_eq!(load_disk_token().as_deref(), Some("abc123"));
        let _ = std::fs::remove_file(dir.join("sharkvis/musixmatch_token.json"));
        let _ = std::fs::remove_dir_all(&dir);
        if let Some(v) = prev {
            std::env::set_var("XDG_CACHE_HOME", v);
        } else {
            std::env::remove_var("XDG_CACHE_HOME");
        }
    }

    #[test]
    fn richsync_word_parsing() {
        let body = r#"[{"ts":29.26,"te":31.59,"x":"And the","words":[{"start":29.26,"end":30.1,"text":"And"},{"start":30.2,"end":31.0,"text":"the"}]}]"#;
        fn mk_calls(status: i64, body: &str) -> Json {
            let header = Json::Obj(vec![("status_code".to_string(), Json::Num(status as f64))]);
            let richsync = Json::Obj(vec![("richsync_body".to_string(), Json::Str(body.to_string()))]);
            let message = Json::Obj(vec![
                ("header".to_string(), header),
                ("body".to_string(), Json::Obj(vec![("richsync".to_string(), richsync)])),
            ]);
            Json::Obj(vec![("track.richsync.get".to_string(), Json::Obj(vec![("message".to_string(), message)]))])
        }
        let lines = richsync_lines(&mk_calls(200, body)).expect("lines");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "And the");
        assert_eq!(lines[0].words.len(), 2);
        assert!((lines[0].words[0].t - 29.26).abs() < 1e-6);
        assert!((lines[0].words[1].t - 30.2).abs() < 1e-6);
        let bad = mk_calls(404, body);
        assert!(richsync_lines(&bad).is_none());
    }
}
