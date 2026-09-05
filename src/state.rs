//! Live state file for external consumers (e.g. jefetch).
//!
//! While running, sharkvis publishes its current look and groove ~20x per
//! second so other tools can follow along:
//!
//! ```text
//! color=#ff8800 energy=0.42 beat=1.00
//! ```
//!
//! * `color` — gradient color lerped by the current energy (`#rrggbb`)
//! * `energy` — mean bar height, `0..1`
//! * `beat` — beat envelope, `0..1` (`1` on onset, decays after)
//!
//! Location: `$XDG_RUNTIME_DIR/sharkvis/state`, falling back to
//! `/tmp/sharkvis-$UID.state`. Readers treat files older than ~1s as stale.
//! All I/O failures are silent by design; set `SHARKVIS_NO_STATE=1` to
//! disable publishing entirely.

use std::time::{Duration, Instant};

const WRITE_EVERY: Duration = Duration::from_millis(50);

pub struct StateWriter {
    path: Option<String>,
    last_write: Option<Instant>,
    avg: f64,
    peak: f64,
    prev: f64,
    beat: f64,
    last_tick: Option<Instant>,
    dir_ready: bool,
}

impl StateWriter {
    pub fn new() -> StateWriter {
        let path = if std::env::var_os("SHARKVIS_NO_STATE").is_some() {
            None
        } else {
            Some(state_path())
        };
        StateWriter {
            path,
            last_write: None,
            avg: 0.0,
            peak: 0.0,
            prev: 0.0,
            beat: 0.0,
            last_tick: None,
            dir_ready: false,
        }
    }

    /// Feed the current levels: overall `energy` (volume, drives color)
    /// plus `bass` (drives the beat detector). Tracks the beat envelope
    /// and publishes the state file (throttled).
    pub fn update(&mut self, energy: f64, bass: f64, low: (u8, u8, u8), high: (u8, u8, u8)) {
        let now = Instant::now();
        let dt = self
            .last_tick
            .map(|t| now.duration_since(t).as_secs_f64().clamp(0.001, 1.0))
            .unwrap_or(1.0 / 60.0);
        self.last_tick = Some(now);

        let e = energy.clamp(0.0, 1.0);
        let (avg, peak, prev, beat) =
            beat_step(bass.clamp(0.0, 1.0), self.avg, self.peak, self.prev, self.beat, dt);
        self.avg = avg;
        self.peak = peak;
        self.prev = prev;
        self.beat = beat;

        let path = match &self.path {
            Some(p) => p.clone(),
            None => return,
        };
        if self.last_write.is_some_and(|t| now.duration_since(t) < WRITE_EVERY) {
            return;
        }
        self.last_write = Some(now);

        if !self.dir_ready {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                if std::fs::create_dir_all(parent).is_ok() {
                    self.dir_ready = true;
                }
            } else {
                self.dir_ready = true;
            }
        }
        let (r, g, b) = lerp_rgb(low, high, e as f32);
        let body = format!(
            "color=#{:02x}{:02x}{:02x} energy={:.2} beat={:.2}\n",
            r, g, b, e, self.beat
        );
        if std::fs::write(&path, body.as_bytes()).is_err() {
            // Retry the mkdir next time (e.g. runtime dir appeared late).
            self.dir_ready = false;
        }
    }
}

impl Default for StateWriter {
    fn default() -> Self {
        StateWriter::new()
    }
}

/// One onset-tracking step. Returns `(avg, peak, prev, beat)`.
///
/// Fixed thresholds miss kicks whenever the loudness changes (dense
/// masters sit high, breakdowns low), so the onset is normalized by the
/// recent dynamic range instead: `strength = (bass - avg) / (peak - avg)`.
/// Anything above ~0.55 of the recent range counts as a kick, at any
/// volume — but only on a rising edge, so sustained loud beds stay quiet.
/// `peak` is a slow-release ceiling, `avg` a slow follower.
pub fn beat_step(energy: f64, avg: f64, peak: f64, prev: f64, beat: f64, dt: f64) -> (f64, f64, f64, f64) {
    let avg = avg + (energy - avg) * (1.0 - (-dt * 1.5).exp());
    let peak = energy.max(peak * (-dt * 0.8).exp());
    let range = (peak - avg).max(0.05);
    let strength = ((energy - avg) / range).clamp(0.0, 1.0);
    let beat = if strength > 0.55 && energy > 0.05 && energy > prev {
        1.0
    } else {
        beat * (-dt * 5.0).exp()
    };
    (avg, peak, energy, beat.clamp(0.0, 1.0))
}

pub fn lerp_rgb(lo: (u8, u8, u8), hi: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t + 0.5) as u8;
    (mix(lo.0, hi.0), mix(lo.1, hi.1), mix(lo.2, hi.2))
}

pub fn state_path() -> String {
    if let Ok(p) = std::env::var("SHARKVIS_STATE") {
        if !p.trim().is_empty() {
            return p;
        }
    }
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        if !rt.is_empty() {
            return format!("{}/sharkvis/state", rt.trim_end_matches('/'));
        }
    }
    let uid = unsafe { libc::getuid() };
    format!("/tmp/sharkvis-{}.state", uid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_fires_on_repeating_kicks() {
        let dt = 1.0 / 60.0;
        let mut avg = 0.0;
        let mut peak = 0.0;
        let mut prev = 0.0;
        let mut beat = 0.0;
        // Settle into quiet room tone.
        for _ in 0..120 {
            (avg, peak, prev, beat) = beat_step(0.1, avg, peak, prev, beat, dt);
        }
        // Three kicks: sharp attack, exponential decay, back to room tone.
        let mut fires = 0;
        for _ in 0..3 {
            for e in [0.35, 0.3, 0.25, 0.2, 0.16, 0.13] {
                (avg, peak, prev, beat) = beat_step(e, avg, peak, prev, beat, dt);
                if beat == 1.0 {
                    fires += 1;
                }
            }
            for _ in 0..24 {
                (avg, peak, prev, beat) = beat_step(0.1, avg, peak, prev, beat, dt);
            }
        }
        assert!(fires >= 3, "each kick should fire, got {} fires", fires);
    }

    #[test]
    fn beat_fires_on_onset_then_decays() {
        let mut avg = 0.0;
        let mut peak = 0.0;
        let mut prev = 0.0;
        let mut beat = 0.0;
        // Silence settles the follower.
        for _ in 0..120 {
            (avg, peak, prev, beat) = beat_step(0.0, avg, peak, prev, beat, 1.0 / 60.0);
        }
        assert!(avg < 0.01);
        // Sudden energy is a beat.
        (avg, peak, prev, beat) = beat_step(0.8, avg, peak, prev, beat, 1.0 / 60.0);
        assert_eq!(beat, 1.0);
        // It decays without new onsets.
        for _ in 0..120 {
            (avg, peak, prev, beat) = beat_step(0.8, avg, peak, prev, beat, 1.0 / 60.0);
        }
        assert!(beat < 0.1, "beat should decay, got {}", beat);
        // Quiet floor never fires.
        (_, _, _, beat) = beat_step(0.05, 0.0, 0.0, 0.0, 0.0, 1.0 / 60.0);
        assert_eq!(beat, 0.0);
    }

    #[test]
    fn weak_kicks_still_fire() {
        // Quiet bass-heavy material: small absolute jumps must fire.
        let dt = 1.0 / 60.0;
        let mut avg = 0.0;
        let mut peak = 0.0;
        let mut prev = 0.0;
        let mut beat = 0.0;
        for _ in 0..120 {
            (avg, peak, prev, beat) = beat_step(0.13, avg, peak, prev, beat, dt);
        }
        (_, _, _, beat) = beat_step(0.26, avg, peak, prev, beat, dt);
        assert_eq!(beat, 1.0, "0.13 -> 0.26 onset must fire");
        // But steady hiss at the same level must not.
        (_, _, _, beat) = beat_step(0.05, 0.0, 0.0, 0.0, 0.0, dt);
        assert_eq!(beat, 0.0);
    }

    #[test]
    fn dense_loud_kicks_fire_without_false_hits() {
        // Compressed master: kicks ride on a hot bed, small headroom.
        let dt = 1.0 / 60.0;
        let mut avg = 0.0;
        let mut peak = 0.0;
        let mut prev = 0.0;
        let mut beat = 0.0;
        for _ in 0..200 {
            (avg, peak, prev, beat) = beat_step(0.5, avg, peak, prev, beat, dt);
        }
        beat = 0.0;
        // Steady loud bed alone must not fire.
        let mut false_hits = 0;
        for _ in 0..120 {
            (avg, peak, prev, beat) = beat_step(0.5, avg, peak, prev, beat, dt);
            if beat == 1.0 {
                false_hits += 1;
            }
        }
        assert_eq!(false_hits, 0, "steady loud bed must not fire");
        // Kicks on top must fire every time.
        let mut fires = 0;
        for _ in 0..3 {
            for e in [0.65, 0.6, 0.55, 0.52] {
                (avg, peak, prev, beat) = beat_step(e, avg, peak, prev, beat, dt);
                if beat == 1.0 {
                    fires += 1;
                }
            }
            for _ in 0..20 {
                (avg, peak, prev, beat) = beat_step(0.5, avg, peak, prev, beat, dt);
            }
        }
        assert!(fires >= 3, "each loud kick should fire, got {}", fires);
    }

    #[test]
    fn lerp_midpoint() {
        assert_eq!(lerp_rgb((0, 0, 0), (255, 255, 255), 0.5), (128, 128, 128));
        assert_eq!(lerp_rgb((255, 255, 0), (255, 0, 0), 0.0), (255, 255, 0));
        assert_eq!(lerp_rgb((255, 255, 0), (255, 0, 0), 1.0), (255, 0, 0));
    }

    #[test]
    fn state_body_matches_jefetch_protocol() {
        // jefetch parses `color=#rrggbb energy=0..1 beat=0..1`.
        let (r, g, b) = lerp_rgb((255, 255, 0), (255, 0, 0), 0.5);
        let body = format!("color=#{:02x}{:02x}{:02x} energy={:.2} beat={:.2}\n", r, g, b, 0.5, 1.0);
        assert!(body.starts_with("color=#"), "got {}", body);
        assert!(body.contains("energy=0.50"), "got {}", body);
        assert!(body.contains("beat=1.00"), "got {}", body);
    }

    #[test]
    fn state_path_override() {
        std::env::set_var("SHARKVIS_STATE", "/tmp/test-sharkvis-state");
        assert_eq!(state_path(), "/tmp/test-sharkvis-state");
        std::env::remove_var("SHARKVIS_STATE");
        assert!(!state_path().is_empty());
    }

    #[test]
    fn disabled_writer_is_noop() {
        std::env::set_var("SHARKVIS_NO_STATE", "1");
        let mut w = StateWriter::new();
        assert!(w.path.is_none());
        w.update(0.9, 0.9, (0, 0, 0), (255, 255, 255));
        std::env::remove_var("SHARKVIS_NO_STATE");
    }
}
