#ifndef SHARK_AUDIO_H
#define SHARK_AUDIO_H

#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>

typedef struct {
    _Atomic size_t head;
    _Atomic size_t tail;
    double *ring[2];
    double *work[2];
    size_t capacity;
    size_t mask;
    volatile bool terminate;
    char error[256];
    pthread_t thread;
    const char *source;
    unsigned rate;
    unsigned channels;
} audio_t;

void audio_init(audio_t *a, size_t capacity);
void audio_start(audio_t *a, const char *source, unsigned rate, unsigned channels);
size_t audio_consume(audio_t *a, const double **left, const double **right);
bool audio_failed(audio_t *a);
const char *audio_error(audio_t *a);
void audio_stop(audio_t *a);

#endif
