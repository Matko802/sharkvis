#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "common.h"
#include "config.h"
#include "settings.h"
#include "term.h"

#define S_MODE 0
#define S_BARS 1
#define S_BARW 2
#define S_SPACING 3
#define S_FPS 4
#define S_SENS 5
#define S_AUTO 6
#define S_NOISE 7
#define S_LOW 8
#define S_HIGH 9
#define S_CMODE 10
#define S_GHI 11
#define S_GLO 12
#define S_RATE 13
#define S_CH 14
#define S_CHARSET 15
#define S_TEXTSIZE 16
#define S_STYLE 17
#define S_PROVIDER 18
#define S_OFFSET 19
#define S_COUNT 20
#define S_RESET 20
#define CONFIRM_TIMEOUT_MS 5000

static const char *LABELS[S_COUNT] = {
    "type", "bars", "bar width", "bar spacing", "framerate", "sensitivity",
    "autosens", "smoothing", "lower cutoff", "upper cutoff", "color mode",
    "color high", "color low", "sample rate", "channels", "charset",
    "lyrics size", "lyrics style", "provider", "offset ms"
};

static const unsigned RATES[] = {8000, 11025, 16000, 22050, 32000, 44100, 48000, 96000, 192000};
static const char *MODES[] = {"bars", "wave", "oscilloscope", "lyrics"};

struct SettingsUi {
    size_t sel;
    int confirm_reset;
    uint64_t confirm_deadline_ms;
};

SettingsUi *settings_new(void) {
    SettingsUi *s = calloc(1, sizeof *s);
    return s;
}

void settings_free(SettingsUi *s) {
    free(s);
}

static long clamp_l(long v, long lo, long hi) {
    if (v < lo)
        return lo;
    if (v > hi)
        return hi;
    return v;
}

static double clamp_d(double v, double lo, double hi) {
    if (v < lo)
        return lo;
    if (v > hi)
        return hi;
    return v;
}

static void adjust(SvConfig *cfg, size_t id, long dir, unsigned *changed) {
    switch (id) {
    case S_BARS: {
        long v = clamp_l((long)cfg->bars + dir, 0, 256);
        if ((size_t)v != cfg->bars) {
            cfg->bars = (size_t)v;
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_BARW: {
        long v = clamp_l((long)cfg->bar_width + dir, 1, 8);
        if ((size_t)v != cfg->bar_width) {
            cfg->bar_width = (size_t)v;
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_SPACING: {
        long v = clamp_l((long)cfg->bar_spacing + dir, 0, 4);
        if ((size_t)v != cfg->bar_spacing) {
            cfg->bar_spacing = (size_t)v;
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_FPS: {
        long v = clamp_l((long)cfg->framerate + dir * 5, 5, 240);
        if ((unsigned)v != cfg->framerate)
            cfg->framerate = (unsigned)v;
        break;
    }
    case S_SENS: {
        double v = clamp_d(cfg->sensitivity + (double)dir * 5.0, 5.0, 200.0);
        if (v != cfg->sensitivity)
            cfg->sensitivity = v;
        break;
    }
    case S_AUTO: {
        int v = !cfg->autosens;
        if (v != cfg->autosens) {
            cfg->autosens = v;
            *changed |= CH_DSP;
        }
        break;
    }
    case S_NOISE: {
        double v = clamp_d(cfg->noise_reduction + (double)dir * 0.05, 0.0, 1.0);
        if (v != cfg->noise_reduction) {
            cfg->noise_reduction = v;
            *changed |= CH_DSP;
        }
        break;
    }
    case S_LOW: {
        long v = clamp_l((long)cfg->lower_cutoff + dir * 25, 25, 20000);
        if (v >= (long)cfg->higher_cutoff)
            v = ((long)cfg->higher_cutoff - 1) / 25 * 25;
        if ((unsigned)v != cfg->lower_cutoff) {
            cfg->lower_cutoff = (unsigned)v;
            *changed |= CH_DSP;
        }
        break;
    }
    case S_HIGH: {
        long v = clamp_l((long)cfg->higher_cutoff + dir * 500, 500, 24000);
        if (v <= (long)cfg->lower_cutoff)
            v = ((long)cfg->lower_cutoff / 500 + 1) * 500;
        if ((unsigned)v != cfg->higher_cutoff) {
            cfg->higher_cutoff = (unsigned)v;
            *changed |= CH_DSP;
        }
        break;
    }
    case S_CMODE: {
        int v = !cfg->color_256;
        if (v != cfg->color_256) {
            cfg->color_256 = v;
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_GLO:
    case S_GHI: {
        const char *cur = id == S_GLO ? cfg->gradient_low : cfg->gradient_high;
        int idx = color_index(cur);
        if (idx < 0)
            idx = 0;
        idx = (idx + (int)dir + SV_PALETTE_COUNT) % SV_PALETTE_COUNT;
        const char *nw = SV_PALETTE_NAMES[idx];
        if (id == S_GLO) {
            snprintf(cfg->gradient_low, sizeof cfg->gradient_low, "%s", nw);
        } else {
            snprintf(cfg->gradient_high, sizeof cfg->gradient_high, "%s", nw);
        }
        *changed |= CH_LAYOUT;
        break;
    }
    case S_MODE: {
        if (!strcmp(cfg->mode, "text"))
            strcpy(cfg->mode, "lyrics");
        long idx = 0;
        for (long i = 0; i < 4; i++) {
            if (!strcmp(cfg->mode, MODES[i])) {
                idx = i;
                break;
            }
        }
        idx = (idx + dir + 4) % 4;
        if (strcmp(cfg->mode, MODES[idx])) {
            snprintf(cfg->mode, sizeof cfg->mode, "%s", MODES[idx]);
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_RATE: {
        long idx = 0;
        for (long i = 0; i < 9; i++) {
            if (RATES[i] <= cfg->sample_rate)
                idx = i;
        }
        idx = clamp_l(idx + dir, 0, 8);
        if (RATES[idx] != cfg->sample_rate) {
            cfg->sample_rate = RATES[idx];
            *changed |= CH_AUDIO;
        }
        break;
    }
    case S_CH: {
        unsigned v = cfg->channels == 1 ? 2 : 1;
        if (v != cfg->channels) {
            cfg->channels = v;
            *changed |= CH_AUDIO;
        }
        break;
    }
    case S_PROVIDER: {
        static const char *ORDER[] = {"auto", "lrclib", "musixmatch"};
        long idx = 0;
        for (long i = 0; i < 3; i++) {
            if (!strcmp(cfg->provider, ORDER[i])) {
                idx = i;
                break;
            }
        }
        idx = (idx + dir + 3) % 3;
        if (strcmp(cfg->provider, ORDER[idx])) {
            snprintf(cfg->provider, sizeof cfg->provider, "%s", ORDER[idx]);
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_OFFSET: {
        long v = cfg->lyric_offset_ms + dir * 500;
        if (v < -10000)
            v = -10000;
        if (v > 10000)
            v = 10000;
        if (v != cfg->lyric_offset_ms) {
            cfg->lyric_offset_ms = v;
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_TEXTSIZE: {
        long v = (long)cfg->text_size + dir;
        if (v < 0)
            v = 0;
        if (v > 5)
            v = 5;
        if ((unsigned)v != cfg->text_size) {
            cfg->text_size = (unsigned)v;
            *changed |= CH_LAYOUT;
        }
        break;
    }
    case S_STYLE: {
        if (!strcmp(cfg->text_style, "normal"))
            strcpy(cfg->text_style, "big ahh");
        else
            strcpy(cfg->text_style, "normal");
        *changed |= CH_LAYOUT;
        break;
    }
    default:
        break;
    }
}

size_t settings_visible_rows(const char *mode, size_t *out, size_t max) {
    size_t ids[24];
    size_t n = 0;
    ids[n++] = S_MODE;
    ids[n++] = S_CMODE;
    ids[n++] = S_GHI;
    ids[n++] = S_GLO;
    ids[n++] = S_FPS;
    ids[n++] = S_RATE;
    ids[n++] = S_CH;
    if (!strcmp(mode, "bars")) {
        size_t extra[] = {S_BARS, S_BARW, S_SPACING, S_CHARSET, S_SENS,
                          S_AUTO, S_NOISE, S_LOW, S_HIGH};
        for (size_t i = 0; i < 9; i++)
            ids[n++] = extra[i];
    } else if (!strcmp(mode, "lyrics") || !strcmp(mode, "text")) {
        size_t extra[] = {S_TEXTSIZE, S_STYLE, S_PROVIDER, S_OFFSET};
        for (size_t i = 0; i < 4; i++)
            ids[n++] = extra[i];
    }
    for (size_t i = 1; i < n; i++) {
        size_t j = i;
        while (j > 0 && ids[j] < ids[j - 1]) {
            size_t t = ids[j];
            ids[j] = ids[j - 1];
            ids[j - 1] = t;
            j--;
        }
    }
    size_t m = n < max ? n : max;
    for (size_t i = 0; i < m; i++)
        out[i] = ids[i];
    return m;
}

static size_t nav_ids(const SvConfig *cfg, size_t *out) {
    size_t n = settings_visible_rows(cfg->mode, out, 24);
    out[n++] = S_RESET;
    return n;
}

static void clamp_sel(SettingsUi *s, const SvConfig *cfg) {
    size_t ids[24];
    size_t n = nav_ids(cfg, ids);
    int found = 0;
    for (size_t i = 0; i < n; i++) {
        if (ids[i] == s->sel) {
            found = 1;
            break;
        }
    }
    if (!found)
        s->sel = S_MODE;
}

static void handle_reset(SettingsUi *s, SvConfig *cfg, unsigned *changed) {
    if (!s->confirm_reset) {
        s->confirm_reset = 1;
        s->confirm_deadline_ms = sv_now_ms() + CONFIRM_TIMEOUT_MS;
        return;
    }
    s->confirm_reset = 0;
    config_default(cfg);
    *changed |= CH_LAYOUT | CH_DSP | CH_AUDIO;
}

void settings_key(SettingsUi *s, SvConfig *cfg, int key,
                  const uint8_t *cp, size_t cplen, unsigned *changed) {
    size_t ids[24];
    size_t n = nav_ids(cfg, ids);
    if (key == KEY_UP) {
        size_t pos = 0;
        for (size_t i = 0; i < n; i++) {
            if (ids[i] == s->sel) {
                pos = i;
                break;
            }
        }
        s->sel = ids[(pos + n - 1) % n];
        s->confirm_reset = 0;
    } else if (key == KEY_DOWN) {
        size_t pos = 0;
        for (size_t i = 0; i < n; i++) {
            if (ids[i] == s->sel) {
                pos = i;
                break;
            }
        }
        s->sel = ids[(pos + 1) % n];
        s->confirm_reset = 0;
    } else if (key == KEY_LEFT) {
        if (s->sel == S_RESET)
            handle_reset(s, cfg, changed);
        else
            adjust(cfg, s->sel, -1, changed);
        clamp_sel(s, cfg);
    } else if (key == KEY_RIGHT) {
        if (s->sel == S_RESET)
            handle_reset(s, cfg, changed);
        else
            adjust(cfg, s->sel, 1, changed);
        clamp_sel(s, cfg);
    } else if (key == KEY_ENTER) {
        if (s->sel == S_CHARSET)
            *changed |= CH_EDITOR;
    } else if (key == KEY_CHAR && cp && cplen > 0) {
        if (cp[0] == '-') {
            if (s->sel != S_RESET)
                adjust(cfg, s->sel, -1, changed);
        } else if (cp[0] == '+' || cp[0] == '=') {
            if (s->sel != S_RESET)
                adjust(cfg, s->sel, 1, changed);
        }
        clamp_sel(s, cfg);
    }
}

static void format_value(const SvConfig *cfg, size_t id, char *out, size_t n) {
    switch (id) {
    case S_BARS:
        if (cfg->bars == 0)
            snprintf(out, n, "auto");
        else
            snprintf(out, n, "%zu", cfg->bars);
        break;
    case S_AUTO:
        snprintf(out, n, "%s", cfg->autosens ? "on" : "off");
        break;
    case S_CMODE:
        snprintf(out, n, "%s", cfg->color_256 ? "256" : "24bit");
        break;
    case S_GLO:
    case S_GHI: {
        const char *hx = id == S_GLO ? cfg->gradient_low : cfg->gradient_high;
        const char *nm = color_name(hx);
        snprintf(out, n, "%s", nm ? nm : hx);
        break;
    }
    case S_MODE:
        snprintf(out, n, "%s", cfg->mode);
        break;
    case S_NOISE:
        snprintf(out, n, "%.2f", cfg->noise_reduction);
        break;
    case S_SENS:
        snprintf(out, n, "%.0f", cfg->sensitivity);
        break;
    case S_BARW:
        snprintf(out, n, "%zu", cfg->bar_width);
        break;
    case S_SPACING:
        snprintf(out, n, "%zu", cfg->bar_spacing);
        break;
    case S_FPS:
        snprintf(out, n, "%u", cfg->framerate);
        break;
    case S_LOW:
        snprintf(out, n, "%u", cfg->lower_cutoff);
        break;
    case S_HIGH:
        snprintf(out, n, "%u", cfg->higher_cutoff);
        break;
    case S_RATE:
        snprintf(out, n, "%u", cfg->sample_rate);
        break;
    case S_CH:
        snprintf(out, n, "%u", cfg->channels);
        break;
    case S_CHARSET:
        snprintf(out, n, "%.*s", (int)cfg->chars_len, cfg->chars);
        break;
    case S_TEXTSIZE:
        if (cfg->text_size == 0)
            snprintf(out, n, "auto");
        else
            snprintf(out, n, "%u", cfg->text_size);
        break;
    case S_PROVIDER:
        snprintf(out, n, "%s", cfg->provider);
        break;
    case S_OFFSET:
        snprintf(out, n, "%+ldms", cfg->lyric_offset_ms);
        break;
    case S_STYLE:
        snprintf(out, n, "%s", cfg->text_style);
        break;
    default:
        out[0] = 0;
        break;
    }
}

static void append_esc(SvBuf *out, size_t cap, const void *p, size_t n) {
    if (out->len >= cap)
        return;
    size_t room = cap - out->len;
    if (n > room)
        n = room;
    sv_buf_put(out, p, n);
}

static void panel_row(SvBuf *out, size_t cap, unsigned y, size_t pw,
                      const char *label, const char *val, const char *style) {
    char text[280];
    if (val) {
        long lw = (long)pw - 13;
        if (lw < 4)
            lw = 4;
        if (lw > 16)
            lw = 16;
        snprintf(text, sizeof text, "  %-*.*s %-10.10s", (int)lw, (int)lw, label, val);
    } else {
        snprintf(text, sizeof text, "  %s", label);
    }
    size_t len = strlen(text);
    if (len > 256)
        len = 256;
    size_t emit = 0;
    size_t vis = 0;
    size_t p = 0;
    while (p < len && vis < pw) {
        unsigned char c = (unsigned char)text[p];
        size_t seq;
        if (c < 0x80)
            seq = 1;
        else if ((c & 0xE0) == 0xC0)
            seq = 2;
        else if ((c & 0xF0) == 0xE0)
            seq = 3;
        else
            seq = 4;
        if (vis + 1 > pw)
            break;
        vis++;
        emit += seq;
        p += seq;
    }
    char header[64];
    int hn = snprintf(header, sizeof header, "\x1b[0m\x1b[%u;1H%s", y, style ? style : "");
    append_esc(out, cap, header, (size_t)hn);
    append_esc(out, cap, text, emit);
    for (size_t i = vis; i < pw; i++)
        append_esc(out, cap, " ", 1);
    append_esc(out, cap, "\x1b[0m", 4);
}

void settings_draw(SettingsUi *s, const SvConfig *cfg, SvBuf *out,
                   size_t cap, unsigned rows, size_t pw) {
    if (s->confirm_reset && sv_now_ms() > s->confirm_deadline_ms)
        s->confirm_reset = 0;
    clamp_sel(s, cfg);
    panel_row(out, cap, 1, pw, "sharkvis settings", NULL, NULL);
    unsigned y = 6;
    size_t ids[24];
    size_t n = settings_visible_rows(cfg->mode, ids, 24);
    char val[128];
    for (size_t i = 0; i < n; i++) {
        size_t id = ids[i];
        format_value(cfg, id, val, sizeof val);
        panel_row(out, cap, y, pw, LABELS[id], val,
                  id == s->sel ? "\x1b[7m" : NULL);
        y++;
        if (id == S_MODE) {
            char sep[128];
            int sn = snprintf(sep, sizeof sep, "\x1b[0m\x1b[%u;1H", y);
            append_esc(out, cap, sep, (size_t)sn);
            for (size_t k = 0; k < pw; k++)
                append_esc(out, cap, "\xe2\x94\x80", 3);
            y++;
        }
    }
    if (s->confirm_reset) {
        panel_row(out, cap, y, pw, "Are you sure?", "press \xe2\x86\x92 again",
                  "\x1b[41m\x1b[97m");
    } else if (s->sel == S_CHARSET) {
        panel_row(out, cap, y, pw, "edit bar symbols", "enter = nano", NULL);
    } else {
        panel_row(out, cap, y, pw, "reset to defaults", "press \xe2\x86\x92",
                  s->sel == S_RESET ? "\x1b[7m" : NULL);
    }
    for (unsigned yy = 1; yy <= rows; yy++) {
        char div[64];
        int dn = snprintf(div, sizeof div, "\x1b[0m\x1b[%u;%zuH\xe2\x94\x82", yy, pw);
        append_esc(out, cap, div, (size_t)dn);
    }
}
