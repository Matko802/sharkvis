#include "unifont.h"
#include "unifont_data.h"

int unifont_get_size(uint32_t cp, int *w, int *h) {
    size_t lo = 0;
    size_t hi = unifont_glyph_count;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        if (unifont_glyphs[mid].cp < cp)
            lo = mid + 1;
        else
            hi = mid;
    }
    if (lo >= unifont_glyph_count || unifont_glyphs[lo].cp != cp)
        return 0;
    *w = unifont_glyphs[lo].w;
    *h = 16;
    return 1;
}

int unifont_get_pixel(uint32_t cp, int x, int y) {
    size_t lo = 0;
    size_t hi = unifont_glyph_count;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        if (unifont_glyphs[mid].cp < cp)
            lo = mid + 1;
        else
            hi = mid;
    }
    if (lo >= unifont_glyph_count || unifont_glyphs[lo].cp != cp)
        return 0;
    if (x < 0 || y < 0 || y >= 16)
        return 0;
    const unifont_glyph_t *g = &unifont_glyphs[lo];
    if (x >= g->w)
        return 0;
    if (g->w <= 8)
        return (g->rows[2 * (size_t)y] >> (7 - x)) & 1;
    return ((g->rows[2 * (size_t)y] << 8) | g->rows[2 * (size_t)y + 1]) >> (15 - x) & 1;
}
