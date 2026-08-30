#ifndef SHARK_RENDER_H
#define SHARK_RENDER_H

#include <stdbool.h>
#include <stddef.h>

typedef enum {
    RENDER_BARS,
    RENDER_WAVE,
    RENDER_LISSAJOUS,
} render_mode;

typedef struct {
    unsigned rows;
    unsigned cols;
    size_t bar_width;
    size_t bar_spacing;
    size_t num_bars;
    bool color_256;
    unsigned grad_lo;
    unsigned grad_hi;
    render_mode mode;
    size_t x_off;
    unsigned char *prev;
    unsigned char *rowbuf;
    unsigned db_x0, db_y0, db_x1, db_y1;
    char *row_col;
    char barstr[9][32];
    char spacestr[32];
    size_t barstr_bw;
    char glyphs[64][8];
    int glyph_n;
    double *wave_buf;
    size_t wave_cap;
    size_t wave_pos;
    size_t wave_filled;
    size_t wave_spc;
    double *lj_l;
    double *lj_r;
    size_t lj_cap;
    size_t lj_pos;
    size_t lj_filled;
    size_t lj_spc;
    size_t lj_win;
    bool stereo_in;
    unsigned char *lj_glow;
} renderer_t;

void renderer_init(renderer_t *r, unsigned rows, unsigned cols, size_t bar_width,
                   size_t bar_spacing, size_t num_bars);
void renderer_resize(renderer_t *r, unsigned rows, unsigned cols, size_t num_bars);
void renderer_set_offset(renderer_t *r, size_t x_off);
void renderer_set_mode(renderer_t *r, render_mode m);
render_mode renderer_mode_parse(const char *name);
void renderer_set_glyphs(renderer_t *r, const char *str);
void renderer_set_wave(renderer_t *r, unsigned sample_rate);
void renderer_feed(renderer_t *r, const double *left, const double *right, size_t n);
void renderer_clear(renderer_t *r);
void renderer_free(renderer_t *r);
void renderer_draw(renderer_t *r, const double *values, char *out, size_t *out_len,
                   size_t cap);
void renderer_draw_stereo(renderer_t *r, const double *left, const double *right,
                          size_t per_ch_l, char *out, size_t *out_len, size_t cap);

#endif
