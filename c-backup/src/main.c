#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#include "audio.h"
#include "config.h"
#include "dsp.h"
#include "render.h"
#include "settings.h"
#include "term.h"

#define VIS_EPS 0.001

#define CLEAR_ESC "\x1b[2J\x1b[3J\x1b[H"

static volatile sig_atomic_t g_sig = 0;
static volatile sig_atomic_t g_resize = 0;

static bool g_debug = false;
static FILE *g_dbg = NULL;

static void on_signal(int sig) {
    (void)sig;
    g_sig = 1;
}

static void on_winch(int sig) {
    (void)sig;
    g_resize = 1;
}

static void on_fatal(int sig) {
    static const char restore[] = "\x1b[?25h\x1b[0m\x1b[2J\x1b[H";
    ssize_t r = write(1, restore, sizeof restore - 1);
    (void)r;
    term_raw_restore(0);
    _exit(128 + sig);
}

static void usage(void) {
    printf("usage: sharkvis [-p config_file]\n");
    printf("  g - settings, q - quit\n");
}

static void print_version(void) {
#ifndef VERSION
#define VERSION "unknown"
#endif
    printf("sharkvis %s\n", VERSION);
}

static int panel_width_for(unsigned cols) {
    unsigned pw = cols / 3;
    if (pw < 28)
        pw = 28;
    if (pw > 44)
        pw = 44;
    if (pw >= cols)
        pw = cols > 2 ? cols / 2 : 1;
    if (pw < 1)
        pw = 1;
    return (int)pw;
}

static size_t bar_count_for(unsigned cols, const srk_config *cfg) {
    size_t step = cfg->bar_width + cfg->bar_spacing;
    size_t avail = step ? cols / step : cols;
    size_t b = cfg->bars ? cfg->bars : avail;
    return b < 1 ? 1 : b;
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

static bool is_k(int key, const char *cp, int ch) {
    if (key == ch)
        return true;
    if (key == KEY_CHAR && cp && cp[0] == (char)ch)
        return true;
    return false;
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
        while (waitpid(pid, &status, 0) < 0 && errno == EINTR)
            ;
        if (WIFEXITED(status) && WEXITSTATUS(status) == 127)
            fprintf(stderr, "sharkvis: could not launch nano\n");
    }
    signal(SIGINT, old_int);

    term_raw_enter(0);
    printf("\x1b[2J\x1b[H\x1b[?25l");
    fflush(stdout);
}

static void apply_colors(renderer_t *rnd, const srk_config *cfg) {
    unsigned r, g, b;
    if (color_to_rgb(cfg->gradient_low, &r, &g, &b) == 0)
        rnd->grad_lo = (r << 16) | (g << 8) | b;
    if (color_to_rgb(cfg->gradient_high, &r, &g, &b) == 0)
        rnd->grad_hi = (r << 16) | (g << 8) | b;
}

static void apply_settings(dsp_t dsp[2], renderer_t *rnd, audio_t *audio,
                           srk_config *cfg, size_t *bars, double *heights[2],
                           double *last_h[2], unsigned rows, unsigned cols,
                           unsigned chmask, bool audio_reinit, size_t x_off) {
    size_t new_bars = bar_count_for(cols, cfg);
    size_t pcl = per_ch_left(new_bars, cfg->channels);
    size_t pcr = per_ch_right(new_bars, cfg->channels);

    if ((chmask & (CH_DSP | CH_AUDIO)) || new_bars != *bars) {
        size_t per[2] = { pcl, pcr };
        for (int ch = 0; ch < 2; ch++) {
            double saved_sens = dsp[ch].sens;
            bool saved_sens_init = dsp[ch].sens_init;
            dsp_free(&dsp[ch]);
            dsp_init(&dsp[ch], per[ch], cfg->sample_rate, cfg->autosens,
                     cfg->noise_reduction, cfg->lower_cutoff, cfg->higher_cutoff);
            dsp[ch].sens = saved_sens;
            dsp[ch].sens_init = saved_sens_init;
        }
    }

    if (new_bars != *bars) {
        free(heights[0]);
        free(heights[1]);
        free(last_h[0]);
        free(last_h[1]);
        heights[0] = calloc(new_bars, sizeof **heights);
        heights[1] = calloc(new_bars, sizeof **heights);
        last_h[0] = calloc(new_bars, sizeof **last_h);
        last_h[1] = calloc(new_bars, sizeof **last_h);
        *bars = new_bars;
        renderer_resize(rnd, rows, cols, new_bars);
    }

    rnd->bar_width = cfg->bar_width;
    rnd->bar_spacing = cfg->bar_spacing;
    rnd->color_256 = cfg->color_256;
    apply_colors(rnd, cfg);
    renderer_set_mode(rnd, renderer_mode_parse(cfg->mode ? cfg->mode : "bars"));
    renderer_set_glyphs(rnd, cfg->glyphs);
    renderer_set_wave(rnd, cfg->sample_rate);
    renderer_set_offset(rnd, x_off);
    renderer_clear(rnd);

    if (chmask) {
        memset(heights[0], 0, *bars * sizeof **heights);
        memset(heights[1], 0, *bars * sizeof **heights);
    }

    if (audio_reinit) {
        audio_stop(audio);
        audio_init(audio, dsp[0].input_buffer_size);
        audio_start(audio, cfg->source, cfg->sample_rate, cfg->channels);
    }
}

int main(int argc, char **argv) {
    const char *cfgpath = NULL;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-p") == 0 && i + 1 < argc) {
            cfgpath = argv[++i];
        } else if (strcmp(argv[i], "-h") == 0 || strcmp(argv[i], "--help") == 0) {
            usage();
            return 0;
        } else if (strcmp(argv[i], "-v") == 0 || strcmp(argv[i], "--version") == 0) {
            print_version();
            return 0;
        } else {
            fprintf(stderr, "sharkvis: unknown option '%s'\n", argv[i]);
            usage();
            return 1;
        }
    }

    srk_config cfg;
    config_default(&cfg);

    if (getenv("SHARKVIS_DEBUG")) {
        g_debug = true;
        g_dbg = fopen("/tmp/sharkvis_dbg.log", "w");
    }

    char *save_path = NULL;
    if (cfgpath) {
        save_path = strdup(cfgpath);
        if (!config_load(&cfg, save_path)) {
            fprintf(stderr, "sharkvis: error loading config %s\n", save_path);
            free(save_path);
            config_free(&cfg);
            return 1;
        }
    } else {
        save_path = config_default_path();
        if (access(save_path, F_OK) == 0 && !config_load(&cfg, save_path))
            fprintf(stderr, "sharkvis: error loading config %s, using defaults\n",
                    save_path);
    }

    if (cfg.bar_width < 1)
        cfg.bar_width = 1;
    if (cfg.framerate < 1)
        cfg.framerate = 1;
    if (cfg.framerate > 240)
        cfg.framerate = 240;
    if (cfg.sensitivity < 0.1)
        cfg.sensitivity = 0.1;
    if (cfg.noise_reduction < 0.0)
        cfg.noise_reduction = 0.0;
    if (cfg.noise_reduction > 1.0)
        cfg.noise_reduction = 1.0;
    if (cfg.lower_cutoff < 1)
        cfg.lower_cutoff = 1;
    if (cfg.higher_cutoff < cfg.lower_cutoff)
        cfg.higher_cutoff = cfg.lower_cutoff + 1;
    if (cfg.channels < 1)
        cfg.channels = 1;
    if (cfg.channels > 2)
        cfg.channels = 2;

    unsigned rows, cols;
    if (!term_winsize(1, &rows, &cols)) {
        rows = 24;
        cols = 80;
    }

    size_t bars = bar_count_for(cols, &cfg);
    size_t per[2] = { per_ch_left(bars, cfg.channels),
                      per_ch_right(bars, cfg.channels) };

    dsp_t dsp[2];
    for (int ch = 0; ch < 2; ch++)
        dsp_init(&dsp[ch], per[ch], cfg.sample_rate, cfg.autosens,
                 cfg.noise_reduction, cfg.lower_cutoff, cfg.higher_cutoff);

    audio_t audio;
    audio_init(&audio, dsp[0].input_buffer_size);
    audio_start(&audio, cfg.source, cfg.sample_rate, cfg.channels);

    if (!term_raw_enter(0)) {
        fprintf(stderr, "sharkvis: not a terminal\n");
        audio_stop(&audio);
        for (int ch = 0; ch < 2; ch++)
            dsp_free(&dsp[ch]);
        config_free(&cfg);
        return 1;
    }

    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_signal;
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
    struct sigaction win;
    memset(&win, 0, sizeof win);
    win.sa_handler = on_winch;
    sigaction(SIGWINCH, &win, NULL);
    struct sigaction fatal;
    memset(&fatal, 0, sizeof fatal);
    fatal.sa_handler = on_fatal;
    sigaction(SIGSEGV, &fatal, NULL);
    sigaction(SIGABRT, &fatal, NULL);
    sigaction(SIGBUS, &fatal, NULL);
    sigaction(SIGFPE, &fatal, NULL);
    sigaction(SIGILL, &fatal, NULL);

    printf("\x1b[2J\x1b[H\x1b[?25l");
    fflush(stdout);

    renderer_t rnd;
    renderer_init(&rnd, rows, cols, cfg.bar_width, cfg.bar_spacing, bars);
    apply_colors(&rnd, &cfg);
    renderer_set_mode(&rnd, renderer_mode_parse(cfg.mode ? cfg.mode : "bars"));
    renderer_set_glyphs(&rnd, cfg.glyphs);
    renderer_set_wave(&rnd, cfg.sample_rate);

    double *heights[2];
    double *last_h[2];
    heights[0] = malloc(bars * sizeof *heights[0]);
    heights[1] = malloc(bars * sizeof *heights[1]);
    last_h[0] = malloc(bars * sizeof *last_h[0]);
    last_h[1] = malloc(bars * sizeof *last_h[1]);
    for (size_t i = 0; i < bars; i++) {
        heights[0][i] = 0.001;
        heights[1][i] = 0.001;
        last_h[0][i] = 0.001;
        last_h[1][i] = 0.001;
    }
    char *out = malloc((size_t)1 << 20);

    settings_ui *st = settings_new();
    bool in_settings = false;
    bool force_draw = true;
    unsigned chmask = 0;

    struct timespec next;
    clock_gettime(CLOCK_MONOTONIC, &next);

    int rc = 0;
    while (!g_sig) {
        struct timespec t_frame0;
        size_t last_bytes = 0;
        long t_write_us = -1;
        bool drew = false;
        if (g_debug)
            clock_gettime(CLOCK_MONOTONIC, &t_frame0);
        char cp[8];
        int key = term_read_codepoint(0, cp, sizeof cp);

        if (in_settings) {
            if (is_k(key, cp, 'g') || is_k(key, cp, 'G') || key == KEY_ESC) {
                in_settings = false;
                printf(CLEAR_ESC);
                fflush(stdout);
                apply_settings(dsp, &rnd, &audio, &cfg, &bars, heights, last_h,
                               rows, cols, chmask, !!(chmask & CH_AUDIO), 0);
                chmask = 0;
                force_draw = true;
                if (!config_save(&cfg, save_path))
                    fprintf(stderr, "sharkvis: could not save config to %s\n",
                            save_path);
            } else if (is_k(key, cp, 'q') || is_k(key, cp, 'Q') || key == 3) {
                break;
            } else {
                settings_key(st, &cfg, key, key == KEY_CHAR ? cp : NULL,
                             &chmask);
                if (chmask & CH_EDITOR) {
                    chmask = 0;
                    if (!config_save(&cfg, save_path))
                        fprintf(stderr,
                                "sharkvis: could not save config to %s\n",
                                save_path);
                    run_editor(save_path);
                    if (!config_load(&cfg, save_path))
                        fprintf(stderr,
                                "sharkvis: error loading config %s\n",
                                save_path);
                    if (cfg.bar_width < 1)
                        cfg.bar_width = 1;
                    if (cfg.framerate < 1)
                        cfg.framerate = 1;
                    if (cfg.framerate > 240)
                        cfg.framerate = 240;
                    if (cfg.sensitivity < 0.1)
                        cfg.sensitivity = 0.1;
                    if (cfg.noise_reduction < 0.0)
                        cfg.noise_reduction = 0.0;
                    if (cfg.noise_reduction > 1.0)
                        cfg.noise_reduction = 1.0;
                    if (cfg.lower_cutoff < 1)
                        cfg.lower_cutoff = 1;
                    if (cfg.higher_cutoff < cfg.lower_cutoff)
                        cfg.higher_cutoff = cfg.lower_cutoff + 1;
                    if (cfg.channels < 1)
                        cfg.channels = 1;
                    if (cfg.channels > 2)
                        cfg.channels = 2;
                    chmask = CH_LAYOUT | CH_DSP | CH_AUDIO;
                }
                if (chmask) {
                    apply_settings(dsp, &rnd, &audio, &cfg, &bars, heights,
                                   last_h, rows, cols, chmask,
                                   !!(chmask & CH_AUDIO),
                                   (size_t)panel_width_for(cols));
                    printf(CLEAR_ESC);
                    fflush(stdout);
                    chmask = 0;
                    force_draw = true;
                }
            }
        } else {
            if (is_k(key, cp, 'g') || is_k(key, cp, 'G')) {
                in_settings = true;
                chmask = 0;
                printf(CLEAR_ESC);
                fflush(stdout);
                renderer_set_offset(&rnd, (size_t)panel_width_for(cols));
                force_draw = true;
            } else if (is_k(key, cp, 'q') || is_k(key, cp, 'Q') || key == 3) {
                break;
            }
        }

        if (g_resize) {
            g_resize = 0;
            unsigned nr, nc;
            if (term_winsize(1, &nr, &nc) && nr > 0 && nc > 0 &&
                (nr != rows || nc != cols)) {
                size_t new_bars = bar_count_for(nc, &cfg);
                if (new_bars < 1)
                    new_bars = 1;
                size_t per[2] = { per_ch_left(new_bars, cfg.channels),
                                  per_ch_right(new_bars, cfg.channels) };
                cols = nc;
                rows = nr;
                bars = new_bars;
                for (int ch = 0; ch < 2; ch++) {
                    double saved_sens = dsp[ch].sens;
                    bool saved_sens_init = dsp[ch].sens_init;
                    dsp_free(&dsp[ch]);
                    dsp_init(&dsp[ch], per[ch], cfg.sample_rate, cfg.autosens,
                             cfg.noise_reduction, cfg.lower_cutoff, cfg.higher_cutoff);
                    dsp[ch].sens = saved_sens;
                    dsp[ch].sens_init = saved_sens_init;
                }
                free(heights[0]);
                free(heights[1]);
                free(last_h[0]);
                free(last_h[1]);
                heights[0] = malloc(bars * sizeof *heights[0]);
                heights[1] = malloc(bars * sizeof *heights[1]);
                last_h[0] = malloc(bars * sizeof *last_h[0]);
                last_h[1] = malloc(bars * sizeof *last_h[1]);
                renderer_resize(&rnd, rows, cols, bars);
                if (in_settings)
                    renderer_set_offset(&rnd, (size_t)panel_width_for(cols));
                printf(CLEAR_ESC);
                fflush(stdout);
                force_draw = true;
            }
        }

        const double *samples_l = NULL, *samples_r = NULL;
        size_t n = audio_consume(&audio, &samples_l, &samples_r);
        if (n > 0) {
            renderer_feed(&rnd, samples_l, samples_r, n);
        }
        dsp_execute(&dsp[0], samples_l, n, heights[0]);
        if (cfg.channels > 1)
            dsp_execute(&dsp[1], samples_r ? samples_r : samples_l, n, heights[1]);
        if (audio_failed(&audio)) {
            fprintf(stderr, "\nsharkvis: audio input failed: %s\n", audio_error(&audio));
            rc = 1;
            break;
        }

        size_t pcl = per_ch_left(bars, cfg.channels);
        size_t pcr = per_ch_right(bars, cfg.channels);
        dsp[0].sens_scale = cfg.sensitivity / 100.0;
        if (cfg.channels > 1)
            dsp[1].sens_scale = cfg.sensitivity / 100.0;

        bool need_draw = force_draw || in_settings;
        if (!need_draw) {
            if (rnd.mode == RENDER_BARS) {
                for (size_t i = 0; i < pcl; i++) {
                    if (heights[0][i] < last_h[0][i] - VIS_EPS ||
                        heights[0][i] > last_h[0][i] + VIS_EPS) {
                        need_draw = true;
                        break;
                    }
                }
                if (!need_draw && cfg.channels > 1)
                    for (size_t i = 0; i < pcr; i++) {
                        if (heights[1][i] < last_h[1][i] - VIS_EPS ||
                            heights[1][i] > last_h[1][i] + VIS_EPS) {
                            need_draw = true;
                            break;
                        }
                    }
            } else {
                need_draw = n > 0;
            }
        }

        if (need_draw) {
            force_draw = false;
            drew = true;
            memcpy(last_h[0], heights[0], pcl * sizeof *last_h[0]);
            if (cfg.channels > 1)
                memcpy(last_h[1], heights[1], pcr * sizeof *last_h[1]);

            size_t olen = 0;
            if (in_settings)
                settings_draw(st, &cfg, out, &olen, (size_t)1 << 20, rows,
                              panel_width_for(cols));
            if (cfg.channels > 1)
                renderer_draw_stereo(&rnd, heights[0], heights[1], pcl, out,
                                     &olen, (size_t)1 << 20);
            else
                renderer_draw(&rnd, heights[0], out, &olen, (size_t)1 << 20);
            if (olen) {
                struct timespec t_write;
                if (g_debug)
                    clock_gettime(CLOCK_MONOTONIC, &t_write);
                fwrite(out, 1, olen, stdout);
                fflush(stdout);
                last_bytes = olen;
                if (g_debug) {
                    struct timespec t_after;
                    clock_gettime(CLOCK_MONOTONIC, &t_after);
                    t_write_us = (t_after.tv_sec - t_write.tv_sec) * 1000000L +
                                 (t_after.tv_nsec - t_write.tv_nsec) / 1000L;
                }
            }
        }

        long frame_ns = (long)(1e9 / (cfg.framerate ? cfg.framerate : 1));
        struct timespec now;
        clock_gettime(CLOCK_MONOTONIC, &now);
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &next, NULL);
        next.tv_nsec += frame_ns;
        next.tv_sec += next.tv_nsec / 1000000000L;
        next.tv_nsec %= 1000000000L;
        if (g_debug && g_dbg) {
            long iter_us = (now.tv_sec - t_frame0.tv_sec) * 1000000L +
                           (now.tv_nsec - t_frame0.tv_nsec) / 1000L;
            fprintf(g_dbg,
                    "iter=%ldus write=%ldus bytes=%zu drew=%d fps=%u\n",
                    iter_us, t_write_us, last_bytes, drew,
                    cfg.framerate);
            fflush(g_dbg);
        }
    }

    if (!config_save(&cfg, save_path))
        fprintf(stderr, "sharkvis: could not save config to %s\n", save_path);

    printf("\x1b[?25h\x1b[0m" CLEAR_ESC);
    fflush(stdout);
    term_raw_restore(0);

    audio_stop(&audio);
    renderer_free(&rnd);
    for (int ch = 0; ch < 2; ch++)
        dsp_free(&dsp[ch]);
    settings_free(st);
    free(heights[0]);
    free(heights[1]);
    free(last_h[0]);
    free(last_h[1]);
    free(out);
    free(save_path);
    config_free(&cfg);
    if (g_dbg)
        fclose(g_dbg);
    return rc;
}
