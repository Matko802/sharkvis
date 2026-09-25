#include <math.h>
#include <stdlib.h>

#include "fft.h"

struct Fft {
    size_t n;
    double *re;
    double *im;
    size_t *rev;
    double *cos_t;
    double *sin_t;
};

Fft *fft_new(size_t n) {
    Fft *f = calloc(1, sizeof *f);
    if (!f)
        return NULL;
    f->n = n;
    f->re = calloc(n, sizeof(double));
    f->im = calloc(n, sizeof(double));
    f->rev = calloc(n, sizeof(size_t));
    f->cos_t = calloc(n / 2, sizeof(double));
    f->sin_t = calloc(n / 2, sizeof(double));
    if (!f->re || !f->im || !f->rev || !f->cos_t || !f->sin_t) {
        fft_free(f);
        return NULL;
    }
    size_t bits = 0;
    size_t m = n;
    while (m > 1) {
        bits++;
        m >>= 1;
    }
    for (size_t i = 0; i < n; i++) {
        size_t r = 0;
        for (size_t b = 0; b < bits; b++) {
            if (i & ((size_t)1 << b))
                r |= (size_t)1 << (bits - 1 - b);
        }
        f->rev[i] = r;
    }
    for (size_t i = 0; i < n / 2; i++) {
        double a = 2.0 * acos(-1.0) * (double)i / (double)n;
        f->cos_t[i] = cos(a);
        f->sin_t[i] = sin(a);
    }
    return f;
}

void fft_free(Fft *f) {
    if (!f)
        return;
    free(f->re);
    free(f->im);
    free(f->rev);
    free(f->cos_t);
    free(f->sin_t);
    free(f);
}

size_t fft_size(const Fft *f) {
    return f->n;
}

void fft_process(Fft *f, const double *in, double *mag, size_t max_bin) {
    size_t n = f->n;
    for (size_t i = 0; i < n; i++) {
        f->re[f->rev[i]] = in[i];
        f->im[f->rev[i]] = 0.0;
    }
    for (size_t len = 2; len <= n; len <<= 1) {
        size_t half = len >> 1;
        size_t step = n / len;
        for (size_t i = 0; i < n; i += len) {
            for (size_t j = 0; j < half; j++) {
                double wr = f->cos_t[j * step];
                double wi = -f->sin_t[j * step];
                double ur = f->re[i + j];
                double ui = f->im[i + j];
                double vr = f->re[i + j + half] * wr - f->im[i + j + half] * wi;
                double vi = f->re[i + j + half] * wi + f->im[i + j + half] * wr;
                f->re[i + j] = ur + vr;
                f->im[i + j] = ui + vi;
                f->re[i + j + half] = ur - vr;
                f->im[i + j + half] = ui - vi;
            }
        }
    }
    size_t last = max_bin + 1;
    if (last > n)
        last = n;
    for (size_t k = 0; k < last; k++)
        mag[k] = sqrt(f->re[k] * f->re[k] + f->im[k] * f->im[k]);
}
