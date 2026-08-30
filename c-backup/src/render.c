#include "render.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *const DEFAULT_GLYPHS =
    "\xe2\x96\x81\xe2\x96\x82\xe2\x96\x83\xe2\x96\x84\xe2\x96\x85\xe2\x96\x86"
    "\xe2\x96\x87\xe2\x96\x88"; /* ▁▂▃▄▅▆▇█ */

static const char *render_glyph(renderer_t *r, int gi) {
    if (gi <= 0)
        return " ";
    if (r->glyph_n <= 0)
        return " ";
    int n = r->glyph_n;
    int idx = (int)((double)(gi - 1) * (n - 1) / 7.0 + 0.5);
    if (idx < 0)
        idx = 0;
    if (idx >= n)
        idx = n - 1;
    return r->glyphs[idx];
}

void renderer_set_glyphs(renderer_t *r, const char *str) {
    const char *src = (str && *str) ? str : DEFAULT_GLYPHS;
    r->glyph_n = 0;
    const unsigned char *p = (const unsigned char *)src;
    while (*p && r->glyph_n < 64) {
        unsigned char c = *p;
        size_t seq = (c < 0x80) ? 1 : ((c & 0xE0) == 0xC0 ? 2 :
                                      (c & 0xF0) == 0xE0 ? 3 : 4);
        char *dst = r->glyphs[r->glyph_n];
        size_t i = 0;
        while (i < seq && i < 7 && *p) {
            dst[i++] = (char)*p++;
        }
        dst[i] = '\0';
        r->glyph_n++;
    }
    if (r->glyph_n == 0)
        r->glyph_n = 1;
    r->barstr_bw = 0; /* force barstr rebuild with the new glyphs */
}

static void app(char *out, size_t *olen, size_t cap, const char *s) {
    while (*s && *olen < cap)
        out[(*olen)++] = *s++;
}

static void appu(char *out, size_t *olen, size_t cap, unsigned v) {
    char buf[12];
    size_t n = 0;
    do {
        buf[n++] = (char)('0' + v % 10);
        v /= 10;
    } while (v);
    while (n && *olen < cap)
        out[(*olen)++] = buf[--n];
}

static void bar_color(const renderer_t *r, unsigned from_bottom, unsigned rows,
                      char *buf, size_t n) {
    long lo_r = (r->grad_lo >> 16) & 0xff, lo_g = (r->grad_lo >> 8) & 0xff,
         lo_b = r->grad_lo & 0xff;
    long hi_r = (r->grad_hi >> 16) & 0xff, hi_g = (r->grad_hi >> 8) & 0xff,
         hi_b = r->grad_hi & 0xff;
    double frac = rows > 1 ? (double)from_bottom / (double)(rows - 1) : 0.0;
    unsigned cr = (unsigned)(lo_r + (hi_r - lo_r) * frac + 0.5);
    unsigned cg = (unsigned)(lo_g + (hi_g - lo_g) * frac + 0.5);
    unsigned cb = (unsigned)(lo_b + (hi_b - lo_b) * frac + 0.5);
    if (cr > 255) cr = 255;
    if (cg > 255) cg = 255;
    if (cb > 255) cb = 255;
    size_t o = 0;
    if (r->color_256) {
        unsigned idx = 16 + 36 * ((cr * 6) / 256) + 6 * ((cg * 6) / 256) +
                       (cb * 6) / 256;
        const char *pre = "\x1b[38;5;";
        while (*pre && o + 1 < n)
            buf[o++] = *pre++;
        char t[12];
        size_t len = 0;
        do {
            t[len++] = (char)('0' + idx % 10);
            idx /= 10;
        } while (idx);
        while (len && o + 1 < n)
            buf[o++] = t[--len];
        buf[o++] = 'm';
        buf[o] = '\0';
        return;
    }
    const char *pre = "\x1b[38;2;";
    while (*pre && o + 1 < n)
        buf[o++] = *pre++;
    char t[12];
    size_t len = 0;
    do {
        t[len++] = (char)('0' + cr % 10);
        cr /= 10;
    } while (cr);
    while (len && o + 1 < n)
        buf[o++] = t[--len];
    buf[o++] = ';';
    len = 0;
    do {
        t[len++] = (char)('0' + cg % 10);
        cg /= 10;
    } while (cg);
    while (len && o + 1 < n)
        buf[o++] = t[--len];
    buf[o++] = ';';
    len = 0;
    do {
        t[len++] = (char)('0' + cb % 10);
        cb /= 10;
    } while (cb);
    while (len && o + 1 < n)
        buf[o++] = t[--len];
    buf[o++] = 'm';
    buf[o] = '\0';
}

typedef struct {
    bool active;
    char col[40];
} color_state;

static void seek_cell(long r, long c, char *out, size_t *out_len, size_t cap) {
    app(out, out_len, cap, "\x1b[");
    appu(out, out_len, cap, (unsigned)(r + 1));
    app(out, out_len, cap, ";");
    appu(out, out_len, cap, (unsigned)(c + 1));
    app(out, out_len, cap, "H");
}

static void emit_color_state(color_state *st, const char *pre, char *out,
                             size_t *out_len, size_t cap) {
    if (st->active && strcmp(st->col, pre) == 0)
        return;
    app(out, out_len, cap, pre);
    snprintf(st->col, sizeof st->col, "%s", pre);
    st->active = true;
}

static void row_colors(renderer_t *r) {
    for (unsigned y = 0; y < r->rows; y++)
        bar_color(r, r->rows - 1 - y, r->rows, r->row_col + (size_t)y * 40, 40);
}

static void emit_cell(renderer_t *r, unsigned y, size_t x, int gi, color_state *st,
                      char *out, size_t *out_len, size_t cap) {
    size_t idx = (size_t)y * r->cols + x;
    if ((unsigned char)gi == r->prev[idx])
        return;
    r->prev[idx] = (unsigned char)gi;
    seek_cell((long)y, (long)x, out, out_len, cap);
    if (gi == 0) {
        app(out, out_len, cap, " ");
    } else {
        emit_color_state(st, r->row_col + (size_t)y * 40, out, out_len, cap);
        app(out, out_len, cap, render_glyph(r, gi));
    }
}

void renderer_init(renderer_t *r, unsigned rows, unsigned cols, size_t bar_width,
                   size_t bar_spacing, size_t num_bars) {
    r->rows = rows;
    r->cols = cols;
    r->bar_width = bar_width ? bar_width : 1;
    r->bar_spacing = bar_spacing;
    r->num_bars = num_bars;
    r->color_256 = false;
    renderer_set_glyphs(r, NULL);
    r->grad_lo = 0xff0000u;
    r->grad_hi = 0x00ff00u;
    r->mode = RENDER_BARS;
    r->x_off = 0;
    r->barstr_bw = 0;
    r->wave_buf = NULL;
    r->wave_cap = 0;
    r->wave_pos = 0;
    r->wave_filled = 0;
    r->wave_spc = 1;
    r->lj_l = NULL;
    r->lj_r = NULL;
    r->lj_cap = 0;
    r->lj_pos = 0;
    r->lj_filled = 0;
    r->lj_spc = 1;
    r->stereo_in = false;
    r->lj_glow = NULL;
    r->prev = malloc((size_t)rows * cols);
    memset(r->prev, 0xFF, (size_t)rows * cols);
    r->lj_glow = calloc((size_t)rows * cols, 1);
    r->row_col = malloc((size_t)rows * 40);
    r->rowbuf = malloc(cols);
    r->db_x0 = 0;
    r->db_y0 = 0;
    r->db_x1 = cols ? cols - 1 : 0;
    r->db_y1 = rows ? rows - 1 : 0;
}

void renderer_resize(renderer_t *r, unsigned rows, unsigned cols, size_t num_bars) {
    free(r->prev);
    free(r->lj_glow);
    free(r->row_col);
    free(r->rowbuf);
    r->rows = rows;
    r->cols = cols;
    r->num_bars = num_bars;
    r->barstr_bw = 0;
    r->prev = malloc((size_t)rows * cols);
    memset(r->prev, 0xFF, (size_t)rows * cols);
    r->lj_glow = calloc((size_t)rows * cols, 1);
    r->row_col = malloc((size_t)rows * 40);
    r->rowbuf = malloc(cols);
    r->db_x0 = 0;
    r->db_y0 = 0;
    r->db_x1 = cols ? cols - 1 : 0;
    r->db_y1 = rows ? rows - 1 : 0;
}

void renderer_set_offset(renderer_t *r, size_t x_off) {
    if (r->x_off == x_off)
        return;
    r->x_off = x_off;
    free(r->prev);
    free(r->lj_glow);
    free(r->rowbuf);
    r->prev = malloc((size_t)r->rows * r->cols);
    memset(r->prev, 0xFF, (size_t)r->rows * r->cols);
    r->lj_glow = calloc((size_t)r->rows * r->cols, 1);
    r->rowbuf = malloc(r->cols);
    r->db_x0 = 0;
    r->db_y0 = 0;
    r->db_x1 = r->cols ? r->cols - 1 : 0;
    r->db_y1 = r->rows ? r->rows - 1 : 0;
}

void renderer_set_mode(renderer_t *r, render_mode m) {
    if (r->mode == m)
        return;
    r->mode = m;
    renderer_clear(r);
    if (m == RENDER_LISSAJOUS) {
        if (r->lj_glow)
            memset(r->lj_glow, 0, (size_t)r->rows * r->cols);
    }
}

render_mode renderer_mode_parse(const char *name) {
    if (name && strcmp(name, "wave") == 0)
        return RENDER_WAVE;
    if (name && strcmp(name, "lissajous") == 0)
        return RENDER_LISSAJOUS;
    return RENDER_BARS;
}

void renderer_set_wave(renderer_t *r, unsigned sample_rate) {
    size_t cap = sample_rate ? (size_t)sample_rate * 2 / 3 : 48000 * 2 / 3;
    if (cap < 4096)
        cap = 4096;
    size_t spc = sample_rate / 2000;
    if (spc < 1)
        spc = 1;
    size_t lj_spc = sample_rate / 800;
    if (lj_spc < 1)
        lj_spc = 1;
    size_t lj_win = sample_rate / 20;
    if (lj_win < 256)
        lj_win = 256;
    if (r->wave_buf && r->wave_cap == cap) {
        r->wave_spc = spc;
        r->lj_spc = lj_spc;
        r->lj_win = lj_win;
        return;
    }
    free(r->wave_buf);
    free(r->lj_l);
    free(r->lj_r);
    r->wave_buf = calloc(cap, sizeof *r->wave_buf);
    r->lj_l = calloc(cap, sizeof *r->lj_l);
    r->lj_r = calloc(cap, sizeof *r->lj_r);
    r->wave_cap = cap;
    r->wave_pos = 0;
    r->wave_filled = 0;
    r->wave_spc = spc;
    r->lj_cap = cap;
    r->lj_pos = 0;
    r->lj_filled = 0;
    r->lj_spc = lj_spc;
    r->lj_win = lj_win;
}

void renderer_feed(renderer_t *r, const double *left, const double *right,
                   size_t n) {
    if (r->mode != RENDER_WAVE && r->mode != RENDER_LISSAJOUS)
        return;
    if (!r->wave_buf || r->wave_cap == 0 || n == 0)
        return;
    r->stereo_in = right != NULL;
    for (size_t i = 0; i < n; i++) {
        double v = left ? left[i] : 0.0;
        if (right)
            v = (v + right[i]) * 0.5;
        r->wave_buf[r->wave_pos] = v;
        r->lj_l[r->lj_pos] = left ? left[i] : 0.0;
        r->lj_r[r->lj_pos] = right ? right[i] : (left ? left[i] : 0.0);
        r->wave_pos = (r->wave_pos + 1) % r->wave_cap;
        if (r->wave_filled < r->wave_cap)
            r->wave_filled++;
        r->lj_pos = (r->lj_pos + 1) % r->lj_cap;
        if (r->lj_filled < r->lj_cap)
            r->lj_filled++;
    }
}

void renderer_clear(renderer_t *r) {
    if (r->prev)
        memset(r->prev, 0xFF, (size_t)r->rows * r->cols);
    r->db_x0 = 0;
    r->db_y0 = 0;
    r->db_x1 = r->cols ? r->cols - 1 : 0;
    r->db_y1 = r->rows ? r->rows - 1 : 0;
}

void renderer_free(renderer_t *r) {
    free(r->prev);
    r->prev = NULL;
    free(r->rowbuf);
    r->rowbuf = NULL;
    free(r->row_col);
    r->row_col = NULL;
    free(r->wave_buf);
    r->wave_buf = NULL;
    free(r->lj_l);
    r->lj_l = NULL;
    free(r->lj_r);
    r->lj_r = NULL;
    free(r->lj_glow);
    r->lj_glow = NULL;
}

static void build_barstrings(renderer_t *r) {
    size_t bw = r->bar_width ? r->bar_width : 1;
    if (bw > 8)
        bw = 8;
    if (r->barstr_bw == bw)
        return;
    r->barstr_bw = bw;
    for (int gi = 0; gi <= 8; gi++) {
        size_t o = 0;
        for (size_t w = 0; w < bw && o + 3 < sizeof r->barstr[gi]; w++)
            app(r->barstr[gi], &o, sizeof r->barstr[gi], render_glyph(r, gi));
        r->barstr[gi][o] = '\0';
    }
    size_t o = 0;
    for (size_t w = 0; w < bw && o + 1 < sizeof r->spacestr; w++)
        app(r->spacestr, &o, sizeof r->spacestr, " ");
    r->spacestr[o] = '\0';
}

static void draw_bars(renderer_t *r, const double *left, const double *right,
                      size_t nbars, size_t per_ch_l, size_t x_start,
                      size_t region_w, char *out, size_t *out_len, size_t cap) {
    unsigned rows = r->rows;
    size_t cols = r->cols;
    if (rows == 0 || region_w == 0)
        return;

    size_t bw = r->bar_width ? r->bar_width : 1;
    size_t step = bw + r->bar_spacing;
    if (step == 0)
        step = 1;

    size_t used = nbars * step;
    size_t lead = used < region_w ? (region_w - used) / 2 : 0;
    size_t region_end = x_start + region_w;
    if (region_end > cols)
        region_end = cols;

    build_barstrings(r);

    color_state st = { 0 };

    for (unsigned y = 0; y < rows; y++) {
        unsigned fb = rows - 1 - y;
        size_t skip = 0;
        bool wrote = false;
        bool color_on = false;
        for (size_t b = 0; b < nbars; b++) {
            size_t col = x_start + lead + b * step;
            if (col >= region_end)
                break;
            const double *src;
            size_t vi;
            if (b < per_ch_l) {
                src = left;
                vi = per_ch_l - 1 - b;
            } else {
                src = right;
                vi = b - per_ch_l;
            }
            double v = src[vi];
            if (!(v > 0.0))
                v = 0.0;
            else if (v > 1.0)
                v = 1.0;
            double h = v * (double)rows;

            double frac = h - (double)fb;
            if (!(frac > 0.0))
                frac = 0.0;
            else if (frac > 1.0)
                frac = 1.0;
            int gi = (int)(frac * 8.0 + 0.9999);
            if (gi < 0)
                gi = 0;
            if (gi > 8)
                gi = 8;

            size_t idx = (size_t)y * cols + col;
            if ((unsigned char)gi == r->prev[idx]) {
                skip += step;
                continue;
            }
            r->prev[idx] = (unsigned char)gi;
            size_t wvis = region_end - col;
            if (wvis > bw)
                wvis = bw;
            for (size_t w = 1; w < wvis; w++)
                r->prev[idx + w] = (unsigned char)gi;

            if (!wrote) {
                seek_cell((long)y, (long)col, out, out_len, cap);
                wrote = true;
            } else if (skip > 0) {
                app(out, out_len, cap, "\x1b[");
                appu(out, out_len, cap, (unsigned)skip);
                app(out, out_len, cap, "C");
            }
            skip = 0;

            if (gi > 0) {
                if (!color_on) {
                    emit_color_state(&st, r->row_col + (size_t)y * 40, out,
                                     out_len, cap);
                    color_on = true;
                }
            if (wvis == bw)
                app(out, out_len, cap, r->barstr[gi]);
            else
                for (size_t w = 0; w < wvis; w++)
                    app(out, out_len, cap, render_glyph(r, gi));
            } else {
                if (wvis == bw)
                    app(out, out_len, cap, r->spacestr);
                else
                    for (size_t w = 0; w < wvis; w++)
                        app(out, out_len, cap, " ");
            }

            if (r->bar_spacing && b + 1 < nbars && col + step < region_end) {
                if (r->bar_spacing == 1)
                    app(out, out_len, cap, " ");
                else {
                    app(out, out_len, cap, "\x1b[");
                    appu(out, out_len, cap, (unsigned)r->bar_spacing);
                    app(out, out_len, cap, "C");
                }
            }
        }
    }

    for (size_t col = 0; col < region_w; col++) {
        bool in_bar;
        if (col >= lead) {
            size_t t = col - lead;
            in_bar = t / step < nbars && t % step < bw;
        } else {
            in_bar = false;
        }
        if (in_bar)
            continue;
        size_t abs_col = x_start + col;
        for (unsigned y = 0; y < rows; y++) {
            size_t idx = (size_t)y * cols + abs_col;
            if (r->prev[idx] == 0xFF)
                continue;
            emit_cell(r, y, abs_col, 0, &st, out, out_len, cap);
        }
    }
}

static void emit_row(renderer_t *r, unsigned y, size_t x_start, size_t region_w,
                     const unsigned char *tgt, color_state *st, char *out,
                     size_t *out_len, size_t cap) {
    size_t skip = 0;
    bool wrote = false;
    bool color_on = false;
    for (size_t c = 0; c < region_w; c++) {
        unsigned char gi = tgt[c];
        size_t idx = (size_t)y * r->cols + x_start + c;
        if (gi == r->prev[idx]) {
            skip++;
            continue;
        }
        r->prev[idx] = gi;
        if (!wrote) {
            seek_cell((long)y, (long)(x_start + c), out, out_len, cap);
            wrote = true;
        } else if (skip > 0) {
            app(out, out_len, cap, "\x1b[");
            appu(out, out_len, cap, (unsigned)skip);
            app(out, out_len, cap, "C");
        }
        skip = 0;
        if (gi > 0) {
            if (!color_on) {
                emit_color_state(st, r->row_col + (size_t)y * 40, out, out_len,
                                 cap);
                color_on = true;
            }
            app(out, out_len, cap, render_glyph(r, gi));
        } else {
            app(out, out_len, cap, " ");
        }
    }
}

static void draw_wave(renderer_t *r, size_t x_start, size_t region_w,
                      char *out, size_t *out_len, size_t cap) {
    if (!r->wave_buf || r->rows < 3 || region_w == 0)
        return;
    long yrow[4096];
    long lo[4096], hi[4096];
    size_t ncol = region_w < 4096 ? region_w : 4096;
    if (ncol == 0)
        return;

    size_t spc = r->wave_spc ? r->wave_spc : 1;
    double center = (double)(r->rows - 1) * 0.5;
    double height = (double)(r->rows - 2) * 0.5;

    for (size_t c = 0; c < ncol; c++) {
        size_t off = (region_w - 1 - c) * spc;
        if (off >= r->wave_filled) {
            yrow[c] = -1;
            lo[c] = (long)r->rows;
            hi[c] = -1;
            continue;
        }
        size_t idx = (r->wave_pos + r->wave_cap - 1 - off) % r->wave_cap;
        double v = r->wave_buf[idx];
        if (v < -1.0)
            v = -1.0;
        else if (v > 1.0)
            v = 1.0;
        yrow[c] = (long)(center - v * height + 0.5);
    }

    for (size_t c = 0; c < ncol; c++) {
        long cur = yrow[c];
        if (cur < 0) {
            lo[c] = -1;
            hi[c] = -1;
            continue;
        }
        long l = cur, h = cur;
        if (c + 1 < ncol && yrow[c + 1] >= 0) {
            long nxt = yrow[c + 1];
            if (nxt < l)
                l = nxt;
            if (nxt > h)
                h = nxt;
        } else if (c + 1 == ncol && c > 0 && yrow[c - 1] >= 0) {
            long nxt = yrow[c - 1]; /* cap the right edge to the previous column */
            if (nxt < l)
                l = nxt;
            if (nxt > h)
                h = nxt;
        }
        lo[c] = l;
        hi[c] = h;
    }

    color_state st = { 0 };
    unsigned cy0 = r->rows, cy1 = 0;
    for (size_t c = 0; c < ncol; c++) {
        if (hi[c] >= 0 && lo[c] <= hi[c]) {
            if ((unsigned)lo[c] < cy0)
                cy0 = (unsigned)lo[c];
            if ((unsigned)hi[c] > cy1)
                cy1 = (unsigned)hi[c];
        }
    }
    unsigned uy0 = cy0 < r->db_y0 ? cy0 : r->db_y0;
    unsigned uy1 = cy1 > r->db_y1 ? cy1 : r->db_y1;
    if (uy1 >= uy0) {
        for (unsigned y = uy0; y <= uy1; y++) {
            for (size_t c = 0; c < ncol; c++)
                r->rowbuf[c] = (long)y >= lo[c] && (long)y <= hi[c] ? 8 : 0;
            emit_row(r, y, x_start, ncol, r->rowbuf, &st, out, out_len, cap);
        }
    }
    r->db_y0 = cy0;
    r->db_y1 = cy1;
}

static void set_beam(renderer_t *r, long x, long y) {
    r->lj_glow[(size_t)y * r->cols + (size_t)x] = 255;
}

static void beam_line(renderer_t *r, long x0, long y0, long x1, long y1) {
    long dx = x1 > x0 ? x1 - x0 : x0 - x1;
    long dy = y1 > y0 ? y1 - y0 : y0 - y1;
    long sx = x0 < x1 ? 1 : -1;
    long sy = y0 < y1 ? 1 : -1;
    long err = dx - dy;
    for (;;) {
        set_beam(r, x0, y0);
        if (x0 == x1 && y0 == y1)
            break;
        long e2 = 2 * err;
        if (e2 > -dy) {
            err -= dy;
            x0 += sx;
        }
        if (e2 < dx) {
            err += dx;
            y0 += sy;
        }
    }
}

static void draw_lissajous(renderer_t *r, size_t x_start, size_t region_w,
                           char *out, size_t *out_len, size_t cap) {
    if (!r->lj_l || !r->lj_r || r->rows < 3 || region_w < 4)
        return;
    unsigned rows = r->rows;
    size_t cols = r->cols;

    memset(r->lj_glow, 0, (size_t)rows * cols);

    unsigned cx0 = cols, cy0 = rows, cx1 = 0, cy1 = 0;
    size_t i;
    size_t n = r->lj_filled;
    if (n > r->lj_win)
        n = r->lj_win;
    if (n > 1) {
        size_t delay = r->lj_spc ? r->lj_spc : 1;
        double cx = x_start + (region_w - 1) * 0.5;
        double cy = (rows - 1) * 0.5;
        double sxc = (region_w - 1) * 0.5;
        double syc = (rows - 1) * 0.5;
        long px = -1, py = -1;
        for (i = 0; i < n; i++) {
            size_t idx = (r->lj_pos + r->lj_cap - n + i) % r->lj_cap;
            double L = r->lj_l[idx];
            double R = r->lj_r[idx];
            if (!r->stereo_in) {
                size_t idx2 = (idx + r->lj_cap - delay) % r->lj_cap;
                R = r->lj_l[idx2];
            }
            if (L < -1.0)
                L = -1.0;
            else if (L > 1.0)
                L = 1.0;
            if (R < -1.0)
                R = -1.0;
            else if (R > 1.0)
                R = 1.0;
            long xx = (long)(cx + L * sxc + 0.5);
            long yy = (long)(cy - R * syc + 0.5);
            if (xx < (long)x_start || xx >= (long)(x_start + region_w) ||
                yy < 0 || yy >= (long)rows) {
                px = py = -1;
                continue;
            }
            if (px >= 0 && py >= 0)
                beam_line(r, px, py, xx, yy);
            else
                set_beam(r, xx, yy);
            if ((unsigned)xx < cx0)
                cx0 = (unsigned)xx;
            if ((unsigned)xx > cx1)
                cx1 = (unsigned)xx;
            if ((unsigned)yy < cy0)
                cy0 = (unsigned)yy;
            if ((unsigned)yy > cy1)
                cy1 = (unsigned)yy;
            px = xx;
            py = yy;
        }
    }

    color_state st = { 0 };
    unsigned ux0 = cx0 < r->db_x0 ? cx0 : r->db_x0;
    unsigned ux1 = cx1 > r->db_x1 ? cx1 : r->db_x1;
    unsigned uy0 = cy0 < r->db_y0 ? cy0 : r->db_y0;
    unsigned uy1 = cy1 > r->db_y1 ? cy1 : r->db_y1;
    if (ux1 >= ux0 && uy1 >= uy0) {
        for (unsigned y = uy0; y <= uy1; y++) {
            for (unsigned x = ux0; x <= ux1; x++)
                r->rowbuf[x - ux0] = r->lj_glow[(size_t)y * cols + x] ? 8 : 0;
            emit_row(r, y, ux0, (size_t)(ux1 - ux0 + 1), r->rowbuf, &st, out,
                     out_len, cap);
        }
    }
    r->db_x0 = cx0;
    r->db_x1 = cx1;
    r->db_y0 = cy0;
    r->db_y1 = cy1;
}

void renderer_draw(renderer_t *r, const double *values, char *out, size_t *out_len,
                   size_t cap) {
    size_t region = r->cols - r->x_off;
    if (region == 0)
        return;
    row_colors(r);
    if (r->mode == RENDER_WAVE)
        draw_wave(r, r->x_off, region, out, out_len, cap);
    else if (r->mode == RENDER_LISSAJOUS)
        draw_lissajous(r, r->x_off, region, out, out_len, cap);
    else
        draw_bars(r, values, NULL, r->num_bars, r->num_bars, r->x_off, region, out,
                  out_len, cap);
}

void renderer_draw_stereo(renderer_t *r, const double *left, const double *right,
                          size_t per_ch_l, char *out, size_t *out_len, size_t cap) {
    size_t region = r->cols - r->x_off;
    if (region == 0)
        return;
    row_colors(r);
    if (r->mode == RENDER_WAVE)
        draw_wave(r, r->x_off, region, out, out_len, cap);
    else if (r->mode == RENDER_LISSAJOUS)
        draw_lissajous(r, r->x_off, region, out, out_len, cap);
    else
        draw_bars(r, left, right, r->num_bars, per_ch_l, r->x_off, region, out,
                  out_len, cap);
}
