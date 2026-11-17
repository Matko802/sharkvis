use std::sync::OnceLock;
use std::time::Instant;

use crate::config::{color_index, color_name, colors_mode, Config, DEFAULT_CHARS, PALETTE};
use crate::render::Renderer;
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
const BAND_H: usize = 8;
const BIG_HEADER_H: usize = 8;

const LABELS: [&str; S_COUNT] = [
    "type",
    "bars",
    "bar width",
    "bar gap",
    "framerate",
    "sensitivity",
    "autosens",
    "smoothing",
    "low cutoff",
    "high cutoff",
    "gradient",
    "color high",
    "color low",
    "sample rate",
    "channels",
    "charset",
    "text size",
    "text style",
    "provider",
    "offset",
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
    offset: usize,
}

impl Default for SettingsUi {
    fn default() -> Self {
        SettingsUi {
            sel: 0,
            confirm_reset: false,
            confirm_deadline_ms: 0,
            offset: 0,
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
                if colors_mode(&cfg.colors) == 1 {
                    return;
                }
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
                if colors_mode(&cfg.colors) != 0 {
                    cfg.colors.clear();
                } else if !cfg.colors.is_empty() {
                    cfg.colors = format!("{},{}", cfg.gradient_low, cfg.gradient_high);
                }
                *changed |= CH_LAYOUT;
            }
            S_COLORS => {
                let cur = colors_mode(&cfg.colors);
                let next = (cur as i64 + dir).rem_euclid(3) as u8;
                if cur == 0 {
                    let mut stash = COLORS_STASH.lock().unwrap_or_else(|e| e.into_inner());
                    if !cfg.colors.is_empty() {
                        stash.custom = cfg.colors.clone();
                    }
                    stash.lo = cfg.gradient_low.clone();
                    stash.hi = cfg.gradient_high.clone();
                }
                match next {
                    0 => {
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
                    }
                    1 => cfg.colors = "jefetch".to_string(),
                    _ => cfg.colors = "mpris".to_string(),
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
        let mut rows = vec![S_MODE, S_COLORS];
        if colors_mode(&cfg.colors) != 1 {
            rows.push(S_GRAD);
        }
        if colors_mode(&cfg.colors) == 0 {
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

    fn header_rows(rows: u32) -> usize {
        if rows as usize >= 32 {
            BIG_HEADER_H
        } else {
            1
        }
    }

    fn visible_count(rows: u32) -> usize {
        let hdr = Self::header_rows(rows);
        let bands = (rows as usize).saturating_sub(hdr + 1) / BAND_H;
        bands.max(1)
    }

    fn scroll_to_sel(&mut self, cfg: &Config, rows: u32) {
        let ids = Self::nav_ids(cfg);
        let vis = Self::visible_count(rows);
        let max_off = ids.len().saturating_sub(vis);
        if self.offset > max_off {
            self.offset = max_off;
        }
        let pos = ids.iter().position(|&id| id == self.sel).unwrap_or(0);
        if pos < self.offset {
            self.offset = pos;
        } else if pos >= self.offset + vis {
            self.offset = pos + 1 - vis;
        }
    }

    fn window_ids(&self, cfg: &Config, rows: u32) -> Vec<usize> {
        let ids = Self::nav_ids(cfg);
        let vis = Self::visible_count(rows);
        let off = self.offset.min(ids.len().saturating_sub(vis));
        ids.into_iter().skip(off).take(vis).collect()
    }

    fn band_at(&self, cfg: &Config, y: u32, rows: u32) -> Option<usize> {
        let hdr = Self::header_rows(rows);
        let y = y as usize;
        if y < hdr {
            return None;
        }
        let idx = (y - hdr) / BAND_H;
        if idx >= Self::visible_count(rows) {
            return None;
        }
        self.window_ids(cfg, rows).get(idx).copied()
    }

    pub fn click(
        &mut self,
        cfg: &mut Config,
        y: u32,
        x: u32,
        rows: u32,
        cols: u32,
        changed: &mut u32,
    ) {
        if y >= rows || x >= cols {
            return;
        }
        let Some(id) = self.band_at(cfg, y, rows) else {
            return;
        };
        self.sel = id;
        self.confirm_reset = false;
        if id == S_RESET {
            self.handle_reset(cfg, changed);
            self.clamp_sel(cfg);
            self.scroll_to_sel(cfg, rows);
            return;
        }
        if x >= cols / 2 {
            if id == S_CHARSET {
                *changed |= CH_EDITOR;
            } else {
                Self::adjust(cfg, id, 1, changed);
            }
        } else {
            Self::adjust(cfg, id, -1, changed);
        }
        self.clamp_sel(cfg);
        self.scroll_to_sel(cfg, rows);
    }

    pub fn draw(
        &mut self,
        cfg: &Config,
        rnd: &Renderer,
        out: &mut Vec<u8>,
        cap: usize,
        rows: u32,
        cols: u32,
    ) {
        if self.confirm_reset && now_ms() > self.confirm_deadline_ms {
            self.confirm_reset = false;
        }
        self.clamp_sel(cfg);
        self.scroll_to_sel(cfg, rows);
        let rows = rows as usize;
        let cols = cols as usize;
        if rows == 0 || cols == 0 {
            return;
        }
        append_esc(out, cap, b"\x1b[0m\x1b[2J");
        let hdr = Self::header_rows(rows as u32);
        if hdr >= BIG_HEADER_H {
            rnd.draw_big_line(out, cap, 0, 0, cols, "sharkvis settings", false);
        } else {
            let esc = rnd.letter_color(0.0, 1.0);
            append_esc(out, cap, b"\x1b[1;1H");
            append_esc(out, cap, &esc);
            append_esc(out, cap, b"sharkvis settings");
            append_esc(out, cap, b"\x1b[0m");
        }
        let ids = self.window_ids(cfg, rows as u32);
        let marker = rnd.letter_color(0.0, 1.0);
        for (i, &id) in ids.iter().enumerate() {
            let y = hdr + i * BAND_H;
            if y + 7 > rows {
                break;
            }
            let selected = id == self.sel;
            let line = if id == S_RESET {
                if self.confirm_reset {
                    "reset? press again".to_string()
                } else {
                    "reset defaults".to_string()
                }
            } else {
                format!("{}: {}", LABELS[id], format_value(cfg, id))
            };
            rnd.draw_big_line(
                out,
                cap,
                y,
                3,
                cols.saturating_sub(6),
                &line,
                !selected,
            );
            if selected {
                for row in 0..7 {
                    append_esc(out, cap, format!("\x1b[{};2H", y + row + 1).as_bytes());
                    append_esc(out, cap, &marker);
                    append_esc(out, cap, "\u{2588}\u{2588}".as_bytes());
                }
                append_esc(out, cap, b"\x1b[0m");
            }
        }
        let ids = Self::nav_ids(cfg);
        let total = ids.len();
        let pos = ids.iter().position(|&id| id == self.sel).unwrap_or(0) + 1;
        let info = format!("{}/{} ", pos, total);
        let hint = " \u{2191}\u{2193} select   \u{2190}\u{2192} change   enter edit   g close";
        let left: String = hint.chars().take(cols.saturating_sub(info.chars().count())).collect();
        let pad = cols.saturating_sub(left.chars().count() + info.chars().count());
        let esc = rnd.letter_color(0.5, 0.7);
        append_esc(out, cap, format!("\x1b[{};1H", rows).as_bytes());
        append_esc(out, cap, &esc);
        append_esc(out, cap, left.as_bytes());
        append_esc(out, cap, " ".repeat(pad).as_bytes());
        append_esc(out, cap, info.as_bytes());
        append_esc(out, cap, b"\x1b[0m");
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
        S_COLORS => match colors_mode(&cfg.colors) {
            1 => "jefetch".to_string(),
            2 => "mpris".to_string(),
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
        S_CHARSET => {
            if cfg.chars == DEFAULT_CHARS {
                "default".to_string()
            } else {
                "custom".to_string()
            }
        }
        S_TEXTSIZE => {
            if cfg.text_size == 0 {
                "auto".to_string()
            } else {
                format!("{}", cfg.text_size)
            }
        }
        S_PROVIDER => cfg.provider.clone(),
        S_OFFSET => format!("{:+}ms", cfg.lyric_offset_ms),
        S_STYLE => match cfg.text_style.as_str() {
            "big ahh" => "big".to_string(),
            "normal" => "normal".to_string(),
            other => other.to_string(),
        },
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
    fn override_colors_hide_custom_gradient_rows() {
        let mut cfg = Config::default();
        cfg.mode = "bars".to_string();
        assert!(SettingsUi::visible_rows(&cfg).contains(&S_GLO));
        for mode in ["jefetch", "mpris"] {
            cfg.colors = mode.to_string();
            let rows = SettingsUi::visible_rows(&cfg);
            assert!(!rows.contains(&S_GLO) && !rows.contains(&S_GHI), "{}", mode);
            assert!(rows.contains(&S_COLORS), "{}", mode);
            assert_eq!(
                rows.contains(&S_GRAD),
                mode == "mpris",
                "jefetch keeps two fixed colors; {} gradient row",
                mode
            );
        }
    }

    #[test]
    fn colors_cycle_custom_jefetch_mpris() {
        let mut cfg = Config::default();
        cfg.gradient_low = "112233".to_string();
        cfg.gradient_high = "445566".to_string();
        let mut changed = 0;
        SettingsUi::adjust(&mut cfg, S_COLORS, 1, &mut changed);
        assert_eq!(colors_mode(&cfg.colors), 1);
        SettingsUi::adjust(&mut cfg, S_COLORS, 1, &mut changed);
        assert_eq!(colors_mode(&cfg.colors), 2);
        SettingsUi::adjust(&mut cfg, S_COLORS, 1, &mut changed);
        assert_eq!(colors_mode(&cfg.colors), 0);
        assert_eq!(cfg.gradient_low, "112233");
        assert_eq!(cfg.gradient_high, "445566");
        assert_ne!(changed & CH_LAYOUT, 0);
    }

    #[test]
    fn click_selects_and_adjusts() {
        let mut cfg = Config::default();
        cfg.mode = "bars".to_string();
        let ids = SettingsUi::visible_rows(&cfg);
        let mut ui = SettingsUi::default();
        assert_eq!(ui.band_at(&cfg, 8, 65), Some(ids[0]));
        assert_eq!(ui.band_at(&cfg, 1, 65), None);
        assert_eq!(ui.band_at(&cfg, 65, 65), None);
        let mut changed = 0;
        let y = (0..65)
            .find(|&yy| ui.band_at(&cfg, yy, 65) == Some(S_FPS))
            .unwrap();
        let before = cfg.framerate;
        ui.click(&mut cfg, y, 10, 65, 126, &mut changed);
        assert_eq!(ui.sel, S_FPS);
        assert_eq!(cfg.framerate, before - 5);
        assert_eq!(changed & CH_LAYOUT, CH_LAYOUT);
        ui.click(&mut cfg, y, 110, 65, 126, &mut changed);
        assert_eq!(cfg.framerate, before);
        ui.click(&mut cfg, y, 200, 65, 126, &mut changed);
        assert_eq!(ui.sel, S_FPS);
    }

    #[test]
    fn scroll_window_follows_the_selection() {
        let mut cfg = Config::default();
        cfg.mode = "bars".to_string();
        let mut ui = SettingsUi::default();
        ui.sel = S_RESET;
        ui.scroll_to_sel(&cfg, 65);
        assert!(ui.offset > 0);
        let ids = ui.window_ids(&cfg, 65);
        assert_eq!(ids.len(), 7);
        assert!(ids.contains(&S_RESET));
        assert_eq!(ui.band_at(&cfg, 8 + 6 * 8, 65), ids.last().copied());
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
