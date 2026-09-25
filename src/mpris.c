#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#include "common.h"
#include "mpris.h"

void track_key(const Track *t, char *out, size_t outn) {
    out[0] = 0;
    if (!t->present)
        return;
    char at[1024], tt[1024], uu[2048];
    size_t i, n;
    const char *s;
    n = 0;
    s = t->artist;
    while (*s == ' ' || *s == '\t')
        s++;
    for (i = 0; s[i] && n + 1 < sizeof at; i++)
        at[n++] = s[i];
    at[n] = 0;
    while (n > 0 && (at[n - 1] == ' ' || at[n - 1] == '\t'))
        at[--n] = 0;
    n = 0;
    s = t->title;
    while (*s == ' ' || *s == '\t')
        s++;
    for (i = 0; s[i] && n + 1 < sizeof tt; i++)
        tt[n++] = s[i];
    tt[n] = 0;
    while (n > 0 && (tt[n - 1] == ' ' || tt[n - 1] == '\t'))
        tt[--n] = 0;
    if (tt[0]) {
        snprintf(out, outn, "%s|%s", at, tt);
        return;
    }
    n = 0;
    s = t->url;
    while (*s == ' ' || *s == '\t')
        s++;
    for (i = 0; s[i] && n + 1 < sizeof uu; i++)
        uu[n++] = s[i];
    uu[n] = 0;
    while (n > 0 && (uu[n - 1] == ' ' || uu[n - 1] == '\t'))
        uu[--n] = 0;
    if (uu[0])
        snprintf(out, outn, "url|%s", uu);
}

char *cmd_out(const char *cmd, const char *const *args, unsigned timeout_ms) {
    int pipefd[2];
    if (pipe(pipefd) != 0)
        return NULL;
    pid_t pid = fork();
    if (pid < 0) {
        close(pipefd[0]);
        close(pipefd[1]);
        return NULL;
    }
    if (pid == 0) {
        setpgid(0, 0);
        dup2(pipefd[1], STDOUT_FILENO);
        close(pipefd[0]);
        close(pipefd[1]);
        int dn = open("/dev/null", O_RDWR);
        if (dn >= 0) {
            dup2(dn, STDIN_FILENO);
            dup2(dn, STDERR_FILENO);
            if (dn > 2)
                close(dn);
        }
        size_t n = 0;
        while (args[n])
            n++;
        char **argv = malloc((n + 2) * sizeof(char *));
        argv[0] = (char *)cmd;
        for (size_t i = 0; i < n; i++)
            argv[i + 1] = (char *)args[i];
        argv[n + 1] = NULL;
        execvp(cmd, argv);
        _exit(127);
    }
    close(pipefd[1]);
    size_t cap = 4096;
    size_t len = 0;
    char *buf = malloc(cap);
    uint64_t deadline = sv_now_ms() + timeout_ms;
    int killed = 0;
    int eof = 0;
    while (!eof) {
        uint64_t now = sv_now_ms();
        if (now >= deadline) {
            kill(-pid, SIGKILL);
            killed = 1;
            break;
        }
        struct pollfd pfd;
        pfd.fd = pipefd[0];
        pfd.events = POLLIN;
        pfd.revents = 0;
        int pr = poll(&pfd, 1, (int)(deadline - now));
        if (pr < 0) {
            if (errno == EINTR)
                continue;
            break;
        }
        if (pr == 0) {
            kill(-pid, SIGKILL);
            killed = 1;
            break;
        }
        if (len + 4096 > cap && cap < 65536) {
            cap = 65536;
            buf = realloc(buf, cap);
        }
        size_t room = cap - len;
        if (room == 0)
            break;
        if (room > 65536)
            room = 65536;
        ssize_t k = read(pipefd[0], buf + len, room);
        if (k > 0) {
            len += (size_t)k;
        } else if (k == 0) {
            eof = 1;
        } else {
            if (errno == EINTR)
                continue;
            eof = 1;
        }
    }
    close(pipefd[0]);
    int status = 0;
    pid_t got;
    do {
        got = waitpid(pid, &status, WNOHANG);
        if (got == 0) {
            if (!killed && sv_now_ms() >= deadline) {
                kill(-pid, SIGKILL);
                killed = 1;
            }
            struct timespec ts = {0, 5000000};
            nanosleep(&ts, NULL);
        }
    } while (got == 0);
    if (killed) {
        free(buf);
        return NULL;
    }
    if (len >= cap) {
        buf = realloc(buf, len + 1);
        cap = len + 1;
    }
    buf[len] = 0;
    while (len > 0 && (buf[len - 1] == ' ' || buf[len - 1] == '\t' ||
                       buf[len - 1] == '\n' || buf[len - 1] == '\r'))
        buf[--len] = 0;
    return buf;
}

char **player_list(size_t *n) {
    const char *args[] = {"-l", NULL};
    char *list = cmd_out("playerctl", args, 500);
    char **out = NULL;
    size_t m = 0;
    *n = 0;
    if (!list)
        return NULL;
    char *save = NULL;
    char *line = strtok_r(list, "\n", &save);
    while (line) {
        while (*line == ' ' || *line == '\t')
            line++;
        char *e = line + strlen(line);
        while (e > line && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\r'))
            *--e = 0;
        if (*line) {
            out = realloc(out, (m + 1) * sizeof(char *));
            out[m++] = strdup(line);
        }
        line = strtok_r(NULL, "\n", &save);
    }
    free(list);
    *n = m;
    return out;
}

void player_list_free(char **p, size_t n) {
    if (!p)
        return;
    for (size_t i = 0; i < n; i++)
        free(p[i]);
    free(p);
}

int any_active_player(void) {
    size_t n = 0;
    char **players = player_list(&n);
    int found = 0;
    for (size_t i = 0; i < n && !found; i++) {
        const char *args[] = {"-p", players[i], "status", NULL};
        char *st = cmd_out("playerctl", args, 500);
        if (st) {
            if (!strcasecmp(st, "playing") || !strcasecmp(st, "paused"))
                found = 1;
            free(st);
        }
    }
    player_list_free(players, n);
    return found;
}

static int poll_position_into(const char *player, double *pos) {
    if (!player || !*player)
        return 0;
    const char *args[] = {"-p", player, "position", NULL};
    char *s = cmd_out("playerctl", args, 300);
    if (!s)
        return 0;
    char *end;
    double v = strtod(s, &end);
    free(s);
    if (end == s)
        return 0;
    *pos = v;
    return 1;
}

int poll_position(const char *player, double *pos) {
    return poll_position_into(player, pos);
}

static void fill_meta(Track *t, const char *player) {
    memset(t, 0, sizeof *t);
    snprintf(t->player, sizeof t->player, "%s", player);
    const char *args[] = {"-p", player, "metadata", "--format",
                          "{{artist}}|{{title}}|{{mpris:length}}|{{xesam:url}}", NULL};
    char *meta = cmd_out("playerctl", args, 500);
    if (!meta)
        meta = strdup("");
    char *parts[4] = {"", "", "", ""};
    char *p = meta;
    for (int i = 0; i < 3; i++) {
        parts[i] = p;
        char *bar = strchr(p, '|');
        if (bar) {
            *bar = 0;
            p = bar + 1;
        } else {
            p += strlen(p);
        }
    }
    parts[3] = p;
    for (int i = 0; i < 4; i++) {
        while (*parts[i] == ' ' || *parts[i] == '\t')
            parts[i]++;
        char *e = parts[i] + strlen(parts[i]);
        while (e > parts[i] && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\r'))
            *--e = 0;
    }
    snprintf(t->artist, sizeof t->artist, "%s", parts[0]);
    snprintf(t->title, sizeof t->title, "%s", parts[1]);
    {
        char *end;
        double us = strtod(parts[2], &end);
        t->duration = end != parts[2] ? us / 1000000.0 : 0.0;
    }
    snprintf(t->url, sizeof t->url, "%s", parts[3]);
    free(meta);
    if (!t->title[0] && !t->url[0])
        return;
    double pos = 0;
    if (poll_position_into(t->player, &pos))
        t->position = pos;
    t->present = 1;
}

Track poll_track(const char *const *allow, size_t nallow) {
    Track t;
    memset(&t, 0, sizeof t);
    const char *largs[] = {"-l", NULL};
    char *list = cmd_out("playerctl", largs, 500);
    if (!list)
        return t;
    char *names[64];
    size_t nn = 0;
    char *save = NULL;
    char *line = strtok_r(list, "\n", &save);
    while (line && nn < 64) {
        while (*line == ' ' || *line == '\t')
            line++;
        char *e = line + strlen(line);
        while (e > line && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\r'))
            *--e = 0;
        if (*line)
            names[nn++] = line;
        line = strtok_r(NULL, "\n", &save);
    }
    for (size_t i = 0; i < nn; i++) {
        for (size_t j = i + 1; j < nn; j++) {
            if (!strcmp(names[i], "playerctld")) {
                char *tmp = names[i];
                names[i] = names[j];
                names[j] = tmp;
                break;
            }
        }
    }
    for (size_t i = 0; i < nn; i++) {
        const char *pl = names[i];
        if (nallow > 0) {
            int ok = 0;
            for (size_t a = 0; a < nallow; a++) {
                if (!strcmp(pl, allow[a]) || !strncmp(pl, allow[a], strlen(allow[a]))) {
                    ok = 1;
                    break;
                }
            }
            if (!ok)
                continue;
        }
        const char *sargs[] = {"-p", pl, "status", NULL};
        char *st = cmd_out("playerctl", sargs, 500);
        int playing = st && !strcasecmp(st, "playing");
        free(st);
        if (playing) {
            fill_meta(&t, pl);
            break;
        }
    }
    free(list);
    return t;
}

Track poll_named(const char *player) {
    Track t;
    memset(&t, 0, sizeof t);
    if (!player)
        return t;
    while (*player == ' ' || *player == '\t')
        player++;
    if (!*player)
        return t;
    fill_meta(&t, player);
    return t;
}
