use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

pub type Rgb = (u8, u8, u8);
type Pair = (Rgb, Rgb);

pub struct CoverWorker {
    tx: Sender<(String, String)>,
    rx: Receiver<(String, Option<Pair>)>,
    last: Option<Pair>,
    pending: Option<String>,
    key: Option<String>,
    art: Option<String>,
}

impl CoverWorker {
    pub fn new() -> CoverWorker {
        let (tx, job_rx): (Sender<(String, String)>, Receiver<(String, String)>) = mpsc::channel();
        let (res_tx, rx): (Sender<(String, Option<Pair>)>, Receiver<(String, Option<Pair>)>) =
            mpsc::channel();
        std::thread::spawn(move || {
            for (key, url) in job_rx {
                if res_tx.send((key, extract_art(&url))).is_err() {
                    break;
                }
            }
        });
        CoverWorker {
            tx,
            rx,
            last: None,
            pending: None,
            key: None,
            art: None,
        }
    }

    pub fn set_target(&mut self, key: &str, art: &str) {
        let art = art.trim();
        if art.is_empty() {
            self.clear();
            return;
        }
        if self.key.as_deref() == Some(key) && self.art.as_deref() == Some(art) {
            return;
        }
        self.key = Some(key.to_string());
        self.art = Some(art.to_string());
        self.pending = Some(key.to_string());
        let _ = self.tx.send((key.to_string(), art.to_string()));
    }

    pub fn poll(&mut self) -> bool {
        let Some(key) = self.pending.as_ref() else {
            return false;
        };
        let key = key.clone();
        let mut changed = false;
        while let Ok((got, pair)) = self.rx.try_recv() {
            if got != key {
                continue;
            }
            if pair.is_some() {
                self.last = pair;
                changed = true;
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
        while self.rx.try_recv().is_ok() {}
        self.last = None;
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
    (!st.success() || !file.exists()).then_some(())?;
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
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
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
}
