use std::time::Instant;

use crate::fft::Fft;

const BASS_CUT_OFF_HZ: f64 = 100.0;
const FFT_INTERVAL_NS: u64 = 0;

pub struct Dsp {
    pub number_of_bars: usize,
    #[allow(dead_code)]
    pub rate: u32,
    pub autosens: bool,
    pub sens: f64,
    pub sens_init: bool,
    pub sens_scale: f64,
    /// Visual refresh rate (frames/sec) used for time-based smoothing.
    /// Must be synced from the display loop (`cfg.framerate`); it is
    /// deliberately independent of the audio sample rate so changing
    /// `sample_rate` never changes fall speed / smoothness.
    pub display_fps: f64,
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
        // Keep the original table's ~85ms analysis window at high rates
        // too: capping N while rate keeps growing makes bins coarser in Hz
        // (23Hz+ at 192kHz), so bands narrower than the main lobe get
        // sliced differently per rate and bar heights diverge again.
        // 16384 covers the whole 192kHz setting range; the per-frame cost
        // (~2x an 8192 FFT) only applies when those rates are selected.
        if s > 16384 {
            s = 16384;
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
        // Nothing can be analyzed above Nyquist; clamp the window so low
        // sample rates (e.g. 8kHz) with a higher default cutoff (8kHz)
        // don't pile every top band onto a single bin and underflow the
        // band math below.
        let nyquist = (rate / 2).max(2);
        let high_cut_off = high_cut_off.clamp(2, nyquist);
        let low_cut_off = low_cut_off.clamp(1, high_cut_off - 1);
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
                        upper[n - 1] = ((relative * half as f64) as usize)
                            .saturating_sub(1);
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
                    upper[n - 1] = lower[n].saturating_sub(1);
                    if lower[n] <= lower[n - 1] {
                        if lower[n - 1] + 1 < half + 1 {
                            lower[n] = lower[n - 1] + 1;
                            upper[n - 1] = lower[n].saturating_sub(1);
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
            // Gain calibration, independent of sample rate: FFT magnitudes
            // grow linearly with N (coherent gain) while a fixed-Hz band
            // spans N/rate bins, so without correction a taller FFT reads
            // louder and `sample_rate` would change bar heights. The
            // 48000/rate factor cancels the N/rate bin-count growth and the
            // /12 pins the old /log2(N) at log2(4096), so 48kHz output is
            // bit-identical to before while every other rate now matches
            // it: same Hz tone at same amplitude gives same bars at any
            // rate.
            eq[n] /= 12.0;
            eq[n] /= (upper[n].saturating_sub(lower[n]) + 1) as f64;
            eq[n] *= 48000.0 / rate as f64;
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
            display_fps: 60.0,
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

        // Ingest whatever audio arrived. When the display loop outruns the
        // audio thread (common at low sample rates where one 512-frame
        // block spans several 16ms frames) there may be nothing new: keep
        // the old buffer and still tick the FFT + falloff below so bars
        // decay at full display rate instead of freezing between blocks.
        // Refresh/smoothness therefore follow `display_fps`, never `rate`.
        if new_samples > 0 {
            if let Some(ci) = cava_in {
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
                self.any_signal = false;
            }
        } else {
            self.any_signal = false;
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
        // Time-based falloff: driven by the display rate, not by the audio
        // sample rate, so `sample_rate` never changes fall speed.
        let fps = self.display_fps.clamp(1.0, 1000.0);
        let mut gravity_mod =
            (60.0 / fps).powf(2.5) * 1.54 / self.noise_reduction.max(0.01);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_response(rate: u32, freq: f64, amp: f64) -> Vec<f64> {
        let bars = 32;
        let mut dsp = Dsp::new(bars, rate, false, 0.2, 50, 8000);
        dsp.display_fps = 60.0;
        let n = dsp.render_frame_size();
        let mut out = vec![0.0; bars];
        // Feed a continuous sine in chunks until the input buffer holds
        // only the tone and everything has settled to steady state. The
        // falloff in leakage bands needs ~35 ticks after the fill to
        // finish, so settle generously (steady-state proof, not speed).
        let chunk = 512.min(n);
        let ticks = n / chunk + 40;
        let mut buf = vec![0.0; chunk];
        let mut idx = 0usize;
        for _ in 0..ticks {
            for i in 0..chunk {
                buf[i] =
                    amp * (2.0 * std::f64::consts::PI * freq * idx as f64 / rate as f64).sin();
                idx += 1;
            }
            dsp.execute(Some(&buf), chunk, &mut out);
        }
        out
    }
    fn windowed_peak(out: &[f64]) -> f64 {
        let pk = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
        let lo = pk.saturating_sub(2);
        let hi = (pk + 2).min(out.len() - 1);
        out[lo..=hi].iter().sum()
    }

    #[test]
    fn bar_height_independent_of_sample_rate() {
        // Overall gain calibration: the same Hz tone at the same amplitude
        // must read the same at every sample rate. (Very low bass bands
        // are excluded: down there a band is ~1 FFT bin wide, so rounding
        // a band edge to the nearest bin reshapes those bands per rate no
        // matter the gain - geometry, not gain.)
        let rates = [8000u32, 11025, 16000, 22050, 32000, 44100, 48000, 96000, 192000];
        for freq in [220.0, 440.0, 1500.0] {
            let mut totals = Vec::new();
            for &r in &rates {
                let out = sine_response(r, freq, 0.5);
                let peak = out.iter().cloned().fold(0.0f64, f64::max);
                assert!(
                    peak > 1e-9,
                    "rate {} must show the {}Hz tone, got {:?}",
                    r,
                    freq,
                    out
                );
                totals.push(windowed_peak(&out));
            }
            let mut sorted = totals.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let med = sorted[sorted.len() / 2];
            for (r, t) in rates.iter().zip(totals.iter()) {
                let rel = (t - med).abs() / med;
                assert!(
                    rel < 0.25,
                    "{}Hz tone: rate {} windowed {} vs median {} (rel {:.2})",
                    freq,
                    r,
                    t,
                    med,
                    rel
                );
            }
        }
    }
}