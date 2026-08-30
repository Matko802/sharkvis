#ifndef SHARK_FFT_H
#define SHARK_FFT_H

#include <stddef.h>

typedef struct {
    size_t n;
    size_t *rev;
    double *cos;
    double *sin;
    double *ccos;
    double *csin;
    double *re;
    double *im;
} fft_t;

void fft_init(fft_t *fft, size_t n);
void fft_free(fft_t *fft);
void fft_process(fft_t *fft, const double *input, double *out_mag, size_t max_bin);

#endif
