use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::pulse;

const BLOCK_FRAMES: usize = 512;

struct Shared {
    head: AtomicUsize,
    tail: AtomicUsize,
    terminate: AtomicBool,
    error: Mutex<String>,
    ring: Vec<AtomicUsize>,
    blocks: Vec<Mutex<Vec<f64>>>,
    mask: usize,
}

pub struct Audio {
    shared: Arc<Shared>,
    left: Vec<f64>,
    right: Vec<f64>,
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
    pub fn new(max_frames: usize) -> Self {
        let max_frames = max_frames.max(BLOCK_FRAMES);
        let nblocks = next_pow2((max_frames / BLOCK_FRAMES).max(2) + 2);
        let shared = Shared {
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            terminate: AtomicBool::new(false),
            error: Mutex::new(String::new()),
            ring: (0..nblocks)
                .map(|_| AtomicUsize::new(usize::MAX))
                .collect(),
            blocks: (0..nblocks)
                .map(|_| Mutex::new(vec![0.0; BLOCK_FRAMES * 2]))
                .collect(),
            mask: nblocks - 1,
        };
        Audio {
            shared: Arc::new(shared),
            left: vec![0.0; max_frames],
            right: vec![0.0; max_frames],
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
        let cap = self.left.len() / BLOCK_FRAMES;
        let mut total = 0usize;

        loop {
            let head = sh.head.load(Ordering::Acquire);
            let tail = sh.tail.load(Ordering::Relaxed);
            if head == tail || total >= cap {
                break;
            }
            let slot = tail & sh.mask;
            let n = sh.ring[slot].load(Ordering::Relaxed);
            if n == usize::MAX {
                break;
            }
            let b = sh.blocks[slot].lock().unwrap();
            let base = total * BLOCK_FRAMES;
            let mut j = 0;
            for i in 0..n {
                self.left[base + i] = b[j];
                self.right[base + i] = b[j + 1];
                j += 2;
            }
            drop(b);
            let _ = sh.ring[slot].store(usize::MAX, Ordering::Relaxed);
            sh.tail.fetch_add(1, Ordering::Release);
            total += 1;
        }

        let n = total * BLOCK_FRAMES;
        let left = if n > 0 { Some(&self.left[..n]) } else { None };
        let right = if self.channels > 1 && n > 0 {
            Some(&self.right[..n])
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
            *shared.error.lock().unwrap() =
                "pulse: could not resolve default sink monitor".to_string();
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

    let nbytes_per_frame = nch as usize * 2;
    let mut raw = vec![0u8; BLOCK_FRAMES * nbytes_per_frame];
    let mut staged = vec![0.0f64; BLOCK_FRAMES * 2];
    let mut staged_cnt = 0usize;

    loop {
        match rec.read(&mut raw, &shared.terminate) {
            Ok(0) => break,
            Ok(n) => {
                let nframes = n / nbytes_per_frame;
                for f in 0..nframes {
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
                    if staged_cnt >= BLOCK_FRAMES {
                        break;
                    }
                    staged[staged_cnt * 2] = l;
                    staged[staged_cnt * 2 + 1] = r;
                    staged_cnt += 1;
                }

                if staged_cnt >= BLOCK_FRAMES {
                    let head = shared.head.load(Ordering::Relaxed);
                    let tail = shared.tail.load(Ordering::Acquire);
                    if head - tail < shared.blocks.len() {
                        let slot = head & shared.mask;
                        let mut b = shared.blocks[slot].lock().unwrap();
                        b.copy_from_slice(&staged);
                        let _ = shared.ring[slot].store(BLOCK_FRAMES, Ordering::Relaxed);
                        shared.head.fetch_add(1, Ordering::Release);
                    }
                    staged_cnt = 0;
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
