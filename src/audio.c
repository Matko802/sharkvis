#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "audio.h"
#include "pulse.h"

#define BLOCK_FRAMES 512

typedef struct {
    _Atomic size_t head;
    _Atomic size_t tail;
    _Atomic int terminate;
    pthread_mutex_t err_mu;
    char error[1024];
    _Atomic size_t *ring;
    pthread_mutex_t *block_mu;
    double *blocks;
    size_t nblocks;
    size_t mask;
} Shared;

struct Audio {
    Shared *shared;
    double *left;
    double *right;
    size_t cap;
    size_t channels;
    pthread_t thread;
    int started;
    char want_source[256];
    unsigned want_rate;
    unsigned want_channels;
};

static size_t next_pow2(size_t v) {
    size_t p = 1;
    while (p < v)
        p <<= 1;
    return p;
}

Audio *audio_new(size_t max_frames) {
    if (max_frames < BLOCK_FRAMES)
        max_frames = BLOCK_FRAMES;
    size_t nb = max_frames / BLOCK_FRAMES;
    if (nb < 2)
        nb = 2;
    nb = next_pow2(nb + 2);
    Audio *a = calloc(1, sizeof *a);
    if (!a)
        return NULL;
    Shared *sh = calloc(1, sizeof *sh);
    if (!sh) {
        free(a);
        return NULL;
    }
    atomic_init(&sh->head, 0);
    atomic_init(&sh->tail, 0);
    atomic_init(&sh->terminate, 0);
    pthread_mutex_init(&sh->err_mu, NULL);
    sh->ring = calloc(nb, sizeof(size_t));
    sh->block_mu = calloc(nb, sizeof(pthread_mutex_t));
    sh->blocks = calloc(nb * BLOCK_FRAMES * 2, sizeof(double));
    if (!sh->ring || !sh->block_mu || !sh->blocks) {
        free(sh->ring);
        free(sh->block_mu);
        free(sh->blocks);
        free(sh);
        free(a);
        return NULL;
    }
    for (size_t i = 0; i < nb; i++) {
        atomic_init(&sh->ring[i], (size_t)-1);
        pthread_mutex_init(&sh->block_mu[i], NULL);
    }
    sh->nblocks = nb;
    sh->mask = nb - 1;
    a->shared = sh;
    a->cap = max_frames;
    a->left = calloc(max_frames, sizeof(double));
    a->right = calloc(max_frames, sizeof(double));
    a->channels = 2;
    if (!a->left || !a->right) {
        audio_free(a);
        return NULL;
    }
    return a;
}

void audio_free(Audio *a) {
    if (!a)
        return;
    audio_stop(a);
    Shared *sh = a->shared;
    if (sh) {
        for (size_t i = 0; i < sh->nblocks; i++)
            pthread_mutex_destroy(&sh->block_mu[i]);
        pthread_mutex_destroy(&sh->err_mu);
        free(sh->ring);
        free(sh->block_mu);
        free(sh->blocks);
        free(sh);
    }
    free(a->left);
    free(a->right);
    free(a);
}

static void set_error(Shared *sh, const char *msg) {
    pthread_mutex_lock(&sh->err_mu);
    snprintf(sh->error, sizeof sh->error, "%s", msg);
    pthread_mutex_unlock(&sh->err_mu);
}

static void push_block(Shared *sh, const double *staged, size_t cnt) {
    if (cnt == 0 || cnt > BLOCK_FRAMES)
        return;
    size_t head = atomic_load(&sh->head);
    size_t tail = atomic_load(&sh->tail);
    if (head - tail < sh->nblocks) {
        size_t slot = head & sh->mask;
        pthread_mutex_lock(&sh->block_mu[slot]);
        memcpy(sh->blocks + slot * BLOCK_FRAMES * 2, staged, cnt * 2 * sizeof(double));
        pthread_mutex_unlock(&sh->block_mu[slot]);
        atomic_store(&sh->ring[slot], cnt);
        atomic_fetch_add(&sh->head, 1);
    }
}

static void *capture_thread(void *arg) {
    Audio *a = arg;
    Shared *sh = a->shared;
    char source[256];
    snprintf(source, sizeof source, "%s", a->want_source);
    unsigned rate = a->want_rate;
    unsigned nch = a->want_channels;
    if (nch < 1)
        nch = 1;
    if (nch > 2)
        nch = 2;
    char err[512];
    Pulse *p = pulse_connect(err, sizeof err);
    if (!p) {
        set_error(sh, err);
        atomic_store(&sh->terminate, 1);
        return NULL;
    }
    char dev[512];
    if (!source[0] || !strcmp(source, "auto") || !strcmp(source, "default")) {
        if (!pulse_default_monitor(p, dev, sizeof dev, err, sizeof err)) {
            set_error(sh, "pulse: could not resolve default sink monitor");
            pulse_free(p);
            atomic_store(&sh->terminate, 1);
            return NULL;
        }
    } else {
        snprintf(dev, sizeof dev, "%s", source);
    }
    unsigned long long bytes_per_sec = (unsigned long long)nch * 2 * rate;
    unsigned fragsize = (unsigned)(5000 * bytes_per_sec / 1000000);
    Record *rec = pulse_record(p, dev, rate, nch, fragsize, err, sizeof err);
    if (!rec) {
        char msg[640];
        snprintf(msg, sizeof msg, "pulse: record stream failed: %s", err);
        set_error(sh, msg);
        atomic_store(&sh->terminate, 1);
        return NULL;
    }
    size_t bpf = nch * 2;
    uint8_t *raw = malloc(BLOCK_FRAMES * bpf);
    double *staged = malloc(BLOCK_FRAMES * 2 * sizeof(double));
    size_t staged_cnt = 0;
    if (!raw || !staged) {
        free(raw);
        free(staged);
        record_free(rec);
        atomic_store(&sh->terminate, 1);
        return NULL;
    }
    for (;;) {
        if (atomic_load(&sh->terminate))
            break;
        long n = record_read(rec, raw, BLOCK_FRAMES * bpf,
                             (const volatile int *)&sh->terminate, err, sizeof err);
        if (n <= 0) {
            if (n < 0 && !atomic_load(&sh->terminate))
                set_error(sh, err);
            if (staged_cnt > 0)
                push_block(sh, staged, staged_cnt);
            atomic_store(&sh->terminate, 1);
            break;
        }
        size_t nframes = (size_t)n / bpf;
        size_t f = 0;
        while (f < nframes) {
            while (f < nframes && staged_cnt < BLOCK_FRAMES) {
                int16_t l = (int16_t)((unsigned)raw[f * bpf] |
                                      ((unsigned)raw[f * bpf + 1] << 8));
                int16_t rr;
                if (nch >= 2) {
                    rr = (int16_t)((unsigned)raw[f * bpf + 2] |
                                   ((unsigned)raw[f * bpf + 3] << 8));
                } else {
                    rr = l;
                }
                staged[staged_cnt * 2] = (double)l / 32768.0;
                staged[staged_cnt * 2 + 1] = (double)rr / 32768.0;
                staged_cnt++;
                f++;
            }
            if (staged_cnt >= BLOCK_FRAMES) {
                push_block(sh, staged, staged_cnt);
                staged_cnt = 0;
            }
        }
        if (staged_cnt > 0) {
            push_block(sh, staged, staged_cnt);
            staged_cnt = 0;
        }
    }
    free(raw);
    free(staged);
    record_free(rec);
    return NULL;
}

int audio_start(Audio *a, const char *source, unsigned rate, unsigned channels) {
    if (channels < 1)
        channels = 1;
    if (channels > 2)
        channels = 2;
    a->channels = channels;
    snprintf(a->want_source, sizeof a->want_source, "%s", source);
    a->want_rate = rate;
    a->want_channels = channels;
    if (pthread_create(&a->thread, NULL, capture_thread, a) != 0)
        return 0;
    a->started = 1;
    return 1;
}

size_t audio_consume(Audio *a, const double **left, const double **right) {
    Shared *sh = a->shared;
    size_t frames = 0;
    for (;;) {
        size_t head = atomic_load(&sh->head);
        size_t tail = atomic_load(&sh->tail);
        if (head == tail || frames >= a->cap)
            break;
        size_t slot = tail & sh->mask;
        size_t n = atomic_load(&sh->ring[slot]);
        if (n == (size_t)-1 || n > BLOCK_FRAMES)
            break;
        if (n > 0 && frames + n > a->cap)
            break;
        pthread_mutex_lock(&sh->block_mu[slot]);
        double *blk = sh->blocks + slot * BLOCK_FRAMES * 2;
        for (size_t i = 0; i < n; i++) {
            a->left[frames + i] = blk[i * 2];
            a->right[frames + i] = blk[i * 2 + 1];
        }
        pthread_mutex_unlock(&sh->block_mu[slot]);
        atomic_store(&sh->ring[slot], (size_t)-1);
        atomic_fetch_add(&sh->tail, 1);
        frames += n;
    }
    if (frames > 0) {
        *left = a->left;
        *right = (a->channels > 1) ? a->right : NULL;
    } else {
        *left = NULL;
        *right = NULL;
    }
    return frames;
}

int audio_failed(const Audio *a) {
    return atomic_load(&a->shared->terminate);
}

const char *audio_error(Audio *a) {
    return a->shared->error;
}

void audio_stop(Audio *a) {
    if (!a || !a->started)
        return;
    atomic_store(&a->shared->terminate, 1);
    pthread_join(a->thread, NULL);
    a->started = 0;
}
