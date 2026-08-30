#ifndef SHARK_DSP_H
#define SHARK_DSP_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "fft.h"

typedef struct {
    size_t number_of_bars;
    unsigned rate;
    bool autosens;
    double sens;
    bool sens_init;
    double sens_scale;
    double framerate;
    size_t frame_skip;
    double noise_reduction;

    size_t fft_size;
    size_t input_buffer_size;
    size_t max_bin;

    uint64_t last_fft_ns;
    uint64_t fft_interval_ns;
    unsigned sens_step;
    bool any_signal;

    double *input_buffer;
    size_t *lower_cut_off;
    size_t *upper_cut_off;
    double *eq;

    double *cava_fall;
    double *cava_mem;
    double *cava_peak;
    double *prev_cava_out;

    double *multiplier;

    fft_t fft;

    double *in_;
    double *out_mag;
} dsp_t;

void dsp_init(dsp_t *dsp, size_t bars, unsigned rate, bool autosens,
              double noise_reduction, unsigned low_cut_off, unsigned high_cut_off);
void dsp_free(dsp_t *dsp);
void dsp_execute(dsp_t *dsp, const double *cava_in, size_t new_samples_in,
                 double *cava_out);

#endif
