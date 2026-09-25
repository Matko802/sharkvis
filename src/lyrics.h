#ifndef SHARKVIS_LYRICS_H
#define SHARKVIS_LYRICS_H

#include <stdbool.h>
#include <stddef.h>

#include "mpris.h"

typedef struct {
    double t;
    char *text;
} LyricWord;

typedef struct {
    double t;
    char *text;
    LyricWord *words;
    size_t nwords;
} LyricLine;

typedef struct {
    char *text;
    int cur;
} LyricDisplay;

void lyric_lines_free(LyricLine *lines, size_t n);
void lyric_display_free(LyricDisplay *d, size_t n);
void url_encode(const char *s, char *out, size_t outn);

typedef struct {
    char local_folder[512];
    char provider[32];
} FetchOpts;

typedef struct LyricWorker LyricWorker;

LyricWorker *lyric_new(void);
void lyric_free(LyricWorker *w);
void lyric_set_offset_ms(LyricWorker *w, long ms);
void lyric_set_follow(LyricWorker *w, int on, double pos);
int lyric_following(const LyricWorker *w);
void lyric_search_override(LyricWorker *w, const char *artist, const char *title);
void lyric_force_reload(LyricWorker *w);
void lyric_poke(LyricWorker *w);
int lyric_loading(const LyricWorker *w);
void lyric_reset(LyricWorker *w);
void lyric_update(LyricWorker *w, const Track *track, const FetchOpts *opts);
void lyric_update_pos(LyricWorker *w, double pos);
LyricDisplay *lyric_display_lines(LyricWorker *w, const Track *track, size_t *n);
LyricDisplay *lyric_display_context(LyricWorker *w, const Track *track, size_t *n);

#endif
