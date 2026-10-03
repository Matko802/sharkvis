use std::fs;
use std::io::Write;
use std::path::Path;

pub const PALETTE: &[(&str, &str)] = &[
    ("white", "ffffff"),
    ("red", "ff0000"),
    ("green", "00ff00"),
    ("blue", "0000ff"),
    ("yellow", "ffff00"),
    ("magenta", "ff00ff"),
    ("cyan", "00ffff"),
    ("orange", "ff8800"),
    ("purple", "8800ff"),
    ("lime", "88ff00"),
    ("teal", "00ff88"),
    ("pink", "ff0088"),
    ("gray", "888888"),
    ("black", "000000"),
];

pub const DEFAULT_CHARS: &[u8] = "\u{2581}\u{2582}\u{2583}\u{2584}\u{2585}\u{2586}\u{2587}\u{2588}".as_bytes();

#[derive(Clone)]
pub struct Config {
    pub bars: usize,
    pub bar_width: usize,
    pub bar_spacing: usize,
    pub framerate: u32,
    pub sensitivity: f64,
    pub autosens: bool,
    pub lower_cutoff: u32,
    pub higher_cutoff: u32,
    pub noise_reduction: f64,
    pub method: String,
    pub source: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub color_256: bool,
    pub gradient_low: String,
    pub gradient_high: String,
    pub colors: String,
    pub gradient_amt: u32,
    pub mode: String,
    pub text_align: String,
    pub text_size: u32,
    pub text_style: String,
    pub provider: String,
    pub lyric_offset_ms: i64,
    pub lyrics_folder: String,
    pub mpris_players: String,
    pub chars: Vec<u8>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bars: 0,
            bar_width: 2,
            bar_spacing: 1,
            framerate: 60,
            sensitivity: 100.0,
            autosens: true,
            lower_cutoff: 50,
            higher_cutoff: 8000,
            noise_reduction: 0.2,
            method: "pulse".to_string(),
            source: "auto".to_string(),
            sample_rate: 48000,
            channels: 2,
            color_256: false,
            gradient_low: "ffffff".to_string(),
            gradient_high: "ffffff".to_string(),
            colors: String::new(),
            gradient_amt: 100,
            mode: "bars".to_string(),
            text_align: "center".to_string(),
            text_size: 1,
            text_style: "big ahh".to_string(),
            provider: "auto".to_string(),
            lyric_offset_ms: 0,
            lyrics_folder: String::new(),
            mpris_players: String::new(),
            chars: DEFAULT_CHARS.to_vec(),
        }
    }
}

fn hexval(c: u8) -> i32 {
    match c {
        b'0'..=b'9' => (c - b'0') as i32,
        b'a'..=b'f' => (c - b'a' + 10) as i32,
        b'A'..=b'F' => (c - b'A' + 10) as i32,
        _ => -1,
    }
}

pub fn parse_hex_rgb(s: &[u8]) -> Option<(u32, u32, u32)> {
    let mut s = s;
    if s.first() == Some(&b'#') {
        s = &s[1..];
    }
    if s.len() != 6 {
        return None;
    }
    let mut v = [0i32; 6];
    for (i, c) in s.iter().enumerate() {
        let h = hexval(*c);
        if h < 0 {
            return None;
        }
        v[i] = h;
    }
    let r = ((v[0] << 4) | v[1]) as u32;
    let g = ((v[2] << 4) | v[3]) as u32;
    let b = ((v[4] << 4) | v[5]) as u32;
    Some((r, g, b))
}

pub fn color_name(hex: &str) -> Option<&'static str> {
    for (name, col) in PALETTE {
        if col.eq_ignore_ascii_case(hex) {
            return Some(name);
        }
    }
    None
}

pub fn color_index(hex: &str) -> i32 {
    PALETTE
        .iter()
        .position(|(_, col)| col.eq_ignore_ascii_case(hex))
        .map(|i| i as i32)
        .unwrap_or(-1)
}

pub fn color_to_rgb(hex: &str) -> Option<(u32, u32, u32)> {
    parse_hex_rgb(hex.as_bytes())
}

pub fn config_default_path() -> String {
    if let Ok(env) = std::env::var("SHARKVIS_CONFIG") {
        if !env.is_empty() {
            return env;
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            let p = format!("{}/.config/sharkvis/config.jsonc", home);
            if Path::new(&p).exists() {
                return p;
            }
        }
    }
    if Path::new("./config.jsonc").exists() {
        return "./config.jsonc".to_string();
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return format!("{}/.config/sharkvis/config.jsonc", home);
        }
    }
    "./config.jsonc".to_string()
}

/// Named color or hex to RGB, mirroring the C resolver (palette names,
/// grey/bright_* approximations, #rrggbb or bare rrggbb).
pub fn color_to_rgb_any(s: &str) -> Option<(u32, u32, u32)> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if parse_hex_rgb(t.as_bytes()).is_some() {
        return parse_hex_rgb(t.as_bytes());
    }
    let low = t.to_ascii_lowercase();
    for (name, col) in PALETTE {
        if *name == low {
            return parse_hex_rgb(col.as_bytes());
        }
    }
    let approx = match low.as_str() {
        "grey" => "888888",
        "bright_black" => "808080",
        "bright_red" => "ff5555",
        "bright_green" => "55ff55",
        "bright_yellow" => "ffff55",
        "bright_blue" => "5555ff",
        "bright_magenta" => "ff55ff",
        "bright_cyan" => "55ffff",
        "bright_white" => "ffffff",
        _ => return None,
    };
    parse_hex_rgb(approx.as_bytes())
}

/// Strip `//...` and `/*...*/` comments, string-aware so `//` inside
/// quoted values (paths, charsets) survives.
fn strip_jsonc_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    let mut in_str = false;
    let mut esc = false;
    while let Some(c) = it.next() {
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            continue;
        }
        if c == '/' {
            match it.peek() {
                Some('/') => {
                    for c2 in it.by_ref() {
                        if c2 == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    it.next();
                    let mut prev = '\0';
                    for c2 in it.by_ref() {
                        if prev == '*' && c2 == '/' {
                            break;
                        }
                        prev = c2;
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

fn jint(v: &serde_json::Value, cur: i64) -> i64 {
    if let Some(n) = v.as_i64() {
        return n;
    }
    if let Some(f) = v.as_f64() {
        return f as i64;
    }
    cur
}

fn jbool(v: &serde_json::Value, cur: bool) -> bool {
    if let Some(b) = v.as_bool() {
        return b;
    }
    if let Some(n) = v.as_i64() {
        return n != 0;
    }
    cur
}

/// `colors` style shortcut applied over the loaded gradients: empty is a
/// no-op, one valid color goes solid, a `low,high` pair sets both ends.
fn apply_color_value(cfg: &mut Config, color: &serde_json::Value) {
    if let Some(s) = color.get("color_mode").and_then(|v| v.as_str()) {
        if s == "256" || s == "indexed" {
            cfg.color_256 = true;
        } else if s == "24bit" || s == "truecolor" {
            cfg.color_256 = false;
        } else if let Ok(n) = s.trim().parse::<i64>() {
            cfg.color_256 = n != 0;
        }
    } else if let Some(v) = color.get("color_mode") {
        if let Some(n) = v.as_i64() {
            cfg.color_256 = n != 0;
        }
    }
    if let Some(s) = color.get("gradient_low").and_then(|v| v.as_str()) {
        if !s.is_empty() && color_to_rgb_any(s).is_some() {
            cfg.gradient_low = s.to_string();
        }
    }
    if let Some(s) = color.get("gradient_high").and_then(|v| v.as_str()) {
        if !s.is_empty() && color_to_rgb_any(s).is_some() {
            cfg.gradient_high = s.to_string();
        }
    }
    if let Some(s) = color.get("colors").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            cfg.colors = s.to_string();
        }
    }
    if let Some(v) = color.get("gradient") {
        cfg.gradient_amt = jint(v, cfg.gradient_amt as i64).clamp(0, 100) as u32;
    }
    apply_colors_style(cfg);
}

/// Re-read only the color section (hot-reload poll). Returns false when
/// the file is unreadable or has no color section.
pub fn reload_colors(cfg: &mut Config, path: &str) -> bool {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let stripped = strip_jsonc_comments(&text);
    let root: serde_json::Value = match serde_json::from_str(&stripped) {
        Ok(v) => v,
        Err(_) => return false,
    };
    match root.get("color") {
        Some(color) => {
            apply_color_value(cfg, color);
            true
        }
        None => false,
    }
}

fn apply_colors_style(cfg: &mut Config) {
    let s = cfg.colors.trim();
    if s.is_empty() {
        return;
    }
    if let Some(comma) = s.find(',') {
        let first = s[..comma].trim();
        let second = s[comma + 1..].trim();
        if first.is_empty() || second.is_empty() {
            return;
        }
        if color_to_rgb_any(first).is_some() && color_to_rgb_any(second).is_some() {
            cfg.gradient_low = first.to_string();
            cfg.gradient_high = second.to_string();
        }
        return;
    }
    if color_to_rgb_any(s).is_some() {
        cfg.gradient_low = s.to_string();
        cfg.gradient_high = s.to_string();
    }
}

pub fn config_load(cfg: &mut Config, path: &str) -> bool {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let stripped = strip_jsonc_comments(&text);
    let root: serde_json::Value = match serde_json::from_str(&stripped) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if !root.is_object() {
        return false;
    }
    if let Some(general) = root.get("general") {
        if let Some(v) = general.get("bars") {
            cfg.bars = jint(v, cfg.bars as i64) as usize;
        }
        if let Some(v) = general.get("bar_width") {
            cfg.bar_width = jint(v, cfg.bar_width as i64) as usize;
        }
        if let Some(v) = general.get("bar_spacing") {
            cfg.bar_spacing = jint(v, cfg.bar_spacing as i64) as usize;
        }
        if let Some(v) = general.get("framerate") {
            cfg.framerate = jint(v, cfg.framerate as i64) as u32;
        }
        if let Some(v) = general.get("sensitivity") {
            if let Some(f) = v.as_f64() {
                cfg.sensitivity = f;
            }
        }
        if let Some(v) = general.get("autosens") {
            cfg.autosens = jbool(v, cfg.autosens);
        }
        if let Some(v) = general.get("lower_cutoff_freq") {
            cfg.lower_cutoff = jint(v, cfg.lower_cutoff as i64) as u32;
        }
        if let Some(v) = general.get("higher_cutoff_freq") {
            cfg.higher_cutoff = jint(v, cfg.higher_cutoff as i64) as u32;
        }
    }
    if let Some(v) = root
        .get("smoothing")
        .and_then(|s| s.get("noise_reduction"))
    {
        if let Some(f) = v.as_f64() {
            cfg.noise_reduction = f;
        }
    }
    if let Some(input) = root.get("input") {
        if let Some(m) = input.get("method").and_then(|v| v.as_str()) {
            if m == "pulse" || m == "pipewire" || m == "auto" {
                cfg.method = m.to_string();
            } else if !m.is_empty() {
                eprintln!("sharkvis: input method '{}' not supported, using pulse", m);
            }
        }
        if let Some(s) = input.get("source").and_then(|v| v.as_str()) {
            if !s.is_empty() {
                cfg.source = s.to_string();
            }
        }
        if let Some(v) = input.get("sample_rate") {
            cfg.sample_rate = jint(v, cfg.sample_rate as i64) as u32;
        }
        if let Some(v) = input.get("channels") {
            cfg.channels = jint(v, cfg.channels as i64) as u32;
        }
    }
    if let Some(lyrics) = root.get("lyrics") {
        if let Some(s) = lyrics.get("folder").and_then(|v| v.as_str()) {
            cfg.lyrics_folder = s.to_string();
        }
    }
    if let Some(mpris) = root.get("mpris") {
        if let Some(s) = mpris.get("players").and_then(|v| v.as_str()) {
            cfg.mpris_players = s.to_string();
        }
    }
    if let Some(color) = root.get("color") {
        apply_color_value(cfg, color);
    }
    if let Some(vis) = root.get("visualizer") {
        if let Some(s) = vis.get("mode").and_then(|v| v.as_str()) {
            if s == "bars" || s == "wave" || s == "oscilloscope" || s == "lissajous" {
                cfg.mode = s.to_string();
            } else if s == "lyrics" {
                cfg.mode = "lyrics".to_string();
            } else if s == "text" {
                // Old name for the lyrics mode.
                cfg.mode = "lyrics".to_string();
            }
        }
        if let Some(s) = vis.get("text_align").and_then(|v| v.as_str()) {
            if s == "left" || s == "center" {
                cfg.text_align = s.to_string();
            }
        }
        if let Some(v) = vis.get("text_size") {
            if let Some(n) = v.as_u64() {
                cfg.text_size = (n as u32).min(5);
            }
        }
        if let Some(s) = vis.get("text_style").and_then(|v| v.as_str()) {
            if s == "big ahh" || s == "normal" {
                cfg.text_style = s.to_string();
            } else if s == "big" {
                cfg.text_style = "big ahh".to_string();
            } else if s == "small" {
                cfg.text_style = "normal".to_string();
            }
        }
        if let Some(s) = vis.get("provider").and_then(|v| v.as_str()) {
            if s == "auto" || s == "musixmatch" || s == "lrclib" {
                cfg.provider = s.to_string();
            }
        }
        if let Some(v) = vis.get("offset_ms") {
            if let Some(n) = v.as_i64() {
                cfg.lyric_offset_ms = n.clamp(-10000, 10000);
            }
        }
        if let Some(s) = vis.get("chars").and_then(|v| v.as_str()) {
            cfg.chars = s.as_bytes().to_vec();
        }
    }
    true
}

fn mkdir_p(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
}

/// JSON string escaping matching the C writer: \" \\ \b \f \n \r \t and
/// \u00xx for other controls, raw UTF-8 otherwise.
fn json_escape_into(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out.push('"');
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    json_escape_into(&mut out, s);
    out
}

pub fn config_save(cfg: &Config, path: &str) -> bool {
    mkdir_p(Path::new(path));
    let mut f = match fs::File::create(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("    \"general\": {\n");
    out.push_str(&format!("        \"bars\": {},\n", cfg.bars));
    out.push_str(&format!("        \"bar_width\": {},\n", cfg.bar_width));
    out.push_str(&format!("        \"bar_spacing\": {},\n", cfg.bar_spacing));
    out.push_str(&format!("        \"framerate\": {},\n", cfg.framerate));
    out.push_str(&format!("        \"sensitivity\": {:.0},\n", cfg.sensitivity));
    out.push_str(&format!(
        "        \"autosens\": {},\n",
        if cfg.autosens { "true" } else { "false" }
    ));
    out.push_str(&format!(
        "        \"lower_cutoff_freq\": {},\n",
        cfg.lower_cutoff
    ));
    out.push_str(&format!(
        "        \"higher_cutoff_freq\": {}\n",
        cfg.higher_cutoff
    ));
    out.push_str("    },\n");
    out.push_str("    \"smoothing\": {\n");
    out.push_str(&format!("        \"noise_reduction\": {:.2}\n", cfg.noise_reduction));
    out.push_str("    },\n");
    out.push_str("    \"input\": {\n");
    out.push_str(&format!("        \"method\": {},\n", json_escape(&cfg.method)));
    out.push_str(&format!("        \"source\": {},\n", json_escape(&cfg.source)));
    out.push_str(&format!("        \"sample_rate\": {},\n", cfg.sample_rate));
    out.push_str(&format!("        \"channels\": {}\n", cfg.channels));
    out.push_str("    },\n");
    out.push_str("    \"color\": {\n");
    out.push_str(&format!(
        "        \"color_mode\": \"{}\",\n",
        if cfg.color_256 { "256" } else { "24bit" }
    ));
    out.push_str(&format!(
        "        \"gradient_low\": {},\n",
        json_escape(&cfg.gradient_low)
    ));
    out.push_str(&format!(
        "        \"gradient_high\": {},\n",
        json_escape(&cfg.gradient_high)
    ));
    out.push_str(&format!("        \"colors\": {},\n", json_escape(&cfg.colors)));
    out.push_str(&format!("        \"gradient\": {}\n", cfg.gradient_amt));
    out.push_str("    },\n");
    out.push_str("    \"visualizer\": {\n");
    out.push_str(&format!("        \"mode\": {},\n", json_escape(&cfg.mode)));
    out.push_str(&format!(
        "        \"text_align\": {},\n",
        json_escape(&cfg.text_align)
    ));
    out.push_str(&format!("        \"text_size\": {},\n", cfg.text_size));
    out.push_str(&format!(
        "        \"text_style\": {},\n",
        json_escape(&cfg.text_style)
    ));
    out.push_str(&format!(
        "        \"provider\": {},\n",
        json_escape(&cfg.provider)
    ));
    out.push_str(&format!("        \"offset_ms\": {},\n", cfg.lyric_offset_ms));
    out.push_str("        \"chars\": \"");
    // Byte-faithful: the charset is raw bytes, not UTF-8 text. Pushing each
    // byte as a char would re-encode values >= 0x80 (mojibake). Escape only
    // the JSON metacharacters, drop ASCII controls, pass the rest through.
    {
        let mut esc = Vec::with_capacity(cfg.chars.len() + 2);
        for &ch in &cfg.chars {
            if ch == b'"' || ch == b'\\' {
                esc.push(b'\\');
            }
            if ch >= 0x20 {
                esc.push(ch);
            }
        }
        out.push_str(&String::from_utf8_lossy(&esc));
    }
    out.push_str("\"\n");
    out.push_str("    },\n");
    out.push_str("    \"lyrics\": {\n");
    out.push_str(&format!(
        "        \"folder\": {}\n",
        json_escape(&cfg.lyrics_folder)
    ));
    out.push_str("    },\n");
    out.push_str("    \"mpris\": {\n");
    out.push_str(&format!(
        "        \"players\": {}\n",
        json_escape(&cfg.mpris_players)
    ));
    out.push_str("    }\n");
    out.push_str("}\n");
    let _ = f.write_all(out.as_bytes());
    drop(f);
    true
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc_load_with_comments() {
        let text = r##"{
            // leading comment
            "general": {
                "bars": 24, /* inline block */
                "sensitivity": 150,
                "autosens": false,
                "lower_cutoff_freq": 30,
                "higher_cutoff_freq": 9000
            },
            "smoothing": {"noise_reduction": 0.5},
            "input": {"method": "auto", "source": "alsa", "sample_rate": 44100, "channels": 1},
            "color": {
                "color_mode": "256",
                "gradient_low": "red",
                "gradient_high": "#00ff00",
                "colors": "jefetch",
                "gradient": 50
            },
            "visualizer": {
                "mode": "wave", "text_align": "left", "text_size": 3,
                "text_style": "small", "provider": "lrclib", "offset_ms": -250,
                "chars": "ab"
            },
            "lyrics": {"folder": "/tmp/x"},
            "mpris": {"players": "a,b"}
        }"##;
        let path =
            std::env::temp_dir().join(format!("sharkvis-cfgtest-{}.jsonc", std::process::id()));
        std::fs::write(&path, text).unwrap();
        let mut c = Config::default();
        assert!(config_load(&mut c, path.to_str().unwrap()));
        assert_eq!(c.bars, 24);
        assert_eq!(c.sensitivity, 150.0);
        assert!(!c.autosens);
        assert_eq!(c.lower_cutoff, 30);
        assert_eq!(c.higher_cutoff, 9000);
        assert_eq!(c.noise_reduction, 0.5);
        assert_eq!(c.method, "auto");
        assert_eq!(c.source, "alsa");
        assert_eq!(c.sample_rate, 44100);
        assert_eq!(c.channels, 1);
        assert!(c.color_256);
        // colors=jefetch leaves file gradients alone
        assert_eq!(c.gradient_low, "red");
        assert_eq!(c.gradient_high, "#00ff00");
        assert_eq!(c.colors, "jefetch");
        assert_eq!(c.gradient_amt, 50);
        assert_eq!(c.mode, "wave");
        assert_eq!(c.text_align, "left");
        assert_eq!(c.text_size, 3);
        assert_eq!(c.text_style, "normal");
        assert_eq!(c.provider, "lrclib");
        assert_eq!(c.lyric_offset_ms, -250);
        assert_eq!(c.chars, b"ab");
        assert_eq!(c.lyrics_folder, "/tmp/x");
        assert_eq!(c.mpris_players, "a,b");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn jsonc_roundtrip_preserves_values() {
        let mut c = Config::default();
        c.gradient_low = "red".to_string();
        c.gradient_high = "#00ff00".to_string();
        c.colors = "jefetch".to_string();
        c.gradient_amt = 42;
        c.method = "auto".to_string();
        c.chars = "x\"y\\z".as_bytes().to_vec();
        let path =
            std::env::temp_dir().join(format!("sharkvis-rt-{}.jsonc", std::process::id()));
        let ps = path.to_string_lossy().into_owned();
        assert!(config_save(&c, &ps));
        let mut c2 = Config::default();
        assert!(config_load(&mut c2, &ps));
        // jefetch style leaves file gradients alone on every load
        assert_eq!(c2.gradient_low, "red");
        assert_eq!(c2.gradient_high, "#00ff00");
        assert_eq!(c2.colors, "jefetch");
        assert_eq!(c2.gradient_amt, 42);
        assert_eq!(c2.method, "auto");
        assert_eq!(c2.chars, "x\"y\\z".as_bytes());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn jsonc_roundtrip_keeps_multibyte_chars_byte_identical() {
        // Regression: pushing each byte as a char re-encodes values >= 0x80
        // (mojibake). The file must hold the exact block-element bytes.
        let mut c = Config::default();
        c.chars = "▁▂▃▄▅▆▇█".as_bytes().to_vec();
        let path =
            std::env::temp_dir().join(format!("sharkvis-rtuni-{}.jsonc", std::process::id()));
        let ps = path.to_string_lossy().into_owned();
        assert!(config_save(&c, &ps));
        let raw = std::fs::read(&path).unwrap();
        assert!(
            raw.windows(3).any(|w| w == "▁".as_bytes()),
            "block bytes must survive the save"
        );
        assert!(
            !raw.contains(&0xC3),
            "no double-encoded bytes (0xC3 appears only via mojibake here)"
        );
        let mut c2 = Config::default();
        assert!(config_load(&mut c2, &ps));
        assert_eq!(c2.chars, "▁▂▃▄▅▆▇█".as_bytes());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn jsonc_rejects_garbage_and_keeps_defaults() {
        let mut c = Config::default();
        let path =
            std::env::temp_dir().join(format!("sharkvis-bad-{}.jsonc", std::process::id()));
        std::fs::write(&path, "{oops").unwrap();
        let ps = path.to_string_lossy().into_owned();
        assert!(!config_load(&mut c, &ps));
        assert_eq!(c.gradient_low, "ffffff");
        std::fs::write(&path, "[1,2]").unwrap();
        assert!(!config_load(&mut c, &ps));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn colors_style_rules() {
        let mut c = Config::default();
        c.colors = "".to_string();
        c.gradient_low = "aa".to_string();
        apply_colors_style(&mut c);
        assert_eq!(c.gradient_low, "aa");
        c.colors = "red".to_string();
        apply_colors_style(&mut c);
        assert_eq!((c.gradient_low.as_str(), c.gradient_high.as_str()), ("red", "red"));
        c.colors = "red, #00ff00".to_string();
        apply_colors_style(&mut c);
        assert_eq!(
            (c.gradient_low.as_str(), c.gradient_high.as_str()),
            ("red", "#00ff00")
        );
        c.colors = "nope, red".to_string();
        apply_colors_style(&mut c);
        assert_eq!(
            (c.gradient_low.as_str(), c.gradient_high.as_str()),
            ("red", "#00ff00"),
            "invalid pair keeps previous"
        );
    }
}
