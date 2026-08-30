#include "settings.h"

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "term.h"

typedef enum {
    S_BARS,
    S_BARW,
    S_SPACING,
    S_FPS,
    S_SENS,
    S_AUTO,
    S_NOISE,
    S_LOW,
    S_HIGH,
    S_CMODE,
    S_GLO,
    S_GHI,
    S_MODE,
    S_RATE,
    S_CH,
    S_CHARSET,
    S_COUNT,
} sid;

#define S_RESET S_COUNT
#define S_ROWS (S_COUNT + 1)
#define CONFIRM_TIMEOUT_MS 5000

static const char *const LABELS[S_COUNT] = {
    "bars",
    "bar width",
    "bar spacing",
    "framerate",
    "sensitivity",
    "autosens",
    "smoothing",
    "lower cutoff",
    "upper cutoff",
    "color mode",
    "color low",
    "color high",
    "visualizer",
    "sample rate",
    "channels",
    "charset",
};

static const unsigned RATES[] = { 8000, 11025, 16000, 22050, 32000, 44100,
                                  48000, 96000, 192000 };
#define RATES_N ((int)(sizeof RATES / sizeof RATES[0]))

static long clamp_l(long v, long lo, long hi) {
    if (v < lo)
        v = lo;
    if (v > hi)
        v = hi;
    return v;
}

static double clamp_d(double v, double lo, double hi) {
    if (v < lo)
        v = lo;
    if (v > hi)
        v = hi;
    return v;
}

struct settings_ui {
    int sel;
    bool confirm_reset;
    long confirm_deadline_ms;
};

static long now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000L + ts.tv_nsec / 1000000L;
}

settings_ui *settings_new(void) {
    return calloc(1, sizeof(settings_ui));
}

void settings_free(settings_ui *s) {
    free(s);
}

static void adjust(srk_config *c, int id, int dir, unsigned *changed) {
    switch (id) {
    case S_BARS: {
        long v = clamp_l((long)c->bars + dir, 0, 256);
        if ((size_t)v != c->bars) {
            c->bars = (size_t)v;
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_BARW: {
        long v = clamp_l((long)c->bar_width + dir, 1, 8);
        if ((size_t)v != c->bar_width) {
            c->bar_width = (size_t)v;
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_SPACING: {
        long v = clamp_l((long)c->bar_spacing + dir, 0, 4);
        if ((size_t)v != c->bar_spacing) {
            c->bar_spacing = (size_t)v;
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_FPS: {
        long v = clamp_l((long)c->framerate + dir * 5, 5, 240);
        if ((unsigned)v != c->framerate)
            c->framerate = (unsigned)v;
        break;
    }
    case S_SENS: {
        double v = clamp_d(c->sensitivity + dir * 5.0, 5.0, 200.0);
        if (v != c->sensitivity)
            c->sensitivity = v;
        break;
    }
    case S_AUTO: {
        bool v = !c->autosens;
        if (v != c->autosens) {
            c->autosens = v;
            *changed |= CH_DSP;
        }
        break;
    }
    case S_NOISE: {
        double v = clamp_d(c->noise_reduction + dir * 0.05, 0.0, 1.0);
        if (v != c->noise_reduction) {
            c->noise_reduction = v;
            *changed |= CH_DSP;
        }
        break;
    }
    case S_LOW: {
        long v = clamp_l((long)c->lower_cutoff + dir * 25, 25, 20000);
        if (v >= (long)c->higher_cutoff)
            v = (long)(c->higher_cutoff - 1) / 25 * 25;
        if ((unsigned)v != c->lower_cutoff) {
            c->lower_cutoff = (unsigned)v;
            *changed |= CH_DSP;
        }
        break;
    }
    case S_HIGH: {
        long v = clamp_l((long)c->higher_cutoff + dir * 500, 500, 24000);
        if (v <= (long)c->lower_cutoff)
            v = ((long)c->lower_cutoff / 500 + 1) * 500;
        if ((unsigned)v != c->higher_cutoff) {
            c->higher_cutoff = (unsigned)v;
            *changed |= CH_DSP;
        }
        break;
    }
    case S_CMODE: {
        bool v = !c->color_256;
        if (v != c->color_256) {
            c->color_256 = v;
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_GLO:
    case S_GHI: {
        char **dst = (id == S_GLO) ? &c->gradient_low : &c->gradient_high;
        int idx = color_index(*dst);
        if (idx < 0)
            idx = 0;
        idx = (idx + dir + g_palette_n) % g_palette_n;
        free(*dst);
        *dst = strdup(g_palette[idx].hex);
        *changed |= CH_LAYOUT;
        break;
    }
    case S_MODE: {
        static const char *const MODES[] = { "bars", "wave", "lissajous" };
        int idx = 0;
        for (int i = 0; i < 3; i++) {
            if (c->mode && strcmp(c->mode, MODES[i]) == 0) {
                idx = i;
                break;
            }
        }
        idx = (idx + dir + 3) % 3;
        if (!c->mode || strcmp(c->mode, MODES[idx]) != 0) {
            free(c->mode);
            c->mode = strdup(MODES[idx]);
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_RATE: {
        int idx = 0;
        for (int i = 0; i < RATES_N; i++) {
            if (RATES[i] <= c->sample_rate)
                idx = i;
        }
        idx = clamp_l(idx + dir, 0, RATES_N - 1);
        if (RATES[idx] != c->sample_rate) {
            c->sample_rate = RATES[idx];
            *changed |= CH_AUDIO;
        }
        break;
    }
    case S_CH: {
        unsigned v = (c->channels == 1) ? 2 : 1;
        if (v != c->channels) {
            c->channels = v;
            *changed |= CH_AUDIO;
        }
        break;
    }
    }
}

static void handle_reset(settings_ui *s, srk_config *cfg, unsigned *changed) {
    if (!s->confirm_reset) {
        s->confirm_reset = true;
        s->confirm_deadline_ms = now_ms() + CONFIRM_TIMEOUT_MS;
        return;
    }
    s->confirm_reset = false;
    free(cfg->source);
    config_default(cfg);
    *changed |= CH_LAYOUT | CH_DSP | CH_AUDIO;
}

void settings_key(settings_ui *s, srk_config *cfg, int key, const char *cp,
                 unsigned *changed) {
    switch (key) {
    case KEY_UP:
        s->sel = (s->sel + S_ROWS - 1) % S_ROWS;
        s->confirm_reset = false;
        break;
    case KEY_DOWN:
        s->sel = (s->sel + 1) % S_ROWS;
        s->confirm_reset = false;
        break;
    case KEY_LEFT:
        if (s->sel == S_RESET)
            handle_reset(s, cfg, changed);
        else
            adjust(cfg, s->sel, -1, changed);
        break;
    case KEY_RIGHT:
        if (s->sel == S_RESET)
            handle_reset(s, cfg, changed);
        else
            adjust(cfg, s->sel, +1, changed);
        break;
    case KEY_ENTER:
        if (s->sel == S_CHARSET)
            *changed |= CH_EDITOR;
        break;
    case KEY_CHAR:
        if (!cp)
            break;
        if (cp[0] == '-') {
            if (s->sel != S_RESET)
                adjust(cfg, s->sel, -1, changed);
        } else if (cp[0] == '+' || cp[0] == '=') {
            if (s->sel != S_RESET)
                adjust(cfg, s->sel, +1, changed);
        }
        break;
    default:
        break;
    }
}

static void format_value(const srk_config *c, int id, char *buf, size_t n) {
    switch (id) {
    case S_BARS:
        if (c->bars == 0)
            snprintf(buf, n, "auto");
        else
            snprintf(buf, n, "%zu", c->bars);
        break;
    case S_AUTO:
        snprintf(buf, n, "%s", c->autosens ? "on" : "off");
        break;
    case S_CMODE:
        snprintf(buf, n, "%s", c->color_256 ? "256" : "24bit");
        break;
    case S_GLO:
    case S_GHI: {
        const char *hx = (id == S_GLO) ? c->gradient_low : c->gradient_high;
        const char *nm = color_name(hx);
        snprintf(buf, n, "%s", nm ? nm : (hx ? hx : "?"));
        break;
    }
    case S_MODE:
        snprintf(buf, n, "%s", c->mode ? c->mode : "bars");
        break;
    case S_NOISE:
        snprintf(buf, n, "%.2f", c->noise_reduction);
        break;
    case S_SENS:
        snprintf(buf, n, "%.0f", c->sensitivity);
        break;
    case S_BARW:
        snprintf(buf, n, "%zu", c->bar_width);
        break;
    case S_SPACING:
        snprintf(buf, n, "%zu", c->bar_spacing);
        break;
    case S_FPS:
        snprintf(buf, n, "%u", c->framerate);
        break;
    case S_LOW:
        snprintf(buf, n, "%u", c->lower_cutoff);
        break;
    case S_HIGH:
        snprintf(buf, n, "%u", c->higher_cutoff);
        break;
    case S_RATE:
        snprintf(buf, n, "%u", c->sample_rate);
        break;
    case S_CH:
        snprintf(buf, n, "%u", c->channels);
        break;
    case S_CHARSET:
        snprintf(buf, n, "%s", c->glyphs ? c->glyphs : "");
        break;
    default:
        buf[0] = '\0';
        break;
    }
}

static void write_esc(char *out, size_t *n, size_t cap, const char *fmt, ...) {
    if (*n >= cap)
        return;
    size_t room = cap - *n;
    va_list ap;
    va_start(ap, fmt);
    int k = vsnprintf(out + *n, room, fmt, ap);
    va_end(ap);
    if (k < 0)
        return;
    if ((size_t)k < room)
        *n += (size_t)k;
    else
        *n = cap;
}

static void panel_row(char *out, size_t *n, size_t cap, unsigned y, int pw,
                      const char *label, const char *val, const char *style) {
    char text[80];
    int len;
    if (val) {
        int lw = pw - 13;
        if (lw < 4)
            lw = 4;
        if (lw > 16)
            lw = 16;
        len = snprintf(text, sizeof text, "  %-*s %-10s", lw, label, val);
    } else {
        len = snprintf(text, sizeof text, "  %s", label);
    }
    if (len < 0)
        return;
    if (len > (int)sizeof text - 1)
        len = (int)sizeof text - 1;

    int emit = 0, vis = 0;
    const unsigned char *p = (const unsigned char *)text;
    const unsigned char *end = p + len;
    while (p < end && vis < pw) {
        unsigned char c = *p;
        int seq = (c < 0x80) ? 1 : ((c & 0xE0) == 0xC0 ? 2 : (c & 0xF0) == 0xE0 ? 3 : 4);
        if (vis + 1 > pw)
            break;
        vis++;
        emit += seq;
        p += seq;
    }

    write_esc(out, n, cap, "\x1b[0m\x1b[%u;1H%s%.*s", y, style ? style : "",
              emit, text);
    for (int i = vis; i < pw; i++)
        write_esc(out, n, cap, " ");
    write_esc(out, n, cap, "\x1b[0m");
}

void settings_draw(settings_ui *s, const srk_config *cfg, char *out,
                   size_t *out_len, size_t cap, unsigned rows, int panel_width) {
    (void)rows;
    if (s->confirm_reset && now_ms() > s->confirm_deadline_ms)
        s->confirm_reset = false;
    size_t n = *out_len;
    panel_row(out, &n, cap, 1, panel_width, "sharkvis settings", NULL, NULL);
    panel_row(out, &n, cap, 2, panel_width, "←, ↑, ↓, → = adjust", NULL, NULL);
    panel_row(out, &n, cap, 3, panel_width, "g = close, q = quit", NULL, NULL);
    unsigned y = 6;
    for (int id = 0; id < S_COUNT; id++) {
        char val[32];
        format_value(cfg, id, val, sizeof val);
        panel_row(out, &n, cap, y++, panel_width, LABELS[id], val,
                  id == s->sel ? "\x1b[7m" : NULL);
    }
    if (s->confirm_reset)
        panel_row(out, &n, cap, y, panel_width, "Are you sure?", "press → again",
                  "\x1b[41m\x1b[97m");
    else if (s->sel == S_CHARSET)
        panel_row(out, &n, cap, y, panel_width, "edit bar symbols", "enter = nano",
                  NULL);
    else
        panel_row(out, &n, cap, y, panel_width, "reset to defaults", "press →",
                  s->sel == S_RESET ? "\x1b[7m" : NULL);
    *out_len = n;
}
