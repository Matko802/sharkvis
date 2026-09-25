#ifndef SHARKVIS_SETTINGS_H
#define SHARKVIS_SETTINGS_H

#include <stddef.h>

#include "common.h"
#include "config.h"

#define CH_LAYOUT 1u
#define CH_DSP 2u
#define CH_AUDIO 4u
#define CH_EDITOR 8u

typedef struct SettingsUi SettingsUi;

SettingsUi *settings_new(void);
void settings_free(SettingsUi *s);
void settings_key(SettingsUi *s, SvConfig *cfg, int key,
                  const uint8_t *cp, size_t cplen, unsigned *changed);
void settings_draw(SettingsUi *s, const SvConfig *cfg, SvBuf *out,
                   size_t cap, unsigned rows, size_t pw);
size_t settings_visible_rows(const char *mode, size_t *out, size_t max);

#endif
