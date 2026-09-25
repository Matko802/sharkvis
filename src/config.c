#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>

#include "config.h"

const char *SV_PALETTE_NAMES[] = {
    "white", "red", "green", "blue", "yellow", "magenta", "cyan",
    "orange", "purple", "lime", "teal", "pink", "gray", "black"
};

const char *SV_PALETTE_HEX[] = {
    "ffffff", "ff0000", "00ff00", "0000ff", "ffff00", "ff00ff", "00ffff",
    "ff8800", "8800ff", "88ff00", "00ff88", "ff0088", "888888", "000000"
};

int SV_PALETTE_COUNT = 14;

const char SV_DEFAULT_CHARS[] = "\xe2\x96\x81\xe2\x96\x82\xe2\x96\x83\xe2\x96\x84"
                                "\xe2\x96\x85\xe2\x96\x86\xe2\x96\x87\xe2\x96\x88";
unsigned SV_DEFAULT_CHARS_LEN = 24;

void config_default(SvConfig *c) {
    memset(c, 0, sizeof *c);
    c->bars = 0;
    c->bar_width = 2;
    c->bar_spacing = 1;
    c->framerate = 60;
    c->sensitivity = 100.0;
    c->autosens = 1;
    c->lower_cutoff = 50;
    c->higher_cutoff = 8000;
    c->noise_reduction = 0.2;
    strcpy(c->source, "auto");
    c->sample_rate = 48000;
    c->channels = 2;
    c->color_256 = 0;
    strcpy(c->gradient_low, "white");
    strcpy(c->gradient_high, "white");
    strcpy(c->mode, "bars");
    strcpy(c->text_align, "center");
    c->text_size = 1;
    strcpy(c->text_style, "big ahh");
    strcpy(c->provider, "auto");
    c->lyric_offset_ms = 0;
    c->lyrics_folder[0] = 0;
    c->mpris_players[0] = 0;
    memcpy(c->chars, SV_DEFAULT_CHARS, SV_DEFAULT_CHARS_LEN);
    c->chars_len = SV_DEFAULT_CHARS_LEN;
}

static int ci_eq(const char *a, const char *b) {
    while (*a && *b) {
        if (tolower((unsigned char)*a) != tolower((unsigned char)*b))
            return 0;
        a++;
        b++;
    }
    return *a == *b;
}

static void ci_lower(const char *s, char *out, size_t n) {
    size_t i = 0;
    while (s[i] && i + 1 < n) {
        out[i] = (char)tolower((unsigned char)s[i]);
        i++;
    }
    out[i] = 0;
}

int palette_ansi(const char *name) {
    char l[64];
    ci_lower(name, l, sizeof l);
    if (!strcmp(l, "black"))
        return 30;
    if (!strcmp(l, "red"))
        return 31;
    if (!strcmp(l, "green"))
        return 32;
    if (!strcmp(l, "yellow"))
        return 33;
    if (!strcmp(l, "blue"))
        return 34;
    if (!strcmp(l, "magenta") || !strcmp(l, "purple"))
        return 35;
    if (!strcmp(l, "cyan"))
        return 36;
    if (!strcmp(l, "white"))
        return 37;
    if (!strcmp(l, "gray") || !strcmp(l, "grey") || !strcmp(l, "bright_black"))
        return 90;
    if (!strcmp(l, "orange") || !strcmp(l, "bright_red"))
        return 91;
    if (!strcmp(l, "lime") || !strcmp(l, "bright_green"))
        return 92;
    if (!strcmp(l, "bright_yellow"))
        return 93;
    if (!strcmp(l, "bright_blue"))
        return 94;
    if (!strcmp(l, "pink") || !strcmp(l, "bright_magenta"))
        return 95;
    if (!strcmp(l, "teal") || !strcmp(l, "bright_cyan"))
        return 96;
    if (!strcmp(l, "bright_white"))
        return 97;
    return -1;
}

int color_to_ansi(const char *s) {
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')
        s++;
    return palette_ansi(s);
}

static int hexval(char c) {
    if (c >= '0' && c <= '9')
        return c - '0';
    if (c >= 'a' && c <= 'f')
        return c - 'a' + 10;
    if (c >= 'A' && c <= 'F')
        return c - 'A' + 10;
    return -1;
}

static int parse_hex_rgb(const char *s, unsigned *r, unsigned *g, unsigned *b) {
    if (*s == '#')
        s++;
    if (strlen(s) != 6)
        return 0;
    int v[6];
    for (int i = 0; i < 6; i++) {
        v[i] = hexval(s[i]);
        if (v[i] < 0)
            return 0;
    }
    *r = (unsigned)((v[0] << 4) | v[1]);
    *g = (unsigned)((v[2] << 4) | v[3]);
    *b = (unsigned)((v[4] << 4) | v[5]);
    return 1;
}

int color_to_rgb(const char *s, unsigned *r, unsigned *g, unsigned *b) {
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')
        s++;
    char t[64];
    size_t n = 0;
    while (s[n] && s[n] != ' ' && s[n] != '\t' && s[n] != '\n' && s[n] != '\r' && n + 1 < sizeof t) {
        t[n] = s[n];
        n++;
    }
    t[n] = 0;
    for (int i = 0; i < SV_PALETTE_COUNT; i++) {
        if (ci_eq(t, SV_PALETTE_NAMES[i]))
            return parse_hex_rgb(SV_PALETTE_HEX[i], r, g, b);
    }
    char l[64];
    ci_lower(t, l, sizeof l);
    const char *approx = NULL;
    if (!strcmp(l, "grey"))
        approx = "888888";
    else if (!strcmp(l, "bright_black"))
        approx = "808080";
    else if (!strcmp(l, "bright_red"))
        approx = "ff5555";
    else if (!strcmp(l, "bright_green"))
        approx = "55ff55";
    else if (!strcmp(l, "bright_yellow"))
        approx = "ffff55";
    else if (!strcmp(l, "bright_blue"))
        approx = "5555ff";
    else if (!strcmp(l, "bright_magenta"))
        approx = "ff55ff";
    else if (!strcmp(l, "bright_cyan"))
        approx = "55ffff";
    else if (!strcmp(l, "bright_white"))
        approx = "ffffff";
    if (approx)
        return parse_hex_rgb(approx, r, g, b);
    return parse_hex_rgb(t, r, g, b);
}

const char *color_name(const char *s) {
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')
        s++;
    char t[64];
    size_t n = 0;
    while (s[n] && s[n] != ' ' && s[n] != '\t' && s[n] != '\n' && s[n] != '\r' && n + 1 < sizeof t) {
        t[n] = s[n];
        n++;
    }
    t[n] = 0;
    for (int i = 0; i < SV_PALETTE_COUNT; i++) {
        if (ci_eq(t, SV_PALETTE_NAMES[i]))
            return SV_PALETTE_NAMES[i];
    }
    if (ci_eq(t, "grey"))
        return "gray";
    static const char *aliases[] = {
        "bright_black", "bright_red", "bright_green", "bright_yellow",
        "bright_blue", "bright_magenta", "bright_cyan", "bright_white"
    };
    static const char *targets[] = {
        "gray", "orange", "lime", "yellow", "blue", "pink", "teal", "white"
    };
    for (int i = 0; i < 8; i++) {
        if (ci_eq(t, aliases[i]))
            return targets[i];
    }
    for (int i = 0; i < SV_PALETTE_COUNT; i++) {
        if (ci_eq(t, SV_PALETTE_HEX[i]))
            return SV_PALETTE_NAMES[i];
    }
    return NULL;
}

int color_index(const char *s) {
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')
        s++;
    char t[64];
    size_t n = 0;
    while (s[n] && s[n] != ' ' && s[n] != '\t' && s[n] != '\n' && s[n] != '\r' && n + 1 < sizeof t) {
        t[n] = s[n];
        n++;
    }
    t[n] = 0;
    for (int i = 0; i < SV_PALETTE_COUNT; i++) {
        if (ci_eq(t, SV_PALETTE_NAMES[i]))
            return i;
    }
    for (int i = 0; i < SV_PALETTE_COUNT; i++) {
        if (ci_eq(t, SV_PALETTE_HEX[i]))
            return i;
    }
    return -1;
}

void config_default_path(char *buf, size_t n) {
    const char *env = getenv("SHARKVIS_CONFIG");
    if (env && *env) {
        snprintf(buf, n, "%s", env);
        return;
    }
    const char *home = getenv("HOME");
    if (home && *home) {
        char p[1024];
        snprintf(p, sizeof p, "%s/.config/sharkvis/config.toml", home);
        struct stat st;
        if (stat(p, &st) == 0) {
            snprintf(buf, n, "%s", p);
            return;
        }
        if (stat("config.toml", &st) == 0) {
            snprintf(buf, n, "config.toml");
            return;
        }
        snprintf(buf, n, "%s", p);
        return;
    }
    snprintf(buf, n, "config.toml");
}

static long geti(const char *v, long def) {
    while (*v == ' ' || *v == '\t')
        v++;
    long sign = 1;
    if (*v == '-' || *v == '+') {
        if (*v == '-')
            sign = -1;
        v++;
    }
    if (*v < '0' || *v > '9')
        return def;
    long val = 0;
    while (*v >= '0' && *v <= '9') {
        val = val * 10 + (*v - '0');
        v++;
    }
    return sign * val;
}

static double getf(const char *v, double def) {
    char *end;
    double d = strtod(v, &end);
    if (end == v)
        return def;
    return d;
}

static void copy_str(char *dst, size_t n, const char *src) {
    if (n == 0)
        return;
    strncpy(dst, src, n - 1);
    dst[n - 1] = 0;
}

int config_load(SvConfig *c, const char *path) {
    FILE *f = fopen(path, "r");
    if (!f)
        return 0;
    char line[1024];
    char section[64] = "general";
    while (fgets(line, sizeof line, f)) {
        size_t len = strlen(line);
        while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r'))
            line[--len] = 0;
        char *s = line;
        while (*s == ' ' || *s == '\t')
            s++;
        char *e = s + strlen(s);
        while (e > s && (e[-1] == ' ' || e[-1] == '\t'))
            *--e = 0;
        if (*s == 0 || *s == ';' || *s == '#')
            continue;
        if (*s == '[') {
            char *end = strchr(s, ']');
            if (end)
                *end = 0;
            char *name = s + 1;
            while (*name == ' ' || *name == '\t')
                name++;
            char *ne = name + strlen(name);
            while (ne > name && (ne[-1] == ' ' || ne[-1] == '\t'))
                *--ne = 0;
            size_t i = 0;
            while (name[i] && i + 1 < sizeof section) {
                section[i] = (char)tolower((unsigned char)name[i]);
                i++;
            }
            section[i] = 0;
            continue;
        }
        char *eq = strchr(s, '=');
        if (!eq)
            continue;
        *eq = 0;
        char *key = s;
        char *ke = key + strlen(key);
        while (ke > key && (ke[-1] == ' ' || ke[-1] == '\t'))
            *--ke = 0;
        char keyl[64];
        size_t ki = 0;
        while (key[ki] && ki + 1 < sizeof keyl) {
            keyl[ki] = (char)tolower((unsigned char)key[ki]);
            ki++;
        }
        keyl[ki] = 0;
        char *val = eq + 1;
        while (*val == ' ' || *val == '\t')
            val++;
        char *ve = val + strlen(val);
        while (ve > val && (ve[-1] == ' ' || ve[-1] == '\t'))
            *--ve = 0;
        if (strcmp(keyl, "chars") != 0) {
            char *semi = strchr(val, ';');
            if (semi) {
                *semi = 0;
                ve = semi;
                while (ve > val && (ve[-1] == ' ' || ve[-1] == '\t'))
                    *--ve = 0;
            }
        }
        if (!strcmp(section, "general")) {
            if (!strcmp(keyl, "bars"))
                c->bars = (size_t)geti(val, (long)c->bars);
            else if (!strcmp(keyl, "bar_width"))
                c->bar_width = (size_t)geti(val, (long)c->bar_width);
            else if (!strcmp(keyl, "bar_spacing"))
                c->bar_spacing = (size_t)geti(val, (long)c->bar_spacing);
            else if (!strcmp(keyl, "framerate"))
                c->framerate = (unsigned)geti(val, (long)c->framerate);
            else if (!strcmp(keyl, "sensitivity"))
                c->sensitivity = getf(val, c->sensitivity);
            else if (!strcmp(keyl, "autosens"))
                c->autosens = geti(val, 1) != 0;
            else if (!strcmp(keyl, "lower_cutoff_freq"))
                c->lower_cutoff = (unsigned)geti(val, (long)c->lower_cutoff);
            else if (!strcmp(keyl, "higher_cutoff_freq"))
                c->higher_cutoff = (unsigned)geti(val, (long)c->higher_cutoff);
        } else if (!strcmp(section, "smoothing")) {
            if (!strcmp(keyl, "noise_reduction"))
                c->noise_reduction = getf(val, c->noise_reduction);
        } else if (!strcmp(section, "input")) {
            if (!strcmp(keyl, "method")) {
                if (*val && strcmp(val, "pulse") && strcmp(val, "pipewire") && strcmp(val, "auto"))
                    fprintf(stderr, "sharkvis: input method '%s' not supported, using pulse\n", val);
            } else if (!strcmp(keyl, "source")) {
                copy_str(c->source, sizeof c->source, val);
            } else if (!strcmp(keyl, "sample_rate")) {
                c->sample_rate = (unsigned)geti(val, (long)c->sample_rate);
            } else if (!strcmp(keyl, "channels")) {
                c->channels = (unsigned)geti(val, (long)c->channels);
            }
        } else if (!strcmp(section, "lyrics")) {
            if (!strcmp(keyl, "folder"))
                copy_str(c->lyrics_folder, sizeof c->lyrics_folder, val);
        } else if (!strcmp(section, "mpris")) {
            if (!strcmp(keyl, "players"))
                copy_str(c->mpris_players, sizeof c->mpris_players, val);
        } else if (!strcmp(section, "color")) {
            if (!strcmp(keyl, "color_mode")) {
                if (!strcmp(val, "256") || !strcmp(val, "indexed"))
                    c->color_256 = 1;
                else if (!strcmp(val, "24bit") || !strcmp(val, "truecolor"))
                    c->color_256 = 0;
                else
                    c->color_256 = geti(val, 0) != 0;
            } else if (!strcmp(keyl, "gradient_low")) {
                unsigned r, g, b;
                if (color_to_rgb(val, &r, &g, &b))
                    copy_str(c->gradient_low, sizeof c->gradient_low, val);
            } else if (!strcmp(keyl, "gradient_high")) {
                unsigned r, g, b;
                if (color_to_rgb(val, &r, &g, &b))
                    copy_str(c->gradient_high, sizeof c->gradient_high, val);
            }
        } else if (!strcmp(section, "visualizer")) {
            if (!strcmp(keyl, "mode")) {
                if (!strcmp(val, "bars") || !strcmp(val, "wave") ||
                    !strcmp(val, "oscilloscope") || !strcmp(val, "lissajous")) {
                    copy_str(c->mode, sizeof c->mode, val);
                } else if (!strcmp(val, "lyrics")) {
                    strcpy(c->mode, "lyrics");
                } else if (!strcmp(val, "text")) {
                    strcpy(c->mode, "lyrics");
                }
            } else if (!strcmp(keyl, "text") || !strcmp(keyl, "text_source")) {
            } else if (!strcmp(keyl, "text_align")) {
                if (!strcmp(val, "left") || !strcmp(val, "center"))
                    copy_str(c->text_align, sizeof c->text_align, val);
            } else if (!strcmp(keyl, "text_size")) {
                char *end;
                long n = strtol(val, &end, 10);
                if (end != val) {
                    if (n > 5)
                        n = 5;
                    c->text_size = (unsigned)n;
                }
            } else if (!strcmp(keyl, "text_style")) {
                if (!strcmp(val, "big ahh") || !strcmp(val, "normal"))
                    copy_str(c->text_style, sizeof c->text_style, val);
                else if (!strcmp(val, "big"))
                    strcpy(c->text_style, "big ahh");
                else if (!strcmp(val, "small"))
                    strcpy(c->text_style, "normal");
            } else if (!strcmp(keyl, "provider")) {
                if (!strcmp(val, "auto") || !strcmp(val, "musixmatch") || !strcmp(val, "lrclib"))
                    copy_str(c->provider, sizeof c->provider, val);
            } else if (!strcmp(keyl, "offset_ms")) {
                char *end;
                long n = strtol(val, &end, 10);
                if (end != val) {
                    if (n < -10000)
                        n = -10000;
                    if (n > 10000)
                        n = 10000;
                    c->lyric_offset_ms = n;
                }
            } else if (!strcmp(keyl, "chars")) {
                size_t n = strlen(val);
                if (n > sizeof c->chars)
                    n = sizeof c->chars;
                memcpy(c->chars, val, n);
                c->chars_len = n;
            }
        }
    }
    fclose(f);
    return 1;
}

static void mkdir_p(const char *path) {
    char tmp[1024];
    snprintf(tmp, sizeof tmp, "%s", path);
    char *slash = strrchr(tmp, '/');
    if (!slash)
        return;
    *slash = 0;
    char cur[1024];
    size_t n = 0;
    if (tmp[0] == '/')
        cur[n++] = '/';
    char *p = tmp + (tmp[0] == '/' ? 1 : 0);
    char *tok = strtok(p, "/");
    while (tok) {
        size_t l = strlen(tok);
        if (n + l + 1 < sizeof cur) {
            memcpy(cur + n, tok, l);
            n += l;
            cur[n] = 0;
            mkdir(cur, 0755);
            cur[n++] = '/';
            cur[n] = 0;
        }
        tok = strtok(NULL, "/");
    }
}

int config_save(const SvConfig *c, const char *path) {
    mkdir_p(path);
    FILE *f = fopen(path, "w");
    if (!f)
        return 0;
    fprintf(f, "[general]\n");
    fprintf(f, "bars = %zu\n", c->bars);
    fprintf(f, "bar_width = %zu\n", c->bar_width);
    fprintf(f, "bar_spacing = %zu\n", c->bar_spacing);
    fprintf(f, "framerate = %u\n", c->framerate);
    fprintf(f, "sensitivity = %.0f\n", c->sensitivity);
    fprintf(f, "autosens = %d\n", c->autosens ? 1 : 0);
    fprintf(f, "lower_cutoff_freq = %u\n", c->lower_cutoff);
    fprintf(f, "higher_cutoff_freq = %u\n", c->higher_cutoff);
    fprintf(f, "\n[smoothing]\n");
    fprintf(f, "noise_reduction = %.2f\n", c->noise_reduction);
    fprintf(f, "\n[input]\n");
    fprintf(f, "method = pulse\n");
    fprintf(f, "source = %s\n", c->source);
    fprintf(f, "sample_rate = %u\n", c->sample_rate);
    fprintf(f, "channels = %u\n", c->channels);
    fprintf(f, "\n[color]\n");
    fprintf(f, "color_mode = %s\n", c->color_256 ? "256" : "24bit");
    fprintf(f, "gradient_low = %s\n", c->gradient_low);
    fprintf(f, "gradient_high = %s\n", c->gradient_high);
    fprintf(f, "\n[visualizer]\n");
    fprintf(f, "mode = %s\n", c->mode);
    fprintf(f, "text_align = %s\n", c->text_align);
    fprintf(f, "text_size = %u\n", c->text_size);
    fprintf(f, "text_style = %s\n", c->text_style);
    fprintf(f, "provider = %s\n", c->provider);
    fprintf(f, "offset_ms = %ld\n", c->lyric_offset_ms);
    fprintf(f, "chars = ");
    fwrite(c->chars, 1, c->chars_len, f);
    fprintf(f, "\n");
    fprintf(f, "\n[lyrics]\n");
    fprintf(f, "folder = %s\n", c->lyrics_folder);
    fprintf(f, "\n[mpris]\n");
    fprintf(f, "players = %s\n", c->mpris_players);
    fclose(f);
    return 1;
}
