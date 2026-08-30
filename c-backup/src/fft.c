#include "fft.h"

#include <math.h>
#include <stdlib.h>

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

void fft_init(fft_t *f, size_t n) {
    size_t bits = 0;
    while (((size_t)1 << bits) < n / 2)
        bits++;
    size_t half = n / 2;

    f->n = n;
    f->rev = malloc(half * sizeof *f->rev);
    f->cos = malloc(half * sizeof *f->cos);
    f->sin = malloc(half * sizeof *f->sin);
    f->ccos = malloc((half / 2) * sizeof *f->ccos);
    f->csin = malloc((half / 2) * sizeof *f->csin);
    f->re = malloc(half * sizeof *f->re);
    f->im = malloc(half * sizeof *f->im);

    for (size_t i = 0; i < half; i++) {
        size_t v = 0, x = i;
        for (size_t b = 0; b < bits; b++) {
            v = (v << 1) | (x & 1);
            x >>= 1;
        }
        f->rev[i] = v;
    }
    /* twiddles for the combining pass: e^{-2*pi*i*k/n} */
    for (size_t k = 0; k < half; k++) {
        double a = -2.0 * M_PI * (double)k / (double)n;
        f->cos[k] = cos(a);
        f->sin[k] = sin(a);
    }
    /* twiddles for the internal half-size complex transform */
    for (size_t k = 0; k < half / 2; k++) {
        double a = -2.0 * M_PI * (double)k / (double)half;
        f->ccos[k] = cos(a);
        f->csin[k] = sin(a);
    }
}

void fft_free(fft_t *f) {
    free(f->rev);
    free(f->cos);
    free(f->sin);
    free(f->ccos);
    free(f->csin);
    free(f->re);
    free(f->im);
    f->rev = NULL;
    f->cos = NULL;
    f->sin = NULL;
    f->ccos = NULL;
    f->csin = NULL;
    f->re = NULL;
    f->im = NULL;
}

static void fft_complex_half(fft_t *f) {
    size_t n = f->n / 2;
    for (size_t size = 2; size <= n; size <<= 1) {
        size_t h = size / 2;
        size_t step = n / size;
        for (size_t i = 0; i < n; i += size) {
            for (size_t j = 0; j < h; j++) {
                size_t k = j * step;
                double c = f->ccos[k], s = f->csin[k];
                double tre = c * f->re[i + j + h] - s * f->im[i + j + h];
                double tim = c * f->im[i + j + h] + s * f->re[i + j + h];
                double ure = f->re[i + j];
                double uim = f->im[i + j];
                f->re[i + j] = ure + tre;
                f->im[i + j] = uim + tim;
                f->re[i + j + h] = ure - tre;
                f->im[i + j + h] = uim - tim;
            }
        }
    }
}

void fft_process(fft_t *f, const double *input, double *out_mag, size_t max_bin) {
    size_t n = f->n;
    size_t half = n / 2;

    double se = 0.0, so = 0.0; /* X[half] = sum x[2j] - sum x[2j+1] */
    for (size_t j = 0; j < half; j++) {
        size_t r = f->rev[j];
        f->re[r] = input[2 * j];
        f->im[r] = input[2 * j + 1];
        se += input[2 * j];
        so += input[2 * j + 1];
    }
    fft_complex_half(f);

    out_mag[0] = fabs(f->re[0] + f->im[0]);
    size_t last = max_bin < half ? max_bin : half - 1;
    for (size_t k = 1; k <= last; k++) {
        size_t hk = half - k;
        double ze = f->re[k], zi = f->im[k];
        double zh_e = f->re[hk], zh_i = f->im[hk];
        double xe = 0.5 * (ze + zh_e);
        double xi = 0.5 * (zi - zh_i);
        double de = 0.5 * (ze - zh_e);
        double di = 0.5 * (zi + zh_i);
        double oe = di, oi = -de;
        double c = f->cos[k], s = f->sin[k];
        double re_k = xe + c * oe - s * oi;
        double im_k = xi + c * oi + s * oe;
        out_mag[k] = sqrt(re_k * re_k + im_k * im_k);
    }
    if (max_bin >= half)
        out_mag[half] = fabs(se - so);
}
