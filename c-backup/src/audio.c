#include "audio.h"

#include <pulse/error.h>
#include <pulse/mainloop.h>
#include <pulse/pulseaudio.h>
#include <pulse/simple.h>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *PA_APP = "sharkvis";
static const char *PA_STREAM = "sharkvis spectrum";

typedef struct {
    char name[512];
    pa_operation *op;
    int done;
} mon_query;

static void sink_info_cb(pa_context *c, const pa_sink_info *i, int eol, void *userdata) {
    (void)c;
    (void)eol;
    mon_query *q = userdata;
    if (q->op) {
        pa_operation_unref(q->op);
        q->op = NULL;
    }
    if (i && i->monitor_source_name)
        snprintf(q->name, sizeof q->name, "%s", i->monitor_source_name);
    q->done = 1;
}

static void server_info_cb(pa_context *c, const pa_server_info *i, void *userdata) {
    mon_query *q = userdata;
    if (q->op) {
        pa_operation_unref(q->op);
        q->op = NULL;
    }
    if (i && i->default_sink_name)
        q->op = pa_context_get_sink_info_by_name(c, i->default_sink_name, sink_info_cb, q);
    else
        q->done = 1;
}

static char *default_sink_monitor(void) {
    pa_mainloop *ml = pa_mainloop_new();
    if (!ml)
        return NULL;
    pa_context *ctx = pa_context_new(pa_mainloop_get_api(ml), PA_APP);
    if (!ctx) {
        pa_mainloop_free(ml);
        return NULL;
    }
    if (pa_context_connect(ctx, NULL, PA_CONTEXT_NOFLAGS, NULL) < 0) {
        pa_context_unref(ctx);
        pa_mainloop_free(ml);
        return NULL;
    }

    mon_query q;
    memset(&q, 0, sizeof q);
    for (int guard = 0; guard < 1000 && !q.done; guard++) {
        int ret = 0;
        if (pa_mainloop_iterate(ml, 1, &ret) < 0 || ret < 0)
            break;
        pa_context_state_t state = pa_context_get_state(ctx);
        if (state == PA_CONTEXT_READY && !q.op) {
            q.op = pa_context_get_server_info(ctx, server_info_cb, &q);
        } else if (state == PA_CONTEXT_FAILED || state == PA_CONTEXT_TERMINATED) {
            break;
        }
    }

    pa_context_disconnect(ctx);
    pa_context_unref(ctx);
    pa_mainloop_free(ml);

    return q.name[0] ? strdup(q.name) : NULL;
}

static size_t next_pow2(size_t v) {
    size_t p = 1;
    while (p < v)
        p <<= 1;
    return p;
}

static void *capture_thread(void *arg) {
    audio_t *a = arg;

    pa_sample_spec ss;
    ss.format = PA_SAMPLE_S16LE;
    ss.rate = a->rate;
    ss.channels = (a->channels < 1) ? 1 : ((a->channels > 2) ? 2 : a->channels);
    const char *dev = NULL;
    char *monitor = NULL;
    if (!a->source || !a->source[0] || strcmp(a->source, "auto") == 0 ||
        strcmp(a->source, "default") == 0) {
        monitor = default_sink_monitor();
        dev = monitor;
    } else {
        dev = a->source;
    }

    int error = 0;
    pa_buffer_attr ba;
    memset(&ba, 0, sizeof ba);
    ba.maxlength = (uint32_t)-1;
    ba.tlength = (uint32_t)-1;
    ba.prebuf = (uint32_t)-1;
    ba.minreq = (uint32_t)-1;
    ba.fragsize = (uint32_t)pa_usec_to_bytes(5000, &ss);
    pa_simple *s = pa_simple_new(NULL, PA_APP, PA_STREAM_RECORD, dev, PA_STREAM, &ss,
                                 NULL, &ba, &error);
    free(monitor);

    if (!s) {
        snprintf(a->error, sizeof a->error, "pulse: pa_simple_new failed: %s",
                 pa_strerror(error));
        a->terminate = true;
        return NULL;
    }

    size_t frames = 512;
    size_t chunk = frames * ss.channels * sizeof(int16_t);
    int16_t *raw = malloc(chunk);
    unsigned nch = ss.channels;

    while (!a->terminate) {
        if (pa_simple_read(s, raw, chunk, &error) < 0) {
            snprintf(a->error, sizeof a->error, "pulse: pa_simple_read failed: %s",
                     pa_strerror(error));
            a->terminate = true;
            break;
        }
        for (size_t f = 0; f < frames; f++) {
            size_t head = atomic_load_explicit(&a->head, memory_order_relaxed);
            size_t tail = atomic_load_explicit(&a->tail, memory_order_acquire);
            if (head - tail >= a->capacity)
                continue; /* ring full: drop newest, never block capture */
            size_t i = head & a->mask;
            for (unsigned ch = 0; ch < nch; ch++)
                a->ring[ch][i] = (double)raw[f * nch + ch] / 32768.0;
            if (nch == 1)
                a->ring[1][i] = a->ring[0][i];
            atomic_store_explicit(&a->head, head + 1, memory_order_release);
        }
    }

    free(raw);
    pa_simple_free(s);
    return NULL;
}

void audio_init(audio_t *a, size_t capacity) {
    memset(a, 0, sizeof *a);
    atomic_init(&a->head, 0);
    atomic_init(&a->tail, 0);
    a->capacity = next_pow2(capacity);
    a->mask = a->capacity - 1;
    a->ring[0] = calloc(a->capacity, sizeof *a->ring[0]);
    a->ring[1] = calloc(a->capacity, sizeof *a->ring[1]);
    a->work[0] = calloc(a->capacity, sizeof *a->work[0]);
    a->work[1] = calloc(a->capacity, sizeof *a->work[1]);
}

void audio_start(audio_t *a, const char *source, unsigned rate, unsigned channels) {
    a->source = source;
    a->rate = rate;
    a->channels = (channels < 1) ? 1 : ((channels > 2) ? 2 : channels);
    a->terminate = false;
    a->error[0] = '\0';
    pthread_create(&a->thread, NULL, capture_thread, a);
}

size_t audio_consume(audio_t *a, const double **left, const double **right) {
    size_t head = atomic_load_explicit(&a->head, memory_order_acquire);
    size_t tail = atomic_load_explicit(&a->tail, memory_order_relaxed);
    size_t n = head - tail;
    if (n > a->capacity)
        n = a->capacity;
    if (n > 0) {
        for (unsigned ch = 0; ch < 2; ch++) {
            size_t t = tail & a->mask;
            size_t part = n;
            if (t + part > a->capacity)
                part = a->capacity - t;
            memcpy(a->work[ch], a->ring[ch] + t, part * sizeof *a->work[ch]);
            if (part < n)
                memcpy(a->work[ch] + part, a->ring[ch],
                       (n - part) * sizeof *a->work[ch]);
        }
        atomic_store_explicit(&a->tail, tail + n, memory_order_release);
    }
    *left = a->work[0];
    *right = (a->channels > 1) ? a->work[1] : NULL;
    return n;
}

bool audio_failed(audio_t *a) {
    return a->terminate;
}

const char *audio_error(audio_t *a) {
    return a->error;
}

void audio_stop(audio_t *a) {
    a->terminate = true;
    pthread_join(a->thread, NULL);
    free(a->ring[0]);
    free(a->ring[1]);
    free(a->work[0]);
    free(a->work[1]);
    a->ring[0] = NULL;
    a->ring[1] = NULL;
    a->work[0] = NULL;
    a->work[1] = NULL;
}
