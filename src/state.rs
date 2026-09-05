use std::time::{Duration, Instant};

const WRITE_EVERY: Duration = Duration::from_millis(50);

pub struct StateWriter {
    path: Option<String>,
    last_write: Option<Instant>,
    tracker: BeatTracker,
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
            tracker: BeatTracker::new(),
            last_tick: None,
            dir_ready: false,
        }
    }

    pub fn update(&mut self, energy: f64, bass: f64, left: f64, right: f64, low: (u8, u8, u8), high: (u8, u8, u8)) {
        let now = Instant::now();
        let dt = self
            .last_tick
            .map(|t| now.duration_since(t).as_secs_f64().clamp(0.001, 1.0))
            .unwrap_or(1.0 / 60.0);
        self.last_tick = Some(now);

        let e = energy.clamp(0.0, 1.0);
        let beat = self.tracker.step(e, bass.clamp(0.0, 1.0), dt);

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
        let (lr, lg, lb) = low;
        let (hr, hg, hb) = high;
        let l = left.clamp(0.0, 1.0);
        let rr = right.clamp(0.0, 1.0);
        let body = format!(
            "color=#{:02x}{:02x}{:02x} energy={:.2} beat={:.2} color_low=#{:02x}{:02x}{:02x} color_high=#{:02x}{:02x}{:02x} bass={:.2} left={:.2} right={:.2}\n",
            r, g, b, e, beat, lr, lg, lb, hr, hg, hb, bass.clamp(0.0, 1.0), l, rr
        );
        let tmp = format!("{}.tmp", path);
        if std::fs::write(&tmp, body.as_bytes()).is_ok() {
            if std::fs::rename(&tmp, &path).is_err() {
                let _ = std::fs::remove_file(&tmp);
                self.dir_ready = false;
            }
        } else {
            self.dir_ready = false;
        }
    }
}

impl Default for StateWriter {
    fn default() -> Self {
        StateWriter::new()
    }
}

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

pub struct BeatTracker {
    avg: f64,
    peak: f64,
    prev: f64,
    favg: f64,
    fpeak: f64,
    fprev: f64,
    beat: f64,
    now: f64,
    onsets: [f64; 8],
    n: usize,
    period: f64,
    next: f64,
    misses: f32,
    cand: f64,
    cstr: u32,
}

impl BeatTracker {
    pub fn new() -> BeatTracker {
        BeatTracker {
            avg: 0.0,
            peak: 0.0,
            prev: 0.0,
            favg: 0.0,
            fpeak: 0.0,
            fprev: 0.0,
            beat: 0.0,
            now: 0.0,
            onsets: [0.0; 8],
            n: 0,
            period: 0.0,
            next: 0.0,
            misses: 0.0,
            cand: 0.0,
            cstr: 0,
        }
    }

    #[allow(dead_code)]
    pub fn period(&self) -> f64 {
        self.period
    }

    pub fn step(&mut self, full: f64, bass: f64, dt: f64) -> f64 {
        let dt = dt.clamp(0.001, 1.0);
        self.now += dt;
        let e = bass.clamp(0.0, 1.0);
        let f = full.clamp(0.0, 1.0);
        let (avg, peak, prev, b) = beat_step(e, self.avg, self.peak, self.prev, self.beat, dt);
        self.avg = avg;
        self.peak = peak;
        self.prev = prev;
        let (favg, fpeak, fprev, fb) = beat_step(f, self.favg, self.fpeak, self.fprev, self.beat, dt);
        self.favg = favg;
        self.fpeak = fpeak;
        self.fprev = fprev;
        if b == 1.0 || fb == 1.0 {
            self.push_onset();
            self.beat = 1.0;
        } else if self.period > 0.0 {
            let window = 0.12 * self.period;
            let brange = (self.peak - self.avg).max(0.05);
            let frange = (self.fpeak - self.favg).max(0.05);
            let bstrength = ((e - self.avg) / brange).clamp(0.0, 1.0);
            let fstrength = ((f - self.favg) / frange).clamp(0.0, 1.0);
            let strength = bstrength.max(fstrength);
            let loud = e.max(f);
            if self.now >= self.next - window && strength > 0.25 && loud > 0.05 {
                self.beat = 1.0;
                self.misses = 0.0;
                self.next += self.period;
            } else {
                self.beat *= (-dt * 5.0).exp();
                if self.now > self.next + window {
                    self.misses += 1.0;
                    self.next += self.period;
                    if self.misses >= 4.0 {
                        self.period = 0.0;
                    }
                }
            }
        } else {
            self.beat = b.max(fb);
        }
        self.beat.clamp(0.0, 1.0)
    }

    fn push_onset(&mut self) {
        let t = self.now;
        if self.n < 8 {
            self.onsets[self.n] = t;
            self.n += 1;
        } else {
            self.onsets.copy_within(1.., 0);
            self.onsets[7] = t;
        }
        if self.n < 5 {
            return;
        }
        if self.period > 0.0 {
            let window = 0.12 * self.period;
            if (self.next - t).abs() <= window {
                self.next = t + self.period;
                self.misses = 0.0;
            }
        }
        let Some(p) = estimate_period(&self.onsets[..self.n]) else {
            return;
        };
        if (p - self.cand).abs() / self.cand.max(1e-6) <= 0.12 {
            self.cstr += 1;
        } else {
            self.cand = p;
            self.cstr = 1;
        }
        if self.cstr < 2 {
            return;
        }
        if self.period <= 0.0 || self.misses >= 3.0 {
            self.period = self.cand;
            self.next = t + self.cand;
            self.misses = 0.0;
        }
    }
}

impl Default for BeatTracker {
    fn default() -> Self {
        BeatTracker::new()
    }
}

fn estimate_period(times: &[f64]) -> Option<f64> {
    let mut iois = [0.0f64; 8];
    let mut n = 0usize;
    for w in times.windows(2) {
        let d = w[1] - w[0];
        if d > 0.05 && n < 8 {
            iois[n] = d;
            n += 1;
        }
    }
    if n < 3 {
        return None;
    }
    for i in 1..n {
        let mut j = i;
        while j > 0 && iois[j] < iois[j - 1] {
            iois.swap(j, j - 1);
            j -= 1;
        }
    }
    let mut p = iois[n / 2].clamp(0.2, 1.5);
    while p > 0.65 {
        p /= 2.0;
    }
    while p < 0.30 {
        p *= 2.0;
    }
    let bpm = (60.0 / p).round().clamp(60.0, 200.0);
    Some(60.0 / bpm)
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
        for _ in 0..120 {
            (avg, peak, prev, beat) = beat_step(0.1, avg, peak, prev, beat, dt);
        }
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
        for _ in 0..120 {
            (avg, peak, prev, beat) = beat_step(0.0, avg, peak, prev, beat, 1.0 / 60.0);
        }
        assert!(avg < 0.01);
        (avg, peak, prev, beat) = beat_step(0.8, avg, peak, prev, beat, 1.0 / 60.0);
        assert_eq!(beat, 1.0);
        for _ in 0..120 {
            (avg, peak, prev, beat) = beat_step(0.8, avg, peak, prev, beat, 1.0 / 60.0);
        }
        assert!(beat < 0.1, "beat should decay, got {}", beat);
        (_, _, _, beat) = beat_step(0.05, 0.0, 0.0, 0.0, 0.0, 1.0 / 60.0);
        assert_eq!(beat, 0.0);
    }

    #[test]
    fn weak_kicks_still_fire() {
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
        (_, _, _, beat) = beat_step(0.05, 0.0, 0.0, 0.0, 0.0, dt);
        assert_eq!(beat, 0.0);
    }

    #[test]
    fn dense_loud_kicks_fire_without_false_hits() {
        let dt = 1.0 / 60.0;
        let mut avg = 0.0;
        let mut peak = 0.0;
        let mut prev = 0.0;
        let mut beat = 0.0;
        for _ in 0..200 {
            (avg, peak, prev, beat) = beat_step(0.5, avg, peak, prev, beat, dt);
        }
        beat = 0.0;
        let mut false_hits = 0;
        for _ in 0..120 {
            (avg, peak, prev, beat) = beat_step(0.5, avg, peak, prev, beat, dt);
            if beat == 1.0 {
                false_hits += 1;
            }
        }
        assert_eq!(false_hits, 0, "steady loud bed must not fire");
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

    fn kick_series(dt: f64, period_frames: usize, kick: f64, bed: f64, n_kicks: usize, tr: &mut BeatTracker) {
        for _ in 0..(period_frames * 2) {
            tr.step(bed, bed, dt);
        }
        for _ in 0..n_kicks {
            tr.step(kick, kick, dt);
            for _ in 1..period_frames {
                tr.step(bed, bed, dt);
            }
        }
    }

    #[test]
    fn tracker_locks_tempo_and_fills_soft_kicks() {
        let dt = 1.0 / 60.0;
        let mut tr = BeatTracker::new();
        kick_series(dt, 30, 0.7, 0.15, 8, &mut tr);
        assert!((tr.period() - 0.5).abs() < 0.05, "locks 120bpm, got {}", tr.period());
        let mut filled = 0;
        for _ in 0..4 {
            let b = tr.step(0.3, 0.3, dt);
            for _ in 1..30 {
                tr.step(0.15, 0.15, dt);
            }
            if b == 1.0 {
                filled += 1;
            }
        }
        assert!(filled >= 3, "soft kicks fire on the grid, got {}", filled);
    }

    #[test]
    fn snare_without_bass_fires() {
        let dt = 1.0 / 60.0;
        let mut tr = BeatTracker::new();
        for _ in 0..120 {
            tr.step(0.12, 0.12, dt);
        }
        let b = tr.step(0.55, 0.12, dt);
        assert_eq!(b, 1.0, "mid/high onset fires with flat bass");
        let b = tr.step(0.12, 0.12, dt);
        assert!(b < 1.0, "bed alone stays quiet");
    }

    #[test]
    fn tracker_recalibrates_on_tempo_change() {
        let dt = 1.0 / 60.0;
        let mut tr = BeatTracker::new();
        kick_series(dt, 30, 0.7, 0.15, 8, &mut tr);
        assert!((tr.period() - 0.5).abs() < 0.05);
        kick_series(dt, 24, 0.7, 0.15, 12, &mut tr);
        assert!(
            (tr.period() - 0.4).abs() < 0.05,
            "recalibrates to 150bpm, got {}",
            tr.period()
        );
    }

    #[test]
    fn tracker_unlocks_in_silence() {
        let dt = 1.0 / 60.0;
        let mut tr = BeatTracker::new();
        kick_series(dt, 30, 0.7, 0.15, 8, &mut tr);
        assert!(tr.period() > 0.0);
        for _ in 0..(60 * 4) {
            tr.step(0.12, 0.12, dt);
        }
        assert_eq!(tr.period(), 0.0, "lock drops after unsupported grid");
    }

    #[test]
    fn estimate_rounds_to_integer_bpm() {
        let steady: Vec<f64> = (0..6).map(|i| i as f64 * 0.5).collect();
        assert_eq!(estimate_period(&steady), Some(0.5));
        let jittered: Vec<f64> = (0..6).map(|i| i as f64 * 0.5 + (i as f64 % 2.0) * 0.01).collect();
        let p = estimate_period(&jittered).unwrap();
        assert!(((60.0 / p).round() - 60.0 / p).abs() < 1e-9, "whole bpm, got {}", p);
    }

    #[test]
    fn octave_wobble_keeps_grid() {
        let dt = 1.0 / 60.0;
        let mut tr = BeatTracker::new();
        for _ in 0..60 {
            tr.step(0.12, 0.12, dt);
        }
        for _ in 0..8 {
            tr.step(0.7, 0.7, dt);
            for _ in 1..30 {
                tr.step(0.12, 0.12, dt);
            }
        }
        assert!((tr.period() - 0.5).abs() < 0.05, "locked, got {}", tr.period());
        for _ in 0..6 {
            tr.step(0.6, 0.6, dt);
            for _ in 1..15 {
                tr.step(0.12, 0.12, dt);
            }
        }
        assert!(
            (tr.period() - 0.5).abs() < 0.1,
            "half-time feel keeps grid, got {}",
            tr.period()
        );
    }

    #[test]
    fn triplet_fill_keeps_grid() {
        let dt = 1.0 / 60.0;
        let mut tr = BeatTracker::new();
        for _ in 0..60 {
            tr.step(0.12, 0.12, dt);
        }
        for _ in 0..8 {
            tr.step(0.7, 0.7, dt);
            for _ in 1..30 {
                tr.step(0.12, 0.12, dt);
            }
        }
        assert!((tr.period() - 0.5).abs() < 0.05, "locked, got {}", tr.period());
        for _ in 0..12 {
            tr.step(0.6, 0.6, dt);
            for _ in 1..20 {
                tr.step(0.12, 0.12, dt);
            }
        }
        assert!(
            (tr.period() - 0.5).abs() < 0.1,
            "triplet fill holds grid, got {}",
            tr.period()
        );
    }

    #[test]
    fn lerp_midpoint() {
        assert_eq!(lerp_rgb((0, 0, 0), (255, 255, 255), 0.5), (128, 128, 128));
        assert_eq!(lerp_rgb((255, 255, 0), (255, 0, 0), 0.0), (255, 255, 0));
        assert_eq!(lerp_rgb((255, 255, 0), (255, 0, 0), 1.0), (255, 0, 0));
    }

    #[test]
    fn state_body_matches_jefetch_protocol() {
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
        w.update(0.9, 0.9, 0.9, 0.9, (0, 0, 0), (255, 255, 255));
        std::env::remove_var("SHARKVIS_NO_STATE");
    }

    #[test]
    fn writes_land_atomically() {
        let path = std::env::temp_dir().join(format!("sharkvis-atomic-{}", std::process::id()));
        let path = path.to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.tmp", path));
        let mut w = StateWriter::new();
        w.path = Some(path.clone());
        w.dir_ready = true;
        for _ in 0..5 {
            w.update(0.5, 0.4, 0.5, 0.5, (0, 0, 255), (255, 0, 0));
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("color_low=#0000ff"), "complete body, got {}", text);
        assert!(text.contains("color_high=#ff0000"), "complete body, got {}", text);
        assert!(
            !std::path::Path::new(&format!("{}.tmp", path)).exists(),
            "no temp leftovers"
        );
        let _ = std::fs::remove_file(&path);
    }
}
