use std::sync::OnceLock;

use crate::term::term_winsize_px;

pub const LOGO_PNG: &[u8] = include_bytes!("../Logo/paperust.png");
const MAX_IMG_PIXELS: usize = 4096 * 4096;
const MAX_INFLATE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogoKind {
    Kitty,
    Blocks,
}

static KIND: OnceLock<LogoKind> = OnceLock::new();

pub fn detect() -> LogoKind {
    *KIND.get_or_init(|| {
        if std::env::var_os("SHARKVIS_FORCE_KITTY").is_some() {
            return LogoKind::Kitty;
        }
        let term = std::env::var("TERM").unwrap_or_default().to_ascii_lowercase();
        let prog = std::env::var("TERM_PROGRAM").unwrap_or_default().to_ascii_lowercase();
        let contains = |s: &str, keys: &[&str]| keys.iter().any(|k| s.contains(k));
        if std::env::var_os("KITTY_WINDOW_ID").is_some()
            || contains(&prog, &["kitty", "wezterm", "ghostty", "warpterm"])
            || contains(&term, &["kitty", "wezterm", "ghostty"])
            || std::env::var_os("WEZTERM_PANE").is_some()
            || std::env::var_os("WEZTERM_EXECUTABLE").is_some()
            || std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some()
        {
            LogoKind::Kitty
        } else {
            LogoKind::Blocks
        }
    })
}

struct Image {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

static IMG: OnceLock<Option<Image>> = OnceLock::new();

fn image() -> Option<&'static Image> {
    IMG.get_or_init(|| decode_png(LOGO_PNG)).as_ref()
}

static TRANSMITTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn append(out: &mut Vec<u8>, cap: usize, bytes: &[u8]) {
    if out.len() >= cap {
        return;
    }
    let room = cap - out.len();
    let take = bytes.len().min(room);
    out.extend_from_slice(&bytes[..take]);
}

static TRUECOLOR: OnceLock<bool> = OnceLock::new();

fn truecolor_ok() -> bool {
    *TRUECOLOR.get_or_init(|| {
        let ct = std::env::var("COLORTERM").unwrap_or_default().to_ascii_lowercase();
        ct.contains("truecolor") || ct.contains("24bit")
    })
}

/// Aspect ratio of a terminal cell (height / width). Uses TIOCGWINSZ pixel
/// dimensions when the terminal reports them, otherwise falls back to the
/// common ~2:1 cell shape.
fn cell_aspect(rows: u32, cols: u32) -> f64 {
    let (xpx, ypx) = term_winsize_px(1);
    if xpx > 0 && ypx > 0 && rows > 0 && cols > 0 {
        let cell_w = xpx as f64 / cols as f64;
        let cell_h = ypx as f64 / rows as f64;
        if cell_w > 0.0 && cell_h > 0.0 {
            return cell_h / cell_w;
        }
    }
    2.0
}

fn box_cells(img: &Image, pw: usize, avail: usize, aspect: f64) -> (usize, usize) {
    // Columns needed per row of cells so the rendered block keeps the source
    // image proportion once real cell shapes (aspect) are accounted for.
    let target = img.w as f64 / img.h as f64 * aspect;
    let mut r = avail;
    let mut c = (r as f64 * target).round() as usize;
    if c > pw {
        c = pw;
        r = (c as f64 / target).round() as usize;
    }
    if r < 1 {
        r = 1;
    }
    if c < 1 {
        c = 1;
    }
    (c, r)
}

pub fn draw(out: &mut Vec<u8>, cap: usize, rows: u32, cols: u32, pw: usize) {
    let kind = detect();
    let some = image();
    if some.is_none() {
        return;
    }
    let img = some.unwrap();
    let avail = (rows as usize).saturating_sub(22);
    if avail < 2 {
        return;
    }
    let avail = avail.min(12);
    let aspect = cell_aspect(rows, cols);
    let (c, r) = box_cells(img, pw, avail, aspect);
    let y0 = rows as usize - r + 1;
    if y0 + r - 1 > rows as usize || c > cols as usize {
        return;
    }
    match kind {
        LogoKind::Kitty => draw_kitty(out, cap, img, c, r, y0),
        LogoKind::Blocks => draw_blocks(out, cap, img, c, r, y0),
    }
}

fn draw_kitty(out: &mut Vec<u8>, cap: usize, _img: &Image, c: usize, r: usize, y0: usize) {
    if !TRANSMITTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let mut b64 = Vec::with_capacity(((LOGO_PNG.len() + 2) / 3) * 4);
        base64(LOGO_PNG, &mut b64);
        let mut start = 0usize;
        let first: &[u8] = b"\x1b_Gf=100,i=1,q=2,m=1;";
        let mid: &[u8] = b"\x1b_Gm=1;";
        let last: &[u8] = b"\x1b_Gm=0;";
        let end = b"\x1b\\";
        while start < b64.len() {
            let piece = (b64.len() - start).min(4096);
            let hdr = if start == 0 {
                first
            } else if start + piece >= b64.len() {
                last
            } else {
                mid
            };
            append(out, cap, hdr);
            append(out, cap, &b64[start..start + piece]);
            append(out, cap, end);
            start += piece;
        }
    }
    let hdr = format!("\x1b[{};1H\x1b_Ga=p,i=1,p=1,c={},r={},z=-1,C=1,q=2\x1b\\", y0, c, r);
    append(out, cap, hdr.as_bytes());
}

fn draw_blocks(out: &mut Vec<u8>, cap: usize, img: &Image, c: usize, r: usize, y0: usize) {
    let src_w = img.w / c;
    let src_h = img.h / r;
    if src_w < 1 || src_h < 1 {
        return;
    }
    let tc = truecolor_ok();
    let mut esc = Vec::with_capacity(64);
    for i in 0..r {
        esc.clear();
        esc.extend_from_slice(b"\x1b[0m");
        esc.extend_from_slice(format!("\x1b[{};1H", y0 + i).as_bytes());
        append(out, cap, &esc);
        let ty0 = i * src_h;
        let ym = ty0 + src_h / 2;
        for j in 0..c {
            let sx = j * src_w;
            let (fr, fg, fb, fa) = avg_region(img, sx, ty0, src_w, ym - ty0);
            let (br, bg, bb, ba) = avg_region(img, sx, ym, src_w, ty0 + src_h - ym);
            if fa == 0 && ba == 0 {
                append(out, cap, b"\x1b[0m ");
                continue;
            }
            append(out, cap, b"\x1b[0m");
            if fa > 0 {
                append_fg(out, cap, fr, fg, fb, tc);
            }
            if ba > 0 {
                append_bg(out, cap, br, bg, bb, tc);
            }
            append(out, cap, "\u{2580}".as_bytes());
        }
    }
    append(out, cap, b"\x1b[0m");
}

fn append_fg(out: &mut Vec<u8>, cap: usize, r: u8, g: u8, b: u8, tc: bool) {
    if tc {
        append(out, cap, format!("\x1b[38;2;{};{};{}m", r, g, b).as_bytes());
    } else {
        append(out, cap, b"\x1b[38;5;");
        append(out, cap, format!("{}", to_256(r, g, b)).as_bytes());
        append(out, cap, b"m");
    }
}

fn append_bg(out: &mut Vec<u8>, cap: usize, r: u8, g: u8, b: u8, tc: bool) {
    if tc {
        append(out, cap, format!("\x1b[48;2;{};{};{}m", r, g, b).as_bytes());
    } else {
        append(out, cap, b"\x1b[48;5;");
        append(out, cap, format!("{}", to_256(r, g, b)).as_bytes());
        append(out, cap, b"m");
    }
}

fn to_256(r: u8, g: u8, b: u8) -> u8 {
    16 + 36 * ((r as u32 * 6) / 256) as u8 + 6 * ((g as u32 * 6) / 256) as u8 + ((b as u32 * 6) / 256) as u8
}

fn avg_region(img: &Image, x: usize, y: usize, w: usize, h: usize) -> (u8, u8, u8, u64) {
    let mut tr = 0u64;
    let mut tg = 0u64;
    let mut tb = 0u64;
    let mut ta = 0u64;
    for yy in y..y + h {
        for xx in x..x + w {
            let idx = (yy.min(img.h - 1) * img.w + xx.min(img.w - 1)) * 4;
            let a = img.rgba[idx + 3] as u64;
            if a == 0 {
                continue;
            }
            tr += img.rgba[idx] as u64 * a;
            tg += img.rgba[idx + 1] as u64 * a;
            tb += img.rgba[idx + 2] as u64 * a;
            ta += a;
        }
    }
    if ta == 0 {
        return (0, 0, 0, 0);
    }
    (
        (tr / ta) as u8,
        (tg / ta) as u8,
        (tb / ta) as u8,
        ta,
    )
}

pub fn delete_seq() -> Vec<u8> {
    if detect() == LogoKind::Kitty && TRANSMITTED.swap(false, std::sync::atomic::Ordering::Relaxed) {
        b"\x1b_Ga=d,d=I,i=1\x1b\\".to_vec()
    } else {
        Vec::new()
    }
}

pub fn delete_now() {
    let seq = delete_seq();
    if !seq.is_empty() {
        use std::io::Write;
        let stdout = std::io::stdout();
        let mut so = stdout.lock();
        let _ = so.write_all(&seq);
        let _ = so.flush();
    }
}

fn base64(src: &[u8], out: &mut Vec<u8>) {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in src.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18) as usize & 63]);
        out.push(T[(n >> 12) as usize & 63]);
        if chunk.len() > 1 {
            out.push(T[(n >> 6) as usize & 63]);
        } else {
            out.push(b'=');
        }
        if chunk.len() > 2 {
            out.push(T[n as usize & 63]);
        } else {
            out.push(b'=');
        }
    }
}

fn png_u32(b: &[u8], i: usize) -> u32 {
    ((b[i] as u32) << 24) | ((b[i + 1] as u32) << 16) | ((b[i + 2] as u32) << 8) | (b[i + 3] as u32)
}

type Decode<T> = Result<T, ()>;

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0, acc: 0, nbits: 0 }
    }
    fn bit(&mut self) -> u32 {
        if self.nbits == 0 {
            if self.pos >= self.data.len() {
                return 0;
            }
            self.acc = self.data[self.pos] as u64;
            self.pos += 1;
            self.nbits = 8;
        }
        let v = (self.acc & 1) as u32;
        self.acc >>= 1;
        self.nbits -= 1;
        v
    }
    fn bits(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            v |= self.bit() << i;
        }
        v
    }
    fn byte_align(&mut self) {
        self.acc = 0;
        self.nbits = 0;
    }
}

struct Huf {
    counts: [u16; 16],
    first: [u16; 16],
    index: [u16; 16],
    symbols: [u16; 288],
}

impl Huf {
    fn build(lengths: &[u8]) -> Decode<Huf> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            if l > 0 {
                if l as usize >= counts.len() {
                    return Err(());
                }
                counts[l as usize] += 1;
            }
        }
        let mut code = 0u16;
        let mut first = [0u16; 16];
        let mut index = [0u16; 16];
        for bits in 1..16usize {
            code = (code + counts[bits - 1]) << 1;
            first[bits] = code;
            index[bits] = index[bits - 1] + counts[bits - 1];
        }
        let mut next = index;
        let mut symbols = [0u16; 288];
        for (i, &l) in lengths.iter().enumerate() {
            if l > 0 {
                if i >= symbols.len() {
                    return Err(());
                }
                let li = l as usize;
                let slot = next[li] as usize;
                if slot >= symbols.len() {
                    return Err(());
                }
                symbols[slot] = i as u16;
                next[li] += 1;
            }
        }
        Ok(Huf { counts, first, index, symbols })
    }

    fn decode(&self, br: &mut BitReader) -> Decode<u16> {
        let mut code = 0u32;
        for bits in 1..16usize {
            code = (code << 1) | br.bit();
            let n = self.counts[bits];
            if n == 0 {
                continue;
            }
            let f = self.first[bits] as u32;
            if code < f + n as u32 {
                return Ok(self.symbols[self.index[bits] as usize + (code - f) as usize]);
            }
        }
        Err(())
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn fixed_huf() -> Decode<(Huf, Huf)> {
    let mut lit = [0u8; 288];
    for i in 0..144 {
        lit[i] = 8;
    }
    for i in 144..256 {
        lit[i] = 9;
    }
    for i in 256..280 {
        lit[i] = 7;
    }
    for i in 280..288 {
        lit[i] = 8;
    }
    let dist = [5u8; 32];
    Ok((Huf::build(&lit)?, Huf::build(&dist)?))
}

const CLEN_ORDER: [u8; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn inflate(zlib: &[u8]) -> Decode<Vec<u8>> {
    if zlib.len() < 6 {
        return Err(());
    }
    let data = &zlib[2..zlib.len() - 4]; // skip 2-byte zlib header, drop 4-byte adler32
    let mut br = BitReader::new(data);
    let mut out: Vec<u8> = Vec::new();
    loop {
        let bfinal = br.bit();
        let btype = br.bits(2);
        match btype {
            0 => {
                br.byte_align();
                let len = br.bits(16) as usize;
                let _nlen = br.bits(16);
                if br.pos + len > data.len() {
                    return Err(());
                }
                if out.len() + len > MAX_INFLATE_BYTES {
                    return Err(());
                }
                out.extend_from_slice(&data[br.pos..br.pos + len]);
                br.pos += len;
            }
            1 | 2 => {
                let (lit_h, dist_h) = if btype == 1 {
                    fixed_huf()?
                } else {
                    let hlit = br.bits(5) as usize + 257;
                    let hdist = br.bits(5) as usize + 1;
                    let hclen = br.bits(4) as usize + 4;
                    let mut clens = [0u8; 19];
                    for i in 0..hclen {
                        clens[CLEN_ORDER[i] as usize] = br.bits(3) as u8;
                    }
                    let clen_h = Huf::build(&clens[..])?;
                    let mut lens = Vec::with_capacity(hlit + hdist);
                    while lens.len() < hlit + hdist {
                        let sym = clen_h.decode(&mut br)?;
                        match sym {
                            0..=15 => lens.push(sym as u8),
                            16 => {
                                let prev = *lens.last().ok_or(())?;
                                let rep = 3 + br.bits(2);
                                for _ in 0..rep {
                                    lens.push(prev);
                                }
                            }
                            17 => {
                                let rep = 3 + br.bits(3);
                                for _ in 0..rep {
                                    lens.push(0);
                                }
                            }
                            18 => {
                                let rep = 11 + br.bits(7);
                                for _ in 0..rep {
                                    lens.push(0);
                                }
                            }
                            _ => return Err(()),
                        }
                    }
                    let lit_h = Huf::build(&lens[..hlit])?;
                    let dist_l = &lens[hlit..];
                    let dist_h = Huf::build(dist_l)?;
                    (lit_h, dist_h)
                };
                loop {
                    let sym = lit_h.decode(&mut br)?;
                    match sym {
                        0..=255 => {
                            if out.len() >= MAX_INFLATE_BYTES {
                                return Err(());
                            }
                            out.push(sym as u8);
                        }
                        256 => break,
                        _ => {
                            let li = (sym - 257) as usize;
                            if li >= LEN_BASE.len() {
                                return Err(());
                            }
                            let len = LEN_BASE[li] as usize + br.bits(LEN_EXTRA[li] as u32) as usize;
                            let dsym = dist_h.decode(&mut br)? as usize;
                            if dsym >= DIST_BASE.len() {
                                return Err(());
                            }
                            let dist = DIST_BASE[dsym] as usize + br.bits(DIST_EXTRA[dsym] as u32) as usize;
                            if dist > out.len() {
                                return Err(());
                            }
                            if out.len() + len > MAX_INFLATE_BYTES {
                                return Err(());
                            }
                            for _ in 0..len {
                                let v = out[out.len() - dist];
                                out.push(v);
                            }
                        }
                    }
                }
            }
            _ => return Err(()),
        }
        if bfinal != 0 {
            break;
        }
    }
    Ok(out)
}

fn decode_png(data: &[u8]) -> Option<Image> {
    if data.len() < 8 || data[0..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        return None;
    }
    let mut pos = 8usize;
    let mut w = 0usize;
    let mut h = 0usize;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 1u8;
    let mut idat: Vec<u8> = Vec::new();
    let mut palette: Vec<u8> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut have_ihdr = false;
    while pos + 12 <= data.len() {
        let len = png_u32(data, pos) as usize;
        let ctype = &data[pos + 4..pos + 8];
        let dstart = pos + 8;
        let dend = dstart + len;
        if dend + 4 > data.len() {
            return None;
        }
        match ctype {
            b"IHDR" => {
                if len != 13 {
                    return None;
                }
                w = png_u32(data, dstart) as usize;
                h = png_u32(data, dstart + 4) as usize;
                bit_depth = data[dstart + 8];
                color_type = data[dstart + 9];
                interlace = data[dstart + 12];
                if w == 0 || h == 0 || w * h > MAX_IMG_PIXELS {
                    return None;
                }
                have_ihdr = true;
            }
            b"PLTE" => palette.extend_from_slice(&data[dstart..dend]),
            b"tRNS" => trns.extend_from_slice(&data[dstart..dend]),
            b"IDAT" => {
                if !have_ihdr {
                    return None;
                }
                idat.extend_from_slice(&data[dstart..dend]);
            }
            b"IEND" => break,
            _ => {}
        }
        pos = dend + 4;
    }
    if !have_ihdr || idat.is_empty() || interlace != 0 {
        return None;
    }
    let channels = match color_type {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return None,
    };
    if bit_depth == 0 {
        return None;
    }
    let ok_depth = match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        2 => matches!(bit_depth, 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        4 => matches!(bit_depth, 8 | 16),
        6 => matches!(bit_depth, 8 | 16),
        _ => false,
    };
    if !ok_depth {
        return None;
    }
    let raw = inflate(&idat).ok()?;
    let bpp = core::cmp::max(1, channels * bit_depth as usize / 8);
    let bits_pp = channels as usize * bit_depth as usize;
    let stride = (w * bits_pp + 7) / 8;
    if raw.len() < h * (stride + 1) {
        return None;
    }
    let mut scan: Vec<u8> = vec![0u8; h * stride];
    for y in 0..h {
        let f = raw[y * (stride + 1)];
        let srow = y * stride;
        if f == 0 {
            scan[srow..srow + stride].copy_from_slice(&raw[y * (stride + 1) + 1..y * (stride + 1) + 1 + stride]);
        } else {
            for x in 0..stride {
                let left = if x >= bpp { scan[srow + x - bpp] } else { 0 };
                let up = if y > 0 { scan[srow - stride + x] } else { 0 };
                let diag = if y > 0 && x >= bpp { scan[srow - stride + x - bpp] } else { 0 };
                let filt = raw[y * (stride + 1) + 1 + x];
                scan[srow + x] = match f {
                    1 => (filt as u16 + left as u16) as u8,
                    2 => (filt as u16 + up as u16) as u8,
                    3 => (filt as u16 + ((left as u16 + up as u16) / 2)) as u8,
                    4 => {
                        let pr = paeth(left as i32, up as i32, diag as i32);
                        (filt as i32 + pr).clamp(0, 255) as u8
                    }
                    _ => return None,
                };
            }
        }
    }
    let mut rgba = vec![0u8; w * h * 4];
    match color_type {
        0 => {
            for y in 0..h {
                for x in 0..w {
                    let (val, a) = gray_sample(&scan, stride, y, x, bit_depth, &trns);
                    let idx = (y * w + x) * 4;
                    rgba[idx] = val;
                    rgba[idx + 1] = val;
                    rgba[idx + 2] = val;
                    rgba[idx + 3] = a;
                }
            }
        }
        2 => {
            for y in 0..h {
                for x in 0..w {
                    let i = y * stride + x * 3 * byte_pp(bit_depth);
                    let r = sample16(&scan, i, bit_depth);
                    let g = sample16(&scan, i + byte_pp(bit_depth), bit_depth);
                    let b = sample16(&scan, i + 2 * byte_pp(bit_depth), bit_depth);
                    let a = if trns.len() >= 6 {
                        let tr = ((trns[0] as u32) << 8) | trns[1] as u32;
                        let tg = ((trns[2] as u32) << 8) | trns[3] as u32;
                        let tb = ((trns[4] as u32) << 8) | trns[5] as u32;
                        let pr = if bit_depth == 16 { (r as u32) << 8 | r as u32 } else { r as u32 };
                        let pg = if bit_depth == 16 { (g as u32) << 8 | g as u32 } else { g as u32 };
                        let pb = if bit_depth == 16 { (b as u32) << 8 | b as u32 } else { b as u32 };
                        if tr == pr && tg == pg && tb == pb {
                            0
                        } else {
                            255
                        }
                    } else {
                        255
                    };
                    let idx = (y * w + x) * 4;
                    rgba[idx] = r;
                    rgba[idx + 1] = g;
                    rgba[idx + 2] = b;
                    rgba[idx + 3] = a;
                }
            }
        }
        3 => {
            if palette.is_empty() {
                return None;
            }
            for y in 0..h {
                for x in 0..w {
                    let pi = palette_index(&scan, stride, x, bit_depth) as usize * 3;
                    if pi + 2 >= palette.len() {
                        return None;
                    }
                    let idx = (y * w + x) * 4;
                    rgba[idx] = palette[pi];
                    rgba[idx + 1] = palette[pi + 1];
                    rgba[idx + 2] = palette[pi + 2];
                    rgba[idx + 3] = if pi / 3 < trns.len() { trns[pi / 3] } else { 255 };
                }
            }
        }
        4 => {
            for y in 0..h {
                for x in 0..w {
                    let i = y * stride + x * 2 * byte_pp(bit_depth);
                    let g = sample16(&scan, i, bit_depth);
                    let a = sample16(&scan, i + byte_pp(bit_depth), bit_depth);
                    let idx = (y * w + x) * 4;
                    rgba[idx] = g;
                    rgba[idx + 1] = g;
                    rgba[idx + 2] = g;
                    rgba[idx + 3] = a;
                }
            }
        }
        6 => {
            for y in 0..h {
                for x in 0..w {
                    let i = y * stride + x * 4 * byte_pp(bit_depth);
                    let idx = (y * w + x) * 4;
                    rgba[idx] = sample16(&scan, i, bit_depth);
                    rgba[idx + 1] = sample16(&scan, i + byte_pp(bit_depth), bit_depth);
                    rgba[idx + 2] = sample16(&scan, i + 2 * byte_pp(bit_depth), bit_depth);
                    rgba[idx + 3] = sample16(&scan, i + 3 * byte_pp(bit_depth), bit_depth);
                }
            }
        }
        _ => return None,
    }
    Some(Image { w, h, rgba })
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn byte_pp(bit_depth: u8) -> usize {
    if bit_depth >= 8 {
        bit_depth as usize / 8
    } else {
        1
    }
}

fn sample16(scan: &[u8], i: usize, bit_depth: u8) -> u8 {
    if bit_depth >= 8 {
        scan.get(i).copied().unwrap_or(0)
    } else {
        0
    }
}

fn gray_sample(
    scan: &[u8],
    stride: usize,
    y: usize,
    x: usize,
    bit_depth: u8,
    trns: &[u8],
) -> (u8, u8) {
    let idx = y * stride + sub_index(x, bit_depth);
    let v = if bit_depth >= 8 {
        sample16(scan, idx, bit_depth)
    } else {
        let byte = scan.get(idx).copied().unwrap_or(0);
        let shift = 8 - bit_depth - (x % (8 / bit_depth as usize)) as u8 * bit_depth;
        (byte >> shift) & ((1u8 << bit_depth) - 1)
    };
    let a = if trns.len() >= 2 && bit_depth < 16 {
        let t = if trns.len() >= 2 { (trns[0] as u16) << 8 | trns[1] as u16 } else { 0 };
        let tv = (t >> 8) as u8;
        if v == tv && trns.len() >= 2 {
            0
        } else {
            255
        }
    } else {
        255
    };
    (v, a)
}

fn sub_index(x: usize, bit_depth: u8) -> usize {
    if bit_depth >= 8 {
        x * (bit_depth as usize / 8)
    } else {
        (x * bit_depth as usize) >> 3
    }
}

fn palette_index(scan: &[u8], stride: usize, x: usize, bit_depth: u8) -> usize {
    let _ = stride;
    let idx = sub_index(x, bit_depth);
    if bit_depth >= 8 {
        scan.get(idx).copied().unwrap_or(0) as usize
    } else {
        let byte = scan.get(idx).copied().unwrap_or(0) as usize;
        let shift = 8 - bit_depth - (x % (8 / bit_depth as usize)) as u8 * bit_depth;
        (byte >> shift) & ((1u8 << bit_depth) - 1) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_logo() {
        let img = decode_png(LOGO_PNG).expect("logo should decode");
        assert_eq!(img.w, 586);
        assert_eq!(img.h, 586);
        assert_eq!(img.rgba.len(), 586 * 586 * 4);
        let non_zero = img.rgba.iter().filter(|&&b| b != 0).take(16).count();
        assert!(non_zero > 0, "logo should have visible pixels");
    }

    #[test]
    fn base64_roundtrip() {
        let src = b"hello world";
        let mut b64 = Vec::new();
        base64(src, &mut b64);
        assert_eq!(&b64[..], b"aGVsbG8gd29ybGQ=");
    }

    // Force the non-image (Blocks) path with 256-color output and check the
    // rendered cell geometry stays proportionate and uses safe escapes.
    #[test]
    fn blocks_mode_layout() {
        for (k, _) in std::env::vars_os() {
            if k == "SHARKVIS_FORCE_KITTY"
                || k == "KITTY_WINDOW_ID"
                || k == "TERM_PROGRAM"
                || k == "WEZTERM_PANE"
                || k == "WEZTERM_EXECUTABLE"
                || k == "GHOSTTY_RESOURCES_DIR"
            {
                std::env::remove_var(k);
            }
        }
        std::env::set_var("COLORTERM", "");
        std::env::set_var("TERM", "xterm");
        assert_eq!(detect(), LogoKind::Blocks, "test must run the Blocks path");
        assert!(!truecolor_ok(), "no truecolor expected in test");

        let mut out = Vec::new();
        draw(&mut out, 1 << 20, 40, 100, 28);

        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("38;5;"), "Blocks mode should use 256-color escapes");
        assert!(!s.contains("38;2;"), "Blocks mode must not emit raw truecolor SGR");
        assert!(s.contains('\u{2580}'), "Blocks mode should draw half-block cells");

        // With a square logo and cell aspect 2.0, 12 rows fit and should take
        // ~24 columns -> bottom-aligned block, not a squashed one.
        for i in 29..40 {
            assert!(
                s.contains(&format!("\x1b[{};1H", i)),
                "logo row {} not drawn",
                i
            );
        }
    }

    #[test]
    fn box_cells_aspect() {
        let img = Image { w: 100, h: 100, rgba: vec![0; 100 * 100 * 4] };
        // Square cells: square image -> square block.
        assert_eq!(box_cells(&img, 80, 12, 1.0), (12, 12));
        // Tall cells (aspect 2.0): need 2 columns per row to stay square.
        assert_eq!(box_cells(&img, 80, 12, 2.0), (24, 12));
        // Wide image on a ~2:1 cell stays within the panel width.
        assert_eq!(box_cells(&img, 28, 12, 2.0), (24, 12));
    }
}