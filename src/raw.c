#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <time.h>
#include <unistd.h>

#include "audio.h"
#include "config.h"
#include "dsp.h"
#include "raw.h"

static volatile sig_atomic_t raw_stop = 0;

static void on_sig(int sig) {
    (void)sig;
    raw_stop = 1;
}

int raw_parse_mode(const char *s, RawMode *out) {
    if (!strcasecmp(s, "bars")) {
        *out = RAW_BARS;
        return 1;
    }
    if (!strcasecmp(s, "wave")) {
        *out = RAW_WAVE;
        return 1;
    }
    if (!strcasecmp(s, "oscilloscope")) {
        *out = RAW_SCOPE;
        return 1;
    }
    return 0;
}

static uint64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000 + (uint64_t)ts.tv_nsec;
}

int raw_run(const SvConfig *cfg, size_t bars, unsigned fps, RawMode mode) {
    if (bars < 1)
        bars = 1;
    if (fps < 1)
        fps = 1;
    if (fps > 240)
        fps = 240;
    int stereo = cfg->channels > 1;
    Dsp *dsp0 = dsp_new(bars, cfg->sample_rate, cfg->autosens,
                        cfg->noise_reduction, cfg->lower_cutoff, cfg->higher_cutoff);
    Dsp *dsp1 = dsp_new(bars, cfg->sample_rate, cfg->autosens,
                        cfg->noise_reduction, cfg->lower_cutoff, cfg->higher_cutoff);
    if (!dsp0 || !dsp1) {
        dsp_free(dsp0);
        dsp_free(dsp1);
        return 1;
    }
    dsp_set_display_fps(dsp0, (double)fps);
    dsp_set_display_fps(dsp1, (double)fps);
    dsp_set_sens_scale(dsp0, cfg->sensitivity / 100.0);
    dsp_set_sens_scale(dsp1, cfg->sensitivity / 100.0);
    Audio *audio = audio_new(dsp_render_frame_size(dsp0));
    if (!audio) {
        dsp_free(dsp0);
        dsp_free(dsp1);
        return 1;
    }
    audio_start(audio, cfg->source, cfg->sample_rate, cfg->channels);
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_sig;
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
    uint64_t frame_ns = 1000000000u / fps;
    uint64_t next = now_ns();
    double *h0 = calloc(bars, sizeof(double));
    double *h1 = calloc(bars, sizeof(double));
    double *last = NULL;
    size_t last_n = 0;
    double *last_r = NULL;
    size_t last_r_n = 0;
    char *line = malloc(bars * 8 + 2);
    int rc = 0;
    while (!raw_stop) {
        const double *sl = NULL;
        const double *sr = NULL;
        size_t n = audio_consume(audio, &sl, &sr);
        if (n > 0) {
            if (sl) {
                double *p = realloc(last, n * sizeof(double));
                if (p) {
                    last = p;
                    memcpy(last, sl, n * sizeof(double));
                    last_n = n;
                }
            }
            const double *s = sr ? sr : sl;
            if (s) {
                double *p = realloc(last_r, n * sizeof(double));
                if (p) {
                    last_r = p;
                    memcpy(last_r, s, n * sizeof(double));
                    last_r_n = n;
                }
            }
        }
        dsp_execute(dsp0, sl, n, h0);
        if (stereo)
            dsp_execute(dsp1, sr ? sr : sl, n, h1);
        if (audio_failed(audio)) {
            fprintf(stderr, "sharkvis: audio input failed: %s\n", audio_error(audio));
            rc = 1;
            break;
        }
        size_t pos = 0;
        size_t cap = bars * 8 + 2;
        for (size_t i = 0; i < bars; i++) {
            char tmp[32];
            if (mode == RAW_BARS) {
                double v = stereo ? (h0[i] + h1[i]) * 50.0 : h0[i] * 100.0;
                if (v < 0.0)
                    v = 0.0;
                if (v > 100.0)
                    v = 100.0;
                snprintf(tmp, sizeof tmp, "%s%d", i > 0 ? ";" : "", (int)round(v));
            } else if (mode == RAW_WAVE) {
                int v;
                if (last_n == 0) {
                    v = 50;
                } else {
                    double s = last[(i * last_n) / bars];
                    if (s < -1.0)
                        s = -1.0;
                    if (s > 1.0)
                        s = 1.0;
                    v = (int)round(50.0 + 50.0 * s);
                }
                snprintf(tmp, sizeof tmp, "%s%d", i > 0 ? ";" : "", v);
            } else {
                int l, r;
                if (last_n == 0) {
                    l = 50;
                    r = 50;
                } else {
                    double x = last[(i * last_n) / bars];
                    if (x < -1.0)
                        x = -1.0;
                    if (x > 1.0)
                        x = 1.0;
                    double y;
                    if (last_r_n == 0) {
                        y = x;
                    } else {
                        y = last_r[(i * last_r_n) / bars];
                        if (y < -1.0)
                            y = -1.0;
                        if (y > 1.0)
                            y = 1.0;
                    }
                    l = (int)round(50.0 + 50.0 * x);
                    r = (int)round(50.0 + 50.0 * y);
                }
                snprintf(tmp, sizeof tmp, "%s%d;%d", i > 0 ? ";" : "", l, r);
            }
            size_t tl = strlen(tmp);
            if (pos + tl + 2 < cap) {
                memcpy(line + pos, tmp, tl);
                pos += tl;
            }
        }
        line[pos++] = '\n';
        if (fwrite(line, 1, pos, stdout) != pos)
            break;
        fflush(stdout);
        uint64_t now = now_ns();
        if (next > now) {
            struct timespec ts;
            uint64_t wait = next - now;
            ts.tv_sec = (time_t)(wait / 1000000000);
            ts.tv_nsec = (long)(wait % 1000000000);
            nanosleep(&ts, NULL);
            next += frame_ns;
        } else {
            next = now + frame_ns;
        }
    }
    free(h0);
    free(h1);
    free(last);
    free(last_r);
    free(line);
    audio_stop(audio);
    audio_free(audio);
    dsp_free(dsp0);
    dsp_free(dsp1);
    return rc;
}
