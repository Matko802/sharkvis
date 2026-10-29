use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

pub type Rgb = (u8, u8, u8);
type Pair = (Rgb, Rgb);

const RETRY_AFTER: Duration = Duration::from_secs(5);

pub struct CoverWorker {
    tx: Sender<(String, String)>,
    rx: Receiver<(String, String, Option<Pair>)>,
    last: Option<Pair>,
    last_target: Option<(String, String)>,
    pending: Option<String>,
    key: Option<String>,
    art: Option<String>,
    retry_at: Option<Instant>,
}

impl CoverWorker {
    pub fn new() -> CoverWorker {
        let (tx, job_rx): (Sender<(String, String)>, Receiver<(String, String)>) = mpsc::channel();
        let (res_tx, rx): (
            Sender<(String, String, Option<Pair>)>,
            Receiver<(String, String, Option<Pair>)>,
        ) = mpsc::channel();
        std::thread::spawn(move || {
            for (key, url) in job_rx {
                let pair = extract_art(&url);
                if res_tx.send((key, url, pair)).is_err() {
                    break;
                }
            }
        });
        CoverWorker {
            tx,
            rx,
            last: None,
            last_target: None,
            pending: None,
            key: None,
            art: None,
            retry_at: None,
        }
    }

    pub fn set_target(&mut self, key: &str, art: &str) {
        let art = art.trim();
        if art.is_empty() {
            self.clear();
            return;
        }
        if self.key.as_deref() == Some(key) && self.art.as_deref() == Some(art) {
            let done = self
                .last_target
                .as_ref()
                .is_some_and(|(k, a)| k == key && a == art);
            if self.pending.is_some() || done {
                return;
            }
            if self.retry_at.is_some_and(|t| Instant::now() < t) {
                return;
            }
        }
        self.key = Some(key.to_string());
        self.art = Some(art.to_string());
        self.pending = Some(key.to_string());
        self.retry_at = None;
        let _ = self.tx.send((key.to_string(), art.to_string()));
    }

    pub fn poll(&mut self) -> bool {
        let Some(key) = self.pending.as_ref() else {
            return false;
        };
        let key = key.clone();
        let art = self.art.clone().unwrap_or_default();
        let mut changed = false;
        while let Ok((got, got_art, pair)) = self.rx.try_recv() {
            if got != key || got_art != art {
                continue;
            }
            if pair.is_some() {
                self.last = pair;
                self.last_target = Some((got, got_art));
                changed = true;
                self.retry_at = None;
            } else {
                self.retry_at = Some(Instant::now() + RETRY_AFTER);
            }
            self.pending = None;
            break;
        }
        changed
    }

    pub fn pair(&self) -> Option<Pair> {
        self.last
    }

    pub fn clear(&mut self) {
        self.pending = None;
        self.key = None;
        self.art = None;
        self.retry_at = None;
        while self.rx.try_recv().is_ok() {}
        self.last = None;
        self.last_target = None;
    }
}

impl Default for CoverWorker {
    fn default() -> Self {
        Self::new()
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn local_art(url: &str) -> Option<PathBuf> {
    let p = if let Some(rest) = url.strip_prefix("file://") {
        let rest = rest.split(['?', '#']).next().unwrap_or(rest);
        let p = percent_decode(rest);
        if p.starts_with("localhost/") {
            format!("/{}", &p["localhost/".len()..])
        } else {
            p
        }
    } else if url.starts_with('/') {
        url.to_string()
    } else {
        return None;
    };
    let p = PathBuf::from(p);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".cache")))?;
    let dir = base.join("sharkvis").join("covers");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn cache_file(url: &str) -> Option<PathBuf> {
    let dir = cache_dir()?;
    let mut h = DefaultHasher::new();
    url.hash(&mut h);
    let end = if url.ends_with(".png") || url.ends_with(".PNG") {
        "png"
    } else {
        "jpg"
    };
    Some(dir.join(format!("{:016x}.{}", h.finish(), end)))
}

fn remote_art(url: &str) -> Option<PathBuf> {
    let file = cache_file(url)?;
    if file.exists() {
        return Some(file);
    }
    let st = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--connect-timeout",
            "5",
            "-m",
            "15",
            "--max-filesize",
            "10000000",
            "--output",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .arg(&file)
        .arg(url)
        .status()
        .ok()?;
    if !st.success() || !file.exists() {
        let _ = std::fs::remove_file(&file);
        return None;
    }
    Some(file)
}

fn resolve_art(url: &str) -> Option<PathBuf> {
    if url.starts_with("http://") || url.starts_with("https://") {
        remote_art(url)
    } else {
        local_art(url)
    }
}

fn extract_art(url: &str) -> Option<Pair> {
    let path = resolve_art(url)?;
    let bytes = std::fs::read(&path).ok()?;
    let img = match image::load_from_memory(&bytes) {
        Ok(img) => img,
        Err(_) => {
            if url.starts_with("http://") || url.starts_with("https://") {
                let _ = std::fs::remove_file(&path);
            }
            return None;
        }
    };
    let img = img.to_rgba8();
    let (w, h) = (img.width().max(1), img.height().max(1));
    let step_x = (w / 64).max(1);
    let step_y = (h / 64).max(1);
    let mut out = Vec::new();
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            out.push(img.get_pixel(x, y).0);
            x += step_x;
        }
        y += step_y;
    }
    pick_pair(&out)
}

fn rgb_to_hsv(c: Rgb) -> (f32, f32, f32) {
    let r = c.0 as f32;
    let g = c.1 as f32;
    let b = c.2 as f32;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = max / 255.0;
    let d = max - min;
    if d <= 0.0 {
        return (0.0, 0.0, v);
    }
    let s = d / max;
    let hrad = if max == r {
        (g - b) / d
    } else if max == g {
        2.0 + (b - r) / d
    } else {
        4.0 + (r - g) / d
    };
    let h = (hrad / 6.0 + 1.0).fract();
    (h, s, v)
}

fn hue_dist(a: f32, b: f32) -> f32 {
    let d = (a - b).abs();
    d.min(1.0 - d) * 360.0
}

#[derive(Clone, Copy)]
struct Cluster {
    rgb: Rgb,
    count: u32,
    sat: f32,
    val: f32,
    hue: f32,
}

impl Cluster {
    fn score(&self) -> f32 {
        (self.count as f32).sqrt() * (1.0 + self.sat * 2.0) * (0.35 + self.val)
    }
}

fn rgb_dist(a: Rgb, b: Rgb) -> u32 {
    let dr = a.0.abs_diff(b.0) as u32;
    let dg = a.1.abs_diff(b.1) as u32;
    let db = a.2.abs_diff(b.2) as u32;
    dr * dr + dg * dg + db * db
}

fn clamp8(v: f32) -> u8 {
    v.clamp(0.0, 255.0).round() as u8
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let hn = (h - h.floor()) * 6.0;
    let c = v * s;
    let x = (1.0 - (hn % 2.0 - 1.0).abs()) * c;
    let m = v - c;
    let (r, g, b) = if hn <= 1.0 {
        (c, x, 0.0)
    } else if hn <= 2.0 {
        (x, c, 0.0)
    } else if hn <= 3.0 {
        (0.0, c, x)
    } else if hn <= 4.0 {
        (0.0, x, c)
    } else if hn <= 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    (
        clamp8((r + m) * 255.0),
        clamp8((g + m) * 255.0),
        clamp8((b + m) * 255.0),
    )
}

fn boost(c: Rgb) -> Rgb {
    let (h, s, v) = rgb_to_hsv(c);
    let s = (s * 1.15).max(0.72).min(1.0);
    let v = v.clamp(0.45, 0.95);
    hsv_to_rgb(h, s, v)
}

fn pick_pair(px: &[[u8; 4]]) -> Option<(Rgb, Rgb)> {
    let mut hue_bins: [Option<(f32, f32, f32, u32)>; 24] = [None; 24];
    for pix in px {
        if pix[3] < 128 {
            continue;
        }
        let (h, s, v) = rgb_to_hsv((pix[0], pix[1], pix[2]));
        if s < 0.18 || v < 0.15 {
            continue;
        }
        let idx = ((h * 24.0) as usize).min(23);
        let slot = &mut hue_bins[idx];
        if let Some(slot) = slot {
            slot.0 += pix[0] as f32;
            slot.1 += pix[1] as f32;
            slot.2 += pix[2] as f32;
            slot.3 += 1;
        } else {
            *slot = Some((pix[0] as f32, pix[1] as f32, pix[2] as f32, 1));
        }
    }

    let mut clusters = Vec::new();
    for (i, slot) in hue_bins.into_iter().enumerate() {
        if let Some((sr, sg, sb, count)) = slot {
            let rgb = (
                clamp8(sr / count as f32),
                clamp8(sg / count as f32),
                clamp8(sb / count as f32),
            );
            let (_, s, v) = rgb_to_hsv(rgb);
            clusters.push(Cluster {
                rgb,
                count,
                sat: s,
                val: v,
                hue: (i as f32 + 0.5) / 24.0,
            });
        }
    }
    clusters.sort_by(|a, b| {
        b.score()
            .partial_cmp(&a.score())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    clusters.truncate(12);
    if clusters.len() < 2 {
        return None;
    }

    let first = clusters[0];
    let mut best: Option<(f32, Cluster)> = None;
    for c in clusters.iter().skip(1) {
        let hd = hue_dist(first.hue, c.hue);
        let d = rgb_dist(first.rgb, c.rgb);
        if hd < 20.0 || d < 55_u32 * 55 {
            continue;
        }
        let score = first.score() + c.score();
        if best.is_none_or(|(old, _)| score > old) {
            best = Some((score, *c));
        }
    }
    let Some((_, second)) = best else {
        return None;
    };
    let (a, b) = (boost(first.rgb), boost(second.rgb));
    if a == b || rgb_dist(a, b) < 40_u32 * 40 {
        return None;
    }
    if lum(a) <= lum(b) {
        Some((a, b))
    } else {
        Some((b, a))
    }
}

fn lum(c: Rgb) -> f32 {
    let (r, g, b) = (c.0 as f32, c.1 as f32, c.2 as f32);
    r * 0.299 + g * 0.587 + b * 0.114
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(c: Rgb) -> [u8; 4] {
        [c.0, c.1, c.2, 255]
    }

    #[test]
    fn cover_pair_prefers_distinct_vivid_colors() {
        let got = pick_pair(&[
            px((204, 0, 0)),
            px((204, 0, 0)),
            px((0, 144, 204)),
            px((0, 144, 204)),
        ])
        .expect("valid pair");
        assert!(lum(got.0) <= lum(got.1));
        assert!(rgb_dist(got.0, got.1) > 40 * 40);
    }

    #[test]
    fn monochrome_cover_has_no_pair() {
        assert!(pick_pair(&[px((10, 10, 10)); 16]).is_none());
    }

    #[test]
    fn file_urls_decode_percent_escapes() {
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(local_art("file:///definitely-not-real%20cover.png"), None);
    }

    #[test]
    fn local_art_ignores_uri_query_and_fragment() {
        let dir = std::env::temp_dir().join(format!("sharkvis-cover-url-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cover space.png");
        std::fs::write(&path, b"x").unwrap();
        let url = format!(
            "file://{}?v=1#frag",
            path.to_string_lossy().replace(' ', "%20")
        );
        assert_eq!(local_art(&url), Some(path));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cover_worker_extracts_via_thread() {
        let dir = std::env::temp_dir().join("sharkvis-cover-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pair.png");
        let mut buf = image::RgbaImage::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                let c = if x < 10 { (204, 0, 0) } else { (0, 144, 204) };
                buf.put_pixel(x, y, image::Rgba([c.0, c.1, c.2, 255]));
            }
        }
        buf.save(&path).unwrap();
        let url = path.to_str().unwrap().to_string();

        let mut w = CoverWorker::new();
        w.set_target("k", &url);
        let mut done = false;
        for _ in 0..300 {
            if w.poll() {
                done = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(done, "worker should deliver a pair for a vivid cover");
        let pair = w.pair().unwrap();
        assert!(rgb_dist(pair.0, pair.1) >= 40 * 40);

        w.set_target("k", "");
        assert!(w.pair().is_none());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn cover_worker_retries_failed_art() {
        let url = "/definitely-not-real/sharkvis-cover.jpg";
        let mut w = CoverWorker::new();
        w.set_target("k", url);
        let mut failed = false;
        for _ in 0..300 {
            if w.poll() {
                break;
            }
            if w.pending.is_none() {
                failed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(failed, "a missing cover should fail without a pair");
        assert!(w.last.is_none());
        assert!(w.retry_at.is_some(), "failed art must schedule a retry");

        w.set_target("k", url);
        assert!(w.pending.is_none(), "no hot-loop retry before the delay");

        w.retry_at = Some(Instant::now() - Duration::from_millis(1));
        w.set_target("k", url);
        assert!(w.pending.is_some(), "art is retried after the delay");
    }

    #[test]
    fn retries_new_art_after_a_previous_cover_succeeded() {
        let dir = std::env::temp_dir().join(format!("sharkvis-cover-retry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let make = |name: &str, c: Rgb| {
            let path = dir.join(name);
            let mut buf = image::RgbaImage::new(16, 16);
            for y in 0..16 {
                for x in 0..16 {
                    let px = if x < 8 {
                        c
                    } else {
                        (255 - c.0, 255 - c.1, 255 - c.2)
                    };
                    buf.put_pixel(x, y, image::Rgba([px.0, px.1, px.2, 255]));
                }
            }
            buf.save(&path).unwrap();
            path
        };
        let first = make("first.png", (204, 0, 0));
        let late = dir.join("late.png");

        let mut w = CoverWorker::new();
        w.set_target("first", first.to_str().unwrap());
        let mut done = false;
        for _ in 0..300 {
            if w.poll() {
                done = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(done, "the first cover should load");
        let old = w.pair();
        assert!(old.is_some());

        w.set_target("second", late.to_str().unwrap());
        for _ in 0..300 {
            w.poll();
            if w.pending.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(w.last.is_some(), "the old pair is kept while retrying");
        assert!(w.retry_at.is_some(), "the missing art must be retried");

        make("late.png", (0, 90, 204));
        w.retry_at = Some(Instant::now() - Duration::from_millis(1));
        w.set_target("second", late.to_str().unwrap());
        assert!(w.pending.is_some(), "a new track retries after a success");

        let mut done = false;
        for _ in 0..300 {
            if w.poll() {
                done = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(done, "the late cover should load on retry");
        assert!(w.pair().is_some());
        assert_ne!(w.pair(), old, "the new cover replaces the old pair");
        std::fs::remove_dir_all(&dir).ok();
    }
}
