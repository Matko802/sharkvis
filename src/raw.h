#ifndef SHARKVIS_RAW_H
#define SHARKVIS_RAW_H

#include <stddef.h>

#include "config.h"

typedef enum { RAW_BARS, RAW_WAVE, RAW_SCOPE } RawMode;

int raw_parse_mode(const char *s, RawMode *out);
int raw_run(const SvConfig *cfg, size_t bars, unsigned fps, RawMode mode);

#endif
