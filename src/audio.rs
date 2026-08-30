use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use libpulse_binding::callbacks::ListResult;
use libpulse_binding::context::introspect::{ServerInfo, SinkInfo};
use libpulse_binding::context::{FlagSet, State};
use libpulse_binding::def::BufferAttr;
use libpulse_binding::mainloop::standard::{IterateResult, Mainloop};
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;

const PA_APP: &str = "sharkvis";
const PA_STREAM: &str = "sharkvis spectrum";

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

#[derive(Default)]
struct MonQuery {
    default_sink: Option<String>,
    name: String,
    done: bool,
}

fn default_sink_monitor() -> Option<String> {
    let mut ml = Mainloop::new()?;
    let mut ctx = libpulse_binding::context::Context::new(&mut ml, PA_APP)?;
    if ctx.connect(None, FlagSet::NOFLAGS, None).is_err() {
        return None;
    }

    let q = Rc::new(RefCell::new(MonQuery::default()));
    let mut server_op: Option<libpulse_binding::operation::Operation<dyn FnMut(&ServerInfo)>> = None;
    let mut sink_op: Option<
        libpulse_binding::operation::Operation<dyn FnMut(ListResult<&SinkInfo>)>,
    > = None;

    for _ in 0..1000 {
        match ml.iterate(true) {
            IterateResult::Err(_) | IterateResult::Quit(_) => break,
            _ => {}
        }
        match ctx.get_state() {
            State::Ready => {
                if server_op.is_none() {
                    let q2 = q.clone();
                    server_op = Some(ctx.introspect().get_server_info(move |si: &ServerInfo| {
                        let mut qq = q2.borrow_mut();
                        qq.default_sink = si
                            .default_sink_name
                            .as_ref()
                            .map(|s| s.to_string());
                        if qq.default_sink.is_none() {
                            qq.done = true;
                        }
                    }));
                } else if sink_op.is_none() {
                    let sink_name = q.borrow().default_sink.clone();
                    if let Some(name) = sink_name {
                        let q3 = q.clone();
                        sink_op = Some(ctx.introspect().get_sink_info_by_name(
                            &name,
                            move |r: ListResult<&SinkInfo>| {
                                let mut qq = q3.borrow_mut();
                                match r {
                                    ListResult::Item(info) => {
                                        qq.name = info
                                            .monitor_source_name
                                            .as_ref()
                                            .map(|s| s.to_string())
                                            .unwrap_or_default();
                                        qq.done = true;
                                    }
                                    _ => qq.done = true,
                                }
                            },
                        ));
                    }
                }
            }
            State::Failed | State::Terminated => break,
            _ => {}
        }
        if q.borrow().done {
            break;
        }
    }

    let res = Rc::try_unwrap(q).ok().map(|q| q.into_inner());
    drop(server_op);
    drop(sink_op);
    ctx.disconnect();
    if let Some(q) = res {
        if !q.name.is_empty() {
            return Some(q.name);
        }
    }
    None
}

fn capture(shared: Arc<Shared>, source: String, rate: u32, channels: u32) {
    let nch = channels.clamp(1, 2);
    let spec = Spec {
        format: Format::S16NE,
        rate,
        channels: nch as u8,
    };

    let dev = if source.is_empty() || source == "auto" || source == "default" {
        default_sink_monitor()
    } else {
        Some(source)
    };

    let bytes_per_sec = nch as u64 * 2 * rate as u64;
    let fragsize = (5000 * bytes_per_sec / 1_000_000) as u32;
    let attr = BufferAttr {
        maxlength: u32::MAX,
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize,
    };

    let simple = match libpulse_simple_binding::Simple::new(
        None,
        PA_APP,
        Direction::Record,
        dev.as_deref(),
        PA_STREAM,
        &spec,
        None,
        Some(&attr),
    ) {
        Ok(s) => s,
        Err(e) => {
            *shared.error.lock().unwrap() = format!("pulse: pa_simple_new failed: {}", e);
            shared.terminate.store(true, Ordering::SeqCst);
            return;
        }
    };

    let frames = 512usize;
    let chunk = frames * nch as usize * 2;
    let mut raw = vec![0u8; chunk];
    let nbytes_per_frame = nch as usize * 2;

    while !shared.terminate.load(Ordering::SeqCst) {
        if let Err(e) = simple.read(&mut raw) {
            *shared.error.lock().unwrap() = format!("pulse: pa_simple_read failed: {}", e);
            shared.terminate.store(true, Ordering::SeqCst);
            break;
        }
        for f in 0..frames {
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
}