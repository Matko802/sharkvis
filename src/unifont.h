#ifndef SHARKVIS_UNIFONT_H
#define SHARKVIS_UNIFONT_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

int unifont_get_size(uint32_t cp, int *w, int *h);
int unifont_get_pixel(uint32_t cp, int x, int y);

#endif
