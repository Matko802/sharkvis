#ifndef SHARK_CONFIG_H
#define SHARK_CONFIG_H

#include <stdbool.h>
#include <stddef.h>

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
    char *source;
    unsigned sample_rate;
    unsigned channels;
    bool color_256;
    char *gradient_low;
    char *gradient_high;
    char *mode;
    char *glyphs;
} srk_config;

typedef struct {
    const char *name;
    const char *hex;
} color_entry;

extern const color_entry g_palette[];
extern const int g_palette_n;

const char *color_name(const char *hex);
int color_index(const char *hex);
int color_to_rgb(const char *hex, unsigned *r, unsigned *g, unsigned *b);

void config_default(srk_config *c);
bool config_load(srk_config *c, const char *path);
bool config_save(const srk_config *c, const char *path);
char *config_default_path(void);
void config_free(srk_config *c);

#endif
