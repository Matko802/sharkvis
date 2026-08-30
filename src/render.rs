use crate::config::DEFAULT_GLYPHS;

#[derive(PartialEq, Clone, Copy)]
pub enum RenderMode {
    Bars,
    Wave,
    Lissajous,
}

pub struct Renderer {
    pub rows: usize,
    pub cols: usize,
    pub bar_width: usize,
    pub bar_spacing: usize,
    pub num_bars: usize,
    pub color_256: bool,
    pub grad_lo: u32,
    pub grad_hi: u32,
    pub mode: RenderMode,
    x_off: usize,
    prev: Vec<u8>,
    rowbuf: Vec<u8>,
    db_x0: usize,
    db_y0: usize,
    db_x1: usize,
    db_y1: usize,
    row_col: Vec<Vec<u8>>,
    barstr: [Vec<u8>; 9],
    spacestr: Vec<u8>,
    barstr_bw: usize,
    glyphs: Vec<Vec<u8>>,
    wave_buf: Vec<f64>,
    wave_cap: usize,
    wave_pos: usize,
    wave_filled: usize,
    wave_spc: usize,
    lj_l: Vec<f64>,
    lj_r: Vec<f64>,
    lj_cap: usize,
    lj_pos: usize,
    lj_filled: usize,
    lj_spc: usize,
    lj_win: usize,
    stereo_in: bool,
    lj_glow: Vec<u8>,
    sc_yrow: Vec<i64>,
    sc_lo: Vec<i64>,
    sc_hi: Vec<i64>,
}

#[derive(Default)]
struct ColorState {
    active: bool,
    col: Vec<u8>,
}

struct Out<'a> {
    buf: &'a mut Vec<u8>,
    cap: usize,
}

impl Out<'_> {
    fn s(&mut self, bytes: &[u8]) {
        let room = self.cap - self.buf.len();
        if room > 0 {
            let take = bytes.len().min(room);
            self.buf.extend_from_slice(&bytes[..take]);
        }
    }
    fn u(&mut self, mut v: u32) {
        let mut d = [0u8; 16];
        let mut n = 0;
        loop {
            d[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
            if v == 0 {
                break;
            }
        }
        let room = self.cap - self.buf.len();
        let take = n.min(room);
        let start = self.buf.len();
        self.buf.resize(start + take, 0);
        for k in 0..take {
            self.buf[start + k] = d[n - 1 - k];
        }
    }
}

fn seek_cell(r: u32, c: u32, out: &mut Out) {
    out.s(b"\x1b[");
    out.u(r + 1);
    out.s(b";");
    out.u(c + 1);
    out.s(b"H");
}

impl Renderer {
    fn render_glyph(&self, gi: i32) -> &[u8] {
        if gi <= 0 || self.glyphs.is_empty() {
            return b" ";
        }
        let n = self.glyphs.len() as i32;
        let mut idx = ((gi - 1) as f64 * (n - 1) as f64 / 7.0 + 0.5) as i32;
        if idx < 0 {
            idx = 0;
        } else if idx >= n {
            idx = n - 1;
        }
        &self.glyphs[idx as usize]
    }

    pub fn set_glyphs(&mut self, src: Option<&[u8]>) {
        let src = match src {
            Some(s) if !s.is_empty() => s,
            _ => DEFAULT_GLYPHS,
        };
        self.glyphs.clear();
        let mut p = 0;
        while p < src.len() && self.glyphs.len() < 64 {
            let c = src[p];
            let seq = if c < 0x80 {
                1
            } else if (c & 0xE0) == 0xC0 {
                2
            } else if (c & 0xF0) == 0xE0 {
                3
            } else {
                4
            };
            let mut dst = Vec::with_capacity(seq);
            let mut i = 0;
            while i < seq && i < 7 && p < src.len() {
                dst.push(src[p]);
                p += 1;
                i += 1;
            }
            self.glyphs.push(dst);
        }
        if self.glyphs.is_empty() {
            self.glyphs.push(vec![b' ']);
        }
        self.barstr_bw = 0;
    }

    fn bar_color(&self, from_bottom: u32, rows: u32) -> Vec<u8> {
        let lo_r = (self.grad_lo >> 16) & 0xff;
        let lo_g = (self.grad_lo >> 8) & 0xff;
        let lo_b = self.grad_lo & 0xff;
        let hi_r = (self.grad_hi >> 16) & 0xff;
        let hi_g = (self.grad_hi >> 8) & 0xff;
        let hi_b = self.grad_hi & 0xff;
        let frac = if rows > 1 {
            from_bottom as f64 / (rows - 1) as f64
        } else {
            0.0
        };
        let mut cr = (lo_r as f64 + (hi_r as f64 - lo_r as f64) * frac + 0.5) as u32;
        let mut cg = (lo_g as f64 + (hi_g as f64 - lo_g as f64) * frac + 0.5) as u32;
        let mut cb = (lo_b as f64 + (hi_b as f64 - lo_b as f64) * frac + 0.5) as u32;
        if cr > 255 {
            cr = 255;
        }
        if cg > 255 {
            cg = 255;
        }
        if cb > 255 {
            cb = 255;
        }
        let mut o = Vec::with_capacity(20);
        if self.color_256 {
            let idx = 16 + 36 * ((cr * 6) / 256) + 6 * ((cg * 6) / 256) + (cb * 6) / 256;
            o.extend_from_slice(b"\x1b[38;5;");
            o.extend_from_slice(idx.to_string().as_bytes());
            o.push(b'm');
        } else {
            o.extend_from_slice(b"\x1b[38;2;");
            o.extend_from_slice(cr.to_string().as_bytes());
            o.push(b';');
            o.extend_from_slice(cg.to_string().as_bytes());
            o.push(b';');
            o.extend_from_slice(cb.to_string().as_bytes());
            o.push(b'm');
        }
        o
    }

    fn row_colors(&mut self) {
        for y in 0..self.rows {
            self.row_col[y] = self.bar_color((self.rows - 1 - y) as u32, self.rows as u32);
        }
    }

    fn emit_color_state(st: &mut ColorState, pre: &[u8], out: &mut Out) {
        if st.active && st.col.as_slice() == pre {
            return;
        }
        out.s(pre);
        st.col.clear();
        st.col.extend_from_slice(pre);
        st.active = true;
    }

    fn emit_cell(&mut self, y: usize, x: usize, gi: i32, st: &mut ColorState, out: &mut Out) {
        let idx = y * self.cols + x;
        if self.prev[idx] == gi as u8 {
            return;
        }
        self.prev[idx] = gi as u8;
        seek_cell(y as u32, x as u32, out);
        if gi == 0 {
            out.s(b" ");
        } else {
            Self::emit_color_state(st, &self.row_col[y], out);
            out.s(self.render_glyph(gi));
        }
    }

    pub fn new(
        rows: usize,
        cols: usize,
        bar_width: usize,
        bar_spacing: usize,
        num_bars: usize,
    ) -> Self {
        let mut r = Renderer {
            rows,
            cols,
            bar_width: if bar_width == 0 { 1 } else { bar_width },
            bar_spacing,
            num_bars,
            color_256: false,
            grad_lo: 0xff0000u32,
            grad_hi: 0x00ff00u32,
            mode: RenderMode::Bars,
            x_off: 0,
            prev: vec![0xFF; rows * cols],
            rowbuf: vec![0; cols],
            db_x0: 0,
            db_y0: 0,
            db_x1: if cols > 0 { cols - 1 } else { 0 },
            db_y1: if rows > 0 { rows - 1 } else { 0 },
            row_col: vec![Vec::new(); rows],
            barstr: Default::default(),
            spacestr: Vec::new(),
            barstr_bw: 0,
            glyphs: Vec::new(),
            wave_buf: Vec::new(),
            wave_cap: 0,
            wave_pos: 0,
            wave_filled: 0,
            wave_spc: 1,
            lj_l: Vec::new(),
            lj_r: Vec::new(),
            lj_cap: 0,
            lj_pos: 0,
            lj_filled: 0,
            lj_spc: 1,
            lj_win: 0,
            stereo_in: false,
            lj_glow: vec![0; rows * cols],
            sc_yrow: Vec::new(),
            sc_lo: Vec::new(),
            sc_hi: Vec::new(),
        };
        r.set_glyphs(None);
        r
    }

    pub fn resize(&mut self, rows: usize, cols: usize, num_bars: usize) {
        self.rows = rows;
        self.cols = cols;
        self.num_bars = num_bars;
        self.barstr_bw = 0;
        self.prev = vec![0xFF; rows * cols];
        self.lj_glow = vec![0; rows * cols];
        self.row_col = vec![Vec::new(); rows];
        self.rowbuf = vec![0; cols];
        self.db_x0 = 0;
        self.db_y0 = 0;
        self.db_x1 = if cols > 0 { cols - 1 } else { 0 };
        self.db_y1 = if rows > 0 { rows - 1 } else { 0 };
    }

    pub fn set_offset(&mut self, x_off: usize) {
        if self.x_off == x_off {
            return;
        }
        self.x_off = x_off;
        self.prev = vec![0xFF; self.rows * self.cols];
        self.lj_glow = vec![0; self.rows * self.cols];
        self.rowbuf = vec![0; self.cols];
        self.db_x0 = 0;
        self.db_y0 = 0;
        self.db_x1 = if self.cols > 0 { self.cols - 1 } else { 0 };
        self.db_y1 = if self.rows > 0 { self.rows - 1 } else { 0 };
    }

    pub fn set_mode(&mut self, m: RenderMode) {
        if self.mode == m {
            return;
        }
        self.mode = m;
        self.clear();
        if m == RenderMode::Lissajous {
            self.lj_glow.fill(0);
        }
    }

    pub fn mode_parse(name: &str) -> RenderMode {
        if name == "wave" {
            RenderMode::Wave
        } else if name == "lissajous" {
            RenderMode::Lissajous
        } else {
            RenderMode::Bars
        }
    }

    pub fn set_wave(&mut self, sample_rate: u32) {
        let cap = if sample_rate > 0 {
            sample_rate as usize * 2 / 3
        } else {
            48000 * 2 / 3
        };
        let mut cap = cap;
        if cap < 4096 {
            cap = 4096;
        }
        let mut spc = sample_rate as usize / 2000;
        if spc < 1 {
            spc = 1;
        }
        let mut lj_spc = sample_rate as usize / 800;
        if lj_spc < 1 {
            lj_spc = 1;
        }
        let mut lj_win = sample_rate as usize / 20;
        if lj_win < 256 {
            lj_win = 256;
        }
        if self.wave_cap == cap {
            self.wave_spc = spc;
            self.lj_spc = lj_spc;
            self.lj_win = lj_win;
            return;
        }
        self.wave_buf = vec![0.0; cap];
        self.lj_l = vec![0.0; cap];
        self.lj_r = vec![0.0; cap];
        self.wave_cap = cap;
        self.wave_pos = 0;
        self.wave_filled = 0;
        self.wave_spc = spc;
        self.lj_cap = cap;
        self.lj_pos = 0;
        self.lj_filled = 0;
        self.lj_spc = lj_spc;
        self.lj_win = lj_win;
    }

    pub fn feed(&mut self, left: Option<&[f64]>, right: Option<&[f64]>, n: usize) {
        if self.mode != RenderMode::Wave && self.mode != RenderMode::Lissajous {
            return;
        }
        if self.wave_cap == 0 || n == 0 {
            return;
        }
        self.stereo_in = right.is_some();
        let left = left.unwrap_or(&[]);
        for i in 0..n.min(left.len()) {
            let mut v = left[i];
            if let Some(r) = right {
                v = (v + r[i]) * 0.5;
            }
            self.wave_buf[self.wave_pos] = v;
            self.lj_l[self.lj_pos] = left[i];
            self.lj_r[self.lj_pos] = match right {
                Some(r) => r[i],
                None => left[i],
            };
            self.wave_pos = (self.wave_pos + 1) % self.wave_cap;
            if self.wave_filled < self.wave_cap {
                self.wave_filled += 1;
            }
            self.lj_pos = (self.lj_pos + 1) % self.lj_cap;
            if self.lj_filled < self.lj_cap {
                self.lj_filled += 1;
            }
        }
    }

    pub fn clear(&mut self) {
        self.prev.fill(0xFF);
        self.db_x0 = 0;
        self.db_y0 = 0;
        self.db_x1 = if self.cols > 0 { self.cols - 1 } else { 0 };
        self.db_y1 = if self.rows > 0 { self.rows - 1 } else { 0 };
    }

    fn build_barstrings(&mut self) {
        let mut bw = if self.bar_width == 0 { 1 } else { self.bar_width };
        if bw > 8 {
            bw = 8;
        }
        if self.barstr_bw == bw {
            return;
        }
        self.barstr_bw = bw;
        for gi in 0..=8usize {
            let mut s = Vec::with_capacity(bw * 3);
            for _ in 0..bw {
                s.extend_from_slice(self.render_glyph(gi as i32));
            }
            self.barstr[gi] = s;
        }
        self.spacestr = vec![b' '; bw];
    }

    fn draw_bars(
        &mut self,
        left: &[f64],
        right: Option<&[f64]>,
        nbars: usize,
        per_ch_l: usize,
        x_start: usize,
        region_w: usize,
        out: &mut Out,
    ) {
        let rows = self.rows;
        let cols = self.cols;
        if rows == 0 || region_w == 0 {
            return;
        }

        let bw = if self.bar_width == 0 { 1 } else { self.bar_width };
        let mut step = bw + self.bar_spacing;
        if step == 0 {
            step = 1;
        }

        let used = nbars * step;
        let lead = if used < region_w { (region_w - used) / 2 } else { 0 };
        let mut region_end = x_start + region_w;
        if region_end > cols {
            region_end = cols;
        }

        self.build_barstrings();

        let mut st = ColorState::default();

        for y in 0..rows {
            let fb = rows - 1 - y;
            let mut skip = 0usize;
            let mut wrote = false;
            let mut color_on = false;
            for b in 0..nbars {
                let col = x_start + lead + b * step;
                if col >= region_end {
                    break;
                }
                let (src, vi) = if b < per_ch_l {
                    (left, per_ch_l - 1 - b)
                } else {
                    match right {
                        Some(r) => (r, b - per_ch_l),
                        None => continue,
                    }
                };
                let mut v = src[vi];
                if !(v > 0.0) {
                    v = 0.0;
                } else if v > 1.0 {
                    v = 1.0;
                }
                let h = v * rows as f64;

                let mut frac = h - fb as f64;
                if !(frac > 0.0) {
                    frac = 0.0;
                } else if frac > 1.0 {
                    frac = 1.0;
                }
                let mut gi = (frac * 8.0 + 0.9999) as i32;
                if gi < 0 {
                    gi = 0;
                }
                if gi > 8 {
                    gi = 8;
                }

                let idx = y * cols + col;
                if self.prev[idx] == gi as u8 {
                    skip += step;
                    continue;
                }
                self.prev[idx] = gi as u8;
                let mut wvis = region_end - col;
                if wvis > bw {
                    wvis = bw;
                }
                for w in 1..wvis {
                    self.prev[idx + w] = gi as u8;
                }

                if !wrote {
                    seek_cell(y as u32, col as u32, out);
                    wrote = true;
                } else if skip > 0 {
                    out.s(b"\x1b[");
                    out.u(skip as u32);
                    out.s(b"C");
                }
                skip = 0;

                if gi > 0 {
                    if !color_on {
                        Self::emit_color_state(&mut st, &self.row_col[y], out);
                        color_on = true;
                    }
                    if wvis == bw {
                        out.s(&self.barstr[gi as usize]);
                    } else {
                        for _ in 0..wvis {
                            out.s(self.render_glyph(gi));
                        }
                    }
                } else {
                    if wvis == bw {
                        out.s(&self.spacestr);
                    } else {
                        for _ in 0..wvis {
                            out.s(b" ");
                        }
                    }
                }

                if self.bar_spacing != 0 && b + 1 < nbars && col + step < region_end {
                    if self.bar_spacing == 1 {
                        out.s(b" ");
                    } else {
                        out.s(b"\x1b[");
                        out.u(self.bar_spacing as u32);
                        out.s(b"C");
                    }
                }
            }
        }

        for col in 0..region_w {
            let in_bar = if col >= lead {
                let t = col - lead;
                t / step < nbars && t % step < bw
            } else {
                false
            };
            if in_bar {
                continue;
            }
            let abs_col = x_start + col;
            for y in 0..rows {
                let idx = y * cols + abs_col;
                if self.prev[idx] == 0xFF {
                    continue;
                }
                self.emit_cell(y, abs_col, 0, &mut st, out);
            }
        }
    }

    fn emit_row(
        &mut self,
        y: usize,
        x_start: usize,
        region_w: usize,
        tgt: &[u8],
        st: &mut ColorState,
        out: &mut Out,
    ) {
        let mut skip = 0usize;
        let mut wrote = false;
        let mut color_on = false;
        for c in 0..region_w {
            let gi = tgt[c];
            let idx = y * self.cols + x_start + c;
            if gi == self.prev[idx] {
                skip += 1;
                continue;
            }
            self.prev[idx] = gi;
            if !wrote {
                seek_cell(y as u32, (x_start + c) as u32, out);
                wrote = true;
            } else if skip > 0 {
                out.s(b"\x1b[");
                out.u(skip as u32);
                out.s(b"C");
            }
            skip = 0;
            if gi > 0 {
                if !color_on {
                    Self::emit_color_state(st, &self.row_col[y], out);
                    color_on = true;
                }
                out.s(self.render_glyph(gi as i32));
            } else {
                out.s(b" ");
            }
        }
    }

    fn draw_wave(&mut self, x_start: usize, region_w: usize, out: &mut Out) {
        if self.wave_cap == 0 || self.rows < 3 || region_w == 0 {
            return;
        }
        let ncol = region_w.min(4096);
        if ncol == 0 {
            return;
        }
        if self.sc_yrow.len() < ncol {
            self.sc_yrow.resize(ncol, 0);
            self.sc_lo.resize(ncol, 0);
            self.sc_hi.resize(ncol, 0);
        }
        let spc = if self.wave_spc == 0 { 1 } else { self.wave_spc };
        let center = (self.rows - 1) as f64 * 0.5;
        let height = (self.rows - 2) as f64 * 0.5;

        for c in 0..ncol {
            let off = (region_w - 1 - c) * spc;
            if off >= self.wave_filled {
                self.sc_yrow[c] = -1;
                self.sc_lo[c] = self.rows as i64;
                self.sc_hi[c] = -1;
                continue;
            }
            let idx = (self.wave_pos + self.wave_cap - 1 - off) % self.wave_cap;
            let mut v = self.wave_buf[idx];
            if v < -1.0 {
                v = -1.0;
            } else if v > 1.0 {
                v = 1.0;
            }
            self.sc_yrow[c] = (center - v * height + 0.5) as i64;
        }

        for c in 0..ncol {
            let cur = self.sc_yrow[c];
            if cur < 0 {
                self.sc_lo[c] = -1;
                self.sc_hi[c] = -1;
                continue;
            }
            let mut l = cur;
            let mut h = cur;
            if c + 1 < ncol && self.sc_yrow[c + 1] >= 0 {
                let nxt = self.sc_yrow[c + 1];
                if nxt < l {
                    l = nxt;
                }
                if nxt > h {
                    h = nxt;
                }
            } else if c + 1 == ncol && c > 0 && self.sc_yrow[c - 1] >= 0 {
                let nxt = self.sc_yrow[c - 1];
                if nxt < l {
                    l = nxt;
                }
                if nxt > h {
                    h = nxt;
                }
            }
            self.sc_lo[c] = l;
            self.sc_hi[c] = h;
        }

        let mut st = ColorState::default();
        let mut cy0 = self.rows;
        let mut cy1 = 0usize;
        for c in 0..ncol {
            if self.sc_hi[c] >= 0 && self.sc_lo[c] <= self.sc_hi[c] {
                let l = self.sc_lo[c] as usize;
                let h = self.sc_hi[c] as usize;
                if l < cy0 {
                    cy0 = l;
                }
                if h > cy1 {
                    cy1 = h;
                }
            }
        }
        let uy0 = cy0.min(self.db_y0);
        let uy1 = cy1.max(self.db_y1);
        if uy1 >= uy0 {
            for y in uy0..=uy1 {
                for c in 0..ncol {
                    self.rowbuf[c] =
                        if (y as i64) >= self.sc_lo[c] && (y as i64) <= self.sc_hi[c] {
                            8
                        } else {
                            0
                        };
                }
                let row = self.rowbuf[..ncol].to_vec();
                self.emit_row(y, x_start, ncol, &row, &mut st, out);
            }
        }
        self.db_y0 = cy0;
        self.db_y1 = cy1;
    }

    fn set_beam(&mut self, x: i64, y: i64) {
        if x >= 0 && y >= 0 && (x as usize) < self.cols && (y as usize) < self.rows {
            self.lj_glow[y as usize * self.cols + x as usize] = 255;
        }
    }

    fn beam_line(&mut self, mut x0: i64, mut y0: i64, x1: i64, y1: i64) {
        let dx = if x1 > x0 { x1 - x0 } else { x0 - x1 };
        let dy = if y1 > y0 { y1 - y0 } else { y0 - y1 };
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx - dy;
        loop {
            self.set_beam(x0, y0);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                x0 += sx;
            }
            if e2 < dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn draw_lissajous(&mut self, x_start: usize, region_w: usize, out: &mut Out) {
        if self.lj_cap == 0 || self.rows < 3 || region_w < 4 {
            return;
        }
        let rows = self.rows;
        let cols = self.cols;

        self.lj_glow.fill(0);

        let mut cx0 = cols;
        let mut cy0 = rows;
        let mut cx1 = 0usize;
        let mut cy1 = 0usize;
        let mut n = self.lj_filled;
        if n > self.lj_win {
            n = self.lj_win;
        }
        if n > 1 {
            let delay = if self.lj_spc == 0 { 1 } else { self.lj_spc };
            let cx = x_start as f64 + (region_w - 1) as f64 * 0.5;
            let cy = (rows - 1) as f64 * 0.5;
            let sxc = (region_w - 1) as f64 * 0.5;
            let syc = (rows - 1) as f64 * 0.5;
            let mut px = -1i64;
            let mut py = -1i64;
            for i in 0..n {
                let idx = (self.lj_pos + self.lj_cap - n + i) % self.lj_cap;
                let mut l = self.lj_l[idx];
                let mut r = self.lj_r[idx];
                if !self.stereo_in {
                    let idx2 = (idx + self.lj_cap - delay) % self.lj_cap;
                    r = self.lj_l[idx2];
                }
                if l < -1.0 {
                    l = -1.0;
                } else if l > 1.0 {
                    l = 1.0;
                }
                if r < -1.0 {
                    r = -1.0;
                } else if r > 1.0 {
                    r = 1.0;
                }
                let xx = (cx + l * sxc + 0.5) as i64;
                let yy = (cy - r * syc + 0.5) as i64;
                if xx < x_start as i64
                    || xx >= (x_start + region_w) as i64
                    || yy < 0
                    || yy >= rows as i64
                {
                    px = -1;
                    py = -1;
                    continue;
                }
                if px >= 0 && py >= 0 {
                    self.beam_line(px, py, xx, yy);
                } else {
                    self.set_beam(xx, yy);
                }
                let uxx = xx as usize;
                let uyy = yy as usize;
                if uxx < cx0 {
                    cx0 = uxx;
                }
                if uxx > cx1 {
                    cx1 = uxx;
                }
                if uyy < cy0 {
                    cy0 = uyy;
                }
                if uyy > cy1 {
                    cy1 = uyy;
                }
                px = xx;
                py = yy;
            }
        }

        let mut st = ColorState::default();
        let ux0 = cx0.min(self.db_x0);
        let ux1 = cx1.max(self.db_x1);
        let uy0 = cy0.min(self.db_y0);
        let uy1 = cy1.max(self.db_y1);
        if ux1 >= ux0 && uy1 >= uy0 {
            for y in uy0..=uy1 {
                for x in ux0..=ux1 {
                    self.rowbuf[x - ux0] = if self.lj_glow[y * cols + x] != 0 { 8 } else { 0 };
                }
                let row = self.rowbuf[..(ux1 - ux0 + 1)].to_vec();
                self.emit_row(y, ux0, ux1 - ux0 + 1, &row, &mut st, out);
            }
        }
        self.db_x0 = cx0;
        self.db_x1 = cx1;
        self.db_y0 = cy0;
        self.db_y1 = cy1;
    }

    pub fn draw(&mut self, values: &[f64], out: &mut Vec<u8>, cap: usize) {
        let region = self.cols - self.x_off;
        if region == 0 {
            return;
        }
        self.row_colors();
        let mut o = Out { buf: out, cap };
        match self.mode {
            RenderMode::Wave => self.draw_wave(self.x_off, region, &mut o),
            RenderMode::Lissajous => self.draw_lissajous(self.x_off, region, &mut o),
            RenderMode::Bars => {
                self.draw_bars(values, None, self.num_bars, self.num_bars, self.x_off, region, &mut o)
            }
        }
    }

    pub fn draw_stereo(
        &mut self,
        left: &[f64],
        right: &[f64],
        per_ch_l: usize,
        out: &mut Vec<u8>,
        cap: usize,
    ) {
        let region = self.cols - self.x_off;
        if region == 0 {
            return;
        }
        self.row_colors();
        let mut o = Out { buf: out, cap };
        match self.mode {
            RenderMode::Wave => self.draw_wave(self.x_off, region, &mut o),
            RenderMode::Lissajous => self.draw_lissajous(self.x_off, region, &mut o),
            RenderMode::Bars => self.draw_bars(
                left,
                Some(right),
                self.num_bars,
                per_ch_l,
                self.x_off,
                region,
                &mut o,
            ),
        }
    }
}