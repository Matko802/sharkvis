use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

pub struct Fft {
    pub n: usize,
    fft: std::sync::Arc<dyn rustfft::Fft<f64>>,
    buf: Vec<Complex<f64>>,
}

impl Fft {
    pub fn new(n: usize) -> Self {
        let mut planner = FftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(n);
        Fft {
            n,
            fft,
            buf: vec![Complex::new(0.0, 0.0); n],
        }
    }

    pub fn process(&mut self, input: &[f64], out_mag: &mut [f64], max_bin: usize) {
        let n = self.n;
        for i in 0..n {
            self.buf[i] = Complex::new(input[i], 0.0);
        }
        self.fft.process(&mut self.buf);
        let last = (max_bin + 1).min(out_mag.len());
        for k in 0..last {
            let z = self.buf[k];
            out_mag[k] = (z.re * z.re + z.im * z.im).sqrt();
        }
    }
}
