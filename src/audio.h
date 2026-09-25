#ifndef SHARKVIS_AUDIO_H
#define SHARKVIS_AUDIO_H

#include <stdbool.h>
#include <stddef.h>

typedef struct Audio Audio;

Audio *audio_new(size_t max_frames);
void audio_free(Audio *a);
int audio_start(Audio *a, const char *source, unsigned rate, unsigned channels);
size_t audio_consume(Audio *a, const double **left, const double **right);
int audio_failed(const Audio *a);
const char *audio_error(Audio *a);
void audio_stop(Audio *a);

#endif
