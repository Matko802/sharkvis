use std::ffi::CString;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

mod audio;
mod config;
mod dsp;
mod fft;
mod lyrics;
mod mpris;
mod musixmatch;
mod pulse;
mod raw;
mod render;
mod settings;
mod state;
mod term;

use crate::audio::Audio;
use crate::config::{color_to_rgb_any, config_default_path, config_load, config_save, Config};
use crate::dsp::Dsp;
use crate::lyrics::{FetchOpts, LyricWorker};
use crate::mpris::{poll_named, poll_position, poll_track, Track};
use crate::render::{RenderMode, Renderer};
use crate::settings::{SettingsUi, CH_AUDIO, CH_DSP, CH_EDITOR, CH_LAYOUT};
use crate::term::{
    mouse_decode, term_cell_aspect, term_mouse_enter, term_mouse_leave, term_raw_enter,
    term_raw_restore, term_read_codepoint, term_winsize, KEY_BACKSPACE, KEY_CHAR, KEY_ENTER,
    KEY_ESC, KEY_MOUSE,
};

const VIS_EPS: f64 = 0.001;
const CLEAR_ESC: &[u8] = b"\x1b[2J\x1b[3J\x1b[H";
const OUT_CAP: usize = 1 << 20;
const VERSION: &str = env!("CARGO_PKG_VERSION");

static G_SIG: AtomicBool = AtomicBool::new(false);
static G_RESIZE: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    G_SIG.store(true, Ordering::SeqCst);
}

extern "C" fn on_winch(_sig: libc::c_int) {
    G_RESIZE.store(true, Ordering::SeqCst);
}

extern "C" fn on_fatal(sig: libc::c_int) {
    const RESTORE: &[u8] = b"\x1b[?25h\x1b[0m\x1b[?1000l\x1b[?1006l\x1b[2J\x1b[H";
    unsafe {
        let _ = libc::write(1, RESTORE.as_ptr() as *const libc::c_void, RESTORE.len());
        term_raw_restore(0);
        libc::_exit(128 + sig);
    }
}

fn set_handler(sig: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
    sa.sa_sigaction = handler as libc::sighandler_t;
    unsafe {
        libc::sigaction(sig, &sa, std::ptr::null_mut());
    }
}

fn usage() {
    println!("usage: sharkvis [-p config_file] [--raw [--bars N] [--fps N] [--raw-mode bars|wave|oscilloscope]]");
    println!("  --raw: print bar levels (0-100, ';'-separated, one line per frame) to stdout");
    println!("  g - settings, q - quit");
}

fn print_version() {
    println!("sharkvis {}", VERSION);
}

fn panel_width_for(cols: u32) -> usize {
    let mut pw = cols / 3;
    if pw < 28 {
        pw = 28;
    }
    if pw > 44 {
        pw = 44;
    }
    if pw >= cols {
        pw = if cols > 2 { cols / 2 } else { 1 };
    }
    if pw < 1 {
        pw = 1;
    }
    pw as usize
}

fn bar_count_for(cols: u32, cfg: &Config) -> usize {
    let step = cfg.bar_width + cfg.bar_spacing;
    let avail = if step > 0 { (cols as usize) / step } else { cols as usize };
    let b = if cfg.bars > 0 { cfg.bars } else { avail };
    if b < 1 {
        1
    } else {
        b
    }
}

fn yscale_for(fd: i32) -> usize {
    ((2.0 / term_cell_aspect(fd)).round() as usize).clamp(1, 4)
}

fn per_ch_left(bars: usize, channels: u32) -> usize {
    if channels > 1 && bars > 1 {
        (bars + 1) / 2
    } else {
        bars
    }
}

fn per_ch_right(bars: usize, channels: u32) -> usize {
    if channels > 1 && bars > 1 {
        bars / 2
    } else {
        bars
    }
}

fn is_k(key: i32, cp: &[u8], ch: u8) -> bool {
    if key == ch as i32 {
        return true;
    }
    if key == KEY_CHAR && cp.first() == Some(&ch) {
        return true;
    }
    false
}

fn run_editor(path: &str) {
    {
        let stdout = std::io::stdout();
        let mut so = stdout.lock();
        let _ = so.write_all(b"\x1b[0m\x1b[2J\x1b[H\x1b[?25h");
        let _ = so.flush();
    }
    term_raw_restore(0);

    let old_int = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        let cpath = CString::new(path).unwrap();
        let prog = CString::new("nano").unwrap();
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::execlp(prog.as_ptr(), prog.as_ptr(), cpath.as_ptr(), std::ptr::null::<libc::c_char>());
            libc::_exit(127);
        }
    }
    if pid > 0 {
        let mut status: libc::c_int = 0;
        loop {
            let r = unsafe { libc::waitpid(pid, &mut status, 0) };
            if r >= 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                break;
            }
        }
        if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 127 {
            eprintln!("sharkvis: could not launch nano");
        }
    }
    unsafe { libc::signal(libc::SIGINT, old_int) };

    term_raw_enter(0);
    {
        let stdout = std::io::stdout();
        let mut so = stdout.lock();
        let _ = so.write_all(b"\x1b[2J\x1b[H\x1b[?25l");
        let _ = so.flush();
    }
}

fn config_use_jefetch_colors(cfg: &Config) -> bool {
    let tok: String = cfg
        .colors
        .trim_start()
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ',')
        .collect();
    tok.eq_ignore_ascii_case("jefetch")
}

fn distinct_pair(lo: (u8, u8, u8), hi: (u8, u8, u8)) -> ((u8, u8, u8), (u8, u8, u8)) {
    if lo != hi {
        return (lo, hi);
    }
    let lum = 0.299 * lo.0 as f32 + 0.587 * lo.1 as f32 + 0.114 * lo.2 as f32;
    let t = if lum > 127.5 { 0 } else { 255 };
    let mix = |a: u8| (a as f32 + (t as f32 - a as f32) * 0.45 + 0.5) as u8;
    (lo, (mix(lo.0), mix(lo.1), mix(lo.2)))
}

const VGA_DEFAULTS: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (170, 0, 0),
    (0, 170, 0),
    (170, 85, 0),
    (0, 0, 170),
    (170, 0, 170),
    (0, 170, 170),
    (170, 170, 170),
    (85, 85, 85),
    (255, 85, 85),
    (85, 255, 85),
    (255, 255, 85),
    (85, 85, 255),
    (255, 85, 255),
    (85, 255, 255),
    (255, 255, 255),
];

fn map_raw_through_term(raw: (u8, u8, u8), pal: &[(u8, u8, u8); 16]) -> (u8, u8, u8) {
    for (i, vga) in VGA_DEFAULTS.iter().enumerate() {
        if *vga == raw {
            return pal[i];
        }
    }
    raw
}

fn osc4_hex_comp(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    let max: u32 = match s.len() {
        1 => 15,
        2 => 255,
        3 => 4095,
        _ => 65535,
    };
    Some((v * 255 + max / 2) / max)
}

fn parse_osc4_spec(s: &str) -> Option<(u8, u8, u8)> {
    if let Some(rest) = s.strip_prefix("rgb:") {
        let mut it = rest.split('/');
        let r = osc4_hex_comp(it.next()?)?;
        let g = osc4_hex_comp(it.next()?)?;
        let b = osc4_hex_comp(it.next()?)?;
        if it.next().is_some() {
            return None;
        }
        return Some((r as u8, g as u8, b as u8));
    }
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() != 3 && hex.len() != 6 {
            return None;
        }
        let w = hex.len() / 3;
        let mut c = [0u32; 3];
        for k in 0..3 {
            c[k] = osc4_hex_comp(&hex[k * w..(k + 1) * w])?;
        }
        return Some((c[0] as u8, c[1] as u8, c[2] as u8));
    }
    None
}

fn osc4_parse(buf: &[u8], pal: &mut [(u8, u8, u8); 16], have: &mut [bool; 16]) {
    let mut i = 0;
    while i + 5 < buf.len() {
        if buf[i] != 0x1b || buf[i + 1] != b']' {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        if j + 1 >= buf.len() || buf[j] != b'4' || buf[j + 1] != b';' {
            i += 1;
            continue;
        }
        j += 2;
        let mut idx: usize = 0;
        let mut digits = 0;
        while j < buf.len() && buf[j].is_ascii_digit() {
            idx = idx * 10 + (buf[j] - b'0') as usize;
            digits += 1;
            j += 1;
        }
        if digits == 0 || idx > 15 {
            i += 1;
            continue;
        }
        if j >= buf.len() || (buf[j] != b';' && buf[j] != b':') {
            i += 1;
            continue;
        }
        j += 1;
        let s = j;
        while j < buf.len()
            && buf[j] != 0x07
            && !(buf[j] == 0x1b && j + 1 < buf.len() && buf[j + 1] == b'\\')
        {
            j += 1;
        }
        if j >= buf.len() {
            break;
        }
        if j > s {
            if let Ok(spec) = std::str::from_utf8(&buf[s..j]) {
                if !have[idx] {
                    if let Some(c) = parse_osc4_spec(spec) {
                        pal[idx] = c;
                        have[idx] = true;
                    }
                }
            }
        }
        i = j;
    }
}

fn osc4_query_uncached() -> Option<[(u8, u8, u8); 16]> {
    let term_ok = match std::env::var("TERM") {
        Ok(t) => !t.is_empty() && !t.eq_ignore_ascii_case("dumb"),
        Err(_) => false,
    };
    if !term_ok {
        return None;
    }
    let fd = unsafe {
        libc::open(
            b"/dev/tty\0".as_ptr() as *const libc::c_char,
            libc::O_RDWR | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return None;
    }
    let mut orig: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut orig) } != 0 {
        unsafe { libc::close(fd) };
        return None;
    }
    let mut raw = orig;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO);
    raw.c_cc[libc::VMIN as usize] = 0;
    raw.c_cc[libc::VTIME as usize] = 1;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        unsafe { libc::close(fd) };
        return None;
    }
    let mut req = Vec::new();
    for i in 0..16 {
        req.extend_from_slice(format!("\x1b]4;{i};?\x1b\\").as_bytes());
    }
    let mut wr = 0;
    while wr < req.len() {
        let k = unsafe {
            libc::write(
                fd,
                req[wr..].as_ptr() as *const libc::c_void,
                (req.len() - wr) as libc::size_t,
            )
        };
        if k <= 0 {
            break;
        }
        wr += k as usize;
    }
    let mut pal = [(0u8, 0u8, 0u8); 16];
    let mut have = [false; 16];
    let mut buf = vec![0u8; 4096];
    let mut bl = 0usize;
    let mut empty = 0;
    for _ in 0..6 {
        if have.iter().all(|x| *x) || bl + 64 >= buf.len() {
            break;
        }
        let k = unsafe {
            libc::read(
                fd,
                buf[bl..].as_mut_ptr() as *mut libc::c_void,
                (buf.len() - bl - 1) as libc::size_t,
            )
        };
        if k < 0 {
            let e = std::io::Error::last_os_error().raw_os_error();
            if e == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if k == 0 {
            empty += 1;
            if empty >= 3 {
                break;
            }
            continue;
        }
        empty = 0;
        bl += k as usize;
        osc4_parse(&buf[..bl], &mut pal, &mut have);
    }
    let mut tmp = [0u8; 1];
    unsafe {
        libc::read(fd, tmp.as_mut_ptr() as *mut libc::c_void, 1);
    }
    unsafe {
        libc::tcsetattr(fd, libc::TCSANOW, &orig);
        libc::close(fd);
    }
    if have.iter().all(|x| *x) {
        Some(pal)
    } else {
        None
    }
}

fn term_palette() -> Option<[(u8, u8, u8); 16]> {
    use std::sync::Mutex;
    use std::time::Instant;
    struct Cache {
        state: i8,
        at: Option<Instant>,
        pal: [(u8, u8, u8); 16],
    }
    static CACHE: Mutex<Cache> = Mutex::new(Cache {
        state: 0,
        at: None,
        pal: [(0, 0, 0); 16],
    });
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let stale = match c.at {
        Some(t) => now.duration_since(t).as_millis() > 10_000,
        None => true,
    };
    if c.state == 0 || stale {
        match osc4_query_uncached() {
            Some(p) => {
                c.state = 1;
                c.pal = p;
            }
            None => {
                c.state = -1;
            }
        }
        c.at = Some(now);
    }
    if c.state < 0 {
        return None;
    }
    Some(c.pal)
}

fn logo_gradient() -> Option<((u8, u8, u8), (u8, u8, u8))> {
    use crate::config::color_to_rgb_any;
    let uid = unsafe { libc::getuid() };
    let mut paths = Vec::new();
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        let t = rt.trim_end_matches('/');
        if !t.is_empty() {
            paths.push(format!("{}/sharkvis/logo_colors", t));
        }
    }
    paths.push(format!("/tmp/sharkvis-{}-logo-colors", uid));
    for p in paths {
        let text = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let mut lo = None;
        let mut hi = None;
        for tok in text.split(|c: char| c.is_whitespace() || c == ',') {
            if let Some(v) = tok.strip_prefix("low=") {
                if color_to_rgb_any(v).is_some() {
                    lo = Some(v.to_string());
                }
            } else if let Some(v) = tok.strip_prefix("high=") {
                if color_to_rgb_any(v).is_some() {
                    hi = Some(v.to_string());
                }
            }
        }
        if let (Some(l), Some(h)) = (lo, hi) {
            let (lr, lg, lb) = color_to_rgb_any(&l).unwrap();
            let (hr, hg, hb) = color_to_rgb_any(&h).unwrap();
            let mut lo_raw = (lr as u8, lg as u8, lb as u8);
            let mut hi_raw = (hr as u8, hg as u8, hb as u8);
            if let Some(pal) = term_palette() {
                lo_raw = map_raw_through_term(lo_raw, &pal);
                hi_raw = map_raw_through_term(hi_raw, &pal);
            }
            return Some(distinct_pair(lo_raw, hi_raw));
        }
    }
    None
}

fn apply_colors(rnd: &mut Renderer, cfg: &Config) {
    let before = (rnd.grad_lo, rnd.grad_hi, rnd.color_256, rnd.grad_amt);
    rnd.color_256 = cfg.color_256;
    rnd.grad_amt = cfg.gradient_amt;
    let mut done = false;
    if config_use_jefetch_colors(cfg) {
        if let Some(((lr, lg, lb), (hr, hg, hb))) = logo_gradient() {
            rnd.grad_lo = ((lr as u32) << 16) | ((lg as u32) << 8) | lb as u32;
            rnd.grad_hi = ((hr as u32) << 16) | ((hg as u32) << 8) | hb as u32;
            done = true;
        }
    }
    if !done {
        if let Some((r, g, b)) = color_to_rgb_any(&cfg.gradient_low) {
            rnd.grad_lo = (r << 16) | (g << 8) | b;
        }
        if let Some((r, g, b)) = color_to_rgb_any(&cfg.gradient_high) {
            rnd.grad_hi = (r << 16) | (g << 8) | b;
        }
    }
    if (rnd.grad_lo, rnd.grad_hi, rnd.color_256, rnd.grad_amt) != before {
        rnd.clear();
    }
}

fn apply_settings(
    dsp: &mut [Dsp; 2],
    rnd: &mut Renderer,
    audio: &mut Audio,
    cfg: &mut Config,
    bars: &mut usize,
    heights: &mut [Vec<f64>; 2],
    last_h: &mut [Vec<f64>; 2],
    rows: u32,
    cols: u32,
    chmask: u32,
    audio_reinit: bool,
    x_off: usize,
) {
    let new_bars = bar_count_for(cols, cfg);
    let pcl = per_ch_left(new_bars, cfg.channels);
    let pcr = per_ch_right(new_bars, cfg.channels);

    if (chmask & (CH_DSP | CH_AUDIO)) != 0 || new_bars != *bars {
        let per = [pcl, pcr];
        for ch in 0..2 {
            let saved_sens = dsp[ch].sens;
            let saved_sens_init = dsp[ch].sens_init;
            dsp[ch] = Dsp::new(
                per[ch],
                cfg.sample_rate,
                cfg.autosens,
                cfg.noise_reduction,
                cfg.lower_cutoff,
                cfg.higher_cutoff,
            );
            dsp[ch].sens = saved_sens;
            dsp[ch].sens_init = saved_sens_init;
            dsp[ch].display_fps = cfg.framerate.max(1) as f64;
        }
    }

    if new_bars != *bars {
        heights[0] = vec![0.0; new_bars];
        heights[1] = vec![0.0; new_bars];
        last_h[0] = vec![0.0; new_bars];
        last_h[1] = vec![0.0; new_bars];
        *bars = new_bars;
        rnd.resize(rows as usize, cols as usize, new_bars);
    }

    rnd.bar_width = cfg.bar_width;
    rnd.bar_spacing = cfg.bar_spacing;
    rnd.color_256 = cfg.color_256;
    rnd.grad_amt = cfg.gradient_amt;
    rnd.wave_sm = cfg.wave_smoothing;
    rnd.wave_fps = cfg.framerate;
    apply_colors(rnd, cfg);
    let m = if cfg.mode.is_empty() { "bars" } else { cfg.mode.as_str() };
    rnd.set_mode(Renderer::mode_parse(m));
    rnd.set_glyphs(Some(&cfg.chars));
    rnd.set_wave(cfg.sample_rate);
    rnd.set_offset(x_off);
    rnd.clear();

    if chmask != 0 {
        heights[0].fill(0.0);
        heights[1].fill(0.0);
    }

    if audio_reinit {
        audio.stop();
        *audio = Audio::new(dsp[0].render_frame_size());
        audio.start(&cfg.source, cfg.sample_rate, cfg.channels);
    }
}

fn clamp_cfg(cfg: &mut Config) {
    if cfg.bar_width < 1 {
        cfg.bar_width = 1;
    }
    if cfg.bar_width > 8 {
        cfg.bar_width = 8;
    }
    if cfg.bar_spacing > 4 {
        cfg.bar_spacing = 4;
    }
    if cfg.bars > 256 {
        cfg.bars = 256;
    }
    if cfg.framerate < 1 {
        cfg.framerate = 1;
    }
    if cfg.framerate > 240 {
        cfg.framerate = 240;
    }
    if cfg.sensitivity < 0.1 {
        cfg.sensitivity = 0.1;
    }
    if cfg.noise_reduction < 0.0 {
        cfg.noise_reduction = 0.0;
    }
    if cfg.noise_reduction > 1.0 {
        cfg.noise_reduction = 1.0;
    }
    if cfg.wave_smoothing.is_nan() {
        cfg.wave_smoothing = 0.5;
    } else {
        cfg.wave_smoothing = cfg.wave_smoothing.clamp(0.0, 0.95);
    }
    if cfg.lower_cutoff < 1 {
        cfg.lower_cutoff = 1;
    }
    if cfg.lower_cutoff > 24000 {
        cfg.lower_cutoff = 24000;
    }
    if cfg.higher_cutoff < 2 {
        cfg.higher_cutoff = 2;
    }
    if cfg.higher_cutoff > 24000 {
        cfg.higher_cutoff = 24000;
    }
    if cfg.higher_cutoff <= cfg.lower_cutoff {
        cfg.higher_cutoff = cfg.lower_cutoff.saturating_add(1).clamp(2, 24000);
        if cfg.higher_cutoff <= cfg.lower_cutoff {
            cfg.lower_cutoff = cfg.higher_cutoff.saturating_sub(1).max(1);
        }
    }
    if cfg.sample_rate < 8000 {
        cfg.sample_rate = 8000;
    }
    if cfg.sample_rate > 192000 {
        cfg.sample_rate = 192000;
    }
    if cfg.gradient_amt < 1 {
        cfg.gradient_amt = 1;
    }
    if cfg.gradient_amt > 256 {
        cfg.gradient_amt = 256;
    }
    if cfg.channels < 1 {
        cfg.channels = 1;
    }
    if cfg.channels > 2 {
        cfg.channels = 2;
    }

    if cfg.mode == "text" {
        cfg.mode = "lyrics".to_string();
    }
    if cfg.text_align != "left" && cfg.text_align != "center" {
        cfg.text_align = "center".to_string();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfgpath: Option<String> = None;
    let mut raw = false;
    let mut raw_bars: Option<usize> = None;
    let mut raw_fps: Option<u32> = None;
    let mut raw_mode: Option<crate::raw::RawMode> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" => {
                if i + 1 < args.len() {
                    i += 1;
                    cfgpath = Some(args[i].clone());
                }
            }
            "--raw" => {
                raw = true;
            }
            "--bars" => {
                if i + 1 < args.len() {
                    i += 1;
                    raw_bars = args[i].parse::<usize>().ok();
                }
            }
            "--fps" => {
                if i + 1 < args.len() {
                    i += 1;
                    raw_fps = args[i].parse::<u32>().ok();
                }
            }
            "--raw-mode" => {
                if i + 1 < args.len() {
                    i += 1;
                    raw_mode = crate::raw::parse_mode(&args[i]);
                }
            }
            "-h" | "--help" => {
                usage();
                return;
            }
            "-v" | "--version" => {
                print_version();
                return;
            }
            other => {
                eprintln!("sharkvis: unknown option '{}'", other);
                usage();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let mut cfg = Config::default();

    let mut g_debug = false;
    let mut g_dbg: Option<std::fs::File> = None;
    if std::env::var_os("SHARKVIS_DEBUG").is_some() {
        g_debug = true;
        g_dbg = std::fs::File::create("/tmp/sharkvis_dbg.log").ok();
    }

    let save_path;
    if let Some(p) = &cfgpath {
        save_path = p.clone();
        if !config_load(&mut cfg, &save_path) {
            eprintln!("sharkvis: error loading config {}", save_path);
            std::process::exit(1);
        }
    } else {
        save_path = config_default_path();
        if std::path::Path::new(&save_path).exists() && !config_load(&mut cfg, &save_path) {
            eprintln!("sharkvis: error loading config {}, using defaults", save_path);
        }
    }

    clamp_cfg(&mut cfg);

    if raw {
        let bars = raw_bars.unwrap_or(if cfg.bars > 0 { cfg.bars } else { 48 });
        let fps = raw_fps.unwrap_or(cfg.framerate.clamp(1, 240));
        let mode = raw_mode.unwrap_or(crate::raw::RawMode::Bars);
        std::process::exit(crate::raw::run_raw(&cfg, bars, fps, mode));
    }

    let mut rows = 24u32;
    let mut cols = 80u32;
    if !term_winsize(1, &mut rows, &mut cols) {
        rows = 24;
        cols = 80;
    }

    let mut bars = bar_count_for(cols, &cfg);
    let per = [
        per_ch_left(bars, cfg.channels),
        per_ch_right(bars, cfg.channels),
    ];

    let mut dsp: [Dsp; 2] = [
        Dsp::new(
            per[0],
            cfg.sample_rate,
            cfg.autosens,
            cfg.noise_reduction,
            cfg.lower_cutoff,
            cfg.higher_cutoff,
        ),
        Dsp::new(
            per[1],
            cfg.sample_rate,
            cfg.autosens,
            cfg.noise_reduction,
            cfg.lower_cutoff,
            cfg.higher_cutoff,
        ),
    ];
    dsp[0].display_fps = cfg.framerate.max(1) as f64;
    dsp[1].display_fps = cfg.framerate.max(1) as f64;

    if let Some(rsens) = state::read_sens() {
        dsp[0].sens = rsens;
        dsp[0].sens_init = false;
        dsp[1].sens = rsens;
        dsp[1].sens_init = false;
    }

    let mut audio = Audio::new(dsp[0].render_frame_size());
    audio.start(&cfg.source, cfg.sample_rate, cfg.channels);

    if !term_raw_enter(0) {
        eprintln!("sharkvis: not a terminal");
        audio.stop();
        return;
    }

    set_handler(libc::SIGINT, on_signal);
    set_handler(libc::SIGTERM, on_signal);
    set_handler(libc::SIGHUP, on_signal);
    set_handler(libc::SIGWINCH, on_winch);
    set_handler(libc::SIGSEGV, on_fatal);
    set_handler(libc::SIGABRT, on_fatal);
    set_handler(libc::SIGBUS, on_fatal);
    set_handler(libc::SIGFPE, on_fatal);
    set_handler(libc::SIGILL, on_fatal);

    {
        let stdout = std::io::stdout();
        let mut so = stdout.lock();
        let _ = so.write_all(b"\x1b[2J\x1b[H\x1b[?25l");
        let _ = so.flush();
    }

    let mut rnd = Renderer::new(
        rows as usize,
        cols as usize,
        cfg.bar_width,
        cfg.bar_spacing,
        bars,
    );
    apply_colors(&mut rnd, &cfg);
    rnd.wave_sm = cfg.wave_smoothing;
    rnd.wave_fps = cfg.framerate;
    let m = if cfg.mode.is_empty() { "bars" } else { cfg.mode.as_str() };
    rnd.set_mode(Renderer::mode_parse(m));
    rnd.set_glyphs(Some(&cfg.chars));
    rnd.set_wave(cfg.sample_rate);
    let mut auto_yscale = yscale_for(1);
    rnd.yscale = auto_yscale;

    let mut heights: [Vec<f64>; 2] = [vec![0.001; bars], vec![0.001; bars]];
    let mut last_h: [Vec<f64>; 2] = [vec![0.001; bars], vec![0.001; bars]];
    let mut out = Vec::with_capacity(OUT_CAP);

    let mut audio_backoff_ms: u64 = 500;
    let mut audio_retry_at = Instant::now();

    let mut st = SettingsUi::default();
    let mut in_settings = false;
    let mut force_draw = true;
    let mut last_static_draw = Instant::now();
    let mut chmask: u32 = 0;

    let mut cfg_dirty = false;
    let had_file = std::path::Path::new(&save_path).exists();

    let mut last_color_check = Instant::now();
    let mut last_color_stamp: Option<(std::time::SystemTime, u64)> = None;
    let mut lyric = LyricWorker::new();
    let mut track = Track::default();
    let mut last_track_poll = Instant::now();
    let mut last_lyric_shown = String::new();
    let mut search_buf: Option<String> = None;
    let mut manual_player: Option<String> = None;
    let mut last_provider = cfg.provider.clone();
    let mut last_pos_poll = Instant::now();

    let mut next = Instant::now();
    let mut live = state::StateWriter::new();

    let mut gate_open = false;

    state::remove_stale_state();

    let rc = 0;
    while !G_SIG.load(Ordering::SeqCst) {
        let t_frame0 = if g_debug { Some(Instant::now()) } else { None };
        let mut last_bytes = 0usize;
        let mut t_write_us: i64 = -1;
        let mut drew = false;

        let mut cp = [0u8; 8];
        let (key, clen) = term_read_codepoint(0, &mut cp);

        if search_buf.is_some() {
            match key {
                KEY_ESC => {
                    search_buf = None;
                    force_draw = true;
                }
                KEY_ENTER => {
                    if let Some(q) = search_buf.take() {
                        let q = q.trim().to_string();
                        if !q.is_empty() {
                            let (a, t) = match q.split_once(" - ") {
                                Some((a, t)) => (a.to_string(), t.to_string()),
                                None => (String::new(), q),
                            };
                            lyric.search_override(a, t);
                            force_draw = true;
                        }
                    }
                }
                KEY_BACKSPACE => {
                    if let Some(b) = search_buf.as_mut() {
                        b.pop();
                        force_draw = true;
                    }
                }
                KEY_CHAR => {
                    let chunk = std::str::from_utf8(&cp[..clen]).unwrap_or("").to_string();
                    if let Some(b) = search_buf.as_mut() {
                        for c in chunk.chars() {
                            if !c.is_control() && b.len() < 120 {
                                b.push(c);
                            }
                        }
                        force_draw = true;
                    }
                }
                _ => {}
            }
        } else if in_settings {
            if is_k(key, &cp[..clen], b'g') || is_k(key, &cp[..clen], b'G') || key == KEY_ESC {
                in_settings = false;
                term_mouse_leave();
                {
                    let stdout = std::io::stdout();
                    let mut so = stdout.lock();
                    let _ = so.write_all(CLEAR_ESC);
                    let _ = so.flush();
                }
                apply_settings(
                    &mut dsp, &mut rnd, &mut audio, &mut cfg, &mut bars, &mut heights, &mut last_h,
                    rows, cols, chmask, (chmask & CH_AUDIO) != 0, 0,
                );
                chmask = 0;
                force_draw = true;
                if !config_save(&cfg, &save_path) {
                    eprintln!("sharkvis: could not save config to {}", save_path);
                }
            } else if is_k(key, &cp[..clen], b'q')
                || is_k(key, &cp[..clen], b'Q')
                || key == 3
            {
                break;
            } else {
                if key == KEY_MOUSE {
                    if let Some((_, x, y)) = mouse_decode(&cp[..clen]) {
                        st.click(&mut cfg, y as u32, x as u32, panel_width_for(cols), &mut chmask);
                    }
                } else {
                    st.key(
                        &mut cfg,
                        key,
                        if key == KEY_CHAR { Some(&cp[..clen]) } else { None },
                        &mut chmask,
                    );
                }
                if (chmask & CH_EDITOR) != 0 {
                    if !config_save(&cfg, &save_path) {
                        eprintln!("sharkvis: could not save config to {}", save_path);
                    }
                    term_mouse_leave();
                    run_editor(&save_path);
                    term_mouse_enter();
                    if !config_load(&mut cfg, &save_path) {
                        eprintln!("sharkvis: error loading config {}", save_path);
                    }
    clamp_cfg(&mut cfg);
                    chmask = CH_LAYOUT | CH_DSP | CH_AUDIO;
                }
                if chmask != 0 {
                    apply_settings(
                        &mut dsp, &mut rnd, &mut audio, &mut cfg, &mut bars, &mut heights,
                        &mut last_h, rows, cols, chmask, (chmask & CH_AUDIO) != 0,
                        panel_width_for(cols),
                    );

                    cfg_dirty = true;
                    if !config_save(&cfg, &save_path) {
                        eprintln!("sharkvis: could not save config to {}", save_path);
                    }
                    {
                        let stdout = std::io::stdout();
                        let mut so = stdout.lock();
                        let _ = so.write_all(CLEAR_ESC);
                        let _ = so.flush();
                    }
                    chmask = 0;
                    force_draw = true;
                }
            }
        } else {
            if is_k(key, &cp[..clen], b'g') || is_k(key, &cp[..clen], b'G') {
                in_settings = true;
                chmask = 0;
                term_mouse_enter();
                {
                    let stdout = std::io::stdout();
                    let mut so = stdout.lock();
                    let _ = so.write_all(CLEAR_ESC);
                    let _ = so.flush();
                }
                rnd.set_offset(panel_width_for(cols));
                force_draw = true;
            } else if is_k(key, &cp[..clen], b'q')
                || is_k(key, &cp[..clen], b'Q')
                || key == 3
            {
                break;
            } else if rnd.mode == RenderMode::Lyrics {
                if is_k(key, &cp[..clen], b's') || is_k(key, &cp[..clen], b'S') {
                    search_buf = Some(String::new());
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'l') || is_k(key, &cp[..clen], b'L') {
                    let players = crate::mpris::player_list();
                    if !players.is_empty() {
                        manual_player = match manual_player
                            .as_ref()
                            .and_then(|m| players.iter().position(|p| p == m))
                        {
                            Some(i) if i + 1 < players.len() => Some(players[i + 1].clone()),
                            Some(_) => None,
                            None => Some(players[0].clone()),
                        };
                        force_draw = true;
                    }
                } else if is_k(key, &cp[..clen], b'r') || is_k(key, &cp[..clen], b'R') {
                    lyric.force_reload();
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'c') || is_k(key, &cp[..clen], b'C') {
                    cfg.text_align = if cfg.text_align == "left" { "center".to_string() } else { "left".to_string() };
                    cfg_dirty = true;
                    if !config_save(&cfg, &save_path) {
                        eprintln!("sharkvis: could not save config to {}", save_path);
                    }
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'a') || is_k(key, &cp[..clen], b'A') {
                    lyric.set_follow(!lyric.following(), track.position);
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'p') || is_k(key, &cp[..clen], b'P') {
                    cfg.provider = match cfg.provider.as_str() {
                        "auto" => "lrclib".to_string(),
                        "lrclib" => "musixmatch".to_string(),
                        _ => "auto".to_string(),
                    };
                    lyric.poke();
                    cfg_dirty = true;
                    if !config_save(&cfg, &save_path) {
                        eprintln!("sharkvis: could not save config to {}", save_path);
                    }
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'+') || is_k(key, &cp[..clen], b'=') {
                    cfg.lyric_offset_ms = (cfg.lyric_offset_ms + 500).clamp(-10000, 10000);
                    cfg_dirty = true;
                    if !config_save(&cfg, &save_path) {
                        eprintln!("sharkvis: could not save config to {}", save_path);
                    }
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'-') || is_k(key, &cp[..clen], b'_') {
                    cfg.lyric_offset_ms = (cfg.lyric_offset_ms - 500).clamp(-10000, 10000);
                    cfg_dirty = true;
                    if !config_save(&cfg, &save_path) {
                        eprintln!("sharkvis: could not save config to {}", save_path);
                    }
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'0') {
                    cfg.lyric_offset_ms = 0;
                    cfg_dirty = true;
                    if !config_save(&cfg, &save_path) {
                        eprintln!("sharkvis: could not save config to {}", save_path);
                    }
                    force_draw = true;
                }
            }
        }

        if G_RESIZE.swap(false, Ordering::SeqCst) {
            let mut nr = 0u32;
            let mut nc = 0u32;
            if term_winsize(1, &mut nr, &mut nc) && nr > 0 && nc > 0 && (nr != rows || nc != cols)
            {
                let new_bars = bar_count_for(nc, &cfg);
                let new_bars = if new_bars < 1 { 1 } else { new_bars };
                let per = [
                    per_ch_left(new_bars, cfg.channels),
                    per_ch_right(new_bars, cfg.channels),
                ];
                cols = nc;
                rows = nr;
                bars = new_bars;
                for ch in 0..2 {
                    let saved_sens = dsp[ch].sens;
                    let saved_sens_init = dsp[ch].sens_init;
                    dsp[ch] = Dsp::new(
                        per[ch],
                        cfg.sample_rate,
                        cfg.autosens,
                        cfg.noise_reduction,
                        cfg.lower_cutoff,
                        cfg.higher_cutoff,
                    );
                    dsp[ch].sens = saved_sens;
                    dsp[ch].sens_init = saved_sens_init;
                    dsp[ch].display_fps = cfg.framerate.max(1) as f64;
                }
                heights[0] = vec![0.001; bars];
                heights[1] = vec![0.001; bars];
                last_h[0] = vec![0.001; bars];
                last_h[1] = vec![0.001; bars];
                rnd.resize(rows as usize, cols as usize, bars);
                auto_yscale = yscale_for(1);
                rnd.yscale = auto_yscale;
                if in_settings {
                    rnd.set_offset(panel_width_for(cols));
                }
                {
                    let stdout = std::io::stdout();
                    let mut so = stdout.lock();
                    let _ = so.write_all(CLEAR_ESC);
                    let _ = so.flush();
                }
                force_draw = true;
            }
        }

        let (n, samples_l, samples_r) = audio.consume();
        if n > 0 {
            rnd.feed(samples_l, samples_r, n);
        }

        let disp_fps = cfg.framerate.max(1) as f64;
        dsp[0].display_fps = disp_fps;
        dsp[1].display_fps = disp_fps;
        dsp[0].execute(samples_l, n, &mut heights[0]);
        if cfg.channels > 1 {
            dsp[1].execute(samples_r.or(samples_l), n, &mut heights[1]);
        }
        if audio.failed() {
            if Instant::now() >= audio_retry_at {
                eprintln!("\nsharkvis: audio input failed: {}; retrying", audio.error());
                let mut na = Audio::new(dsp[0].render_frame_size());
                na.start(&cfg.source, cfg.sample_rate, cfg.channels);
                audio.stop();
                audio = na;
                for h in heights.iter_mut() {
                    h.fill(0.0);
                }
                dsp[0].flush();
                dsp[1].flush();
                audio_retry_at = Instant::now() + Duration::from_millis(audio_backoff_ms);
                if audio_backoff_ms < 5000 {
                    audio_backoff_ms *= 2;
                }
            }
        } else {
            audio_backoff_ms = 500;
        }

        let pcl = per_ch_left(bars, cfg.channels);
        let pcr = per_ch_right(bars, cfg.channels);
        dsp[0].sens_scale = cfg.sensitivity / 100.0;
        if cfg.channels > 1 {
            dsp[1].sens_scale = cfg.sensitivity / 100.0;
        }

        {
            let nbass = per_ch_left(bars, cfg.channels).max(2) / 4 + 1;
            let mut sum = 0.0f64;
            let mut cnt = 0usize;
            let mut bsum = 0.0f64;
            let mut bcnt = 0usize;
            let mut lsum = 0.0f64;
            let mut lcnt = 0usize;
            for i in 0..pcl.min(heights[0].len()) {
                sum += heights[0][i];
                cnt += 1;
                lsum += heights[0][i];
                lcnt += 1;
                if i < nbass {
                    bsum += heights[0][i];
                    bcnt += 1;
                }
            }
            let mut rsum = 0.0f64;
            let mut rcnt = 0usize;
            if cfg.channels > 1 {
                let nbass_r = pcr.max(2) / 4 + 1;
                for i in 0..pcr.min(heights[1].len()) {
                    sum += heights[1][i];
                    cnt += 1;
                    rsum += heights[1][i];
                    rcnt += 1;
                    if i < nbass_r {
                        bsum += heights[1][i];
                        bcnt += 1;
                    }
                }
            }
            let energy = if cnt > 0 { sum / cnt as f64 } else { 0.0 };
            let bass = if bcnt > 0 { bsum / bcnt as f64 } else { energy };
            let left = if lcnt > 0 { lsum / lcnt as f64 } else { energy };
            let right = if rcnt > 0 {
                rsum / rcnt as f64
            } else {
                left
            };
            let mut energy = energy;
            let mut bass = bass;
            let mut left = left;
            let mut right = right;
            {
                let mut raw = dsp[0].raw_peak();
                if cfg.channels > 1 {
                    let r1 = dsp[1].raw_peak();
                    if r1 > raw {
                        raw = r1;
                    }
                }
                if gate_open {
                    if raw < 0.01 {
                        gate_open = false;
                    }
                } else if raw > 0.02 {
                    gate_open = true;
                }
                if !gate_open {
                    energy = 0.0;
                    bass = 0.0;
                    left = 0.0;
                    right = 0.0;
                }
            }

            let lo_u = rnd.grad_lo;
            let hi_u = rnd.grad_hi;
            let lo = (
                ((lo_u >> 16) & 0xff) as u8,
                ((lo_u >> 8) & 0xff) as u8,
                (lo_u & 0xff) as u8,
            );
            let hi = (
                ((hi_u >> 16) & 0xff) as u8,
                ((hi_u >> 8) & 0xff) as u8,
                (hi_u & 0xff) as u8,
            );
            live.update(energy, bass, left, right, lo, hi, dsp[0].sens, cfg.gradient_amt);
        }

        if last_color_check.elapsed() >= Duration::from_millis(1000) {
            last_color_check = Instant::now();
            if let Ok(meta) = std::fs::metadata(&save_path) {
                if let Ok(mtime) = meta.modified() {
                    let stamp = (mtime, meta.len());
                    if last_color_stamp.as_ref() != Some(&stamp) {
                        if last_color_stamp.is_none() {
                            last_color_stamp = Some(stamp);
                        } else if crate::config::reload_colors(&mut cfg, &save_path) {
                            apply_colors(&mut rnd, &cfg);
                            force_draw = true;
                            last_color_stamp = Some(stamp);
                        }
                    }
                }
            }
            if config_use_jefetch_colors(&cfg) {
                let before = (rnd.grad_lo, rnd.grad_hi);
                apply_colors(&mut rnd, &cfg);
                if (rnd.grad_lo, rnd.grad_hi) != before {
                    force_draw = true;
                }
            }
        }
        let need_lyrics = rnd.mode == RenderMode::Lyrics;
        if need_lyrics {
            if last_track_poll.elapsed() >= Duration::from_millis(2000) {
                last_track_poll = Instant::now();
                let allow: Vec<String> = cfg
                    .mpris_players
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let fresh = match manual_player.clone() {
                    Some(m) => {
                        let t = poll_named(&m);
                        if t.present {
                            t
                        } else {
                            manual_player = None;
                            poll_track(&allow)
                        }
                    }
                    None => poll_track(&allow),
                };
                if fresh.present {
                    track = fresh;
                } else {
                    track.present = false;
                    if !crate::mpris::any_active_player() {
                        lyric.reset();
                    }
                }
            }
            if last_pos_poll.elapsed() >= Duration::from_millis(200) {
                last_pos_poll = Instant::now();
                if track.present && !track.player.is_empty() {
                    if let Some(pos) = poll_position(&track.player) {
                        track.position = pos;
                        lyric.update_pos(pos);
                    }
                }
            }
            lyric.update(
                &track,
                &FetchOpts {
                    local_folder: cfg.lyrics_folder.clone(),
                    provider: cfg.provider.clone(),
                },
            );
        } else {

            last_track_poll = Instant::now();
            last_pos_poll = Instant::now();
        }
        lyric.set_offset_ms(cfg.lyric_offset_ms);
        rnd.text_left = cfg.text_align == "left";
        rnd.text_size = cfg.text_size.min(5) as usize;
        rnd.yscale = auto_yscale;
        rnd.text_small = cfg.text_style == "normal";
        rnd.loading = need_lyrics && lyric.loading();
        if cfg.provider != last_provider {
            last_provider = cfg.provider.clone();
            lyric.poke();
            force_draw = true;
        }
        if rnd.mode == RenderMode::Lyrics {
            let rows = if cfg.text_style == "normal" {
                lyric.display_context(&track)
            } else {
                lyric.display_lines(&track)
            };
            let shown: String =
                rows.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join("\n");
            if shown != last_lyric_shown {
                last_lyric_shown = shown;
                force_draw = true;
            }
            rnd.set_rich(&rows);
        }

        let mut need_draw = force_draw || in_settings;
        if !need_draw {

            if rnd.mode == RenderMode::Bars
            {
                for i in 0..pcl {
                    if heights[0][i] < last_h[0][i] - VIS_EPS || heights[0][i] > last_h[0][i] + VIS_EPS
                    {
                        need_draw = true;
                        break;
                    }
                }
                if !need_draw && cfg.channels > 1 {
                    for i in 0..pcr {
                        if heights[1][i] < last_h[1][i] - VIS_EPS
                            || heights[1][i] > last_h[1][i] + VIS_EPS
                        {
                            need_draw = true;
                            break;
                        }
                    }
                }
            } else if rnd.mode == RenderMode::Lyrics {
                need_draw = n > 0;
                if !need_draw
                    && last_static_draw.elapsed() >= Duration::from_millis(500)
                {
                    need_draw = true;
                }
            } else {
                need_draw = true;
            }
        }

        if need_draw {
            force_draw = false;
            if rnd.mode != RenderMode::Bars {
                last_static_draw = Instant::now();
            }
            drew = true;
            last_h[0][..pcl].copy_from_slice(&heights[0][..pcl]);
            if cfg.channels > 1 {
                last_h[1][..pcr].copy_from_slice(&heights[1][..pcr]);
            }

            out.clear();
            if in_settings {
                st.draw(&cfg, &mut out, OUT_CAP, rows, panel_width_for(cols));
            }
            if cfg.channels > 1 {
                rnd.draw_stereo(&heights[0], &heights[1], pcl, pcr, &mut out, OUT_CAP);
            } else {
                rnd.draw(&heights[0], &mut out, OUT_CAP);
            }
            if let Some(buf) = search_buf.as_ref() {
                let line = format!("\x1b[0m\x1b[{};1Hsearch: {}_", rows, buf);
                if out.len() + line.len() < OUT_CAP {
                    out.extend_from_slice(line.as_bytes());
                }
            }
            if !out.is_empty() {
                let t_write = if g_debug { Some(Instant::now()) } else { None };
                {
                    let stdout = std::io::stdout();
                    let mut so = stdout.lock();
                    let _ = so.write_all(&out);
                    let _ = so.flush();
                }
                last_bytes = out.len();
                if let Some(t0) = t_write {
                    t_write_us = t0.elapsed().as_micros() as i64;
                }
            }
        }

        let frame_dur = Duration::from_nanos((1_000_000_000u64) / (cfg.framerate as u64).max(1));
        let now = Instant::now();
        if let Some(until) = next.checked_duration_since(now) {
            thread::sleep(until);
            next = next.checked_add(frame_dur).unwrap_or_else(Instant::now);
        } else {

            next = Instant::now().checked_add(frame_dur).unwrap_or_else(Instant::now);
        }

        if g_debug {
            if let Some(dbg) = g_dbg.as_mut() {
                if let Some(t0) = t_frame0 {
                    let iter_us = t0.elapsed().as_micros() as i64;
                    let _ = writeln!(
                        dbg,
                        "iter={}us write={}us bytes={} drew={} fps={}",
                        iter_us, t_write_us, last_bytes, drew, cfg.framerate
                    );
                    let _ = dbg.flush();
                }
            }
        }
    }

    if cfg_dirty || !had_file {
        if !config_save(&cfg, &save_path) {
            eprintln!("sharkvis: could not save config to {}", save_path);
        }
    }

    {
        let stdout = std::io::stdout();
        let mut so = stdout.lock();
        let mut tail = Vec::with_capacity(CLEAR_ESC.len() + 8);
        tail.extend_from_slice(b"\x1b[?25h\x1b[0m\x1b[?1000l\x1b[?1006l");
        tail.extend_from_slice(CLEAR_ESC);
        let _ = so.write_all(&tail);
        let _ = so.flush();
    }
    term_raw_restore(0);

    audio.stop();

    state::clear_state_file();

    std::process::exit(rc);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_vga_maps_through_terminal_palette() {
        let mut pal = [(0u8, 0u8, 0u8); 16];
        pal[4] = (10, 20, 30);
        pal[6] = (40, 50, 60);
        assert_eq!(map_raw_through_term((0, 0, 170), &pal), (10, 20, 30));
        assert_eq!(map_raw_through_term((0, 170, 170), &pal), (40, 50, 60));
    }

    #[test]
    fn truecolor_passthrough_keeps_raw() {
        let pal = [(1u8, 2u8, 3u8); 16];
        assert_eq!(map_raw_through_term((255, 136, 0), &pal), (255, 136, 0));
    }

    #[test]
    fn osc4_response_parses() {
        let mut pal = [(0u8, 0u8, 0u8); 16];
        let mut have = [false; 16];
        let buf = b"\x1b]4;4;rgb:0000/0000/aaaa\x1b\\".to_vec();
        osc4_parse(&buf, &mut pal, &mut have);
        assert!(have[4]);
        assert_eq!(pal[4], (0, 0, 170));
    }
}