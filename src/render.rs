use crate::config::DEFAULT_GLYPHS;

#[derive(PartialEq, Clone, Copy)]
pub enum RenderMode {
    Bars,
    Wave,
    Oscilloscope,
    Text,
}

enum BigGlyph {
    Block([u8; 7]),
    Uni(&'static unifont::Glyph),
    Blank,
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
    grad_sig: (u32, u32, bool, usize),
    barstr: [Vec<u8>; 9],
    spacestr: Vec<u8>,
    barstr_bw: usize,
    glyphs: Vec<Vec<u8>>,
    wave_buf: Vec<f64>,
    wave_cap: usize,
    wave_pos: usize,
    wave_filled: usize,
    wave_spc: usize,
    osc_l: Vec<f64>,
    osc_r: Vec<f64>,
    osc_cap: usize,
    osc_pos: usize,
    osc_filled: usize,
    osc_spc: usize,
    osc_win: usize,
    stereo_in: bool,
    osc_glow: Vec<u8>,
    sc_yrow: Vec<i64>,
    sc_lo: Vec<i64>,
    sc_hi: Vec<i64>,
    text: Vec<char>,
    text_dim: Vec<bool>,
    focus: usize,
    pub text_left: bool,
    pub text_size: usize,
    pub text_small: bool,
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
        let sig = (self.grad_lo, self.grad_hi, self.color_256, self.rows);
        if self.grad_sig == sig {
            return;
        }
        self.grad_sig = sig;
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
            grad_sig: (0, 0, false, 0),
            barstr: Default::default(),
            spacestr: Vec::new(),
            barstr_bw: 0,
            glyphs: Vec::new(),
            wave_buf: Vec::new(),
            wave_cap: 0,
            wave_pos: 0,
            wave_filled: 0,
            wave_spc: 1,
            osc_l: Vec::new(),
            osc_r: Vec::new(),
            osc_cap: 0,
            osc_pos: 0,
            osc_filled: 0,
            osc_spc: 1,
            osc_win: 0,
            stereo_in: false,
            osc_glow: vec![0; rows * cols],
            sc_yrow: Vec::new(),
            sc_lo: Vec::new(),
            sc_hi: Vec::new(),
            text: "SHARKVIS".chars().collect(),
            text_dim: Vec::new(),
            focus: 0,
            text_left: false,
            text_size: 1,
            text_small: false,
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
        self.osc_glow = vec![0; rows * cols];
        self.row_col = vec![Vec::new(); rows];
        self.grad_sig = (0, 0, false, 0);
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
        self.osc_glow = vec![0; self.rows * self.cols];
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
        if m == RenderMode::Oscilloscope {
            self.osc_glow.fill(0);
        }
    }

    pub fn mode_parse(name: &str) -> RenderMode {
        if name == "wave" {
            RenderMode::Wave
        } else if name == "oscilloscope" || name == "lissajous" {
            RenderMode::Oscilloscope
        } else if name == "text" {
            RenderMode::Text
        } else {
            RenderMode::Bars
        }
    }

    pub fn set_text(&mut self, s: &str) {
        let v: Vec<char> = s.chars().take(512).collect();
        if self.text != v {
            self.focus = v.len().saturating_sub(1);
            self.text = v;
            self.text_dim.clear();
            self.clear();
        }
    }

    pub fn set_rich(&mut self, entries: &[(String, bool)]) {
        let mut text = Vec::new();
        let mut dim = Vec::new();
        let mut focus = 0;
        let mut found = false;
        for (i, (s, cur)) in entries.iter().enumerate() {
            if i > 0 {
                text.push('\n');
                dim.push(false);
            }
            if *cur && !found {
                focus = text.len();
                found = true;
            }
            for c in s.chars().take(512) {
                text.push(c);
                dim.push(!cur);
            }
        }
        if self.text != text {
            self.text = text;
            self.text_dim = dim;
            self.focus = focus;
            self.clear();
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
        let mut osc_spc = sample_rate as usize / 800;
        if osc_spc < 1 {
            osc_spc = 1;
        }
        let mut osc_win = sample_rate as usize / 20;
        if osc_win < 256 {
            osc_win = 256;
        }
        if self.wave_cap == cap {
            self.wave_spc = spc;
            self.osc_spc = osc_spc;
            self.osc_win = osc_win;
            return;
        }
        self.wave_buf = vec![0.0; cap];
        self.osc_l = vec![0.0; cap];
        self.osc_r = vec![0.0; cap];
        self.wave_cap = cap;
        self.wave_pos = 0;
        self.wave_filled = 0;
        self.wave_spc = spc;
        self.osc_cap = cap;
        self.osc_pos = 0;
        self.osc_filled = 0;
        self.osc_spc = osc_spc;
        self.osc_win = osc_win;
    }

    pub fn feed(&mut self, left: Option<&[f64]>, right: Option<&[f64]>, n: usize) {
        if self.mode != RenderMode::Wave && self.mode != RenderMode::Oscilloscope {
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
            self.osc_l[self.osc_pos] = left[i];
            self.osc_r[self.osc_pos] = match right {
                Some(r) => r[i],
                None => left[i],
            };
            self.wave_pos = (self.wave_pos + 1) % self.wave_cap;
            if self.wave_filled < self.wave_cap {
                self.wave_filled += 1;
            }
            self.osc_pos = (self.osc_pos + 1) % self.osc_cap;
            if self.osc_filled < self.osc_cap {
                self.osc_filled += 1;
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

    fn block_glyph(c: char) -> [u8; 7] {
        match c.to_ascii_uppercase() {
            'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
            'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
            'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
            'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
            'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
            'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
            'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
            'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
            'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
            'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
            'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
            'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
            'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
            'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
            'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
            'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
            'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
            'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
            'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
            'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
            'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
            'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
            'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
            'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
            'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
            'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
            '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
            '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
            '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
            '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
            '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
            '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
            '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
            '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
            '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
            '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
            '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
            '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
            ',' => [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08],
            '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
            '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
            ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
            '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
            '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
            ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
            '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
            '*' => [0x00, 0x04, 0x15, 0x0E, 0x15, 0x04, 0x00],
            '#' => [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A],
            '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
            '\'' => [0x04, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00],
            '%' => [0x19, 0x1A, 0x02, 0x04, 0x08, 0x0B, 0x13],
            '=' => [0x00, 0x00, 0x1F, 0x00, 0x1F, 0x00, 0x00],
            '[' => [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E],
            ']' => [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E],
            '|' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
            _ => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        }
    }

    fn base_latin(c: char) -> Option<char> {
        Some(match c {
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'à' | 'á' | 'â' | 'ã' | 'ä' | 'å'
            | 'Ā' | 'ā' | 'Ă' | 'ă' | 'Ą' | 'ą' => 'A',
            'Ç' | 'ç' | 'Ć' | 'ć' | 'Ĉ' | 'ĉ' | 'Ċ' | 'ċ' | 'Č' | 'č' => 'C',
            'Ð' | 'ð' | 'Ď' | 'ď' | 'Đ' | 'đ' => 'D',
            'È' | 'É' | 'Ê' | 'Ë' | 'è' | 'é' | 'ê' | 'ë' | 'Ē' | 'ē' | 'Ĕ' | 'ĕ'
            | 'Ė' | 'ė' | 'Ę' | 'ę' | 'Ě' | 'ě' => 'E',
            'Ĝ' | 'ĝ' | 'Ğ' | 'ğ' | 'Ġ' | 'ġ' | 'Ģ' | 'ģ' => 'G',
            'Ĥ' | 'ĥ' | 'Ħ' | 'ħ' => 'H',
            'Ì' | 'Í' | 'Î' | 'Ï' | 'ì' | 'í' | 'î' | 'ï' | 'Ĩ' | 'ĩ' | 'Ī' | 'ī'
            | 'Ĭ' | 'ĭ' | 'Į' | 'į' | 'İ' => 'I',
            'Ĵ' | 'ĵ' => 'J',
            'Ķ' | 'ķ' | 'ĸ' => 'K',
            'Ĺ' | 'ĺ' | 'Ļ' | 'ļ' | 'Ľ' | 'ľ' | 'Ŀ' | 'ŀ' | 'Ł' | 'ł' => 'L',
            'Ñ' | 'ñ' | 'Ń' | 'ń' | 'Ņ' | 'ņ' | 'Ň' | 'ň' => 'N',
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø'
            | 'Ō' | 'ō' | 'Ŏ' | 'ŏ' | 'Ő' | 'ő' => 'O',
            'Ŕ' | 'ŕ' | 'Ŗ' | 'ŗ' | 'Ř' | 'ř' => 'R',
            'Ś' | 'ś' | 'Ŝ' | 'ŝ' | 'Ş' | 'ş' | 'Š' | 'š' => 'S',
            'Ţ' | 'ţ' | 'Ť' | 'ť' | 'Ŧ' | 'ŧ' => 'T',
            'Ù' | 'Ú' | 'Û' | 'Ü' | 'ù' | 'ú' | 'û' | 'ü' | 'Ũ' | 'ũ' | 'Ū' | 'ū'
            | 'Ŭ' | 'ŭ' | 'Ů' | 'ů' | 'Ű' | 'ű' | 'Ų' | 'ų' => 'U',
            'Ŵ' | 'ŵ' => 'W',
            'Ý' | 'ý' | 'ÿ' | 'Ŷ' | 'ŷ' => 'Y',
            'Ź' | 'ź' | 'Ż' | 'ż' | 'Ž' | 'ž' => 'Z',
            'Þ' | 'þ' => 'P',
            _ => return None,
        })
    }

    fn resolve_glyph(c: char) -> (BigGlyph, usize, usize) {
        let g = Self::block_glyph(c);
        if c == ' ' || g.iter().any(|&r| r != 0) {
            return (BigGlyph::Block(g), 5, 7);
        }
        if let Some(b) = Self::base_latin(c) {
            return (BigGlyph::Block(Self::block_glyph(b)), 5, 7);
        }
        match unifont::get_glyph(c) {
            Some(gl) => {
                let w = gl.get_width();
                (BigGlyph::Uni(gl), w, 16)
            }
            None => (BigGlyph::Blank, 5, 7),
        }
    }

    fn glyph_wh(c: char) -> (usize, usize) {
        let (_, w, h) = Self::resolve_glyph(c);
        (w, h)
    }

    fn glyph_on(g: &BigGlyph, x: usize, y: usize, h: usize) -> bool {
        match g {
            BigGlyph::Block(rows) => {
                let oy = h.saturating_sub(7) / 2;
                if y < oy || y >= oy + 7 || x >= 5 {
                    return false;
                }
                (rows[y - oy] >> (4 - x)) & 1 == 1
            }
            BigGlyph::Uni(gl) => gl.get_pixel(x, y),
            BigGlyph::Blank => false,
        }
    }

    fn cell_width(c: char) -> usize {
        if c.is_ascii() {
            return 1;
        }
        match c as u32 {
            0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x20000..=0x3FFFD => 2,
            _ => 1,
        }
    }

    fn letter_color(&self, xfrac: f64, v: f64) -> Vec<u8> {
        let lo_r = ((self.grad_lo >> 16) & 0xff) as f64;
        let lo_g = ((self.grad_lo >> 8) & 0xff) as f64;
        let lo_b = (self.grad_lo & 0xff) as f64;
        let hi_r = ((self.grad_hi >> 16) & 0xff) as f64;
        let hi_g = ((self.grad_hi >> 8) & 0xff) as f64;
        let hi_b = (self.grad_hi & 0xff) as f64;
        let t = xfrac.clamp(0.0, 1.0);
        let b = 0.10 + 0.90 * v.clamp(0.0, 1.0);
        let mix = |l: f64, h: f64| ((l + (h - l) * t) * b + 0.5).clamp(0.0, 255.0) as u32;
        let (cr, cg, cb) = (mix(lo_r, hi_r), mix(lo_g, hi_g), mix(lo_b, hi_b));
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

    fn wrap_chars(
        chars: &[char],
        widths: &[usize],
        base: usize,
        cap_px: usize,
    ) -> Vec<Vec<usize>> {
        let cap_px = cap_px.max(1);
        let mut lines: Vec<Vec<usize>> = Vec::new();
        let mut cur: Vec<usize> = Vec::new();
        let mut used = 0usize;
        let mut i = 0;
        while i < chars.len() {
            let mut sep = None;
            while i < chars.len() && chars[i] == ' ' {
                sep = Some(i);
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let mut j = i;
            while j < chars.len() && chars[j] != ' ' {
                j += 1;
            }
            let mut ww = 0usize;
            for t in i..j {
                ww += widths[base + t] + 1;
            }
            ww = ww.saturating_sub(1);
            let sep_w = sep.map(|t| widths[base + t]).unwrap_or(0);
            let need = if cur.is_empty() { ww } else { 1 + sep_w + ww };
            if !cur.is_empty() && used + need > cap_px {
                lines.push(std::mem::take(&mut cur));
                used = 0;
            }
            if cur.is_empty() {
                if ww <= cap_px {
                    cur.extend(i..j);
                    used = ww;
                } else {
                    let mut k = i;
                    while k < j {
                        let mut cw = 0usize;
                        let mut e = k;
                        while e < j {
                            let add = widths[base + e] + if e > k { 1 } else { 0 };
                            if cw + add > cap_px {
                                break;
                            }
                            cw += add;
                            e += 1;
                        }
                        if e == k {
                            e = k + 1;
                            cw = widths[base + k];
                        }
                        cur.extend(k..e);
                        k = e;
                        if k < j {
                            lines.push(std::mem::take(&mut cur));
                            used = 0;
                        } else {
                            used = cw;
                        }
                    }
                }
            } else if let Some(s) = sep {
                cur.push(s);
                used += 1 + sep_w;
                cur.extend(i..j);
                used += 1 + ww;
            } else {
                cur.extend(i..j);
                used += 1 + ww;
            }
            i = j;
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    }

    fn line_height(line: &[usize], heights: &[usize]) -> usize {
        let mut h = 7;
        for &idx in line {
            h = h.max(heights[idx]);
        }
        h
    }

    fn layout_text(
        chars: &[char],
        region_w: usize,
        rows: usize,
        focus: usize,
        max_s: usize,
    ) -> (usize, Vec<Vec<usize>>) {
        if chars.is_empty() || region_w == 0 || rows == 0 {
            return (1, Vec::new());
        }
        let auto_s = (rows / 7).max(1);
        let mut paras: Vec<(usize, Vec<char>)> = Vec::new();
        let mut start = 0;
        for (i, c) in chars.iter().enumerate() {
            if *c == '\n' {
                paras.push((start, chars[start..i].to_vec()));
                start = i + 1;
            }
        }
        paras.push((start, chars[start..].to_vec()));
        let mut widths = vec![5usize; chars.len()];
        let mut heights = vec![7usize; chars.len()];
        for (i, c) in chars.iter().enumerate() {
            let (w, h) = Self::glyph_wh(*c);
            widths[i] = w;
            heights[i] = h;
        }
        let wrap_all = |cap_px: usize| -> Vec<Vec<usize>> {
            let mut lines = Vec::new();
            for (base, pc) in paras.iter() {
                for mut line in Self::wrap_chars(pc, &widths, *base, cap_px) {
                    for idx in line.iter_mut() {
                        *idx += *base;
                    }
                    lines.push(line);
                }
            }
            lines
        };
        let top_s = if max_s == 0 { auto_s } else { max_s.min(auto_s).max(1) };
        for s in (1..=top_s).rev() {
            let cap_px = (region_w / s).max(1);
            let lines = wrap_all(cap_px);
            if lines.is_empty() {
                continue;
            }
            let mut need_h = lines.len().saturating_sub(1) * s;
            for line in &lines {
                need_h += Self::line_height(line, &heights) * s;
            }
            if need_h <= rows {
                return (s, lines);
            }
        }
        let lines = wrap_all(region_w.max(1));
        let keep = (rows / 8).max(1);
        if lines.len() <= keep {
            return (1, lines);
        }
        let mut fi = 0;
        for (i, line) in lines.iter().enumerate() {
            if line.contains(&focus) {
                fi = i;
            }
        }
        let start = fi.saturating_sub(keep - 1);
        (1, lines[start..(start + keep).min(lines.len())].to_vec())
    }

    fn char_raw_mags(values: &[f64], right: Option<&[f64]>, m: usize) -> (Vec<f64>, Vec<f64>) {
        let mut mags_l = vec![0.0f64; m];
        let mut mags_r = vec![0.0f64; m];
        for i in 0..m {
            let avg = |src: &[f64]| -> f64 {
                if src.is_empty() {
                    return 0.0;
                }
                let n = src.len();
                let mut b = (i + 1) * n / m;
                if b <= i * n / m {
                    b = i * n / m + 1;
                }
                if b > n {
                    b = n;
                }
                let s = &src[i * n / m..b];
                s.iter().sum::<f64>() / s.len() as f64
            };
            mags_l[i] = avg(values);
            mags_r[i] = match right {
                Some(r) => avg(r),
                None => mags_l[i],
            };
        }
        (mags_l, mags_r)
    }

    fn draw_text_mode(
        &mut self,
        values: &[f64],
        right: Option<&[f64]>,
        x_start: usize,
        region_w: usize,
        out: &mut Out,
    ) {
        if self.text_small {
            self.draw_small_text(values, right, x_start, region_w, out);
        } else {
            self.draw_text(values, right, x_start, region_w, out);
        }
    }

    fn draw_small_text(
        &mut self,
        values: &[f64],
        right: Option<&[f64]>,
        x_start: usize,
        region_w: usize,
        out: &mut Out,
    ) {
        let rows = self.rows;
        let cols = self.cols;
        if rows == 0 || region_w == 0 {
            return;
        }
        let text = self.text.clone();
        if text.is_empty() {
            return;
        }
        let m = text.len();
        let (mags_l, mags_r) = Self::char_raw_mags(values, right, m);
        let mut v = mags_l.iter().chain(mags_r.iter()).cloned().fold(0.0f64, f64::max);
        if !(v > 0.0) {
            v = 0.0;
        } else if v > 1.0 {
            v = 1.0;
        }
        let marker = 64 + (v * 15.0 + 0.5) as u8;
        let esc = self.letter_color(0.5, v);
        let mut paras: Vec<&[char]> = Vec::new();
        let mut start = 0;
        for (i, c) in text.iter().enumerate() {
            if *c == '\n' {
                paras.push(&text[start..i]);
                start = i + 1;
            }
        }
        paras.push(&text[start..]);
        let mut lines: Vec<&[char]> = Vec::new();
        for p in paras {
            if p.is_empty() {
                lines.push(&p[0..0]);
                continue;
            }
            let mut k = 0;
            while k < p.len() {
                let mut cells = 0usize;
                let mut end = k;
                while end < p.len() {
                    let cw = Self::cell_width(p[end]);
                    if end > k && cells + cw > region_w {
                        break;
                    }
                    cells += cw;
                    end += 1;
                }
                if end == k {
                    end = k + 1;
                }
                lines.push(&p[k..end]);
                k = end;
            }
        }
        if lines.len() > rows {
            lines.truncate(rows);
        }
        let x_end = (x_start + region_w).min(cols);
        let top = rows.saturating_sub(lines.len()) / 2;
        let mut boxes: Vec<(usize, usize, usize)> = Vec::new();
        let mut enc = [0u8; 4];
        for (li, line) in lines.iter().enumerate() {
            let y = top + li;
            if y >= rows {
                break;
            }
            let cells: usize = line.iter().map(|&c| Self::cell_width(c)).sum();
            let x0 = if self.text_left {
                x_start
            } else {
                x_start + region_w.saturating_sub(cells) / 2
            };
            boxes.push((x0, y, cells.min(x_end.saturating_sub(x0))));
            let mut changed = false;
            let mut cx = x0;
            for c in line.iter() {
                for _ in 0..Self::cell_width(*c) {
                    if cx >= x_end {
                        break;
                    }
                    if self.prev[y * cols + cx] != marker {
                        changed = true;
                        break;
                    }
                    cx += 1;
                }
                if changed || cx >= x_end {
                    break;
                }
            }
            if !changed {
                continue;
            }
            seek_cell(y as u32, x0 as u32, out);
            out.s(&esc);
            let mut cx = x0;
            for c in line.iter() {
                if cx >= x_end {
                    break;
                }
                for _ in 0..Self::cell_width(*c) {
                    if cx >= x_end {
                        break;
                    }
                    self.prev[y * cols + cx] = marker;
                    cx += 1;
                }
                out.s(c.encode_utf8(&mut enc).as_bytes());
            }
        }
        for y in 0..rows {
            for x in x_start..x_end {
                let mut in_box = false;
                for &(x0, y0, w) in boxes.iter() {
                    if y == y0 && x >= x0 && x < x0 + w {
                        in_box = true;
                        break;
                    }
                }
                if in_box {
                    continue;
                }
                let idx = y * cols + x;
                if self.prev[idx] != 0 {
                    seek_cell(y as u32, x as u32, out);
                    out.s(b" ");
                    self.prev[idx] = 0;
                }
            }
        }
    }

    fn draw_text(
        &mut self,
        values: &[f64],
        right: Option<&[f64]>,
        x_start: usize,
        region_w: usize,
        out: &mut Out,
    ) {
        let rows = self.rows;
        let cols = self.cols;
        if rows == 0 || region_w == 0 {
            return;
        }
        let text = self.text.clone();
        let m = text.len();
        if m == 0 {
            return;
        }
        let (mags_l, mags_r) = Self::char_raw_mags(values, right, m);
        let (s, lines) = Self::layout_text(&text, region_w, rows, self.focus, self.text_size);
        if lines.is_empty() {
            return;
        }
        let mut heights = vec![7usize; m];
        for (i, c) in text.iter().enumerate() {
            heights[i] = Self::glyph_wh(*c).1;
        }
        let mut total_h = lines.len().saturating_sub(1) * s;
        for line in &lines {
            total_h += Self::line_height(line, &heights) * s;
        }
        let top = rows.saturating_sub(total_h) / 2;
        let x_end = (x_start + region_w).min(cols);
        let full = self.render_glyph(8).to_vec();
        let dim = self.text_dim.clone();
        let mut boxes: Vec<(usize, usize, usize, usize)> = Vec::new();
        let mut y0 = top;
        for line in lines.iter() {
            if line.is_empty() {
                continue;
            }
            let mut gs: Vec<(BigGlyph, usize, usize)> = Vec::with_capacity(line.len());
            let mut lw = 0usize;
            let mut lh = 7usize;
            for &ci in line {
                let (g, w, h) = Self::resolve_glyph(text[ci]);
                lw += w + 1;
                lh = lh.max(h);
                gs.push((g, w, h));
            }
            lw = lw.saturating_sub(1);
            let wline = lw * s;
            let lead = if self.text_left {
                0
            } else {
                region_w.saturating_sub(wline) / 2
            };
            let mid_x = x_start + lead + wline / 2;
            let mut x0 = x_start + lead;
            for (k, &ci) in line.iter().enumerate() {
                let (g, w, _) = &gs[k];
                let dim_f = if dim.get(ci).copied().unwrap_or(false) { 0.35 } else { 1.0 };
                let box_w = if k + 1 < line.len() { (w + 1) * s } else { w * s };
                let left_side = x0 + box_w / 2 <= mid_x;
                let raw = if left_side { mags_l[ci] } else { mags_r[ci] };
                let mut v = raw * dim_f;
                if !(v > 0.0) {
                    v = 0.0;
                } else if v > 1.0 {
                    v = 1.0;
                }
                let marker = 64 + (v * 15.0 + 0.5) as u8;
                let xfrac = (ci as f64 + 0.5) / m as f64;
                let esc = self.letter_color(xfrac, v);
                boxes.push((x0, y0, box_w, lh));
                let mut changed = false;
                for gr in 0..lh {
                    for pr in 0..s {
                        let y = y0 + gr * s + pr;
                        if y >= rows {
                            break;
                        }
                        for px in 0..box_w {
                            let x = x0 + px;
                            if x >= x_end {
                                break;
                            }
                            let on = Self::glyph_on(g, px / s, gr, lh);
                            let want = if on { marker } else { 0 };
                            if self.prev[y * cols + x] != want {
                                changed = true;
                            }
                        }
                    }
                }
                if !changed {
                    x0 += box_w;
                    continue;
                }
                for gr in 0..lh {
                    for pr in 0..s {
                        let y = y0 + gr * s + pr;
                        if y >= rows {
                            break;
                        }
                        seek_cell(y as u32, x0 as u32, out);
                        out.s(&esc);
                        for px in 0..box_w {
                            let x = x0 + px;
                            if x >= x_end {
                                break;
                            }
                            let on = Self::glyph_on(g, px / s, gr, lh);
                            let want = if on { marker } else { 0 };
                            let idx = y * cols + x;
                            self.prev[idx] = want;
                            if on {
                                out.s(&full);
                            } else {
                                out.s(b" ");
                            }
                        }
                    }
                }
                x0 += box_w;
            }
            y0 += (lh + 1) * s;
        }
        for y in 0..rows {
            for x in x_start..x_end {
                let mut in_box = false;
                for &(x0, y0, w, h) in boxes.iter() {
                    if y >= y0 && y < y0 + h * s && x >= x0 && x < x0 + w {
                        in_box = true;
                        break;
                    }
                }
                if in_box {
                    continue;
                }
                let idx = y * cols + x;
                if self.prev[idx] != 0 {
                    seek_cell(y as u32, x as u32, out);
                    out.s(b" ");
                    self.prev[idx] = 0;
                }
            }
        }
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
            self.osc_glow[y as usize * self.cols + x as usize] = 255;
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

    fn draw_oscilloscope(&mut self, x_start: usize, region_w: usize, out: &mut Out) {
        if self.osc_cap == 0 || self.rows < 3 || region_w < 4 {
            return;
        }
        let rows = self.rows;
        let cols = self.cols;

        self.osc_glow.fill(0);

        let mut cx0 = cols;
        let mut cy0 = rows;
        let mut cx1 = 0usize;
        let mut cy1 = 0usize;
        let mut n = self.osc_filled;
        if n > self.osc_win {
            n = self.osc_win;
        }
        if n > 1 {
            let delay = if self.osc_spc == 0 { 1 } else { self.osc_spc };
            let cx = x_start as f64 + (region_w - 1) as f64 * 0.5;
            let cy = (rows - 1) as f64 * 0.5;
            let sxc = (region_w - 1) as f64 * 0.5;
            let syc = (rows - 1) as f64 * 0.5;
            let mut px = -1i64;
            let mut py = -1i64;
            for i in 0..n {
                let idx = (self.osc_pos + self.osc_cap - n + i) % self.osc_cap;
                let mut l = self.osc_l[idx];
                let mut r = self.osc_r[idx];
                if !self.stereo_in {
                    let idx2 = (idx + self.osc_cap - delay) % self.osc_cap;
                    r = self.osc_l[idx2];
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
                    self.rowbuf[x - ux0] = if self.osc_glow[y * cols + x] != 0 { 8 } else { 0 };
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
            RenderMode::Oscilloscope => self.draw_oscilloscope(self.x_off, region, &mut o),
            RenderMode::Text => self.draw_text_mode(values, None, self.x_off, region, &mut o),
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
        per_ch_r: usize,
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
            RenderMode::Oscilloscope => self.draw_oscilloscope(self.x_off, region, &mut o),
            RenderMode::Text => {
                let nl = per_ch_l.min(left.len());
                let nr = per_ch_r.min(right.len());
                self.draw_text_mode(&left[..nl], Some(&right[..nr]), self.x_off, region, &mut o)
            }
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_font_shapes() {
        assert_eq!(Renderer::block_glyph('A'), [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]);
        assert_eq!(Renderer::block_glyph('a'), Renderer::block_glyph('A'));
        assert_eq!(Renderer::block_glyph(' '), [0; 7]);
        assert_eq!(Renderer::block_glyph('~'), [0; 7]);
        let e = Renderer::block_glyph('E');
        assert_eq!(e[0], 0x1F);
        assert_eq!(e[3], 0x1E);
    }

    #[test]
    fn mode_parses_text() {
        assert!(Renderer::mode_parse("text") == RenderMode::Text);
        assert!(Renderer::mode_parse("wave") == RenderMode::Wave);
        assert!(Renderer::mode_parse("nope") == RenderMode::Bars);
    }

    #[test]
    fn text_draws_letters() {
        let mut r = Renderer::new(24, 80, 2, 1, 8);
        r.set_text("AB");
        r.grad_lo = 0x000000;
        r.grad_hi = 0xffffff;
        let vals = vec![1.0; 8];
        let mut out = Vec::new();
        r.draw_text(&vals, None, 0, 80, &mut Out { buf: &mut out, cap: 1 << 20 });
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains('█'), "lit letters must emit full blocks");
        assert!(out.windows(2).any(|w| w == b"\x1b["));
    }

    #[test]
    fn text_stereo_sides() {
        let mut r = Renderer::new(24, 80, 2, 1, 8);
        r.set_text("AB");
        r.grad_lo = 0x000000;
        r.grad_hi = 0xffffff;
        let left = vec![1.0; 8];
        let right = vec![0.0; 8];
        let mut out = Vec::new();
        r.draw_text(&left, Some(&right), 0, 80, &mut Out { buf: &mut out, cap: 1 << 20 });
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("\x1b[38;2;64;64;64m"), "loud left side must be bright, got {:?}", &text[..text.len().min(200)]);
        assert!(text.contains("\x1b[38;2;19;19;19m"), "quiet right side must be dim");
    }

    #[test]
    fn text_brightness_follows_bins_directly() {
        let mut r = Renderer::new(24, 80, 2, 1, 8);
        r.set_text("AB");
        r.grad_lo = 0x000000;
        r.grad_hi = 0xffffff;
        let vals = vec![1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let mut out = Vec::new();
        r.draw_text(&vals, None, 0, 80, &mut Out { buf: &mut out, cap: 1 << 20 });
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("\x1b[38;2;64;64;64m"), "loud bins must be bright, got {:?}", &text[..text.len().min(200)]);
        assert!(text.contains("\x1b[38;2;19;19;19m"), "quiet bins must stay dim like bars");
    }

    #[test]
    fn text_stereo_ignores_stale_tail() {
        let mut r = Renderer::new(24, 80, 2, 1, 8);
        r.mode = RenderMode::Text;
        r.set_text("AB");
        r.grad_lo = 0x000000;
        r.grad_hi = 0xffffff;
        let mut left = vec![0.001; 64];
        let mut right = vec![0.001; 64];
        for v in left.iter_mut().take(4) {
            *v = 1.0;
        }
        for v in right.iter_mut().take(4) {
            *v = 1.0;
        }
        let mut out = Vec::new();
        r.draw_stereo(&left, &right, 4, 4, &mut out, 1 << 20);
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("\x1b[38;2;64;64;64m"), "left letter must be bright, got {:?}", &text[..text.len().min(200)]);
        assert!(text.contains("\x1b[38;2;191;191;191m"), "right letter must use fresh bins, got {:?}", &text[..text.len().min(200)]);
        assert!(!text.contains("19;19;19"), "no letter may fall into the stale tail");
    }

    #[test]
    fn small_text_draws_plain_centered_line() {
        let mut r = Renderer::new(24, 80, 2, 1, 8);
        r.set_text("Hi there");
        r.text_small = true;
        let vals = vec![1.0; 16];
        let mut out = Vec::new();
        r.draw_small_text(&vals, None, 0, 80, &mut Out { buf: &mut out, cap: 1 << 20 });
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains("Hi there"), "plain line must be emitted, got {:?}", &text[..text.len().min(120)]);
        assert!(!text.contains('█'), "small mode must not use block glyphs");
        assert!(text.contains("\x1b[12;37H"), "8-char line centers at col 37 row 12, got {:?}", &text[..text.len().min(120)]);
    }

    #[test]
    fn small_text_empty_draws_nothing() {
        let mut r = Renderer::new(24, 80, 2, 1, 8);
        r.set_text("");
        r.text_small = true;
        let vals = vec![1.0; 16];
        let mut out = Vec::new();
        r.draw_small_text(&vals, None, 0, 80, &mut Out { buf: &mut out, cap: 1 << 20 });
        assert!(out.is_empty());
    }

    #[test]
    fn cjk_metrics_and_accent_fold() {
        assert_eq!(Renderer::glyph_wh('A'), (5, 7));
        assert_eq!(Renderer::glyph_wh(' '), (5, 7));
        assert_eq!(Renderer::glyph_wh('é'), (5, 7));
        assert_eq!(Renderer::glyph_wh('中'), (16, 16));
        assert_eq!(Renderer::glyph_wh('あ'), (16, 16));
        assert_eq!(Renderer::glyph_wh('Ä'), (5, 7));
        assert_eq!(Renderer::glyph_wh('\u{10FFFF}'), (5, 7));
    }

    #[test]
    fn accent_folds_to_base_bitmap() {
        let (g1, _, _) = Renderer::resolve_glyph('é');
        let (g2, _, _) = Renderer::resolve_glyph('E');
        match (g1, g2) {
            (BigGlyph::Block(a), BigGlyph::Block(b)) => assert_eq!(a, b),
            _ => panic!("accents must fold to block base"),
        }
    }

    #[test]
    fn mixed_line_picks_tall_scale() {
        let text: Vec<char> = "A中".chars().collect();
        let (s, lines) = Renderer::layout_text(&text, 80, 24, 1, 0);
        assert_eq!(s, 1);
        assert_eq!(lines, vec![vec![0, 1]]);
    }

    #[test]
    fn cjk_draws_lit_pixels() {
        let mut r = Renderer::new(24, 80, 2, 1, 8);
        r.set_text("中文");
        let vals = vec![1.0; 64];
        let mut out = Vec::new();
        r.draw_text(&vals, None, 0, 80, &mut Out { buf: &mut out, cap: 1 << 20 });
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(text.contains('█'), "CJK fallback must light pixels");
    }

    #[test]
    fn cell_width_wide_chars() {
        assert_eq!(Renderer::cell_width('A'), 1);
        assert_eq!(Renderer::cell_width('é'), 1);
        assert_eq!(Renderer::cell_width('中'), 2);
        assert_eq!(Renderer::cell_width('あ'), 2);
    }

        #[test]
    fn text_empty_text_draws_nothing() {        let mut r = Renderer::new(24, 80, 2, 1, 8);
        r.set_text("");
        let vals = vec![1.0; 8];
        let mut out = Vec::new();
        r.draw_text(&vals, None, 0, 80, &mut Out { buf: &mut out, cap: 1 << 20 });
        assert!(out.is_empty());
    }
}

#[cfg(test)]
mod layout_tests {
    use super::Renderer;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn short_text_stays_big_single_line() {
        let (s, lines) = Renderer::layout_text(&chars("HI"), 80, 24, 1, 0);
        assert_eq!(s, 3);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], vec![0, 1]);
    }

    #[test]
    fn fixed_size_caps_scale() {
        let (s, lines) = Renderer::layout_text(&chars("HI"), 80, 24, 1, 1);
        assert_eq!(s, 1);
        assert_eq!(lines.len(), 1);
        let (s, lines) = Renderer::layout_text(&chars("HI"), 80, 24, 1, 5);
        assert_eq!(s, 3);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn long_text_shrinks_before_wrapping() {
        let (s, lines) = Renderer::layout_text(&chars("HELLO WORLD"), 80, 24, 10, 0);
        assert_eq!(lines.len(), 1);
        assert!(s < 3);
        let flat: Vec<usize> = lines.concat();
        assert_eq!(flat.len(), 11);
    }

    #[test]
    fn overflow_wraps_to_two_lines() {
        let text = chars("ONE TWO THREE FOUR");
        let (s, lines) = Renderer::layout_text(&text, 60, 24, 17, 0);
        assert!(lines.len() >= 2);
        for line in &lines {
            assert!((line.len() * 6 - 1) * s <= 60);
        }
        let flat: Vec<usize> = lines.concat();
        assert!(flat.contains(&17));
        assert!(flat.len() < text.len() + 1);
    }

    #[test]
    fn narrow_region_splits_words() {
        let text = chars("AB CD EF");
        let (s, lines) = Renderer::layout_text(&text, 18, 24, 7, 0);
        assert_eq!(s, 1);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], vec![0, 1]);
        assert_eq!(lines[2], vec![6, 7]);
    }

    #[test]
    fn impossible_sizes_keep_focus_visible() {
        let text = chars("ONE TWO THREE FOUR FIVE SIX SEVEN EIGHT");
        let (_, lines) = Renderer::layout_text(&text, 40, 24, 38, 0);
        let flat: Vec<usize> = lines.concat();
        assert!(flat.contains(&38));
        assert!(flat.len() < text.len());
    }

    #[test]
    fn empty_or_zero_is_safe() {
        assert_eq!(Renderer::layout_text(&[], 80, 24, 0, 0).1.len(), 0);
        assert_eq!(Renderer::layout_text(&chars("HI"), 0, 24, 1, 0).1.len(), 0);
        assert_eq!(Renderer::layout_text(&chars("HI"), 80, 0, 1, 0).1.len(), 0);
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;

    #[test]
    fn probe_stereo_ramps() {
        let mut r = Renderer::new(24, 80, 2, 1, 8);
        r.set_text("AB");
        r.grad_lo = 0x000000;
        r.grad_hi = 0xffffff;
        let left: Vec<f64> = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        let right: Vec<f64> = vec![0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1];
        let mut out = Vec::new();
        r.draw_text(&left, Some(&right), 0, 80, &mut Out { buf: &mut out, cap: 1 << 20 });
        let text = String::from_utf8_lossy(&out).into_owned();
        println!("RAMPS-OUT: {:?}", text);
    }
}
