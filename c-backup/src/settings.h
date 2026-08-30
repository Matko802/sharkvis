#ifndef SHARK_SETTINGS_H
#define SHARK_SETTINGS_H

#include <stdbool.h>
#include <stddef.h>

#include "config.h"

enum {
    CH_LAYOUT = 1 << 0,
    CH_DSP    = 1 << 1,
    CH_AUDIO  = 1 << 2,
    CH_EDITOR = 1 << 3,
};

typedef struct settings_ui settings_ui;

settings_ui *settings_new(void);
void settings_free(settings_ui *s);
void settings_key(settings_ui *s, srk_config *cfg, int key, const char *cp,
                 unsigned *changed);
void settings_draw(settings_ui *s, const srk_config *cfg, char *out,
                   size_t *out_len, size_t cap, unsigned rows, int panel_width);

#endif
