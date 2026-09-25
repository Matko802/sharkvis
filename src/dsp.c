#include <math.h>
#include <stdlib.h>
#include <string.h>

#include "common.h"
#include "dsp.h"
#include "fft.h"

#define BASS_CUT_OFF_HZ 100.0

struct Dsp {
    size_t number_of_bars;
    unsigned rate;
    int autosens;
    double sens;
    int sens_init;
    double sens_scale;
    double display_fps;
    double noise_reduction;
    size_t fft_size;
    size_t input_buffer_size;
    size_t max_bin;
    int have_last_fft;
    uint64_t last_fft_ns;
    unsigned sens_step;
    int any_signal;
    double *input_buffer;
    size_t *lower_cut_off;
    size_t *upper_cut_off;
    double *eq;
    double *cava_fall;
    double *cava_mem;
    double *cava_peak;
    double *prev_cava_out;
    double *multiplier;
    Fft *fft;
    double *in_;
    double *out_mag;
};

static size_t pick_fft_size(unsigned rate) {
    size_t s = 512;
    if (rate > 8125 && rate <= 16250)
        s *= 2;
    else if (rate > 16250 && rate <= 32500)
        s *= 4;
    else if (rate > 32500 && rate <= 75000)
        s *= 8;
    else if (rate > 75000 && rate <= 150000)
        s *= 16;
    else if (rate > 150000 && rate <= 300000)
        s *= 32;
    else if (rate > 300000)
        s *= 64;
    if (s > 16384)
        s = 16384;
    return s;
}

Dsp *dsp_new(size_t bars, unsigned rate, int autosens, double noise,
             unsigned lo_cut, unsigned hi_cut) {
    Dsp *d = calloc(1, sizeof *d);
    if (!d)
        return NULL;
    size_t fft_size = pick_fft_size(rate);
    d->number_of_bars = bars;
    d->rate = rate;
    d->autosens = autosens;
    d->sens = 100.0;
    d->sens_init = 1;
    d->sens_scale = 1.0;
    d->display_fps = 60.0;
    d->noise_reduction = noise;
    d->fft_size = fft_size;
    d->input_buffer_size = fft_size;
    unsigned nyquist = rate / 2;
    if (nyquist < 2)
        nyquist = 2;
    if (hi_cut < 2)
        hi_cut = 2;
    if (hi_cut > nyquist)
        hi_cut = nyquist;
    if (lo_cut < 1)
        lo_cut = 1;
    if (lo_cut > hi_cut - 1)
        lo_cut = hi_cut - 1;
    size_t max_bin = (size_t)ceil((double)hi_cut / (double)rate * (double)fft_size);
    if (max_bin > fft_size / 2)
        max_bin = fft_size / 2;
    d->max_bin = max_bin;
    double lower_lo = (double)lo_cut;
    double upper_hi = (double)hi_cut;
    size_t half = fft_size / 2;
    double fc = log10(lower_lo / upper_hi) / (1.0 / ((double)bars + 1.0) - 1.0);
    size_t *lower = calloc(bars + 1, sizeof(size_t));
    size_t *upper = calloc(bars + 1, sizeof(size_t));
    double *cut_freq = calloc(bars + 1, sizeof(double));
    if (!lower || !upper || !cut_freq) {
        free(lower);
        free(upper);
        free(cut_freq);
        free(d);
        return NULL;
    }
    size_t bass_cut_off_bar = 0;
    int first_bar = 1;
    double min_bw = (double)rate / (double)fft_size;
    for (size_t n = 0; n <= bars; n++) {
        double bdc = fc * -1.0;
        bdc += ((double)n + 1.0) / ((double)bars + 1.0) * fc;
        cut_freq[n] = upper_hi * pow(10.0, bdc);
        if (n > 0 && cut_freq[n - 1] >= cut_freq[n])
            cut_freq[n] = cut_freq[n - 1] + min_bw;
        double relative = cut_freq[n] / ((double)rate / 2.0);
        if (cut_freq[n] < BASS_CUT_OFF_HZ) {
            lower[n] = (size_t)(relative * (double)half);
            bass_cut_off_bar += 1;
            if (bass_cut_off_bar > 1)
                first_bar = 0;
            if (lower[n] > half)
                lower[n] = half;
        } else {
            lower[n] = (size_t)ceil(relative * (double)half);
            if (n == bass_cut_off_bar) {
                first_bar = 1;
                if (n > 0) {
                    size_t u = (size_t)(relative * (double)half);
                    upper[n - 1] = u > 0 ? u - 1 : 0;
                }
            } else {
                first_bar = 0;
            }
            if (lower[n] > half)
                lower[n] = half;
        }
        if (n > 0) {
            if (!first_bar) {
                upper[n - 1] = lower[n] > 0 ? lower[n] - 1 : 0;
                if (lower[n] <= lower[n - 1]) {
                    if (lower[n - 1] + 1 < half + 1) {
                        lower[n] = lower[n - 1] + 1;
                        upper[n - 1] = lower[n] - 1;
                    }
                }
            } else if (upper[n - 1] < lower[n - 1]) {
                upper[n - 1] = lower[n - 1] + 1;
            }
        }
        cut_freq[n] = (double)lower[n] / (double)half * ((double)rate / 2.0);
    }
    double *eq = calloc(bars, sizeof(double));
    for (size_t n = 0; n < bars; n++) {
        eq[n] = 1.0 / pow(2.0, 28.0);
        eq[n] *= pow(cut_freq[n + 1], 0.85);
        eq[n] /= 12.0;
        size_t span = upper[n] >= lower[n] ? upper[n] - lower[n] + 1 : 1;
        eq[n] /= (double)span;
        eq[n] *= 48000.0 / (double)rate;
    }
    free(cut_freq);
    double *multiplier = calloc(fft_size, sizeof(double));
    for (size_t i = 0; i < fft_size; i++)
        multiplier[i] = 0.5 * (1.0 - cos(2.0 * acos(-1.0) * (double)i / ((double)fft_size - 1.0)));
    d->lower_cut_off = lower;
    d->upper_cut_off = upper;
    d->eq = eq;
    d->multiplier = multiplier;
    d->input_buffer = calloc(d->input_buffer_size, sizeof(double));
    d->cava_fall = calloc(bars, sizeof(double));
    d->cava_mem = calloc(bars, sizeof(double));
    d->cava_peak = calloc(bars, sizeof(double));
    d->prev_cava_out = calloc(bars, sizeof(double));
    d->in_ = calloc(fft_size, sizeof(double));
    d->out_mag = calloc(fft_size / 2 + 1, sizeof(double));
    d->fft = fft_new(fft_size);
    if (!d->input_buffer || !d->cava_fall || !d->cava_mem || !d->cava_peak ||
        !d->prev_cava_out || !d->in_ || !d->out_mag || !d->fft || !multiplier || !eq) {
        dsp_free(d);
        return NULL;
    }
    return d;
}

void dsp_free(Dsp *d) {
    if (!d)
        return;
    free(d->input_buffer);
    free(d->lower_cut_off);
    free(d->upper_cut_off);
    free(d->eq);
    free(d->cava_fall);
    free(d->cava_mem);
    free(d->cava_peak);
    free(d->prev_cava_out);
    free(d->multiplier);
    free(d->in_);
    free(d->out_mag);
    fft_free(d->fft);
    free(d);
}

size_t dsp_render_frame_size(const Dsp *d) {
    return d->input_buffer_size;
}

void dsp_set_sens(Dsp *d, double sens, int init) {
    d->sens = sens;
    d->sens_init = init;
}

void dsp_get_sens(const Dsp *d, double *sens, int *init) {
    *sens = d->sens;
    *init = d->sens_init;
}

void dsp_set_sens_scale(Dsp *d, double s) {
    d->sens_scale = s;
}

void dsp_set_display_fps(Dsp *d, double fps) {
    d->display_fps = fps;
}

void dsp_execute(Dsp *d, const double *in_or_null, size_t n, double *out) {
    size_t size = d->input_buffer_size;
    size_t new_samples = n < size ? n : size;
    if (new_samples > 0) {
        if (in_or_null) {
            size_t i = size;
            while (i > new_samples) {
                i -= 1;
                d->input_buffer[i] = d->input_buffer[i - new_samples];
            }
            d->any_signal = 0;
            for (size_t k = 0; k < new_samples; k++) {
                double v = in_or_null[k];
                d->input_buffer[new_samples - k - 1] = v;
                if (v != 0.0)
                    d->any_signal = 1;
            }
        } else {
            d->any_signal = 0;
        }
    } else {
        d->any_signal = 0;
    }
    d->have_last_fft = 1;
    d->last_fft_ns = sv_now_ns();
    for (size_t i = 0; i < d->fft_size; i++)
        d->in_[i] = d->multiplier[i] * d->input_buffer[i];
    fft_process(d->fft, d->in_, d->out_mag, d->max_bin);
    for (size_t k = 0; k < d->number_of_bars; k++) {
        double temp = 0.0;
        for (size_t i = d->lower_cut_off[k]; i <= d->upper_cut_off[k]; i++)
            temp += d->out_mag[i];
        temp *= d->eq[k];
        out[k] = temp;
    }
    if (d->autosens) {
        for (size_t k = 0; k < d->number_of_bars; k++)
            out[k] *= d->sens;
    }
    int overshoot = 0;
    double fps = d->display_fps;
    if (fps < 1.0)
        fps = 1.0;
    if (fps > 1000.0)
        fps = 1000.0;
    double nr = d->noise_reduction > 0.01 ? d->noise_reduction : 0.01;
    double gravity_mod = pow(60.0 / fps, 2.5) * 1.54 / nr;
    if (gravity_mod < 1.0)
        gravity_mod = 1.0;
    for (size_t k = 0; k < d->number_of_bars; k++) {
        if (out[k] < d->prev_cava_out[k] && d->noise_reduction > 0.1) {
            out[k] = d->cava_peak[k] * (1.0 - d->cava_fall[k] * d->cava_fall[k] * gravity_mod);
            if (out[k] < 0.0)
                out[k] = 0.0;
            d->cava_fall[k] += 0.028;
        } else {
            d->cava_peak[k] = out[k];
            d->cava_fall[k] = 0.0;
        }
        d->prev_cava_out[k] = out[k];
        out[k] = d->cava_mem[k] * d->noise_reduction + out[k] * (1.0 - d->noise_reduction);
        d->cava_mem[k] = out[k];
        if (!d->any_signal && out[k] < 0.001) {
            out[k] = 0.001;
            d->cava_mem[k] = 0.001;
        }
        if (d->autosens && out[k] > 1.0) {
            overshoot = 1;
            out[k] = 1.0;
        }
    }
    if (d->autosens) {
        d->sens_step += 1;
        if (d->sens_step >= 3) {
            d->sens_step = 0;
            if (overshoot) {
                d->sens *= 0.98;
                d->sens_init = 0;
            } else if (d->any_signal) {
                d->sens *= 1.001;
                if (d->sens_init)
                    d->sens *= 2.0;
            }
        }
    }
    if (d->sens_scale != 1.0) {
        for (size_t k = 0; k < d->number_of_bars; k++)
            out[k] *= d->sens_scale;
    }
}
