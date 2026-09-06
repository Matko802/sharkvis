use std::fs;
use std::io::{BufRead, BufReader, Write};
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

pub const DEFAULT_GLYPHS: &[u8] = "\u{2581}\u{2582}\u{2583}\u{2584}\u{2585}\u{2586}\u{2587}\u{2588}".as_bytes();

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
    pub source: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub color_256: bool,
    pub gradient_low: String,
    pub gradient_high: String,
    pub mode: String,
    pub sptlrx_text: String,
    pub text_source: String,
    pub ai_model: String,
    pub speech: bool,
    pub ollama_host: String,
    pub glyphs: Vec<u8>,
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
            source: "auto".to_string(),
            sample_rate: 48000,
            channels: 2,
            color_256: false,
            gradient_low: "ffffff".to_string(),
            gradient_high: "ffffff".to_string(),
            mode: "bars".to_string(),
            sptlrx_text: "SHARKVIS".to_string(),
            text_source: "static".to_string(),
            ai_model: "deepseek-r1:14b".to_string(),
            speech: true,
            ollama_host: "http://localhost:11434".to_string(),
            glyphs: DEFAULT_GLYPHS.to_vec(),
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
        let p = format!("{}/.config/sharkvis/config", home);
        if Path::new(&p).exists() {
            return p;
        }
        if Path::new("config").exists() {
            return "config".to_string();
        }
        return p;
    }
    if Path::new("config").exists() {
        return "config".to_string();
    }
    "config".to_string()
}

fn geti(v: &[u8], def: i64) -> i64 {
    let s = String::from_utf8_lossy(v).trim().to_string();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut sign: i64 = 1;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        if bytes[i] == b'-' {
            sign = -1;
        }
        i += 1;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return def;
    }
    match s[start..i].parse::<i64>() {
        Ok(v) => sign * v,
        Err(_) => def,
    }
}

fn getf(v: &[u8], def: f64) -> f64 {
    let s = String::from_utf8_lossy(v).trim().to_string();
    s.parse::<f64>().unwrap_or(def)
}

fn trim_ascii(s: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && (s[start] as char).is_whitespace() {
        start += 1;
    }
    while end > start && (s[end - 1] as char).is_whitespace() {
        end -= 1;
    }
    &s[start..end]
}

pub fn config_load(cfg: &mut Config, path: &str) -> bool {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let reader = BufReader::new(file);

    let mut section: Vec<u8> = "general".bytes().collect();
    for line in reader.split(b'\n') {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let mut s = trim_ascii(&line).to_vec();
        if s.is_empty() || s[0] == b';' || s[0] == b'#' {
            continue;
        }
        if s[0] == b'[' {
            if let Some(end) = s.iter().position(|&c| c == b']') {
                s.truncate(end);
            }
            section = trim_ascii(&s[1..]).to_ascii_lowercase();
            continue;
        }
        let eq = match s.iter().position(|&c| c == b'=') {
            Some(e) => e,
            None => continue,
        };
        let key = trim_ascii(&s[..eq]).to_ascii_lowercase();
        let mut val = trim_ascii(&s[eq + 1..]).to_vec();
        if key.as_slice() != b"glyphs" {
            if let Some(semi) = val.iter().position(|&c| c == b';') {
                val.truncate(semi);
            }
        }
        let val = trim_ascii(&val).to_vec();

        match section.as_slice() {
            b"general" => match key.as_slice() {
                b"bars" => cfg.bars = geti(&val, cfg.bars as i64) as usize,
                b"bar_width" => cfg.bar_width = geti(&val, cfg.bar_width as i64) as usize,
                b"bar_spacing" => cfg.bar_spacing = geti(&val, cfg.bar_spacing as i64) as usize,
                b"framerate" => cfg.framerate = geti(&val, cfg.framerate as i64) as u32,
                b"sensitivity" => cfg.sensitivity = getf(&val, cfg.sensitivity),
                b"autosens" => cfg.autosens = geti(&val, 1) != 0,
                b"lower_cutoff_freq" => {
                    cfg.lower_cutoff = geti(&val, cfg.lower_cutoff as i64) as u32
                }
                b"higher_cutoff_freq" => {
                    cfg.higher_cutoff = geti(&val, cfg.higher_cutoff as i64) as u32
                }
                _ => {}
            },
            b"smoothing" => match key.as_slice() {
                b"noise_reduction" => cfg.noise_reduction = getf(&val, cfg.noise_reduction),
                _ => {}
            },
            b"input" => match key.as_slice() {
                b"method" => {
                    let v = String::from_utf8_lossy(&val);
                    if !v.is_empty() && v != "pulse" && v != "pipewire" && v != "auto" {
                        eprintln!("sharkvis: input method '{}' not supported, using pulse", v);
                    }
                }
                b"source" => cfg.source = String::from_utf8_lossy(&val).into_owned(),
                b"sample_rate" => cfg.sample_rate = geti(&val, cfg.sample_rate as i64) as u32,
                b"channels" => cfg.channels = geti(&val, cfg.channels as i64) as u32,
                _ => {}
            },
            b"color" => match key.as_slice() {
                b"color_mode" => {
                    let v = val.as_slice();
                    if v == b"256" || v == b"indexed" {
                        cfg.color_256 = true;
                    } else if v == b"24bit" || v == b"truecolor" {
                        cfg.color_256 = false;
                    } else {
                        cfg.color_256 = geti(v, 0) != 0;
                    }
                }
                b"gradient_low" => {
                    if parse_hex_rgb(&val).is_some() {
                        cfg.gradient_low = String::from_utf8_lossy(&val).into_owned();
                    }
                }
                b"gradient_high" => {
                    if parse_hex_rgb(&val).is_some() {
                        cfg.gradient_high = String::from_utf8_lossy(&val).into_owned();
                    }
                }
                _ => {}
            },
            b"visualizer" => match key.as_slice() {
                b"mode" => {
                    let v = val.as_slice();
                    if v == b"bars" || v == b"wave" || v == b"oscilloscope" || v == b"lissajous" || v == b"text" || v == b"ai" {
                        cfg.mode = String::from_utf8_lossy(v).into_owned();
                    }
                }
                b"text" => {
                    let v = String::from_utf8_lossy(&val).into_owned();
                    if !v.trim().is_empty() {
                        cfg.sptlrx_text = v;
                    }
                }
                b"text_source" => {
                    let v = String::from_utf8_lossy(&val).into_owned();
                    if v == "lyrics" {
                        cfg.text_source = v;
                    }
                }
                b"ai_model" => {
                    let v = String::from_utf8_lossy(&val).into_owned();
                    if !v.trim().is_empty() {
                        cfg.ai_model = v;
                    }
                }
                b"speech" => {
                    cfg.speech = geti(&val, 1) != 0;
                }
                b"ollama_host" => {
                    let v = String::from_utf8_lossy(&val).into_owned();
                    if !v.trim().is_empty() {
                        cfg.ollama_host = v;
                    }
                }
                b"glyphs" => cfg.glyphs = val,
                _ => {}
            },
            _ => {}
        }
    }
    true
}

fn mkdir_p(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
}

pub fn config_save(cfg: &Config, path: &str) -> bool {
    mkdir_p(Path::new(path));
    let mut f = match fs::File::create(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut out = String::new();
    out.push_str("[general]\n");
    out.push_str(&format!("bars = {}\n", cfg.bars));
    out.push_str(&format!("bar_width = {}\n", cfg.bar_width));
    out.push_str(&format!("bar_spacing = {}\n", cfg.bar_spacing));
    out.push_str(&format!("framerate = {}\n", cfg.framerate));
    out.push_str(&format!("sensitivity = {:.0}\n", cfg.sensitivity));
    out.push_str(&format!("autosens = {}\n", if cfg.autosens { 1 } else { 0 }));
    out.push_str(&format!("lower_cutoff_freq = {}\n", cfg.lower_cutoff));
    out.push_str(&format!("higher_cutoff_freq = {}\n", cfg.higher_cutoff));
    out.push_str("\n[smoothing]\n");
    out.push_str(&format!("noise_reduction = {:.2}\n", cfg.noise_reduction));
    out.push_str("\n[input]\n");
    out.push_str("method = pulse\n");
    out.push_str(&format!("source = {}\n", cfg.source));
    out.push_str(&format!("sample_rate = {}\n", cfg.sample_rate));
    out.push_str(&format!("channels = {}\n", cfg.channels));
    out.push_str("\n[color]\n");
    out.push_str(&format!(
        "color_mode = {}\n",
        if cfg.color_256 { "256" } else { "24bit" }
    ));
    out.push_str(&format!("gradient_low = {}\n", cfg.gradient_low));
    out.push_str(&format!("gradient_high = {}\n", cfg.gradient_high));
    out.push_str("\n[visualizer]\n");
    out.push_str(&format!("mode = {}\n", cfg.mode));
    out.push_str(&format!("text = {}\n", cfg.sptlrx_text));
    out.push_str(&format!("text_source = {}\n", cfg.text_source));
    out.push_str(&format!("ai_model = {}\n", cfg.ai_model));
    out.push_str(&format!("speech = {}\n", if cfg.speech { 1 } else { 0 }));
    out.push_str(&format!("ollama_host = {}\n", cfg.ollama_host));
    out.push_str("glyphs = ");
    let _ = f.write_all(out.as_bytes());
    let _ = f.write_all(&cfg.glyphs);
    let _ = f.write_all(b"\n");
    out.clear();
    drop(f);
    true
}