use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::audio::Audio;
use crate::config::Config;
use crate::dsp::Dsp;

static RAW_STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sig(_sig: libc::c_int) {
    RAW_STOP.store(true, Ordering::SeqCst);
}

fn set_handler(sig: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
    sa.sa_sigaction = handler as libc::sighandler_t;
    unsafe {
        libc::sigaction(sig, &sa, std::ptr::null_mut());
    }
}

pub enum RawMode {
    Bars,
    Wave,
}

pub fn parse_mode(s: &str) -> Option<RawMode> {
    match s.to_ascii_lowercase().as_str() {
        "bars" => Some(RawMode::Bars),
        "wave" | "oscilloscope" => Some(RawMode::Wave),
        _ => None,
    }
}

pub fn run_raw(cfg: &Config, bars: usize, fps: u32, mode: RawMode) -> i32 {
    let bars = bars.max(1);
    let fps = fps.clamp(1, 240);
    let stereo = cfg.channels > 1;

    let mut dsp = [
        Dsp::new(
            bars,
            cfg.sample_rate,
            cfg.autosens,
            cfg.noise_reduction,
            cfg.lower_cutoff,
            cfg.higher_cutoff,
        ),
        Dsp::new(
            bars,
            cfg.sample_rate,
            cfg.autosens,
            cfg.noise_reduction,
            cfg.lower_cutoff,
            cfg.higher_cutoff,
        ),
    ];
    dsp[0].display_fps = fps as f64;
    dsp[1].display_fps = fps as f64;
    dsp[0].sens_scale = cfg.sensitivity / 100.0;
    dsp[1].sens_scale = cfg.sensitivity / 100.0;

    let mut audio = Audio::new(dsp[0].render_frame_size());
    audio.start(&cfg.source, cfg.sample_rate, cfg.channels);

    set_handler(libc::SIGINT, on_sig);
    set_handler(libc::SIGTERM, on_sig);

    let frame_dur = Duration::from_nanos(1_000_000_000u64 / fps as u64);
    let mut next = Instant::now();
    let mut h0 = vec![0.0f64; bars];
    let mut h1 = vec![0.0f64; bars];
    let mut last: Vec<f64> = Vec::new();
    let mut line = String::with_capacity(bars * 4);
    let stdout = std::io::stdout();
    let mut rc = 0;

    loop {
        if RAW_STOP.load(Ordering::SeqCst) {
            break;
        }
        let (n, sl, sr) = audio.consume();
        if n > 0 {
            if let Some(s) = sl {
                last.clear();
                last.extend_from_slice(&s[..n.min(s.len())]);
            }
        }
        dsp[0].execute(sl, n, &mut h0);
        if stereo {
            dsp[1].execute(sr.or(sl), n, &mut h1);
        }
        if audio.failed() {
            eprintln!("sharkvis: audio input failed: {}", audio.error());
            rc = 1;
            break;
        }
        line.clear();
        match mode {
            RawMode::Bars => {
                for i in 0..bars {
                    let v = if stereo {
                        (h0[i] + h1[i]) * 50.0
                    } else {
                        h0[i] * 100.0
                    };
                    let v = v.clamp(0.0, 100.0).round() as i32;
                    if i > 0 {
                        line.push(';');
                    }
                    line.push_str(&v.to_string());
                }
            }
            RawMode::Wave => {
                for i in 0..bars {
                    let v = if last.is_empty() {
                        50
                    } else {
                        let s = last[(i * last.len()) / bars].clamp(-1.0, 1.0);
                        (50.0 + 50.0 * s).round() as i32
                    };
                    if i > 0 {
                        line.push(';');
                    }
                    line.push_str(&v.to_string());
                }
            }
        }
        line.push('\n');
        {
            let mut so = stdout.lock();
            if so.write_all(line.as_bytes()).is_err() {
                break;
            }
            if so.flush().is_err() {
                break;
            }
        }
        let now = Instant::now();
        if let Some(until) = next.checked_duration_since(now) {
            thread::sleep(until);
            next = next.checked_add(frame_dur).unwrap_or_else(Instant::now);
        } else {
            next = Instant::now().checked_add(frame_dur).unwrap_or_else(Instant::now);
        }
    }

    audio.stop();
    rc
}
