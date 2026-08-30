#include "dsp.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

#define BASS_CUT_OFF_HZ 100.0
#define FFT_INTERVAL_NS 1000000000ull / 60 /* cap FFT rate, decoupled from render */

static uint64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

static size_t pick_fft_size(unsigned rate) {
    size_t s = 512;
    if (rate > 8125 && rate <= 16250) {
        s *= 2;
    } else if (rate > 16250 && rate <= 32500) {
        s *= 4;
    } else if (rate > 32500 && rate <= 75000) {
        s *= 8;
    } else if (rate > 75000 && rate <= 150000) {
        s *= 16;
    } else if (rate > 150000 && rate <= 300000) {
        s *= 32;
    } else if (rate > 300000) {
        s *= 64;
    }
    if (s > 4096)
        s = 4096;
    return s;
}

void dsp_init(dsp_t *d, size_t number_of_bars, unsigned rate, bool autosens,
              double noise_reduction, unsigned low_cut_off, unsigned high_cut_off) {
    d->number_of_bars = number_of_bars;
    d->rate = rate;
    d->autosens = autosens;
    d->noise_reduction = noise_reduction;
    d->sens = 100.0;
    d->sens_init = true;
    d->sens_scale = 1.0;
    d->framerate = 75.0;
    d->frame_skip = 1;
    d->sens_step = 0;
    d->any_signal = false;

    d->fft_size = pick_fft_size(rate);
    d->input_buffer_size = d->fft_size;
    d->last_fft_ns = 0;
    d->fft_interval_ns = 0;

    d->max_bin =
        (size_t)ceil((double)high_cut_off / (double)rate * (double)d->fft_size);
    if (d->max_bin > d->fft_size / 2)
        d->max_bin = d->fft_size / 2;

    double lower_lo = (double)low_cut_off;
    double upper_hi = (double)high_cut_off;
    size_t half = d->fft_size / 2;

    double frequency_constant =
        log10(lower_lo / upper_hi) / (1.0 / ((double)number_of_bars + 1.0) - 1.0);

    size_t *lower = malloc((number_of_bars + 1) * sizeof *lower);
    size_t *upper = calloc((number_of_bars + 1), sizeof *upper);
    double *cut_freq = malloc((number_of_bars + 1) * sizeof *cut_freq);

    size_t bass_cut_off_bar = 0;
    bool first_bar = true;
    double min_bandwidth = (double)rate / (double)d->fft_size;

    for (size_t n = 0; n <= number_of_bars; n++) {
        double bdc = frequency_constant * -1.0;
        bdc += ((double)n + 1.0) / ((double)number_of_bars + 1.0) * frequency_constant;
        cut_freq[n] = upper_hi * pow(10.0, bdc);

        if (n > 0 && cut_freq[n - 1] >= cut_freq[n])
            cut_freq[n] = cut_freq[n - 1] + min_bandwidth;

        double relative = cut_freq[n] / ((double)rate / 2.0);

        if (cut_freq[n] < BASS_CUT_OFF_HZ) {
            lower[n] = (size_t)(relative * (double)half);
            bass_cut_off_bar++;
            if (bass_cut_off_bar > 1)
                first_bar = false;
            if (lower[n] > half)
                lower[n] = half;
        } else {
            lower[n] = (size_t)ceil(relative * (double)half);
            if (n == bass_cut_off_bar) {
                first_bar = true;
                if (n > 0)
                    upper[n - 1] = (size_t)(relative * (double)half) - 1;
            } else {
                first_bar = false;
            }
            if (lower[n] > half)
                lower[n] = half;
        }

        if (n > 0) {
            if (!first_bar) {
                upper[n - 1] = lower[n] - 1;
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

    double *eq = malloc(number_of_bars * sizeof *eq);
    for (size_t n = 0; n < number_of_bars; n++) {
        eq[n] = 1.0 / pow(2.0, 28);
        eq[n] *= pow(cut_freq[n + 1], 0.85);
        eq[n] /= log2((double)d->fft_size);
        eq[n] /= (double)(upper[n] - lower[n] + 1);
    }

    free(cut_freq);

    double *multiplier = malloc(d->fft_size * sizeof *multiplier);
    for (size_t i = 0; i < d->fft_size; i++)
        multiplier[i] =
            0.5 * (1.0 - cos(2.0 * M_PI * (double)i / ((double)d->fft_size - 1.0)));

    d->lower_cut_off = lower;
    d->upper_cut_off = upper;
    d->eq = eq;
    d->multiplier = multiplier;

    d->input_buffer = calloc(d->input_buffer_size, sizeof *d->input_buffer);
    d->cava_fall = calloc(number_of_bars, sizeof *d->cava_fall);
    d->cava_mem = calloc(number_of_bars, sizeof *d->cava_mem);
    d->cava_peak = calloc(number_of_bars, sizeof *d->cava_peak);
    d->prev_cava_out = calloc(number_of_bars, sizeof *d->prev_cava_out);

    d->in_ = malloc(d->fft_size * sizeof *d->in_);
    d->out_mag = malloc((d->fft_size / 2 + 1) * sizeof *d->out_mag);

    fft_init(&d->fft, d->fft_size);
}

void dsp_free(dsp_t *d) {
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
    fft_free(&d->fft);
}

void dsp_execute(dsp_t *d, const double *cava_in, size_t new_samples_in,
                 double *cava_out) {
    size_t new_samples = new_samples_in < d->input_buffer_size
        ? new_samples_in
        : d->input_buffer_size;

    if (new_samples > 0) {
        d->framerate -= d->framerate / 64.0;
        d->framerate += (double)(d->rate * d->frame_skip) / (double)new_samples / 64.0;
        d->frame_skip = 1;

        size_t size = d->input_buffer_size;
        memmove(d->input_buffer + new_samples, d->input_buffer,
                (size - new_samples) * sizeof *d->input_buffer);
        d->any_signal = false;
        for (size_t n = 0; n < new_samples; n++) {
            double v = cava_in[n];
            d->input_buffer[new_samples - n - 1] = v;
            if (v != 0.0)
                d->any_signal = true;
        }
    } else {
        d->frame_skip++;
        return;
    }

    if (now_ns() - d->last_fft_ns < d->fft_interval_ns)
        return;
    d->last_fft_ns = now_ns();

    for (size_t i = 0; i < d->fft_size; i++)
        d->in_[i] = d->multiplier[i] * d->input_buffer[i];
    fft_process(&d->fft, d->in_, d->out_mag, d->max_bin);

    for (size_t n = 0; n < d->number_of_bars; n++) {
        double temp = 0.0;
        for (size_t i = d->lower_cut_off[n]; i <= d->upper_cut_off[n]; i++)
            temp += d->out_mag[i];
        temp *= d->eq[n];
        cava_out[n] = temp;
    }

    if (d->autosens) {
        for (size_t n = 0; n < d->number_of_bars; n++)
            cava_out[n] *= d->sens;
    }

    bool overshoot = false;
    double gravity_mod =
        pow(60.0 / d->framerate, 2.5) * 1.54 / fmax(d->noise_reduction, 0.01);
    if (gravity_mod < 1.0)
        gravity_mod = 1.0;

    for (size_t n = 0; n < d->number_of_bars; n++) {
        if (cava_out[n] < d->prev_cava_out[n] && d->noise_reduction > 0.1) {
            cava_out[n] =
                d->cava_peak[n] * (1.0 - d->cava_fall[n] * d->cava_fall[n] * gravity_mod);
            if (cava_out[n] < 0.0)
                cava_out[n] = 0.0;
            d->cava_fall[n] += 0.028;
        } else {
            d->cava_peak[n] = cava_out[n];
            d->cava_fall[n] = 0.0;
        }
        d->prev_cava_out[n] = cava_out[n];

        cava_out[n] = d->cava_mem[n] * d->noise_reduction +
                      cava_out[n] * (1.0 - d->noise_reduction);
        d->cava_mem[n] = cava_out[n];

        if (!d->any_signal && cava_out[n] < 0.001) {
            cava_out[n] = 0.001;
            d->cava_mem[n] = 0.001;
        }

        if (d->autosens) {
            if (cava_out[n] > 1.0) {
                overshoot = true;
                cava_out[n] = 1.0;
            }
        }
    }

    if (d->autosens && ++d->sens_step >= 3) {
        d->sens_step = 0;
        if (overshoot) {
            d->sens *= 0.98;
            d->sens_init = false;
        } else if (d->any_signal) {
            d->sens *= 1.001;
            if (d->sens_init)
                d->sens *= 2.0;
        }
    }

    if (d->sens_scale != 1.0) {
        for (size_t n = 0; n < d->number_of_bars; n++)
            cava_out[n] *= d->sens_scale;
    }
}
