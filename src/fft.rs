const PI: f64 = std::f64::consts::PI;

pub struct Fft {
    pub n: usize,
    rev: Vec<usize>,
    cos: Vec<f64>,
    sin: Vec<f64>,
    ccos: Vec<f64>,
    csin: Vec<f64>,
    re: Vec<f64>,
    im: Vec<f64>,
}

impl Fft {
    pub fn new(n: usize) -> Self {
        let mut bits = 0usize;
        while (1usize << bits) < n / 2 {
            bits += 1;
        }
        let half = n / 2;

        let mut rev = vec![0usize; half];
        let mut cos = vec![0.0; half];
        let mut sin = vec![0.0; half];
        let mut ccos = vec![0.0; half / 2];
        let mut csin = vec![0.0; half / 2];

        for i in 0..half {
            let mut v = 0usize;
            let mut x = i;
            for _ in 0..bits {
                v = (v << 1) | (x & 1);
                x >>= 1;
            }
            rev[i] = v;
        }
        for k in 0..half {
            let a = -2.0 * PI * k as f64 / n as f64;
            cos[k] = a.cos();
            sin[k] = a.sin();
        }
        for k in 0..half / 2 {
            let a = -2.0 * PI * k as f64 / half as f64;
            ccos[k] = a.cos();
            csin[k] = a.sin();
        }

        Fft {
            n,
            rev,
            cos,
            sin,
            ccos,
            csin,
            re: vec![0.0; half],
            im: vec![0.0; half],
        }
    }

    fn complex_half(&mut self) {
        let n = self.n / 2;
        let mut size = 2;
        while size <= n {
            let h = size / 2;
            let step = n / size;
            let mut i = 0;
            while i < n {
                let mut j = 0;
                while j < h {
                    let k = j * step;
                    let c = self.ccos[k];
                    let s = self.csin[k];
                    let tre = c * self.re[i + j + h] - s * self.im[i + j + h];
                    let tim = c * self.im[i + j + h] + s * self.re[i + j + h];
                    let ure = self.re[i + j];
                    let uim = self.im[i + j];
                    self.re[i + j] = ure + tre;
                    self.im[i + j] = uim + tim;
                    self.re[i + j + h] = ure - tre;
                    self.im[i + j + h] = uim - tim;
                    j += 1;
                }
                i += size;
            }
            size <<= 1;
        }
    }

    pub fn process(&mut self, input: &[f64], out_mag: &mut [f64], max_bin: usize) {
        let n = self.n;
        let half = n / 2;

        let mut se = 0.0;
        let mut so = 0.0;
        for j in 0..half {
            let r = self.rev[j];
            self.re[r] = input[2 * j];
            self.im[r] = input[2 * j + 1];
            se += input[2 * j];
            so += input[2 * j + 1];
        }
        self.complex_half();

        out_mag[0] = (self.re[0] + self.im[0]).abs();
        let last = if max_bin < half { max_bin } else { half - 1 };
        for k in 1..=last {
            let hk = half - k;
            let ze = self.re[k];
            let zi = self.im[k];
            let zh_e = self.re[hk];
            let zh_i = self.im[hk];
            let xe = 0.5 * (ze + zh_e);
            let xi = 0.5 * (zi - zh_i);
            let de = 0.5 * (ze - zh_e);
            let di = 0.5 * (zi + zh_i);
            let oe = di;
            let oi = -de;
            let c = self.cos[k];
            let s = self.sin[k];
            let re_k = xe + c * oe - s * oi;
            let im_k = xi + c * oi + s * oe;
            out_mag[k] = (re_k * re_k + im_k * im_k).sqrt();
        }
        if max_bin >= half {
            out_mag[half] = (se - so).abs();
        }
    }
}