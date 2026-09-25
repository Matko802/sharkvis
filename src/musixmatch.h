#ifndef SHARKVIS_MUSIXMATCH_H
#define SHARKVIS_MUSIXMATCH_H

#include <stddef.h>

#include "lyrics.h"

LyricLine *musix_fetch(const char *artist, const char *title,
                       double duration, size_t *nlines);

#endif
