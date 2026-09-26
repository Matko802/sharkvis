#include <errno.h>
#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#include "audio.h"
#include "common.h"
#include "config.h"
#include "dsp.h"
#include "lyrics.h"
#include "mpris.h"
#include "raw.h"
#include "render.h"
#include "settings.h"
#include "state.h"
#include "term.h"

#ifndef SHARKVIS_VERSION
#define SHARKVIS_VERSION "0.2.9"
#endif

#define VIS_EPS 0.001
#define OUT_CAP ((size_t)1 << 20)

static volatile sig_atomic_t g_sig = 0;
static volatile sig_atomic_t g_resize = 0;

static void on_signal(int sig) {
    (void)sig;
    g_sig = 1;
}

static void on_winch(int sig) {
    (void)sig;
    g_resize = 1;
}

static void on_fatal(int sig) {
    const char *msg = "\x1b[?25h\x1b[0m\x1b[2J\x1b[H";
    write(1, msg, strlen(msg));
    term_raw_restore(0);
    _exit(128 + sig);
}

static void set_handler(int sig, void (*h)(int)) {
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = h;
    sigemptyset(&sa.sa_mask);
    sigaction(sig, &sa, NULL);
}

static void usage(void) {
    printf("usage: sharkvis [-p config_file] [--raw [--bars N] [--fps N] [--raw-mode bars|wave|oscilloscope]]\n");
    printf("  --raw: print bar levels (0-100, ';'-separated, one line per frame) to stdout\n");
    printf("  g - settings, q - quit\n");
}

static size_t panel_width_for(unsigned cols) {
    unsigned pw = cols / 3;
    if (pw < 28)
        pw = 28;
    if (pw > 44)
        pw = 44;
    if (pw >= cols)
        pw = cols > 2 ? cols / 2 : 1;
    if (pw < 1)
        pw = 1;
    return pw;
}

static size_t bar_count_for(unsigned cols, const SvConfig *cfg) {
    size_t step = cfg->bar_width + cfg->bar_spacing;
    size_t avail = step > 0 ? cols / step : cols;
    size_t b = cfg->bars > 0 ? cfg->bars : avail;
    return b < 1 ? 1 : b;
}

static size_t yscale_for(int fd) {
    double a = term_cell_aspect(fd);
    long v = lround(2.0 / a);
    if (v < 1)
        v = 1;
    if (v > 4)
        v = 4;
    return (size_t)v;
}

static size_t per_ch_left(size_t bars, unsigned channels) {
    if (channels > 1 && bars > 1)
        return (bars + 1) / 2;
    return bars;
}

static size_t per_ch_right(size_t bars, unsigned channels) {
    if (channels > 1 && bars > 1)
        return bars / 2;
    return bars;
}

static int is_k(int key, const uint8_t *cp, size_t cplen, uint8_t ch) {
    if (key == (int)ch)
        return 1;
    if (key == KEY_CHAR && cplen > 0 && cp[0] == ch)
        return 1;
    return 0;
}

static void run_editor(const char *path) {
    printf("\x1b[0m\x1b[2J\x1b[H\x1b[?25h");
    fflush(stdout);
    term_raw_restore(0);
    void (*old_int)(int) = signal(SIGINT, SIG_IGN);
    pid_t pid = fork();
    if (pid == 0) {
        signal(SIGINT, SIG_DFL);
        execlp("nano", "nano", path, (char *)NULL);
        _exit(127);
    }
    if (pid > 0) {
        int status = 0;
        for (;;) {
            pid_t r = waitpid(pid, &status, 0);
            if (r >= 0 || errno != EINTR)
                break;
        }
        if (WIFEXITED(status) && WEXITSTATUS(status) == 127)
            fprintf(stderr, "sharkvis: could not launch nano\n");
    }
    signal(SIGINT, old_int);
    term_raw_enter(0);
    printf("\x1b[2J\x1b[H\x1b[?25l");
    fflush(stdout);
}

static void apply_colors(Renderer *rnd, const SvConfig *cfg) {
    unsigned r, g, b;
    if (color_to_rgb(cfg->gradient_low, &r, &g, &b))
        rnd->grad_lo = (r << 16) | (g << 8) | b;
    if (color_to_rgb(cfg->gradient_high, &r, &g, &b))
        rnd->grad_hi = (r << 16) | (g << 8) | b;
    rnd->grad_lo_term = color_to_ansi(cfg->gradient_low);
    rnd->grad_hi_term = color_to_ansi(cfg->gradient_high);
}

static void apply_settings(Dsp **dsp, Renderer *rnd, Audio **audio, SvConfig *cfg,
                           size_t *bars, double **heights, double **last_h,
                           unsigned rows, unsigned cols, unsigned chmask,
                           int audio_reinit, size_t x_off) {
    size_t new_bars = bar_count_for(cols, cfg);
    size_t pcl = per_ch_left(new_bars, cfg->channels);
    size_t pcr = per_ch_right(new_bars, cfg->channels);
    if ((chmask & (CH_DSP | CH_AUDIO)) || new_bars != *bars) {
        size_t per[2] = {pcl, pcr};
        for (int ch = 0; ch < 2; ch++) {
            double ss;
            int si;
            dsp_get_sens(dsp[ch], &ss, &si);
            Dsp *nd = dsp_new(per[ch], cfg->sample_rate, cfg->autosens,
                              cfg->noise_reduction, cfg->lower_cutoff,
                              cfg->higher_cutoff);
            if (nd) {
                dsp_set_sens(nd, ss, si);
                double fps = cfg->framerate > 1 ? (double)cfg->framerate : 1.0;
                dsp_set_display_fps(nd, fps);
                dsp_free(dsp[ch]);
                dsp[ch] = nd;
            }
        }
    }
    if (new_bars != *bars) {
        for (int ch = 0; ch < 2; ch++) {
            heights[ch] = realloc(heights[ch], new_bars * sizeof(double));
            last_h[ch] = realloc(last_h[ch], new_bars * sizeof(double));
            for (size_t i = 0; i < new_bars; i++) {
                heights[ch][i] = 0.0;
                last_h[ch][i] = 0.0;
            }
        }
        *bars = new_bars;
        renderer_resize(rnd, rows, cols, new_bars);
    }
    rnd->bar_width = cfg->bar_width;
    rnd->bar_spacing = cfg->bar_spacing;
    rnd->color_256 = cfg->color_256 ? 1 : 0;
    apply_colors(rnd, cfg);
    renderer_set_mode(rnd, mode_parse(cfg->mode[0] ? cfg->mode : "bars"));
    renderer_set_glyphs(rnd, cfg->chars, cfg->chars_len);
    renderer_set_wave(rnd, cfg->sample_rate);
    renderer_set_offset(rnd, x_off);
    renderer_clear(rnd);
    if (chmask) {
        for (size_t i = 0; i < *bars; i++) {
            heights[0][i] = 0.0;
            heights[1][i] = 0.0;
        }
    }
    if (audio_reinit) {
        audio_stop(*audio);
        audio_free(*audio);
        Audio *na = audio_new(dsp_render_frame_size(dsp[0]));
        if (na) {
            audio_start(na, cfg->source, cfg->sample_rate, cfg->channels);
            *audio = na;
        }
    }
}

static void clamp_cfg(SvConfig *cfg) {
    if (cfg->bar_width < 1)
        cfg->bar_width = 1;
    if (cfg->framerate < 1)
        cfg->framerate = 1;
    if (cfg->framerate > 240)
        cfg->framerate = 240;
    if (cfg->sensitivity < 0.1)
        cfg->sensitivity = 0.1;
    if (cfg->noise_reduction < 0.0)
        cfg->noise_reduction = 0.0;
    if (cfg->noise_reduction > 1.0)
        cfg->noise_reduction = 1.0;
    if (cfg->lower_cutoff < 1)
        cfg->lower_cutoff = 1;
    if (cfg->higher_cutoff < cfg->lower_cutoff)
        cfg->higher_cutoff = cfg->lower_cutoff + 1;
    if (cfg->channels < 1)
        cfg->channels = 1;
    if (cfg->channels > 2)
        cfg->channels = 2;
    if (!strcmp(cfg->mode, "text"))
        strcpy(cfg->mode, "lyrics");
}

static uint64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000 + (uint64_t)ts.tv_nsec;
}

int main(int argc, char **argv) {
    const char *cfgpath = NULL;
    int raw = 0;
    long raw_bars = -1;
    long raw_fps = -1;
    RawMode raw_mode = RAW_BARS;
    int have_raw_mode = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "-p")) {
            if (i + 1 < argc)
                cfgpath = argv[++i];
        } else if (!strcmp(argv[i], "--raw")) {
            raw = 1;
        } else if (!strcmp(argv[i], "--bars")) {
            if (i + 1 < argc) {
                char *end;
                long v = strtol(argv[++i], &end, 10);
                if (end != argv[i])
                    raw_bars = v;
            }
        } else if (!strcmp(argv[i], "--fps")) {
            if (i + 1 < argc) {
                char *end;
                long v = strtol(argv[++i], &end, 10);
                if (end != argv[i])
                    raw_fps = v;
            }
        } else if (!strcmp(argv[i], "--raw-mode")) {
            if (i + 1 < argc) {
                RawMode m;
                if (raw_parse_mode(argv[++i], &m)) {
                    raw_mode = m;
                    have_raw_mode = 1;
                }
            }
        } else if (!strcmp(argv[i], "-h") || !strcmp(argv[i], "--help")) {
            usage();
            return 0;
        } else if (!strcmp(argv[i], "-v") || !strcmp(argv[i], "--version")) {
            printf("sharkvis %s\n", SHARKVIS_VERSION);
            return 0;
        } else {
            fprintf(stderr, "sharkvis: unknown option '%s'\n", argv[i]);
            usage();
            return 1;
        }
    }
    (void)have_raw_mode;

    SvConfig cfg;
    config_default(&cfg);

    int g_debug = getenv("SHARKVIS_DEBUG") != NULL;
    FILE *g_dbg = NULL;
    if (g_debug)
        g_dbg = fopen("/tmp/sharkvis_dbg.log", "w");

    char save_path[1024];
    int cfg_dirty = 0;
    int had_file = 0;
    if (cfgpath) {
        snprintf(save_path, sizeof save_path, "%s", cfgpath);
        if (!config_load(&cfg, save_path)) {
            fprintf(stderr, "sharkvis: error loading config %s\n", save_path);
            return 1;
        }
        had_file = 1;
    } else {
        config_default_path(save_path, sizeof save_path);
        struct stat st;
        if (stat(save_path, &st) == 0) {
            had_file = 1;
            if (!config_load(&cfg, save_path))
                fprintf(stderr, "sharkvis: error loading config %s, using defaults\n",
                        save_path);
        }
    }

    clamp_cfg(&cfg);

    if (raw) {
        size_t bars = raw_bars >= 0 ? (size_t)raw_bars : (cfg.bars > 0 ? cfg.bars : 48);
        unsigned fps;
        if (raw_fps >= 0) {
            fps = (unsigned)raw_fps;
            if (fps < 1)
                fps = 1;
            if (fps > 240)
                fps = 240;
        } else {
            fps = cfg.framerate;
            if (fps < 1)
                fps = 1;
            if (fps > 240)
                fps = 240;
        }
        return raw_run(&cfg, bars, fps, raw_mode);
    }

    unsigned rows = 24, cols = 80;
    if (!term_winsize(1, &rows, &cols)) {
        rows = 24;
        cols = 80;
    }

    size_t bars = bar_count_for(cols, &cfg);
    size_t per0 = per_ch_left(bars, cfg.channels);
    size_t per1 = per_ch_right(bars, cfg.channels);

    Dsp *dsp[2];
    dsp[0] = dsp_new(per0, cfg.sample_rate, cfg.autosens, cfg.noise_reduction,
                     cfg.lower_cutoff, cfg.higher_cutoff);
    dsp[1] = dsp_new(per1, cfg.sample_rate, cfg.autosens, cfg.noise_reduction,
                     cfg.lower_cutoff, cfg.higher_cutoff);
    if (!dsp[0] || !dsp[1])
        return 1;
    double fpsf = cfg.framerate > 1 ? (double)cfg.framerate : 1.0;
    dsp_set_display_fps(dsp[0], fpsf);
    dsp_set_display_fps(dsp[1], fpsf);

    Audio *audio = audio_new(dsp_render_frame_size(dsp[0]));
    if (!audio)
        return 1;
    audio_start(audio, cfg.source, cfg.sample_rate, cfg.channels);

    if (!term_raw_enter(0)) {
        fprintf(stderr, "sharkvis: not a terminal\n");
        audio_stop(audio);
        audio_free(audio);
        return 0;
    }

    set_handler(SIGINT, on_signal);
    set_handler(SIGTERM, on_signal);
    set_handler(SIGHUP, on_signal);
    set_handler(SIGWINCH, on_winch);
    set_handler(SIGSEGV, on_fatal);
    set_handler(SIGABRT, on_fatal);
    set_handler(SIGBUS, on_fatal);
    set_handler(SIGFPE, on_fatal);
    set_handler(SIGILL, on_fatal);

    printf("\x1b[2J\x1b[H\x1b[?25l");
    fflush(stdout);

    Renderer *rnd = renderer_new(rows, cols, cfg.bar_width, cfg.bar_spacing, bars);
    apply_colors(rnd, &cfg);
    renderer_set_mode(rnd, mode_parse(cfg.mode[0] ? cfg.mode : "bars"));
    renderer_set_glyphs(rnd, cfg.chars, cfg.chars_len);
    renderer_set_wave(rnd, cfg.sample_rate);
    size_t auto_yscale = yscale_for(1);
    rnd->yscale = auto_yscale;

    double *heights[2] = {calloc(bars, sizeof(double)), calloc(bars, sizeof(double))};
    double *last_h[2] = {calloc(bars, sizeof(double)), calloc(bars, sizeof(double))};
    for (size_t i = 0; i < bars; i++) {
        heights[0][i] = 0.001;
        heights[1][i] = 0.001;
        last_h[0][i] = 0.001;
        last_h[1][i] = 0.001;
    }
    SvBuf out = {0};

    SettingsUi *st = settings_new();
    int in_settings = 0;
    int force_draw = 1;
    unsigned chmask = 0;
    LyricWorker *lyric = lyric_new();
    Track track;
    memset(&track, 0, sizeof track);
    uint64_t last_track_poll = sv_now_ms();
    char *last_lyric_shown = strdup("");
    char *search_buf = NULL;
    char *manual_player = NULL;
    char last_provider[32];
    snprintf(last_provider, sizeof last_provider, "%s", cfg.provider);
    uint64_t last_pos_poll = sv_now_ms();

    uint64_t next = now_ns();
    StateWriter *live = state_writer_new();

    remove_stale_state();

    int rc = 0;
    while (!g_sig) {
        uint64_t t_frame0 = g_debug ? now_ns() : 0;
        size_t last_bytes = 0;
        long t_write_us = -1;
        int drew = 0;

        uint8_t cp[8];
        size_t clen = 0;
        int key = term_read_codepoint(0, cp, &clen);

        if (search_buf) {
            if (key == KEY_ESC) {
                free(search_buf);
                search_buf = NULL;
                force_draw = 1;
            } else if (key == KEY_ENTER) {
                char *orig = search_buf;
                search_buf = NULL;
                char *q = orig;
                while (*q == ' ' || *q == '\t')
                    q++;
                char *e = q + strlen(q);
                while (e > q && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\n'))
                    *--e = 0;
                if (*q) {
                    char *sep = strstr(q, " - ");
                    if (sep) {
                        *sep = 0;
                        lyric_search_override(lyric, q, sep + 3);
                    } else {
                        lyric_search_override(lyric, "", q);
                    }
                    force_draw = 1;
                }
                free(orig);
            } else if (key == KEY_BACKSPACE) {
                size_t l = strlen(search_buf);
                while (l > 0 && (search_buf[l - 1] & 0xC0) == 0x80)
                    l--;
                if (l > 0)
                    l--;
                search_buf[l] = 0;
                force_draw = 1;
            } else if (key == KEY_CHAR && clen > 0) {
                size_t l = strlen(search_buf);
                if (l + clen < 120) {
                    memcpy(search_buf + l, cp, clen);
                    search_buf[l + clen] = 0;
                    force_draw = 1;
                }
            }
        } else if (in_settings) {
            if (is_k(key, cp, clen, 'g') || is_k(key, cp, clen, 'G') || key == KEY_ESC) {
                in_settings = 0;
                printf("\x1b[2J\x1b[3J\x1b[H");
                fflush(stdout);
                apply_settings(dsp, rnd, &audio, &cfg, &bars, heights, last_h,
                               rows, cols, chmask, (chmask & CH_AUDIO) != 0, 0);
                chmask = 0;
                force_draw = 1;
                if (!config_save(&cfg, save_path)) {
                    fprintf(stderr, "sharkvis: could not save config to %s\n", save_path);
                } else {
                    cfg_dirty = 1;
                    if (config_load(&cfg, save_path)) {
                        clamp_cfg(&cfg);
                        apply_settings(dsp, rnd, &audio, &cfg, &bars, heights, last_h,
                                       rows, cols, CH_LAYOUT | CH_DSP | CH_AUDIO, 0, 0);
                    }
                }
            } else if (is_k(key, cp, clen, 'q') || is_k(key, cp, clen, 'Q') || key == 3) {
                break;
            } else {
                unsigned mask_before = chmask;
                settings_key(st, &cfg, key,
                             key == KEY_CHAR ? cp : NULL,
                             key == KEY_CHAR ? clen : 0, &chmask);
                if (chmask != mask_before)
                    cfg_dirty = 1;
                if (chmask & CH_EDITOR) {
                    if (!config_save(&cfg, save_path))
                        fprintf(stderr, "sharkvis: could not save config to %s\n", save_path);
                    run_editor(save_path);
                    if (!config_load(&cfg, save_path))
                        fprintf(stderr, "sharkvis: error loading config %s\n", save_path);
                    clamp_cfg(&cfg);
                    chmask = CH_LAYOUT | CH_DSP | CH_AUDIO;
                }
                if (chmask) {
                    apply_settings(dsp, rnd, &audio, &cfg, &bars, heights, last_h,
                                   rows, cols, chmask, (chmask & CH_AUDIO) != 0,
                                   panel_width_for(cols));
                    printf("\x1b[2J\x1b[3J\x1b[H");
                    fflush(stdout);
                    chmask = 0;
                    force_draw = 1;
                }
            }
        } else {
            if (is_k(key, cp, clen, 'g') || is_k(key, cp, clen, 'G')) {
                in_settings = 1;
                chmask = 0;
                printf("\x1b[2J\x1b[3J\x1b[H");
                fflush(stdout);
                renderer_set_offset(rnd, panel_width_for(cols));
                force_draw = 1;
            } else if (is_k(key, cp, clen, 'q') || is_k(key, cp, clen, 'Q') || key == 3) {
                break;
            } else if (rnd->mode == RM_LYRICS) {
                if (is_k(key, cp, clen, 's') || is_k(key, cp, clen, 'S')) {
                    search_buf = calloc(128, 1);
                    force_draw = 1;
                } else if (is_k(key, cp, clen, 'l') || is_k(key, cp, clen, 'L')) {
                    size_t np = 0;
                    char **players = player_list(&np);
                    if (np > 0) {
                        if (manual_player) {
                            size_t pos = np;
                            for (size_t i = 0; i < np; i++) {
                                if (!strcmp(players[i], manual_player)) {
                                    pos = i;
                                    break;
                                }
                            }
                            free(manual_player);
                            manual_player = NULL;
                            if (pos + 1 < np)
                                manual_player = strdup(players[pos + 1]);
                        } else {
                            manual_player = strdup(players[0]);
                        }
                        force_draw = 1;
                    }
                    player_list_free(players, np);
                } else if (is_k(key, cp, clen, 'r') || is_k(key, cp, clen, 'R')) {
                    lyric_force_reload(lyric);
                    force_draw = 1;
                } else if (is_k(key, cp, clen, 'c') || is_k(key, cp, clen, 'C')) {
                    if (!strcmp(cfg.text_align, "left"))
                        strcpy(cfg.text_align, "center");
                    else
                        strcpy(cfg.text_align, "left");
                    force_draw = 1;
                } else if (is_k(key, cp, clen, 'a') || is_k(key, cp, clen, 'A')) {
                    lyric_set_follow(lyric, !lyric_following(lyric), track.position);
                    force_draw = 1;
                } else if (is_k(key, cp, clen, 'p') || is_k(key, cp, clen, 'P')) {
                    if (!strcmp(cfg.provider, "auto"))
                        strcpy(cfg.provider, "lrclib");
                    else if (!strcmp(cfg.provider, "lrclib"))
                        strcpy(cfg.provider, "musixmatch");
                    else
                        strcpy(cfg.provider, "auto");
                    lyric_poke(lyric);
                    force_draw = 1;
                } else if (is_k(key, cp, clen, '+') || is_k(key, cp, clen, '=')) {
                    cfg.lyric_offset_ms += 500;
                    if (cfg.lyric_offset_ms > 10000)
                        cfg.lyric_offset_ms = 10000;
                    force_draw = 1;
                } else if (is_k(key, cp, clen, '-') || is_k(key, cp, clen, '_')) {
                    cfg.lyric_offset_ms -= 500;
                    if (cfg.lyric_offset_ms < -10000)
                        cfg.lyric_offset_ms = -10000;
                    force_draw = 1;
                } else if (is_k(key, cp, clen, '0')) {
                    cfg.lyric_offset_ms = 0;
                    force_draw = 1;
                }
            }
        }

        if (g_resize) {
            g_resize = 0;
            unsigned nr = 0, nc = 0;
            if (term_winsize(1, &nr, &nc) && nr > 0 && nc > 0 &&
                (nr != rows || nc != cols)) {
                size_t new_bars = bar_count_for(nc, &cfg);
                if (new_bars < 1)
                    new_bars = 1;
                size_t per[2] = {per_ch_left(new_bars, cfg.channels),
                                 per_ch_right(new_bars, cfg.channels)};
                cols = nc;
                rows = nr;
                bars = new_bars;
                for (int ch = 0; ch < 2; ch++) {
                    double ss;
                    int si;
                    dsp_get_sens(dsp[ch], &ss, &si);
                    Dsp *nd = dsp_new(per[ch], cfg.sample_rate, cfg.autosens,
                                      cfg.noise_reduction, cfg.lower_cutoff,
                                      cfg.higher_cutoff);
                    if (nd) {
                        dsp_set_sens(nd, ss, si);
                        dsp_set_display_fps(nd, fpsf);
                        dsp_free(dsp[ch]);
                        dsp[ch] = nd;
                    }
                }
                for (int ch = 0; ch < 2; ch++) {
                    heights[ch] = realloc(heights[ch], bars * sizeof(double));
                    last_h[ch] = realloc(last_h[ch], bars * sizeof(double));
                    for (size_t i = 0; i < bars; i++) {
                        heights[ch][i] = 0.001;
                        last_h[ch][i] = 0.001;
                    }
                }
                renderer_resize(rnd, rows, cols, bars);
                auto_yscale = yscale_for(1);
                rnd->yscale = auto_yscale;
                if (in_settings)
                    renderer_set_offset(rnd, panel_width_for(cols));
                printf("\x1b[2J\x1b[3J\x1b[H");
                fflush(stdout);
                force_draw = 1;
            }
        }

        const double *samples_l = NULL;
        const double *samples_r = NULL;
        size_t n = audio_consume(audio, &samples_l, &samples_r);
        if (n > 0)
            renderer_feed(rnd, samples_l, samples_r, n);

        double disp_fps = cfg.framerate > 1 ? (double)cfg.framerate : 1.0;
        dsp_set_display_fps(dsp[0], disp_fps);
        dsp_set_display_fps(dsp[1], disp_fps);
        dsp_execute(dsp[0], samples_l, n, heights[0]);
        if (cfg.channels > 1)
            dsp_execute(dsp[1], samples_r ? samples_r : samples_l, n, heights[1]);
        static int audio_backoff_ms = 500;
        static uint64_t audio_retry_at = 0;
        if (audio_failed(audio)) {
            uint64_t now_ms = sv_now_ms();
            if ((int64_t)(now_ms - audio_retry_at) >= 0) {
                fprintf(stderr, "\nsharkvis: audio input failed: %s; retrying\n",
                        audio_error(audio));
                Audio *na = audio_new(dsp_render_frame_size(dsp[0]));
                if (na) {
                    audio_stop(audio);
                    audio_free(audio);
                    audio = na;
                    audio_start(audio, cfg.source, cfg.sample_rate, cfg.channels);
                    for (int ch = 0; ch < 2; ch++) {
                        memset(heights[ch], 0, bars * sizeof(double));
                        dsp_flush(dsp[ch]);
                    }
                }
                audio_retry_at = now_ms + (uint64_t)audio_backoff_ms;
                if (audio_backoff_ms < 5000)
                    audio_backoff_ms *= 2;
            }
        } else {
            audio_backoff_ms = 500;
        }

        size_t pcl = per_ch_left(bars, cfg.channels);
        size_t pcr = per_ch_right(bars, cfg.channels);
        dsp_set_sens_scale(dsp[0], cfg.sensitivity / 100.0);
        if (cfg.channels > 1)
            dsp_set_sens_scale(dsp[1], cfg.sensitivity / 100.0);

        {
            size_t nbass = per_ch_left(bars, cfg.channels);
            if (nbass < 2)
                nbass = 2;
            nbass = nbass / 4 + 1;
            double sum = 0, bsum = 0, lsum = 0, rsum = 0;
            size_t cnt = 0, bcnt = 0, lcnt = 0, rcnt = 0;
            for (size_t i = 0; i < pcl; i++) {
                sum += heights[0][i];
                cnt++;
                lsum += heights[0][i];
                lcnt++;
                if (i < nbass) {
                    bsum += heights[0][i];
                    bcnt++;
                }
            }
            if (cfg.channels > 1) {
                size_t nbass_r = pcr < 2 ? 2 : pcr;
                nbass_r = nbass_r / 4 + 1;
                for (size_t i = 0; i < pcr; i++) {
                    sum += heights[1][i];
                    cnt++;
                    rsum += heights[1][i];
                    rcnt++;
                    if (i < nbass_r) {
                        bsum += heights[1][i];
                        bcnt++;
                    }
                }
            }
            double energy = cnt > 0 ? sum / (double)cnt : 0.0;
            double bass = bcnt > 0 ? bsum / (double)bcnt : energy;
            double left = lcnt > 0 ? lsum / (double)lcnt : energy;
            double right = rcnt > 0 ? rsum / (double)rcnt : left;
            {
                static int gate_open = 0;
                double raw = dsp_raw_peak(dsp[0]);
                if (cfg.channels > 1) {
                    double r1 = dsp_raw_peak(dsp[1]);
                    if (r1 > raw)
                        raw = r1;
                }
                if (gate_open) {
                    if (raw < 0.01)
                        gate_open = 0;
                } else if (raw > 0.02) {
                    gate_open = 1;
                }
                if (!gate_open)
                    energy = bass = left = right = 0.0;
            }
            unsigned lr, lg, lb, hr, hg, hb;
            if (!color_to_rgb(cfg.gradient_low, &lr, &lg, &lb)) {
                lr = 255;
                lg = 255;
                lb = 255;
            }
            if (!color_to_rgb(cfg.gradient_high, &hr, &hg, &hb)) {
                hr = 255;
                hg = 255;
                hb = 255;
            }
            state_writer_update(live, energy, bass, left, right,
                                (uint8_t)lr, (uint8_t)lg, (uint8_t)lb,
                                (uint8_t)hr, (uint8_t)hg, (uint8_t)hb);
        }

        int need_lyrics = rnd->mode == RM_LYRICS;
        if (need_lyrics) {
            uint64_t now = sv_now_ms();
            if (now - last_track_poll >= 2000) {
                last_track_poll = now;
                char allow_buf[1024];
                snprintf(allow_buf, sizeof allow_buf, "%s", cfg.mpris_players);
                const char *allow[64];
                size_t nallow = 0;
                char *save = NULL;
                char *tok = strtok_r(allow_buf, ",", &save);
                while (tok && nallow < 64) {
                    while (*tok == ' ' || *tok == '\t')
                        tok++;
                    char *e = tok + strlen(tok);
                    while (e > tok && (e[-1] == ' ' || e[-1] == '\t'))
                        *--e = 0;
                    if (*tok)
                        allow[nallow++] = tok;
                    tok = strtok_r(NULL, ",", &save);
                }
                Track fresh;
                memset(&fresh, 0, sizeof fresh);
                if (manual_player) {
                    fresh = poll_named(manual_player);
                    if (!fresh.present) {
                        free(manual_player);
                        manual_player = NULL;
                        fresh = poll_track(allow, nallow);
                    }
                } else {
                    fresh = poll_track(allow, nallow);
                }
                if (fresh.present) {
                    track = fresh;
                } else {
                    track.present = 0;
                    if (!any_active_player())
                        lyric_reset(lyric);
                }
            }
            now = sv_now_ms();
            if (now - last_pos_poll >= 200) {
                last_pos_poll = now;
                if (track.present && track.player[0]) {
                    double pos;
                    if (poll_position(track.player, &pos)) {
                        track.position = pos;
                        lyric_update_pos(lyric, pos);
                    }
                }
            }
            FetchOpts fo;
            snprintf(fo.local_folder, sizeof fo.local_folder, "%s", cfg.lyrics_folder);
            snprintf(fo.provider, sizeof fo.provider, "%s", cfg.provider);
            lyric_update(lyric, &track, &fo);
        } else {
            last_track_poll = sv_now_ms();
            last_pos_poll = sv_now_ms();
        }
        lyric_set_offset_ms(lyric, cfg.lyric_offset_ms);
        rnd->text_left = !strcmp(cfg.text_align, "left") ? 1 : 0;
        rnd->text_size = cfg.text_size < 5 ? cfg.text_size : 5;
        rnd->yscale = auto_yscale;
        rnd->text_small = !strcmp(cfg.text_style, "normal") ? 1 : 0;
        rnd->loading = need_lyrics && lyric_loading(lyric);
        if (strcmp(cfg.provider, last_provider)) {
            snprintf(last_provider, sizeof last_provider, "%s", cfg.provider);
            lyric_poke(lyric);
            force_draw = 1;
        }
        if (rnd->mode == RM_LYRICS) {
            size_t nd = 0;
            LyricDisplay *rows_d;
            if (!strcmp(cfg.text_style, "normal"))
                rows_d = lyric_display_context(lyric, &track, &nd);
            else
                rows_d = lyric_display_lines(lyric, &track, &nd);
            size_t need = 1;
            for (size_t i = 0; i < nd; i++)
                need += strlen(rows_d[i].text) + 1;
            char *shown = malloc(need);
            shown[0] = 0;
            for (size_t i = 0; i < nd; i++) {
                if (i > 0)
                    strcat(shown, "\n");
                strcat(shown, rows_d[i].text);
            }
            if (strcmp(shown, last_lyric_shown)) {
                free(last_lyric_shown);
                last_lyric_shown = shown;
                force_draw = 1;
            } else {
                free(shown);
            }
            const char **strs = malloc((nd ? nd : 1) * sizeof(char *));
            char *curs = malloc(nd ? nd : 1);
            for (size_t i = 0; i < nd; i++) {
                strs[i] = rows_d[i].text;
                curs[i] = rows_d[i].cur ? 1 : 0;
            }
            renderer_set_rich(rnd, strs, curs, nd);
            free(strs);
            free(curs);
            lyric_display_free(rows_d, nd);
        }

        int need_draw = force_draw || in_settings;
        if (!need_draw) {
            if (rnd->mode == RM_BARS) {
                for (size_t i = 0; i < pcl; i++) {
                    if (heights[0][i] < last_h[0][i] - VIS_EPS ||
                        heights[0][i] > last_h[0][i] + VIS_EPS) {
                        need_draw = 1;
                        break;
                    }
                }
                if (!need_draw && cfg.channels > 1) {
                    for (size_t i = 0; i < pcr; i++) {
                        if (heights[1][i] < last_h[1][i] - VIS_EPS ||
                            heights[1][i] > last_h[1][i] + VIS_EPS) {
                            need_draw = 1;
                            break;
                        }
                    }
                }
            } else {
                need_draw = n > 0;
            }
        }

        if (need_draw) {
            force_draw = 0;
            drew = 1;
            for (size_t i = 0; i < pcl; i++)
                last_h[0][i] = heights[0][i];
            if (cfg.channels > 1) {
                for (size_t i = 0; i < pcr; i++)
                    last_h[1][i] = heights[1][i];
            }

            out.len = 0;
            if (in_settings)
                settings_draw(st, &cfg, &out, OUT_CAP, rows, panel_width_for(cols));
            if (cfg.channels > 1)
                renderer_draw_stereo(rnd, heights[0], heights[1], pcl, pcr, &out, OUT_CAP);
            else
                renderer_draw(rnd, heights[0], &out, OUT_CAP);
            if (search_buf) {
                char line[256];
                int ln = snprintf(line, sizeof line, "\x1b[0m\x1b[%u;1Hsearch: %s_",
                                  rows, search_buf);
                if (out.len + (size_t)ln < OUT_CAP)
                    sv_buf_put(&out, line, (size_t)ln);
            }
            if (out.len > 0) {
                uint64_t t0 = 0;
                if (g_debug)
                    t0 = now_ns();
                fwrite(out.data, 1, out.len, stdout);
                fflush(stdout);
                last_bytes = out.len;
                if (g_debug)
                    t_write_us = (long)((now_ns() - t0) / 1000);
            }
        }

        uint64_t frame_ns = 1000000000u / (cfg.framerate > 1 ? (uint64_t)cfg.framerate : 1);
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

        if (g_debug && g_dbg) {
            uint64_t iter_us = (now_ns() - t_frame0) / 1000;
            fprintf(g_dbg, "iter=%lluus write=%ldus bytes=%zu drew=%d fps=%u\n",
                    (unsigned long long)iter_us, t_write_us, last_bytes, drew,
                    cfg.framerate);
            fflush(g_dbg);
        }
    }

    if (cfg_dirty || !had_file) {
        if (!config_save(&cfg, save_path))
            fprintf(stderr, "sharkvis: could not save config to %s\n", save_path);
    }

    printf("\x1b[?25h\x1b[0m\x1b[2J\x1b[3J\x1b[H");
    fflush(stdout);
    term_raw_restore(0);

    audio_stop(audio);
    audio_free(audio);
    renderer_free(rnd);
    settings_free(st);
    lyric_free(lyric);
    state_writer_free(live);
    sv_buf_free(&out);
    free(heights[0]);
    free(heights[1]);
    free(last_h[0]);
    free(last_h[1]);
    free(last_lyric_shown);
    free(search_buf);
    free(manual_player);
    if (g_dbg)
        fclose(g_dbg);
    dsp_free(dsp[0]);
    dsp_free(dsp[1]);

    clear_state_file();

    return rc;
}
