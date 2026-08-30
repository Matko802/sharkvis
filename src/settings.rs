use std::sync::OnceLock;
use std::time::Instant;

use crate::config::{color_index, color_name, Config, PALETTE};
use crate::term::{KEY_CHAR, KEY_DOWN, KEY_ENTER, KEY_LEFT, KEY_RIGHT, KEY_UP};

pub const CH_LAYOUT: u32 = 1 << 0;
pub const CH_DSP: u32 = 1 << 1;
pub const CH_AUDIO: u32 = 1 << 2;
pub const CH_EDITOR: u32 = 1 << 3;

const S_BARS: usize = 0;
const S_BARW: usize = 1;
const S_SPACING: usize = 2;
const S_FPS: usize = 3;
const S_SENS: usize = 4;
const S_AUTO: usize = 5;
const S_NOISE: usize = 6;
const S_LOW: usize = 7;
const S_HIGH: usize = 8;
const S_CMODE: usize = 9;
const S_GLO: usize = 10;
const S_GHI: usize = 11;
const S_MODE: usize = 12;
const S_RATE: usize = 13;
const S_CH: usize = 14;
const S_CHARSET: usize = 15;
const S_COUNT: usize = 16;
const S_RESET: usize = S_COUNT;
const S_ROWS: usize = S_COUNT + 1;
const CONFIRM_TIMEOUT_MS: i64 = 5000;

const LABELS: [&str; S_COUNT] = [
    "bars",
    "bar width",
    "bar spacing",
    "framerate",
    "sensitivity",
    "autosens",
    "smoothing",
    "lower cutoff",
    "upper cutoff",
    "color mode",
    "color low",
    "color high",
    "visualizer",
    "sample rate",
    "channels",
    "charset",
];

const RATES: [u32; 9] = [8000, 11025, 16000, 22050, 32000, 44100, 48000, 96000, 192000];
const MODES: [&str; 3] = ["bars", "wave", "lissajous"];

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
                }
            }
            S_SENS => {
                let v = clamp_d(cfg.sensitivity + dir as f64 * 5.0, 5.0, 200.0);
                if v != cfg.sensitivity {
                    cfg.sensitivity = v;
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
            S_CMODE => {
                let v = !cfg.color_256;
                if v != cfg.color_256 {
                    cfg.color_256 = v;
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
                *changed |= CH_LAYOUT;
            }
            S_MODE => {
                let mut idx = 0;
                for i in 0..MODES.len() {
                    if cfg.mode == MODES[i] {
                        idx = i as i64;
                        break;
                    }
                }
                idx = (idx + dir + 3) % 3;
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

    pub fn key(&mut self, cfg: &mut Config, key: i32, cp: Option<&[u8]>, changed: &mut u32) {
        match key {
            KEY_UP => {
                self.sel = (self.sel + S_ROWS - 1) % S_ROWS;
                self.confirm_reset = false;
            }
            KEY_DOWN => {
                self.sel = (self.sel + 1) % S_ROWS;
                self.confirm_reset = false;
            }
            KEY_LEFT => {
                if self.sel == S_RESET {
                    self.handle_reset(cfg, changed);
                } else {
                    Self::adjust(cfg, self.sel, -1, changed);
                }
            }
            KEY_RIGHT => {
                if self.sel == S_RESET {
                    self.handle_reset(cfg, changed);
                } else {
                    Self::adjust(cfg, self.sel, 1, changed);
                }
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
                }
            }
            _ => {}
        }
    }

    pub fn draw(&mut self, cfg: &Config, out: &mut Vec<u8>, cap: usize, _rows: u32, pw: usize) {
        if self.confirm_reset && now_ms() > self.confirm_deadline_ms {
            self.confirm_reset = false;
        }
        panel_row(out, cap, 1, pw, "sharkvis settings", None, None);
        panel_row(out, cap, 2, pw, "←, ↑, ↓, → = adjust", None, None);
        panel_row(out, cap, 3, pw, "g = close, q = quit", None, None);
        let mut y = 6;
        for id in 0..S_COUNT {
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
        S_CMODE => {
            if cfg.color_256 {
                "256".to_string()
            } else {
                "24bit".to_string()
            }
        }
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
        S_SENS => format!("{:.0}", cfg.sensitivity),
        S_BARW => format!("{}", cfg.bar_width),
        S_SPACING => format!("{}", cfg.bar_spacing),
        S_FPS => format!("{}", cfg.framerate),
        S_LOW => format!("{}", cfg.lower_cutoff),
        S_HIGH => format!("{}", cfg.higher_cutoff),
        S_RATE => format!("{}", cfg.sample_rate),
        S_CH => format!("{}", cfg.channels),
        S_CHARSET => String::from_utf8_lossy(&cfg.glyphs).into_owned(),
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
        if text.len() > 79 {
            text.truncate(79);
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