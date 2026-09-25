#ifndef SHARKVIS_FFT_H
#define SHARKVIS_FFT_H

#include <stddef.h>

typedef struct Fft Fft;

Fft *fft_new(size_t n);
void fft_free(Fft *f);
size_t fft_size(const Fft *f);
void fft_process(Fft *f, const double *in, double *mag, size_t max_bin);

#endif
