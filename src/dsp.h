#ifndef SHARKVIS_DSP_H
#define SHARKVIS_DSP_H

#include <stdbool.h>
#include <stddef.h>

typedef struct Dsp Dsp;

Dsp *dsp_new(size_t bars, unsigned rate, int autosens, double noise,
             unsigned lo_cut, unsigned hi_cut);
void dsp_free(Dsp *d);
size_t dsp_render_frame_size(const Dsp *d);
void dsp_execute(Dsp *d, const double *in_or_null, size_t n, double *out);

void dsp_set_sens(Dsp *d, double sens, int init);
void dsp_get_sens(const Dsp *d, double *sens, int *init);
void dsp_set_sens_scale(Dsp *d, double s);
void dsp_set_display_fps(Dsp *d, double fps);
double dsp_raw_peak(const Dsp *d);

#endif
