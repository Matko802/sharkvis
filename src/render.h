#ifndef SHARKVIS_RENDER_H
#define SHARKVIS_RENDER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "common.h"

typedef enum {
    RM_BARS,
    RM_WAVE,
    RM_SCOPE,
    RM_LYRICS
} RenderMode;

typedef struct {
    size_t rows;
    size_t cols;
    size_t bar_width;
    size_t bar_spacing;
    size_t num_bars;
    int color_256;
    unsigned grad_lo;
    unsigned grad_hi;
    int grad_lo_term;
    int grad_hi_term;
    RenderMode mode;
    size_t x_off;
    uint8_t *prev;
    uint8_t *rowbuf;
    size_t db_x0;
    size_t db_y0;
    size_t db_x1;
    size_t db_y1;
    uint8_t **row_col;
    size_t *row_col_len;
    unsigned gs_lo;
    unsigned gs_hi;
    int gs_256;
    size_t gs_rows;
    int gs_lo_term;
    int gs_hi_term;
    uint8_t *barstr[9];
    size_t barstr_len[9];
    uint8_t *spacestr;
    size_t spacestr_len;
    size_t barstr_bw;
    uint8_t **glyphs;
    size_t *glyph_len;
    size_t glyph_count;
    double *wave_buf;
    size_t wave_cap;
    size_t wave_pos;
    size_t wave_filled;
    size_t wave_spc;
    unsigned wave_rate;
    double *osc_l;
    double *osc_r;
    size_t osc_cap;
    size_t osc_pos;
    size_t osc_filled;
    size_t osc_spc;
    size_t osc_win;
    int stereo_in;
    uint8_t *osc_glow;
    long long *sc_yrow;
    long long *sc_lo;
    long long *sc_hi;
    long long *sc_lo2;
    long long *sc_hi2;
    uint32_t *text;
    char *text_dim;
    size_t text_len;
    size_t focus;
    int text_left;
    size_t text_size;
    int text_small;
    int loading;
    size_t yscale;
    int spin_active;
    uint64_t spin_t0_ms;
} Renderer;

Renderer *renderer_new(size_t rows, size_t cols, size_t bar_width,
                       size_t bar_spacing, size_t num_bars);
void renderer_free(Renderer *r);
void renderer_resize(Renderer *r, size_t rows, size_t cols, size_t num_bars);
void renderer_set_offset(Renderer *r, size_t x_off);
void renderer_set_mode(Renderer *r, RenderMode m);
RenderMode mode_parse(const char *name);
void renderer_set_text(Renderer *r, const char *s);
void renderer_set_rich(Renderer *r, const char **strs, const char *cur, size_t n);
void renderer_set_wave(Renderer *r, unsigned sample_rate);
void renderer_set_glyphs(Renderer *r, const uint8_t *src, size_t len);
void renderer_feed(Renderer *r, const double *left, const double *right, size_t n);
void renderer_clear(Renderer *r);
void renderer_draw(Renderer *r, const double *values, SvBuf *out, size_t cap);
void renderer_draw_stereo(Renderer *r, const double *left, const double *right,
                          size_t per_ch_l, size_t per_ch_r, SvBuf *out, size_t cap);

#endif
