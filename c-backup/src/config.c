#include "config.h"

#include <ctype.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/stat.h>
#include <unistd.h>

static char *trim(char *s) {
    while (isspace((unsigned char)*s))
        s++;
    char *e = s + strlen(s);
    while (e > s && isspace((unsigned char)e[-1]))
        *--e = '\0';
    return s;
}

static long geti(const char *v, long def) {
    errno = 0;
    char *end;
    long r = strtol(v, &end, 10);
    if (errno != 0 || end == v)
        return def;
    return r;
}

static double getf(const char *v, double def) {
    errno = 0;
    char *end;
    double r = strtod(v, &end);
    if (errno != 0 || end == v)
        return def;
    return r;
}

const color_entry g_palette[] = {
    { "white",   "ffffff" },
    { "red",     "ff0000" },
    { "green",   "00ff00" },
    { "blue",    "0000ff" },
    { "yellow",  "ffff00" },
    { "magenta", "ff00ff" },
    { "cyan",    "00ffff" },
    { "orange",  "ff8800" },
    { "purple",  "8800ff" },
    { "lime",    "88ff00" },
    { "teal",    "00ff88" },
    { "pink",    "ff0088" },
    { "gray",    "888888" },
    { "black",   "000000" },
};
const int g_palette_n = (int)(sizeof g_palette / sizeof g_palette[0]);

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
    if (!s)
        return -1;
    if (*s == '#')
        s++;
    size_t len = strlen(s);
    if (len != 6)
        return -1;
    int v[6];
    for (int i = 0; i < 6; i++) {
        v[i] = hexval(s[i]);
        if (v[i] < 0)
            return -1;
    }
    *r = (unsigned)((v[0] << 4) | v[1]);
    *g = (unsigned)((v[2] << 4) | v[3]);
    *b = (unsigned)((v[4] << 4) | v[5]);
    return 0;
}

const char *color_name(const char *hex) {
    if (!hex)
        return NULL;
    for (int i = 0; i < g_palette_n; i++) {
        if (strcasecmp(hex, g_palette[i].hex) == 0)
            return g_palette[i].name;
    }
    return NULL;
}

int color_index(const char *hex) {
    if (!hex)
        return -1;
    for (int i = 0; i < g_palette_n; i++) {
        if (strcasecmp(hex, g_palette[i].hex) == 0)
            return i;
    }
    return -1;
}

int color_to_rgb(const char *hex, unsigned *r, unsigned *g, unsigned *b) {
    return parse_hex_rgb(hex, r, g, b);
}

void config_default(srk_config *c) {
    memset(c, 0, sizeof *c);
    c->bars = 0;
    c->bar_width = 2;
    c->bar_spacing = 1;
    c->framerate = 60;
    c->sensitivity = 100.0;
    c->autosens = true;
    c->lower_cutoff = 50;
    c->higher_cutoff = 8000;
    c->noise_reduction = 0.2;
    free(c->source);
    c->source = strdup("auto");
    c->sample_rate = 48000;
    c->channels = 2;
    c->color_256 = false;
    free(c->gradient_low);
    c->gradient_low = strdup("ffffff");
    free(c->gradient_high);
    c->gradient_high = strdup("ffffff");
    free(c->mode);
    c->mode = strdup("bars");
    free(c->glyphs);
    c->glyphs = strdup("\xe2\x96\x81\xe2\x96\x82\xe2\x96\x83\xe2\x96\x84\xe2\x96\x85\xe2\x96\x86\xe2\x96\x87\xe2\x96\x88");
}

char *config_default_path(void) {
    const char *env = getenv("SHARKVIS_CONFIG");
    if (env && env[0])
        return strdup(env);
    const char *home = getenv("HOME");
    if (home) {
        size_t n = strlen(home);
        char *p = malloc(n + 25);
        snprintf(p, n + 25, "%s/.config/sharkvis/config", home);
        if (access(p, F_OK) == 0)
            return p;
        free(p);
    }
    if (access("config", F_OK) == 0)
        return strdup("config");
    if (home) {
        size_t n = strlen(home);
        char *p = malloc(n + 25);
        snprintf(p, n + 25, "%s/.config/sharkvis/config", home);
        return p;
    }
    return strdup("config");
}

void config_free(srk_config *c) {
    free(c->source);
    free(c->gradient_low);
    free(c->gradient_high);
    free(c->mode);
    free(c->glyphs);
}

static void mkdir_p(const char *path) {
    char tmp[1024];
    snprintf(tmp, sizeof tmp, "%s", path);
    for (char *p = tmp + 1; *p; p++) {
        if (*p == '/') {
            *p = '\0';
            mkdir(tmp, 0755);
            *p = '/';
        }
    }
    mkdir(tmp, 0755);
}

bool config_save(const srk_config *c, const char *path) {
    char dir[1024];
    snprintf(dir, sizeof dir, "%s", path);
    char *slash = strrchr(dir, '/');
    if (slash) {
        *slash = '\0';
        if (dir[0])
            mkdir_p(dir);
    }

    FILE *f = fopen(path, "w");
    if (!f)
        return false;

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
    fprintf(f, "source = %s\n", c->source ? c->source : "auto");
    fprintf(f, "sample_rate = %u\n", c->sample_rate);
    fprintf(f, "channels = %u\n", c->channels);
    fprintf(f, "\n[color]\n");
    fprintf(f, "color_mode = %s\n", c->color_256 ? "256" : "24bit");
    fprintf(f, "gradient_low = %s\n", c->gradient_low ? c->gradient_low : "ffffff");
    fprintf(f, "gradient_high = %s\n", c->gradient_high ? c->gradient_high : "ffffff");
    fprintf(f, "\n[visualizer]\n");
    fprintf(f, "mode = %s\n", c->mode ? c->mode : "bars");
    fprintf(f, "glyphs = %s\n", c->glyphs ? c->glyphs : "\xe2\x96\x81\xe2\x96\x82\xe2\x96\x83\xe2\x96\x84\xe2\x96\x85\xe2\x96\x86\xe2\x96\x87\xe2\x96\x88");

    return fclose(f) == 0;
}

bool config_load(srk_config *c, const char *path) {
    FILE *f = fopen(path, "r");
    if (!f)
        return false;

    char line[512];
    char section[64] = "general";
    while (fgets(line, sizeof line, f)) {
        char *s = trim(line);
        if (*s == '\0' || *s == ';' || *s == '#')
            continue;
        if (*s == '[') {
            char *end = strchr(s, ']');
            if (end)
                *end = '\0';
            snprintf(section, sizeof section, "%s", trim(s + 1));
            for (char *p = section; *p; p++)
                *p = (char)tolower((unsigned char)*p);
            continue;
        }
        char *eq = strchr(s, '=');
        if (!eq)
            continue;
        *eq = '\0';
        char *key = trim(s);
        char *val = trim(eq + 1);
        if (strcmp(key, "glyphs") != 0) {
            char *semi = strchr(val, ';');
            if (semi)
                *semi = '\0';
        }
        val = trim(val);
        for (char *p = key; *p; p++)
            *p = (char)tolower((unsigned char)*p);

        if (strcmp(section, "general") == 0) {
            if (strcmp(key, "bars") == 0)
                c->bars = (size_t)geti(val, (long)c->bars);
            else if (strcmp(key, "bar_width") == 0)
                c->bar_width = (size_t)geti(val, (long)c->bar_width);
            else if (strcmp(key, "bar_spacing") == 0)
                c->bar_spacing = (size_t)geti(val, (long)c->bar_spacing);
            else if (strcmp(key, "framerate") == 0)
                c->framerate = (unsigned)geti(val, (long)c->framerate);
            else if (strcmp(key, "sensitivity") == 0)
                c->sensitivity = getf(val, c->sensitivity);
            else if (strcmp(key, "autosens") == 0)
                c->autosens = geti(val, 1) != 0;
            else if (strcmp(key, "lower_cutoff_freq") == 0)
                c->lower_cutoff = (unsigned)geti(val, (long)c->lower_cutoff);
            else if (strcmp(key, "higher_cutoff_freq") == 0)
                c->higher_cutoff = (unsigned)geti(val, (long)c->higher_cutoff);
        } else if (strcmp(section, "smoothing") == 0) {
            if (strcmp(key, "noise_reduction") == 0)
                c->noise_reduction = getf(val, c->noise_reduction);
        } else if (strcmp(section, "input") == 0) {
            if (strcmp(key, "method") == 0) {
                if (*val && strcmp(val, "pulse") != 0 && strcmp(val, "pipewire") != 0 &&
                    strcmp(val, "auto") != 0)
                    fprintf(stderr, "sharkvis: input method '%s' not supported, using pulse\n",
                            val);
            } else if (strcmp(key, "source") == 0) {
                free(c->source);
                c->source = strdup(val);
            } else if (strcmp(key, "sample_rate") == 0) {
                c->sample_rate = (unsigned)geti(val, (long)c->sample_rate);
            } else if (strcmp(key, "channels") == 0) {
                c->channels = (unsigned)geti(val, (long)c->channels);
            }
        } else if (strcmp(section, "color") == 0) {
            if (strcmp(key, "color_mode") == 0) {
                if (strcmp(val, "256") == 0 || strcmp(val, "indexed") == 0)
                    c->color_256 = true;
                else if (strcmp(val, "24bit") == 0 || strcmp(val, "truecolor") == 0)
                    c->color_256 = false;
                else
                    c->color_256 = geti(val, 0) != 0;
            } else if (strcmp(key, "gradient_low") == 0) {
                char tmp[8];
                snprintf(tmp, sizeof tmp, "%s", val);
                unsigned rr, gg, bb;
                if (parse_hex_rgb(tmp, &rr, &gg, &bb) == 0) {
                    free(c->gradient_low);
                    c->gradient_low = strdup(tmp);
                }
            } else if (strcmp(key, "gradient_high") == 0) {
                char tmp[8];
                snprintf(tmp, sizeof tmp, "%s", val);
                unsigned rr, gg, bb;
                if (parse_hex_rgb(tmp, &rr, &gg, &bb) == 0) {
                    free(c->gradient_high);
                    c->gradient_high = strdup(tmp);
                }
            }
        } else if (strcmp(section, "visualizer") == 0) {
            if (strcmp(key, "mode") == 0 &&
                (strcmp(val, "bars") == 0 || strcmp(val, "wave") == 0 ||
                 strcmp(val, "lissajous") == 0)) {
                free(c->mode);
                c->mode = strdup(val);
            } else if (strcmp(key, "glyphs") == 0) {
                free(c->glyphs);
                c->glyphs = strdup(val);
            }
        }
    }

    fclose(f);
    return true;
}
