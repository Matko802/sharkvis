use std::time::Instant;

use crate::fft::Fft;

const BASS_CUT_OFF_HZ: f64 = 100.0;
const FFT_INTERVAL_NS: u64 = 1_000_000_000 / 60;

pub struct Dsp {
    pub number_of_bars: usize,
    pub rate: u32,
    pub autosens: bool,
    pub sens: f64,
    pub sens_init: bool,
    pub sens_scale: f64,
    framerate: f64,
    frame_skip: usize,
    pub noise_reduction: f64,

    fft_size: usize,
    input_buffer_size: usize,
    max_bin: usize,

    last_fft: Option<Instant>,
    fft_interval: std::time::Duration,
    sens_step: u32,
    any_signal: bool,

    input_buffer: Vec<f64>,
    lower_cut_off: Vec<usize>,
    upper_cut_off: Vec<usize>,
    eq: Vec<f64>,

    cava_fall: Vec<f64>,
    cava_mem: Vec<f64>,
    cava_peak: Vec<f64>,
    prev_cava_out: Vec<f64>,

    multiplier: Vec<f64>,

    fft: Fft,

    in_: Vec<f64>,
    out_mag: Vec<f64>,
}

impl Dsp {
    pub fn render_frame_size(&self) -> usize {
        self.input_buffer_size
    }

    fn pick_fft_size(rate: u32) -> usize {
        let mut s: usize = 512;
        if rate > 8125 && rate <= 16250 {
            s *= 2;
        } else if rate > 16250 && rate <= 32500 {
            s *= 4;
        } else if rate > 32500 && rate <= 75000 {
            s *= 8;
        } else if rate > 75000 && rate <= 150000 {
            s *= 16;
        } else if rate > 150000 && rate <= 300000 {
            s *= 32;
        } else if rate > 300000 {
            s *= 64;
        }
        if s > 4096 {
            s = 4096;
        }
        s
    }

    pub fn new(
        number_of_bars: usize,
        rate: u32,
        autosens: bool,
        noise_reduction: f64,
        low_cut_off: u32,
        high_cut_off: u32,
    ) -> Self {
        let fft_size = Self::pick_fft_size(rate);
        let input_buffer_size = fft_size;
        let mut max_bin =
            (high_cut_off as f64 / rate as f64 * fft_size as f64).ceil() as usize;
        if max_bin > fft_size / 2 {
            max_bin = fft_size / 2;
        }

        let lower_lo = low_cut_off as f64;
        let upper_hi = high_cut_off as f64;
        let half = fft_size / 2;

        let frequency_constant =
            (lower_lo / upper_hi).log10() / (1.0 / (number_of_bars as f64 + 1.0) - 1.0);

        let mut lower = vec![0usize; number_of_bars + 1];
        let mut upper = vec![0usize; number_of_bars + 1];
        let mut cut_freq = vec![0.0f64; number_of_bars + 1];

        let mut bass_cut_off_bar = 0usize;
        let mut first_bar = true;
        let min_bandwidth = rate as f64 / fft_size as f64;

        for n in 0..=number_of_bars {
            let mut bdc = frequency_constant * -1.0;
            bdc += (n as f64 + 1.0) / (number_of_bars as f64 + 1.0) * frequency_constant;
            cut_freq[n] = upper_hi * 10.0f64.powf(bdc);

            if n > 0 && cut_freq[n - 1] >= cut_freq[n] {
                cut_freq[n] = cut_freq[n - 1] + min_bandwidth;
            }

            let relative = cut_freq[n] / (rate as f64 / 2.0);

            if cut_freq[n] < BASS_CUT_OFF_HZ {
                lower[n] = (relative * half as f64) as usize;
                bass_cut_off_bar += 1;
                if bass_cut_off_bar > 1 {
                    first_bar = false;
                }
                if lower[n] > half {
                    lower[n] = half;
                }
            } else {
                lower[n] = (relative * half as f64).ceil() as usize;
                if n == bass_cut_off_bar {
                    first_bar = true;
                    if n > 0 {
                        upper[n - 1] = (relative * half as f64) as usize - 1;
                    }
                } else {
                    first_bar = false;
                }
                if lower[n] > half {
                    lower[n] = half;
                }
            }

            if n > 0 {
                if !first_bar {
                    upper[n - 1] = lower[n] - 1;
                    if lower[n] <= lower[n - 1] {
                        if lower[n - 1] + 1 < half + 1 {
                            lower[n] = lower[n - 1] + 1;
                            upper[n - 1] = lower[n] - 1;
                        }
                    }
                } else if upper[n - 1] < lower[n - 1] {
                    upper[n - 1] = lower[n - 1] + 1;
                }
            }

            cut_freq[n] = lower[n] as f64 / half as f64 * (rate as f64 / 2.0);
        }

        let mut eq = vec![0.0f64; number_of_bars];
        for n in 0..number_of_bars {
            eq[n] = 1.0 / 2.0f64.powf(28.0);
            eq[n] *= cut_freq[n + 1].powf(0.85);
            eq[n] /= (fft_size as f64).log2();
            eq[n] /= (upper[n] - lower[n] + 1) as f64;
        }

        let mut multiplier = vec![0.0f64; fft_size];
        for i in 0..fft_size {
            multiplier[i] =
                0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / (fft_size as f64 - 1.0)).cos());
        }

        Dsp {
            number_of_bars,
            rate,
            autosens,
            sens: 100.0,
            sens_init: true,
            sens_scale: 1.0,
            framerate: 75.0,
            frame_skip: 1,
            noise_reduction,
            fft_size,
            input_buffer_size,
            max_bin,
            last_fft: None,
            fft_interval: std::time::Duration::from_nanos(FFT_INTERVAL_NS),
            sens_step: 0,
            any_signal: false,
            input_buffer: vec![0.0; input_buffer_size],
            lower_cut_off: lower,
            upper_cut_off: upper,
            eq,
            cava_fall: vec![0.0; number_of_bars],
            cava_mem: vec![0.0; number_of_bars],
            cava_peak: vec![0.0; number_of_bars],
            prev_cava_out: vec![0.0; number_of_bars],
            multiplier,
            fft: Fft::new(fft_size),
            in_: vec![0.0; fft_size],
            out_mag: vec![0.0; fft_size / 2 + 1],
        }
    }

    pub fn execute(&mut self, cava_in: Option<&[f64]>, new_samples_in: usize, cava_out: &mut [f64]) {
        let new_samples = if new_samples_in < self.input_buffer_size {
            new_samples_in
        } else {
            self.input_buffer_size
        };

        if new_samples > 0 {
            let ci = match cava_in {
                Some(s) => s,
                None => return,
            };
            self.framerate -= self.framerate / 64.0;
            self.framerate += (self.rate as f64 * self.frame_skip as f64) / new_samples as f64 / 64.0;
            self.frame_skip = 1;

            let size = self.input_buffer_size;
            let mut i = size;
            while i > new_samples {
                i -= 1;
                self.input_buffer[i] = self.input_buffer[i - new_samples];
            }
            self.any_signal = false;
            for n in 0..new_samples {
                let v = ci[n];
                self.input_buffer[new_samples - n - 1] = v;
                if v != 0.0 {
                    self.any_signal = true;
                }
            }
        } else {
            self.frame_skip += 1;
            return;
        }

        match self.last_fft {
            Some(t) if t.elapsed() < self.fft_interval => return,
            _ => {}
        }
        self.last_fft = Some(Instant::now());

        for i in 0..self.fft_size {
            self.in_[i] = self.multiplier[i] * self.input_buffer[i];
        }
        self.fft.process(&self.in_, &mut self.out_mag, self.max_bin);

        for n in 0..self.number_of_bars {
            let mut temp = 0.0;
            for i in self.lower_cut_off[n]..=self.upper_cut_off[n] {
                temp += self.out_mag[i];
            }
            temp *= self.eq[n];
            cava_out[n] = temp;
        }

        if self.autosens {
            for n in 0..self.number_of_bars {
                cava_out[n] *= self.sens;
            }
        }

        let mut overshoot = false;
        let mut gravity_mod =
            (60.0 / self.framerate).powf(2.5) * 1.54 / self.noise_reduction.max(0.01);
        if gravity_mod < 1.0 {
            gravity_mod = 1.0;
        }

        for n in 0..self.number_of_bars {
            if cava_out[n] < self.prev_cava_out[n] && self.noise_reduction > 0.1 {
                cava_out[n] =
                    self.cava_peak[n] * (1.0 - self.cava_fall[n] * self.cava_fall[n] * gravity_mod);
                if cava_out[n] < 0.0 {
                    cava_out[n] = 0.0;
                }
                self.cava_fall[n] += 0.028;
            } else {
                self.cava_peak[n] = cava_out[n];
                self.cava_fall[n] = 0.0;
            }
            self.prev_cava_out[n] = cava_out[n];

            cava_out[n] = self.cava_mem[n] * self.noise_reduction
                + cava_out[n] * (1.0 - self.noise_reduction);
            self.cava_mem[n] = cava_out[n];

            if !self.any_signal && cava_out[n] < 0.001 {
                cava_out[n] = 0.001;
                self.cava_mem[n] = 0.001;
            }

            if self.autosens {
                if cava_out[n] > 1.0 {
                    overshoot = true;
                    cava_out[n] = 1.0;
                }
            }
        }

        if self.autosens {
            self.sens_step += 1;
            if self.sens_step >= 3 {
                self.sens_step = 0;
                if overshoot {
                    self.sens *= 0.98;
                    self.sens_init = false;
                } else if self.any_signal {
                    self.sens *= 1.001;
                    if self.sens_init {
                        self.sens *= 2.0;
                    }
                }
            }
        }

        if self.sens_scale != 1.0 {
            for n in 0..self.number_of_bars {
                cava_out[n] *= self.sens_scale;
            }
        }
    }
}