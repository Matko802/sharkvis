#ifndef SHARKVIS_CONFIG_H
#define SHARKVIS_CONFIG_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

extern const char *SV_PALETTE_NAMES[];
extern const char *SV_PALETTE_HEX[];
extern int SV_PALETTE_COUNT;

extern const char SV_DEFAULT_CHARS[];
extern unsigned SV_DEFAULT_CHARS_LEN;

typedef struct {
    size_t bars;
    size_t bar_width;
    size_t bar_spacing;
    unsigned framerate;
    double sensitivity;
    bool autosens;
    unsigned lower_cutoff;
    unsigned higher_cutoff;
    double noise_reduction;
    char source[256];
    char method[32];
    unsigned sample_rate;
    unsigned channels;
    bool color_256;
    char gradient_low[64];
    char gradient_high[64];
    char colors[128];
    char mode[32];
    char text_align[16];
    unsigned text_size;
    char text_style[16];
    char provider[16];
    long lyric_offset_ms;
    char lyrics_folder[512];
    char mpris_players[512];
    uint8_t chars[64];
    size_t chars_len;
} SvConfig;

void config_default(SvConfig *c);
int config_load(SvConfig *c, const char *path);
int config_save(const SvConfig *c, const char *path);
void config_default_path(char *buf, size_t n);

int palette_ansi(const char *name);
int color_to_ansi(const char *s);
int color_to_rgb(const char *s, unsigned *r, unsigned *g, unsigned *b);
const char *color_name(const char *s);
int color_index(const char *s);

#endif
