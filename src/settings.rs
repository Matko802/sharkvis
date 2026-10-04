use std::sync::OnceLock;
use std::time::Instant;

use crate::config::{color_index, color_name, Config, PALETTE};
use crate::term::{KEY_CHAR, KEY_DOWN, KEY_ENTER, KEY_LEFT, KEY_RIGHT, KEY_UP};

pub const CH_LAYOUT: u32 = 1 << 0;
pub const CH_DSP: u32 = 1 << 1;
pub const CH_AUDIO: u32 = 1 << 2;
pub const CH_EDITOR: u32 = 1 << 3;

const S_MODE: usize = 0;
const S_BARS: usize = 1;
const S_BARW: usize = 2;
const S_SPACING: usize = 3;
const S_FPS: usize = 4;
const S_SENS: usize = 5;
const S_AUTO: usize = 6;
const S_NOISE: usize = 7;
const S_LOW: usize = 8;
const S_HIGH: usize = 9;
const S_GRAD: usize = 10;
const S_GHI: usize = 11;
const S_GLO: usize = 12;
const S_RATE: usize = 13;
const S_CH: usize = 14;
const S_CHARSET: usize = 15;
const S_TEXTSIZE: usize = 16;
const S_STYLE: usize = 17;
const S_PROVIDER: usize = 18;
const S_OFFSET: usize = 19;
const S_COLORS: usize = 20;
const S_ALIGN: usize = 21;
const S_WAVESM: usize = 22;
const S_COUNT: usize = 23;
const S_RESET: usize = S_COUNT;
const CONFIRM_TIMEOUT_MS: i64 = 5000;

const LABELS: [&str; S_COUNT] = [
    "type",
    "bars",
    "bar width",
    "bar spacing",
    "framerate",
    "sensitivity",
    "autosens",
    "smoothing",
    "lower cutoff",
    "upper cutoff",
    "gradient",
    "color high",
    "color low",
    "sample rate",
    "channels",
    "charset",
    "lyrics size",
    "lyrics style",
    "provider",
    "offset ms",
    "colors",
    "align",
    "wave smooth",
];

const RATES: [u32; 9] = [8000, 11025, 16000, 22050, 32000, 44100, 48000, 96000, 192000];
const MODES: [&str; 4] = ["bars", "wave", "oscilloscope", "lyrics"];

fn now_ms() -> i64 {
    static REF: OnceLock<Instant> = OnceLock::new();
    let r = REF.get_or_init(Instant::now);
    r.elapsed().as_millis() as i64
}

fn clamp_l(v: i64, lo: i64, hi: i64) -> i64 {
    v.max(lo).min(hi)
}

fn clamp_d(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

fn colors_style(cfg: &Config) -> u8 {
    let tok: String = cfg
        .colors
        .trim_start()
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ',')
        .collect();
    if tok.eq_ignore_ascii_case("jefetch") {
        1
    } else {
        0
    }
}

struct ColorsStash {
    custom: String,
    lo: String,
    hi: String,
}

static COLORS_STASH: std::sync::Mutex<ColorsStash> = std::sync::Mutex::new(ColorsStash {
    custom: String::new(),
    lo: String::new(),
    hi: String::new(),
});

pub struct SettingsUi {
    sel: usize,
    confirm_reset: bool,
    confirm_deadline_ms: i64,
}

impl Default for SettingsUi {
    fn default() -> Self {
        SettingsUi {
            sel: 0,
            confirm_reset: false,
            confirm_deadline_ms: 0,
        }
    }
}

impl SettingsUi {
    fn adjust(cfg: &mut Config, id: usize, dir: i64, changed: &mut u32) {
        match id {
            S_BARS => {
                let v = clamp_l(cfg.bars as i64 + dir, 0, 256);
                if v as usize != cfg.bars {
                    cfg.bars = v as usize;
                    *changed |= CH_LAYOUT;
                }
            }
            S_BARW => {
                let v = clamp_l(cfg.bar_width as i64 + dir, 1, 8);
                if v as usize != cfg.bar_width {
                    cfg.bar_width = v as usize;
                    *changed |= CH_LAYOUT;
                }
            }
            S_SPACING => {
                let v = clamp_l(cfg.bar_spacing as i64 + dir, 0, 4);
                if v as usize != cfg.bar_spacing {
                    cfg.bar_spacing = v as usize;
                    *changed |= CH_LAYOUT;
                }
            }
            S_FPS => {
                let v = clamp_l(cfg.framerate as i64 + dir * 5, 5, 240);
                if v as u32 != cfg.framerate {
                    cfg.framerate = v as u32;
                    *changed |= CH_LAYOUT;
                }
            }
            S_SENS => {
                let v = clamp_d(cfg.sensitivity + dir as f64 * 5.0, 5.0, 200.0);
                if v != cfg.sensitivity {
                    cfg.sensitivity = v;
                    *changed |= CH_LAYOUT;
                }
            }
            S_AUTO => {
                let v = !cfg.autosens;
                if v != cfg.autosens {
                    cfg.autosens = v;
                    *changed |= CH_DSP;
                }
            }
            S_NOISE => {
                let v = clamp_d(cfg.noise_reduction + dir as f64 * 0.05, 0.0, 1.0);
                if v != cfg.noise_reduction {
                    cfg.noise_reduction = v;
                    *changed |= CH_DSP;
                }
            }
            S_WAVESM => {
                let v = clamp_d(cfg.wave_smoothing + dir as f64 * 0.05, 0.0, 0.95);
                if v != cfg.wave_smoothing {
                    cfg.wave_smoothing = v;
                    *changed |= CH_LAYOUT;
                }
            }
            S_LOW => {
                let mut v = clamp_l(cfg.lower_cutoff as i64 + dir * 25, 25, 20000);
                if v >= cfg.higher_cutoff as i64 {
                    v = (cfg.higher_cutoff as i64 - 1) / 25 * 25;
                }
                if v as u32 != cfg.lower_cutoff {
                    cfg.lower_cutoff = v as u32;
                    *changed |= CH_DSP;
                }
            }
            S_HIGH => {
                let mut v = clamp_l(cfg.higher_cutoff as i64 + dir * 500, 500, 24000);
                if v <= cfg.lower_cutoff as i64 {
                    v = (cfg.lower_cutoff as i64 / 500 + 1) * 500;
                }
                if v as u32 != cfg.higher_cutoff {
                    cfg.higher_cutoff = v as u32;
                    *changed |= CH_DSP;
                }
            }
            S_GRAD => {
                let v = clamp_l(cfg.gradient_amt as i64 + dir, 1, 256);
                if v as u32 != cfg.gradient_amt {
                    cfg.gradient_amt = v as u32;
                    *changed |= CH_LAYOUT;
                }
            }
            S_GLO | S_GHI => {
                let cur = if id == S_GLO {
                    cfg.gradient_low.clone()
                } else {
                    cfg.gradient_high.clone()
                };
                let mut idx = color_index(&cur);
                if idx < 0 {
                    idx = 0;
                }
                idx = (idx + dir as i32 + PALETTE.len() as i32) % PALETTE.len() as i32;
                let new = PALETTE[idx as usize].1.to_string();
                if id == S_GLO {
                    cfg.gradient_low = new;
                } else {
                    cfg.gradient_high = new;
                }
                if colors_style(cfg) != 0 {
                    cfg.colors.clear();
                } else if !cfg.colors.is_empty() {
                    cfg.colors = format!("{},{}", cfg.gradient_low, cfg.gradient_high);
                }
                *changed |= CH_LAYOUT;
            }
            S_COLORS => {
                let cur = colors_style(cfg);
                let next = (cur as i64 + dir).rem_euclid(2) as u8;
                if next == 0 {
                    let stash = COLORS_STASH.lock().unwrap_or_else(|e| e.into_inner());
                    if !stash.custom.is_empty() {
                        cfg.colors = stash.custom.clone();
                    } else {
                        cfg.colors.clear();
                    }
                    if !stash.lo.is_empty() && !stash.hi.is_empty() {
                        cfg.gradient_low = stash.lo.clone();
                        cfg.gradient_high = stash.hi.clone();
                    }
                } else {
                    if cur == 0 {
                        let mut stash =
                            COLORS_STASH.lock().unwrap_or_else(|e| e.into_inner());
                        if !cfg.colors.is_empty() {
                            stash.custom = cfg.colors.clone();
                        }
                        stash.lo = cfg.gradient_low.clone();
                        stash.hi = cfg.gradient_high.clone();
                    }
                    cfg.colors = "jefetch".to_string();
                }
                *changed |= CH_LAYOUT;
            }
            S_MODE => {

                if cfg.mode == "text" {
                    cfg.mode = "lyrics".to_string();
                }
                let mut idx = 0;
                for i in 0..MODES.len() {
                    if cfg.mode == MODES[i] {
                        idx = i as i64;
                        break;
                    }
                }
                idx = (idx + dir + MODES.len() as i64) % MODES.len() as i64;
                if cfg.mode != MODES[idx as usize] {
                    cfg.mode = MODES[idx as usize].to_string();
                    *changed |= CH_LAYOUT;
                }
            }
            S_RATE => {
                let mut idx = 0;
                for i in 0..RATES.len() {
                    if RATES[i] <= cfg.sample_rate {
                        idx = i as i64;
                    }
                }
                idx = clamp_l(idx + dir, 0, RATES.len() as i64 - 1);
                if RATES[idx as usize] != cfg.sample_rate {
                    cfg.sample_rate = RATES[idx as usize];
                    *changed |= CH_AUDIO;
                }
            }
            S_CH => {
                let v = if cfg.channels == 1 { 2 } else { 1 };
                if v != cfg.channels {
                    cfg.channels = v;
                    *changed |= CH_AUDIO;
                }
            }
            S_PROVIDER => {
                const ORDER: [&str; 3] = ["auto", "lrclib", "musixmatch"];
                let mut idx = 0;
                for (i, name) in ORDER.iter().enumerate() {
                    if cfg.provider == *name {
                        idx = i as i64;
                        break;
                    }
                }
                idx = (idx + dir + ORDER.len() as i64) % ORDER.len() as i64;
                let v = ORDER[idx as usize].to_string();
                if v != cfg.provider {
                    cfg.provider = v;
                    *changed |= CH_LAYOUT;
                }
            }
            S_OFFSET => {
                let v = (cfg.lyric_offset_ms + dir * 500).clamp(-10000, 10000);
                if v != cfg.lyric_offset_ms {
                    cfg.lyric_offset_ms = v;
                    *changed |= CH_LAYOUT;
                }
            }
            S_TEXTSIZE => {
                let v = (cfg.text_size as i64 + dir).clamp(0, 5) as u32;
                if v != cfg.text_size {
                    cfg.text_size = v;
                    *changed |= CH_LAYOUT;
                }
            }
            S_STYLE => {
                cfg.text_style = if cfg.text_style == "normal" { "big ahh".to_string() } else { "normal".to_string() };
                *changed |= CH_LAYOUT;
            }
            S_ALIGN => {
                cfg.text_align = if cfg.text_align == "left" { "center".to_string() } else { "left".to_string() };
                *changed |= CH_LAYOUT;
            }
            _ => {}
        }
    }

    fn handle_reset(&mut self, cfg: &mut Config, changed: &mut u32) {
        if !self.confirm_reset {
            self.confirm_reset = true;
            self.confirm_deadline_ms = now_ms() + CONFIRM_TIMEOUT_MS;
            return;
        }
        self.confirm_reset = false;
        *cfg = Config::default();
        *changed |= CH_LAYOUT | CH_DSP | CH_AUDIO;
    }

    pub fn visible_rows(cfg: &Config) -> Vec<usize> {
        let mut rows = vec![S_MODE, S_GRAD, S_COLORS];
        if colors_style(cfg) != 1 {
            rows.push(S_GHI);
            rows.push(S_GLO);
        }
        rows.extend_from_slice(&[S_FPS, S_RATE, S_CH]);
        match cfg.mode.as_str() {
            "bars" => rows.extend_from_slice(&[
                S_BARS, S_BARW, S_SPACING, S_CHARSET, S_SENS, S_AUTO, S_NOISE, S_LOW, S_HIGH,
            ]),

            "lyrics" | "text" => {
                rows.extend_from_slice(&[S_TEXTSIZE, S_STYLE, S_ALIGN, S_PROVIDER, S_OFFSET])
            }
            "wave" => rows.extend_from_slice(&[S_WAVESM]),
            _ => {}
        }
        rows.sort_unstable();
        rows
    }

    fn nav_ids(cfg: &Config) -> Vec<usize> {
        let mut ids = Self::visible_rows(cfg);
        ids.push(S_RESET);
        ids
    }

    fn clamp_sel(&mut self, cfg: &Config) {
        let ids = Self::nav_ids(cfg);
        if !ids.contains(&self.sel) {
            self.sel = S_MODE;
        }
    }

    pub fn key(&mut self, cfg: &mut Config, key: i32, cp: Option<&[u8]>, changed: &mut u32) {
        match key {
            KEY_UP => {
                let ids = Self::nav_ids(cfg);
                let pos = ids.iter().position(|&id| id == self.sel).unwrap_or(0);
                self.sel = ids[(pos + ids.len() - 1) % ids.len()];
                self.confirm_reset = false;
            }
            KEY_DOWN => {
                let ids = Self::nav_ids(cfg);
                let pos = ids.iter().position(|&id| id == self.sel).unwrap_or(0);
                self.sel = ids[(pos + 1) % ids.len()];
                self.confirm_reset = false;
            }
            KEY_LEFT => {
                if self.sel == S_RESET {
                    self.handle_reset(cfg, changed);
                } else {
                    Self::adjust(cfg, self.sel, -1, changed);
                }
                self.clamp_sel(cfg);
            }
            KEY_RIGHT => {
                if self.sel == S_RESET {
                    self.handle_reset(cfg, changed);
                } else {
                    Self::adjust(cfg, self.sel, 1, changed);
                }
                self.clamp_sel(cfg);
            }
            KEY_ENTER => {
                if self.sel == S_CHARSET {
                    *changed |= CH_EDITOR;
                }
            }
            KEY_CHAR => {
                if let Some(cp) = cp {
                    if cp.first() == Some(&b'-') {
                        if self.sel != S_RESET {
                            Self::adjust(cfg, self.sel, -1, changed);
                        }
                    } else if cp.first() == Some(&b'+') || cp.first() == Some(&b'=') {
                        if self.sel != S_RESET {
                            Self::adjust(cfg, self.sel, 1, changed);
                        }
                    }
                    self.clamp_sel(cfg);
                }
            }
            _ => {}
        }
    }

    pub fn row_at_y(cfg: &Config, y: u32) -> Option<usize> {
        let mut yy = 6u32;
        for id in Self::visible_rows(cfg) {
            if yy == y {
                return Some(id);
            }
            yy += 1;
            if id == S_MODE {
                yy += 1;
            }
        }
        if yy == y {
            return Some(S_RESET);
        }
        None
    }

    pub fn click(&mut self, cfg: &mut Config, y: u32, x: u32, pw: usize, changed: &mut u32) {
        if x == 0 || x as usize > pw {
            return;
        }
        let Some(id) = Self::row_at_y(cfg, y) else {
            return;
        };
        if id == S_RESET {
            self.sel = S_RESET;
            self.handle_reset(cfg, changed);
            self.clamp_sel(cfg);
            return;
        }
        self.sel = id;
        self.confirm_reset = false;
        if x > pw as u32 / 2 {
            if id == S_CHARSET {
                *changed |= CH_EDITOR;
            } else {
                Self::adjust(cfg, id, 1, changed);
            }
        } else {
            Self::adjust(cfg, id, -1, changed);
        }
        self.clamp_sel(cfg);
    }

    pub fn draw(&mut self, cfg: &Config, out: &mut Vec<u8>, cap: usize, rows: u32, pw: usize) {
        if self.confirm_reset && now_ms() > self.confirm_deadline_ms {
            self.confirm_reset = false;
        }
        self.clamp_sel(cfg);
        panel_row(out, cap, 1, pw, "sharkvis settings", None, None);
        let mut y = 6;
        for id in Self::visible_rows(cfg) {
            let val = format_value(cfg, id);
            panel_row(
                out,
                cap,
                y,
                pw,
                LABELS[id],
                Some(&val),
                if id == self.sel { Some("\x1b[7m") } else { None },
            );
            y += 1;
            if id == S_MODE {
                append_esc(out, cap, format!("\x1b[0m\x1b[{};1H{}", y, "─".repeat(pw)).as_bytes());
                y += 1;
            }
        }
        if self.confirm_reset {
            panel_row(out, cap, y, pw, "Are you sure?", Some("press → again"), Some("\x1b[41m\x1b[97m"));
        } else if self.sel == S_CHARSET {
            panel_row(out, cap, y, pw, "edit bar symbols", Some("enter = nano"), None);
        } else {
            panel_row(
                out,
                cap,
                y,
                pw,
                "reset to defaults",
                Some("press →"),
                if self.sel == S_RESET { Some("\x1b[7m") } else { None },
            );
        }
        for yy in 1..=rows {
            append_esc(out, cap, format!("\x1b[0m\x1b[{};{}H│", yy, pw).as_bytes());
        }
    }
}

fn format_value(cfg: &Config, id: usize) -> String {
    match id {
        S_BARS => {
            if cfg.bars == 0 {
                "auto".to_string()
            } else {
                format!("{}", cfg.bars)
            }
        }
        S_AUTO => {
            if cfg.autosens {
                "on".to_string()
            } else {
                "off".to_string()
            }
        }
        S_GRAD => format!("{}", cfg.gradient_amt),
        S_COLORS => match colors_style(cfg) {
            1 => "jefetch".to_string(),
            _ => "custom".to_string(),
        },
        S_GLO | S_GHI => {
            let hx = if id == S_GLO {
                cfg.gradient_low.as_str()
            } else {
                cfg.gradient_high.as_str()
            };
            match color_name(hx) {
                Some(nm) => nm.to_string(),
                None => hx.to_string(),
            }
        }
        S_MODE => cfg.mode.clone(),
        S_NOISE => format!("{:.2}", cfg.noise_reduction),
        S_WAVESM => format!("{:.2}", cfg.wave_smoothing),
        S_SENS => format!("{:.0}", cfg.sensitivity),
        S_BARW => format!("{}", cfg.bar_width),
        S_SPACING => format!("{}", cfg.bar_spacing),
        S_FPS => format!("{}", cfg.framerate),
        S_LOW => format!("{}", cfg.lower_cutoff),
        S_HIGH => format!("{}", cfg.higher_cutoff),
        S_RATE => format!("{}", cfg.sample_rate),
        S_CH => format!("{}", cfg.channels),
        S_CHARSET => String::from_utf8_lossy(&cfg.chars).into_owned(),
        S_TEXTSIZE => {
            if cfg.text_size == 0 {
                "auto".to_string()
            } else {
                format!("{}", cfg.text_size)
            }
        }
        S_PROVIDER => cfg.provider.clone(),
        S_OFFSET => format!("{:+}ms", cfg.lyric_offset_ms),
        S_STYLE => cfg.text_style.clone(),
        S_ALIGN => cfg.text_align.clone(),
        _ => String::new(),
    }
}

fn append_esc(out: &mut Vec<u8>, cap: usize, bytes: &[u8]) {
    if out.len() >= cap {
        return;
    }
    let room = cap - out.len();
    let take = bytes.len().min(room);
    out.extend_from_slice(&bytes[..take]);
}

fn panel_row(
    out: &mut Vec<u8>,
    cap: usize,
    y: u32,
    pw: usize,
    label: &str,
    val: Option<&str>,
    style: Option<&str>,
) {
    let mut text: Vec<u8>;
    if let Some(v) = val {
        let mut lw = pw as i64 - 13;
        if lw < 4 {
            lw = 4;
        }
        if lw > 16 {
            lw = 16;
        }
        text = format!("  {:<lw$} {:<10}", label, v, lw = lw as usize).into_bytes();
    while text.len() > 256 {
        text.pop();
    }
    } else {
        text = format!("  {}", label).into_bytes();
    }
    let len = text.len();

    let mut emit = 0usize;
    let mut vis = 0usize;
    let mut p = 0usize;
    while p < len && vis < pw {
        let c = text[p];
        let seq = if c < 0x80 {
            1
        } else if (c & 0xE0) == 0xC0 {
            2
        } else if (c & 0xF0) == 0xE0 {
            3
        } else {
            4
        };
        if vis + 1 > pw {
            break;
        }
        vis += 1;
        emit += seq;
        p += seq;
    }

    let header = format!("\x1b[0m\x1b[{};1H{}", y, style.unwrap_or(""));
    append_esc(out, cap, header.as_bytes());
    append_esc(out, cap, &text[..emit]);
    for _ in vis..pw {
        append_esc(out, cap, b" ");
    }
    append_esc(out, cap, b"\x1b[0m");
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn visible_rows_per_mode() {
        fn rows_for(mode: &str) -> Vec<usize> {
            let mut cfg = Config::default();
            cfg.mode = mode.to_string();
            SettingsUi::visible_rows(&cfg)
        }
        let bars = rows_for("bars");
        assert!(bars.contains(&S_BARS) && bars.contains(&S_CHARSET) && bars.contains(&S_SENS));
        assert!(!bars.contains(&S_TEXTSIZE));
        assert!(bars.contains(&S_GRAD) && bars.contains(&S_COLORS));
        let wave = rows_for("wave");
        assert!(!wave.contains(&S_BARS) && !wave.contains(&S_TEXTSIZE) && !wave.contains(&S_SENS));
        assert!(wave.contains(&S_MODE) && wave.contains(&S_FPS) && wave.contains(&S_RATE));
        assert!(wave.contains(&S_WAVESM) && !wave.contains(&S_ALIGN));
        let scope = rows_for("oscilloscope");
        assert!(!scope.contains(&S_BARS) && !scope.contains(&S_TEXTSIZE));
        let lyr = rows_for("lyrics");
        assert!(lyr.contains(&S_TEXTSIZE) && lyr.contains(&S_PROVIDER) && lyr.contains(&S_OFFSET));
        assert!(lyr.contains(&S_ALIGN) && !lyr.contains(&S_CHARSET));
        assert!(!lyr.contains(&S_SENS) && !lyr.contains(&S_BARS) && !lyr.contains(&S_CHARSET));

        assert_eq!(rows_for("text"), lyr);
        let unknown = rows_for("ai");
        assert!(!unknown.contains(&S_BARS) && !unknown.contains(&S_TEXTSIZE));
        assert!(unknown.contains(&S_MODE) && unknown.contains(&S_FPS));
        for m in ["bars", "wave", "oscilloscope", "lyrics", "text", "bogus"] {
            let mut v = rows_for(m);
            let mut s = v.clone();
            s.sort_unstable();
            s.dedup();
            assert_eq!(v.len(), s.len(), "no dupes for {}", m);
            v = s;
        }
    }

    #[test]
    fn jefetch_colors_hide_custom_gradient_rows() {
        let mut cfg = Config::default();
        cfg.mode = "bars".to_string();
        assert!(SettingsUi::visible_rows(&cfg).contains(&S_GLO));
        cfg.colors = "jefetch".to_string();
        let rows = SettingsUi::visible_rows(&cfg);
        assert!(!rows.contains(&S_GLO) && !rows.contains(&S_GHI));
        assert!(rows.contains(&S_COLORS) && rows.contains(&S_GRAD));
    }

    #[test]
    fn click_selects_and_adjusts() {
        let mut cfg = Config::default();
        cfg.mode = "bars".to_string();
        let ids = SettingsUi::visible_rows(&cfg);
        assert_eq!(SettingsUi::row_at_y(&cfg, 6), Some(ids[0]));
        assert_eq!(SettingsUi::row_at_y(&cfg, 1), None);
        let mut ui = SettingsUi::default();
        let mut changed = 0;
        let y = (6..40)
            .find(|&yy| SettingsUi::row_at_y(&cfg, yy) == Some(S_FPS))
            .unwrap();
        let before = cfg.framerate;
        ui.click(&mut cfg, y, 4, 40, &mut changed);
        assert_eq!(ui.sel, S_FPS);
        assert_eq!(cfg.framerate, before - 5);
        assert_eq!(changed & CH_LAYOUT, CH_LAYOUT);
        ui.click(&mut cfg, y, 39, 40, &mut changed);
        assert_eq!(cfg.framerate, before);
        ui.click(&mut cfg, y, 99, 40, &mut changed);
        assert_eq!(ui.sel, S_FPS);
    }

    #[test]
    fn nav_never_lands_offscreen() {
        let mut cfg = Config::default();
        cfg.mode = "wave".to_string();
        let mut ui = SettingsUi::default();
        ui.sel = S_BARS;
        ui.clamp_sel(&cfg);
        assert_eq!(ui.sel, S_MODE);
        let ids = SettingsUi::nav_ids(&cfg);
        assert!(ids.contains(&S_RESET));
        assert!(!ids.contains(&S_BARS));
        ui.sel = S_MODE;
        ui.key(&mut cfg, KEY_DOWN, None, &mut 0);
        assert!(SettingsUi::nav_ids(&cfg).contains(&ui.sel));
    }

    #[test]
    fn provider_cycle_respects_direction() {
        let mut cfg = Config::default();
        let mut changed = 0;
        cfg.provider = "auto".to_string();
        SettingsUi::adjust(&mut cfg, S_PROVIDER, 1, &mut changed);
        assert_eq!(cfg.provider, "lrclib");
        SettingsUi::adjust(&mut cfg, S_PROVIDER, 1, &mut changed);
        assert_eq!(cfg.provider, "musixmatch");
        SettingsUi::adjust(&mut cfg, S_PROVIDER, -1, &mut changed);
        assert_eq!(cfg.provider, "lrclib");
        SettingsUi::adjust(&mut cfg, S_PROVIDER, -1, &mut changed);
        assert_eq!(cfg.provider, "auto");
        SettingsUi::adjust(&mut cfg, S_PROVIDER, -1, &mut changed);
        assert_eq!(cfg.provider, "musixmatch");
        SettingsUi::adjust(&mut cfg, S_PROVIDER, 1, &mut changed);
        assert_eq!(cfg.provider, "auto");
    }
}
