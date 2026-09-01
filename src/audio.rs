use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::pulse;

struct Shared {
    head: AtomicUsize,
    tail: AtomicUsize,
    terminate: AtomicBool,
    error: Mutex<String>,
    ring: Vec<AtomicU64>,
    capacity: usize,
    mask: usize,
}

pub struct Audio {
    shared: Arc<Shared>,
    work: [Vec<f64>; 2],
    channels: usize,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn next_pow2(v: usize) -> usize {
    let mut p = 1usize;
    while p < v {
        p <<= 1;
    }
    p
}

impl Audio {
    pub fn new(capacity: usize) -> Self {
        let capacity = next_pow2(capacity);
        let shared = Shared {
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            terminate: AtomicBool::new(false),
            error: Mutex::new(String::new()),
            ring: (0..2 * capacity).map(|_| AtomicU64::new(0.0f64.to_bits())).collect(),
            capacity,
            mask: capacity - 1,
        };
        Audio {
            shared: Arc::new(shared),
            work: [vec![0.0; capacity], vec![0.0; capacity]],
            channels: 2,
            thread: None,
        }
    }

    pub fn start(&mut self, source: &str, rate: u32, channels: u32) {
        let channels = channels.clamp(1, 2);
        self.channels = channels as usize;
        let shared = self.shared.clone();
        let source = source.to_string();
        self.thread = Some(std::thread::spawn(move || {
            capture(shared, source, rate, channels);
        }));
    }

    pub fn consume(&mut self) -> (usize, Option<&[f64]>, Option<&[f64]>) {
        let sh = &self.shared;
        let head = sh.head.load(Ordering::Acquire);
        let tail = sh.tail.load(Ordering::Relaxed);
        let mut n = head - tail;
        if n > sh.capacity {
            n = sh.capacity;
        }
        if n > 0 {
            for ch in 0..2 {
                let mut j = 0;
                while j < n {
                    let t = (tail + j) & sh.mask;
                    let v = f64::from_bits(sh.ring[2 * t + ch].load(Ordering::Relaxed));
                    self.work[ch][j] = v;
                    j += 1;
                }
            }
            sh.tail.store(tail + n, Ordering::Release);
        }
        let left = Some(&self.work[0][..n]);
        let right = if self.channels > 1 {
            Some(&self.work[1][..n])
        } else {
            None
        };
        (n, left, right)
    }

    pub fn failed(&self) -> bool {
        self.shared.terminate.load(Ordering::SeqCst)
    }

    pub fn error(&self) -> String {
        self.shared.error.lock().unwrap().clone()
    }

    pub fn stop(&mut self) {
        self.shared.terminate.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn dev_name(p: &mut pulse::Pulse, source: &str) -> Option<String> {
    if source.is_empty() || source == "auto" || source == "default" {
        p.default_monitor().ok()
    } else {
        Some(source.to_string())
    }
}

fn capture(shared: Arc<Shared>, source: String, rate: u32, channels: u32) {
    let nch = channels.clamp(1, 2);
    let mut p = match pulse::Pulse::connect() {
        Ok(p) => p,
        Err(e) => {
            *shared.error.lock().unwrap() = e;
            shared.terminate.store(true, Ordering::SeqCst);
            return;
        }
    };

    let dev = match dev_name(&mut p, &source) {
        Some(d) => d,
        None => {
            *shared.error.lock().unwrap() = "pulse: could not resolve default sink monitor".to_string();
            shared.terminate.store(true, Ordering::SeqCst);
            return;
        }
    };

    let bytes_per_sec = nch as u64 * 2 * rate as u64;
    let fragsize = (5000 * bytes_per_sec / 1_000_000) as u32;

    let rec = match p.record(&dev, rate, nch as u8, fragsize) {
        Ok(r) => r,
        Err(e) => {
            *shared.error.lock().unwrap() = format!("pulse: record stream failed: {e}");
            shared.terminate.store(true, Ordering::SeqCst);
            return;
        }
    };
    let mut rec = rec;

    let frames = 512usize;
    let chunk = frames * nch as usize * 2;
    let mut raw = vec![0u8; chunk];
    let nbytes_per_frame = nch as usize * 2;

    loop {
        match rec.read(&mut raw, &shared.terminate) {
            Ok(0) => break,
            Ok(n) => {
                let nframes = n / nbytes_per_frame;
                for f in 0..nframes {
                    let head = shared.head.load(Ordering::Relaxed);
                    let tail = shared.tail.load(Ordering::Acquire);
                    if head - tail >= shared.capacity {
                        continue;
                    }
                    let i = head & shared.mask;
                    let l = i16::from_le_bytes([
                        raw[f * nbytes_per_frame],
                        raw[f * nbytes_per_frame + 1],
                    ]) as f64
                        / 32768.0;
                    let r = if nch >= 2 {
                        i16::from_le_bytes([
                            raw[f * nbytes_per_frame + 2],
                            raw[f * nbytes_per_frame + 3],
                        ]) as f64
                            / 32768.0
                    } else {
                        l
                    };
                    shared.ring[2 * i].store(l.to_bits(), Ordering::Relaxed);
                    shared.ring[2 * i + 1].store(r.to_bits(), Ordering::Relaxed);
                    shared.head.store(head + 1, Ordering::Release);
                }
            }
            Err(e) => {
                if !shared.terminate.load(Ordering::SeqCst) {
                    *shared.error.lock().unwrap() = e;
                    shared.terminate.store(true, Ordering::SeqCst);
                }
                break;
            }
        }
    }
}