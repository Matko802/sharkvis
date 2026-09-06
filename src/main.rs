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
mod render;
mod settings;
mod state;
mod term;

use crate::audio::Audio;
use crate::config::{color_to_rgb, config_default_path, config_load, config_save, Config};
use crate::dsp::Dsp;
use crate::lyrics::{FetchOpts, LyricWorker};
use crate::mpris::{poll_named, poll_position, poll_track, Track};
use crate::render::{RenderMode, Renderer};
use crate::settings::{SettingsUi, CH_AUDIO, CH_DSP, CH_EDITOR, CH_LAYOUT};
use crate::term::{
    term_raw_enter, term_raw_restore, term_read_codepoint, term_winsize, KEY_BACKSPACE, KEY_CHAR,
    KEY_ENTER, KEY_ESC,
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
    const RESTORE: &[u8] = b"\x1b[?25h\x1b[0m\x1b[2J\x1b[H";
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
    println!("usage: sharkvis [-p config_file]");
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

fn apply_colors(rnd: &mut Renderer, cfg: &Config) {
    if let Some((r, g, b)) = color_to_rgb(&cfg.gradient_low) {
        rnd.grad_lo = (r << 16) | (g << 8) | b;
    }
    if let Some((r, g, b)) = color_to_rgb(&cfg.gradient_high) {
        rnd.grad_hi = (r << 16) | (g << 8) | b;
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
    apply_colors(rnd, cfg);
    let m = if cfg.mode.is_empty() { "bars" } else { cfg.mode.as_str() };
    rnd.set_mode(Renderer::mode_parse(m));
    rnd.set_glyphs(Some(&cfg.glyphs));
    rnd.set_text(&cfg.sptlrx_text.clone());
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
    if cfg.lower_cutoff < 1 {
        cfg.lower_cutoff = 1;
    }
    if cfg.higher_cutoff < cfg.lower_cutoff {
        cfg.higher_cutoff = cfg.lower_cutoff + 1;
    }
    if cfg.channels < 1 {
        cfg.channels = 1;
    }
    if cfg.channels > 2 {
        cfg.channels = 2;
    }
    if cfg.text_source != "lyrics" {
        cfg.text_source = "static".to_string();
    }
    let clean: String = cfg.sptlrx_text.to_ascii_uppercase().chars().take(24).collect();
    if clean.trim().is_empty() {
        cfg.sptlrx_text = "SHARKVIS".to_string();
    } else {
        cfg.sptlrx_text = clean;
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfgpath: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" => {
                if i + 1 < args.len() {
                    i += 1;
                    cfgpath = Some(args[i].clone());
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

    let mut audio = Audio::new(dsp[0].render_frame_size());
    audio.start(&cfg.source, cfg.sample_rate, cfg.channels);

    if !term_raw_enter(0) {
        eprintln!("sharkvis: not a terminal");
        audio.stop();
        return;
    }

    set_handler(libc::SIGINT, on_signal);
    set_handler(libc::SIGTERM, on_signal);
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
    let m = if cfg.mode.is_empty() { "bars" } else { cfg.mode.as_str() };
    rnd.set_mode(Renderer::mode_parse(m));
    rnd.set_glyphs(Some(&cfg.glyphs));
    rnd.set_text(&cfg.sptlrx_text.clone());
    rnd.set_wave(cfg.sample_rate);

    let mut heights: [Vec<f64>; 2] = [vec![0.001; bars], vec![0.001; bars]];
    let mut last_h: [Vec<f64>; 2] = [vec![0.001; bars], vec![0.001; bars]];
    let mut out = Vec::with_capacity(OUT_CAP);

    let mut st = SettingsUi::default();
    let mut in_settings = false;
    let mut force_draw = true;
    let mut chmask: u32 = 0;
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

    let mut rc = 0;
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
                st.key(
                    &mut cfg,
                    key,
                    if key == KEY_CHAR { Some(&cp[..clen]) } else { None },
                    &mut chmask,
                );
                if (chmask & CH_EDITOR) != 0 {
                    if !config_save(&cfg, &save_path) {
                        eprintln!("sharkvis: could not save config to {}", save_path);
                    }
                    run_editor(&save_path);
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
            } else if rnd.mode == RenderMode::Text {
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
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'a') || is_k(key, &cp[..clen], b'A') {
                    lyric.set_follow(!lyric.following(), track.position);
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'p') || is_k(key, &cp[..clen], b'P') {
                    cfg.provider = match cfg.provider.as_str() {
                        "auto" => "lrclib".to_string(),
                        "lrclib" => "musixmatch".to_string(),
                        "musixmatch" => "genius".to_string(),
                        _ => "auto".to_string(),
                    };
                    lyric.poke();
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'+') || is_k(key, &cp[..clen], b'=') {
                    cfg.lyric_offset_ms = (cfg.lyric_offset_ms + 500).clamp(-10000, 10000);
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'-') || is_k(key, &cp[..clen], b'_') {
                    cfg.lyric_offset_ms = (cfg.lyric_offset_ms - 500).clamp(-10000, 10000);
                    force_draw = true;
                } else if is_k(key, &cp[..clen], b'0') {
                    cfg.lyric_offset_ms = 0;
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
                }
                heights[0] = vec![0.001; bars];
                heights[1] = vec![0.001; bars];
                last_h[0] = vec![0.001; bars];
                last_h[1] = vec![0.001; bars];
                rnd.resize(rows as usize, cols as usize, bars);
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
        dsp[0].execute(samples_l, n, &mut heights[0]);
        if cfg.channels > 1 {
            dsp[1].execute(samples_r.or(samples_l), n, &mut heights[1]);
        }
        if audio.failed() {
            eprintln!("\nsharkvis: audio input failed: {}", audio.error());
            rc = 1;
            break;
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
            let lo = color_to_rgb(&cfg.gradient_low).unwrap_or((255, 255, 255));
            let hi = color_to_rgb(&cfg.gradient_high).unwrap_or((255, 255, 255));
            let cv = |(r, g, b): (u32, u32, u32)| (r as u8, g as u8, b as u8);
            live.update(energy, bass, left, right, cv(lo), cv(hi));
        }

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
        lyric.set_offset_ms(cfg.lyric_offset_ms);
        rnd.text_left = cfg.text_align == "left";
        rnd.text_size = cfg.text_size.min(5) as usize;
        rnd.text_small = cfg.text_style == "normal";
        if cfg.provider != last_provider {
            last_provider = cfg.provider.clone();
            lyric.poke();
            force_draw = true;
        }
        if rnd.mode == RenderMode::Text {
            if cfg.text_source == "lyrics" {
                let rows = if cfg.text_style == "normal" {
                    lyric.display_context(&track, &cfg.sptlrx_text)
                } else {
                    lyric.display_lines(&track, &cfg.sptlrx_text)
                };
                let shown: String =
                    rows.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join("\n");
                if shown != last_lyric_shown {
                    last_lyric_shown = shown;
                    force_draw = true;
                }
                rnd.set_rich(&rows);
            } else {
                rnd.set_text(&cfg.sptlrx_text);
            }
        }

        let mut need_draw = force_draw || in_settings;
        if !need_draw {
            if rnd.mode == RenderMode::Bars
                || rnd.mode == RenderMode::Text
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
            } else {
                need_draw = n > 0;
            }
        }

        if need_draw {
            force_draw = false;
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

        let now = Instant::now();
        if let Some(until) = next.checked_duration_since(now) {
            thread::sleep(until);
        }
        let frame_ns = 1_000_000_000i64 / (cfg.framerate as i64);
        let next2 = next.checked_add(Duration::from_nanos(frame_ns as u64));
        next = next2.unwrap_or_else(|| Instant::now());

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

    if !config_save(&cfg, &save_path) {
        eprintln!("sharkvis: could not save config to {}", save_path);
    }

    {
        let stdout = std::io::stdout();
        let mut so = stdout.lock();
        let mut tail = Vec::with_capacity(CLEAR_ESC.len() + 8);
        tail.extend_from_slice(b"\x1b[?25h\x1b[0m");
        tail.extend_from_slice(CLEAR_ESC);
        let _ = so.write_all(&tail);
        let _ = so.flush();
    }
    term_raw_restore(0);

    audio.stop();

    std::process::exit(rc);
}