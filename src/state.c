#include <dirent.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#include "common.h"
#include "state.h"

typedef struct {
    double avg;
    double peak;
    double prev;
    double favg;
    double fpeak;
    double fprev;
    double beat;
    double now;
    double onsets[8];
    size_t n;
    double period;
    double next;
    float misses;
    double cand;
    unsigned cstr;
} BeatTracker;

struct StateWriter {
    char *path;
    int custom;
    uint64_t started_ms;
    unsigned pid;
    int have_last_write;
    uint64_t last_write_ms;
    BeatTracker tracker;
    int have_tick;
    uint64_t tick_ns;
    int dir_ready;
};

void beat_step(double energy, double *avg, double *peak, double *prev,
               double *beat, double dt) {
    double a = *avg + (energy - *avg) * (1.0 - exp(-dt * 1.5));
    double p = energy > *peak * exp(-dt * 0.8) ? energy : *peak * exp(-dt * 0.8);
    double range = (p - a) > 0.05 ? (p - a) : 0.05;
    double strength = (energy - a) / range;
    if (strength < 0.0)
        strength = 0.0;
    if (strength > 1.0)
        strength = 1.0;
    double b;
    if (strength > 0.55 && energy > 0.05 && energy > *prev)
        b = 1.0;
    else
        b = *beat * exp(-dt * 5.0);
    if (b < 0.0)
        b = 0.0;
    if (b > 1.0)
        b = 1.0;
    *avg = a;
    *peak = p;
    *prev = energy;
    *beat = b;
}

static void tracker_init(BeatTracker *t) {
    memset(t, 0, sizeof *t);
}

static double estimate_period(const double *times, size_t n) {
    double iois[8];
    size_t m = 0;
    for (size_t i = 0; i + 1 < n && m < 8; i++) {
        double d = times[i + 1] - times[i];
        if (d > 0.05)
            iois[m++] = d;
    }
    if (m < 3)
        return -1.0;
    for (size_t i = 1; i < m; i++) {
        size_t j = i;
        while (j > 0 && iois[j] < iois[j - 1]) {
            double tmp = iois[j];
            iois[j] = iois[j - 1];
            iois[j - 1] = tmp;
            j--;
        }
    }
    double p = iois[m / 2];
    if (p < 0.2)
        p = 0.2;
    if (p > 1.5)
        p = 1.5;
    while (p > 0.65)
        p /= 2.0;
    while (p < 0.30)
        p *= 2.0;
    double bpm = round(60.0 / p);
    if (bpm < 60.0)
        bpm = 60.0;
    if (bpm > 200.0)
        bpm = 200.0;
    return 60.0 / bpm;
}

static double tracker_step(BeatTracker *t, double full, double bass, double dt) {
    if (dt < 0.001)
        dt = 0.001;
    if (dt > 1.0)
        dt = 1.0;
    t->now += dt;
    double e = bass;
    if (e < 0.0)
        e = 0.0;
    if (e > 1.0)
        e = 1.0;
    double fl = full;
    if (fl < 0.0)
        fl = 0.0;
    if (fl > 1.0)
        fl = 1.0;
    double bavg = t->avg, bpeak = t->peak, bprev = t->prev, bb = t->beat;
    beat_step(e, &bavg, &bpeak, &bprev, &bb, dt);
    t->avg = bavg;
    t->peak = bpeak;
    t->prev = bprev;
    double favg = t->favg, fpeak = t->fpeak, fprev = t->fprev, fb = t->beat;
    beat_step(fl, &favg, &fpeak, &fprev, &fb, dt);
    t->favg = favg;
    t->fpeak = fpeak;
    t->fprev = fprev;
    if (bb == 1.0 || fb == 1.0) {
        double tt = t->now;
        if (t->n < 8) {
            t->onsets[t->n++] = tt;
        } else {
            memmove(t->onsets, t->onsets + 1, 7 * sizeof(double));
            t->onsets[7] = tt;
        }
        if (t->n >= 5) {
            if (t->period > 0.0) {
                double window = 0.12 * t->period;
                double d = t->next - tt;
                if (d < 0)
                    d = -d;
                if (d <= window) {
                    t->next = tt + t->period;
                    t->misses = 0.0f;
                }
            }
            double p = estimate_period(t->onsets, t->n);
            if (p > 0.0) {
                double denom = t->cand > 1e-6 ? t->cand : 1e-6;
                double rel = (p - t->cand) / denom;
                if (rel < 0)
                    rel = -rel;
                if (rel <= 0.12)
                    t->cstr++;
                else {
                    t->cand = p;
                    t->cstr = 1;
                }
                if (t->cstr >= 2 && (t->period <= 0.0 || t->misses >= 3.0f)) {
                    t->period = t->cand;
                    t->next = tt + t->cand;
                    t->misses = 0.0f;
                }
            }
        }
        t->beat = 1.0;
    } else if (t->period > 0.0) {
        double window = 0.12 * t->period;
        double brange = (t->peak - t->avg) > 0.05 ? (t->peak - t->avg) : 0.05;
        double frange = (t->fpeak - t->favg) > 0.05 ? (t->fpeak - t->favg) : 0.05;
        double bs = (e - t->avg) / brange;
        double fs = (fl - t->favg) / frange;
        if (bs < 0.0)
            bs = 0.0;
        if (bs > 1.0)
            bs = 1.0;
        if (fs < 0.0)
            fs = 0.0;
        if (fs > 1.0)
            fs = 1.0;
        double strength = bs > fs ? bs : fs;
        double loud = e > fl ? e : fl;
        if (t->now >= t->next - window && strength > 0.25 && loud > 0.05) {
            t->beat = 1.0;
            t->misses = 0.0f;
            t->next += t->period;
        } else {
            t->beat *= exp(-dt * 5.0);
            if (t->now > t->next + window) {
                t->misses += 1.0f;
                t->next += t->period;
                if (t->misses >= 4.0f)
                    t->period = 0.0;
            }
        }
    } else {
        t->beat = bb > fb ? bb : fb;
    }
    if (t->beat < 0.0)
        t->beat = 0.0;
    if (t->beat > 1.0)
        t->beat = 1.0;
    return t->beat;
}

void lerp_rgb(uint8_t lr, uint8_t lg, uint8_t lb, uint8_t hr, uint8_t hg,
              uint8_t hb, float t, uint8_t *r, uint8_t *g, uint8_t *b) {
    if (t < 0.0f)
        t = 0.0f;
    if (t > 1.0f)
        t = 1.0f;
    *r = (uint8_t)(lr + (hr - lr) * t + 0.5f);
    *g = (uint8_t)(lg + (hg - lg) * t + 0.5f);
    *b = (uint8_t)(lb + (hb - lb) * t + 0.5f);
}

static int custom_state_path(char *buf, size_t n) {
    const char *p = getenv("SHARKVIS_STATE");
    if (p && *p) {
        const char *s = p;
        while (*s == ' ' || *s == '\t')
            s++;
        if (*s) {
            snprintf(buf, n, "%s", p);
            return 1;
        }
    }
    return 0;
}

void state_path(char *buf, size_t n) {
    char tmp[1024];
    if (custom_state_path(tmp, sizeof tmp)) {
        snprintf(buf, n, "%s", tmp);
        return;
    }
    const char *rt = getenv("XDG_RUNTIME_DIR");
    if (rt && *rt) {
        size_t l = strlen(rt);
        while (l > 0 && rt[l - 1] == '/')
            l--;
        snprintf(buf, n, "%.*s/sharkvis/state", (int)l, rt);
        return;
    }
    snprintf(buf, n, "/tmp/sharkvis-%u.state", (unsigned)getuid());
}

uint64_t session_started_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (uint64_t)ts.tv_sec * 1000 + (uint64_t)ts.tv_nsec / 1000000;
}

StateWriter *state_writer_new(void) {
    StateWriter *w = calloc(1, sizeof *w);
    if (!w)
        return NULL;
    char tmp[1024];
    w->custom = custom_state_path(tmp, sizeof tmp);
    if (getenv("SHARKVIS_NO_STATE")) {
        w->path = NULL;
    } else {
        char sp[1024];
        state_path(sp, sizeof sp);
        w->path = strdup(sp);
    }
    w->started_ms = session_started_ms();
    w->pid = (unsigned)getpid();
    tracker_init(&w->tracker);
    return w;
}

void state_writer_free(StateWriter *w) {
    if (!w)
        return;
    free(w->path);
    free(w);
}

static void write_atomic(const char *path, const char *body, int *dir_ready) {
    if (strlen(path) + 5 >= 2112) {
        *dir_ready = 0;
        return;
    }
    char tmp[2112];
    snprintf(tmp, sizeof tmp, "%s.tmp", path);
    FILE *f = fopen(tmp, "w");
    if (!f) {
        *dir_ready = 0;
        return;
    }
    fwrite(body, 1, strlen(body), f);
    fclose(f);
    if (rename(tmp, path) != 0) {
        remove(tmp);
        *dir_ready = 0;
    }
}

static int session_sibling(const char *state_path_in, char *out, size_t n) {
    char parent[1024];
    snprintf(parent, sizeof parent, "%s", state_path_in);
    char *slash = strrchr(parent, '/');
    if (!slash)
        return 0;
    *slash = 0;
    char *base = strrchr(parent, '/');
    base = base ? base + 1 : parent;
    if (strcmp(base, "sharkvis") != 0)
        return 0;
    if (strlen(parent) + 24 >= n)
        return 0;
    snprintf(out, n, "%s/state-%u", parent, (unsigned)getpid());
    return 1;
}

void state_writer_update(StateWriter *w, double energy, double bass,
                         double left, double right,
                         uint8_t lr, uint8_t lg, uint8_t lb,
                         uint8_t hr, uint8_t hg, uint8_t hb) {
    uint64_t now_ns = sv_now_ns();
    double dt;
    if (w->have_tick) {
        dt = (double)(now_ns - w->tick_ns) / 1e9;
        if (dt < 0.001)
            dt = 0.001;
        if (dt > 1.0)
            dt = 1.0;
    } else {
        dt = 1.0 / 60.0;
    }
    w->have_tick = 1;
    w->tick_ns = now_ns;
    double e = energy;
    if (e < 0.0)
        e = 0.0;
    if (e > 1.0)
        e = 1.0;
    double bs = bass;
    if (bs < 0.0)
        bs = 0.0;
    if (bs > 1.0)
        bs = 1.0;
    double beat = tracker_step(&w->tracker, e, bs, dt);
    if (!w->path)
        return;
    uint64_t now_ms = sv_now_ms();
    if (w->have_last_write && now_ms - w->last_write_ms < 50)
        return;
    w->have_last_write = 1;
    w->last_write_ms = now_ms;
    if (!w->dir_ready) {
        char parent[1024];
        snprintf(parent, sizeof parent, "%s", w->path);
        char *slash = strrchr(parent, '/');
        if (slash) {
            *slash = 0;
            char cur[1024];
            size_t n = 0;
            if (parent[0] == '/')
                cur[n++] = '/';
            char *save = NULL;
            char *tok = strtok_r(parent + (parent[0] == '/' ? 1 : 0), "/", &save);
            while (tok) {
                size_t l = strlen(tok);
                if (n + l + 1 < sizeof cur) {
                    memcpy(cur + n, tok, l);
                    n += l;
                    cur[n] = 0;
                    mkdir(cur, 0755);
                    cur[n++] = '/';
                    cur[n] = 0;
                }
                tok = strtok_r(NULL, "/", &save);
            }
            w->dir_ready = 1;
        } else {
            w->dir_ready = 1;
        }
    }
    uint8_t r, g, b;
    lerp_rgb(lr, lg, lb, hr, hg, hb, (float)e, &r, &g, &b);
    double l = left;
    if (l < 0.0)
        l = 0.0;
    if (l > 1.0)
        l = 1.0;
    double rr = right;
    if (rr < 0.0)
        rr = 0.0;
    if (rr > 1.0)
        rr = 1.0;
    char body[512];
    snprintf(body, sizeof body,
             "color=#%02x%02x%02x energy=%.2f beat=%.2f "
             "color_low=#%02x%02x%02x color_high=#%02x%02x%02x "
             "bass=%.2f left=%.2f right=%.2f started=%llu pid=%u\n",
             r, g, b, e, beat, lr, lg, lb, hr, hg, hb, bs, l, rr,
             (unsigned long long)w->started_ms, w->pid);
    if (!w->custom) {
        char sess[1024];
        if (session_sibling(w->path, sess, sizeof sess))
            write_atomic(sess, body, &w->dir_ready);
    }
    write_atomic(w->path, body, &w->dir_ready);
}

static int state_disabled(void) {
    return getenv("SHARKVIS_NO_STATE") != NULL;
}

static void remove_with_tmp(const char *path) {
    remove(path);
    if (strlen(path) + 5 >= 2112)
        return;
    char tmp[2112];
    snprintf(tmp, sizeof tmp, "%s.tmp", path);
    remove(tmp);
}

void clear_state_file(void) {
    if (state_disabled())
        return;
    char path[1024];
    state_path(path, sizeof path);
    char tmp[1024];
    if (!custom_state_path(tmp, sizeof tmp)) {
        char sess[1024];
        if (session_sibling(path, sess, sizeof sess))
            remove_with_tmp(sess);
    }
    remove_with_tmp(path);
}

static int file_age_ms(const char *path, uint64_t *age_ms) {
    struct stat st;
    if (stat(path, &st) != 0)
        return 0;
    struct timespec now;
    clock_gettime(CLOCK_REALTIME, &now);
    int64_t sm = (int64_t)now.tv_sec - (int64_t)st.st_mtim.tv_sec;
    int64_t nm = (int64_t)now.tv_nsec - (int64_t)st.st_mtim.tv_nsec;
    int64_t age = sm * 1000 + nm / 1000000;
    if (age < 0)
        age = 0;
    *age_ms = (uint64_t)age;
    return 1;
}

static void sweep_session_dir(const char *dir) {
    DIR *dp = opendir(dir);
    if (!dp)
        return;
    struct dirent *de;
    while ((de = readdir(dp)) != NULL) {
        const char *name = de->d_name;
        if (strncmp(name, "state-", 6) != 0)
            continue;
        size_t l = strlen(name);
        if (l > 4 && !strcmp(name + l - 4, ".tmp"))
            continue;
        char full[2304];
        snprintf(full, sizeof full, "%s/%s", dir, name);
        uint64_t age;
        if (file_age_ms(full, &age) && age > 1000)
            remove_with_tmp(full);
    }
    closedir(dp);
}

void remove_stale_state(void) {
    if (state_disabled())
        return;
    char path[1024];
    state_path(path, sizeof path);
    uint64_t age;
    if (file_age_ms(path, &age) && age > 1000)
        remove_with_tmp(path);
    char tmp[1024];
    if (!custom_state_path(tmp, sizeof tmp)) {
        char parent[1024];
        snprintf(parent, sizeof parent, "%s", path);
        char *slash = strrchr(parent, '/');
        if (slash) {
            *slash = 0;
            char *base = strrchr(parent, '/');
            base = base ? base + 1 : parent;
            if (!strcmp(base, "sharkvis"))
                sweep_session_dir(parent);
        }
    }
}
