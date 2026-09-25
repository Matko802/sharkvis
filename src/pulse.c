#include <errno.h>
#include <poll.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

#include "pulse.h"

#define PA_PROTOCOL_VERSION 35u
#define PA_INVALID_INDEX 0xFFFFFFFFu

#define TAG_STRING 't'
#define TAG_STRING_NULL 'N'
#define TAG_U32 'L'
#define TAG_U8 'B'
#define TAG_SAMPLE_SPEC 'a'
#define TAG_ARBITRARY 'x'
#define TAG_BOOL_TRUE '1'
#define TAG_BOOL_FALSE '0'
#define TAG_CHANNEL_MAP 'm'
#define TAG_CVOLUME 'v'
#define TAG_PROPLIST 'P'

#define CMD_ERROR 0u
#define CMD_REPLY 2u
#define CMD_CREATE_RECORD_STREAM 5u
#define CMD_AUTH 8u
#define CMD_SET_CLIENT_NAME 9u
#define CMD_GET_SINK_INFO 21u
#define CMD_RECORD_STREAM_KILLED 65u

#define COOKIE_LEN 256
#define PA_SAMPLE_S16NE 3
#define PA_VOLUME_NORM 0x10000u

typedef struct {
    uint8_t *data;
    size_t len;
    size_t cap;
} Writer;

typedef struct {
    const uint8_t *data;
    size_t len;
    size_t pos;
    int failed;
} Reader;

struct Pulse {
    int fd;
    uint32_t tag;
};

struct Record {
    int fd;
    uint32_t stream_index;
    uint8_t *pending;
    size_t plen;
    size_t off;
};

static void w_reserve(Writer *w, size_t extra) {
    if (w->len + extra <= w->cap)
        return;
    size_t cap = w->cap ? w->cap : 256;
    while (cap < w->len + extra)
        cap *= 2;
    uint8_t *p = realloc(w->data, cap);
    if (!p)
        abort();
    w->data = p;
    w->cap = cap;
}

static void w_byte(Writer *w, uint8_t b) {
    w_reserve(w, 1);
    w->data[w->len++] = b;
}

static void w_raw(Writer *w, const void *p, size_t n) {
    w_reserve(w, n);
    memcpy(w->data + w->len, p, n);
    w->len += n;
}

static void w_tag(Writer *w, uint8_t t) {
    w_byte(w, t);
}

static void w_u32(Writer *w, uint32_t v) {
    uint8_t b[4];
    b[0] = (uint8_t)(v >> 24);
    b[1] = (uint8_t)(v >> 16);
    b[2] = (uint8_t)(v >> 8);
    b[3] = (uint8_t)v;
    w_tag(w, TAG_U32);
    w_raw(w, b, 4);
}

static void w_u8(Writer *w, uint8_t v) {
    w_tag(w, TAG_U8);
    w_byte(w, v);
}

static void w_string(Writer *w, const char *s) {
    w_tag(w, TAG_STRING);
    w_raw(w, s, strlen(s) + 1);
}

static void w_string_null(Writer *w) {
    w_tag(w, TAG_STRING_NULL);
}

static void w_arbitrary(Writer *w, const uint8_t *d, size_t n) {
    uint8_t b[4];
    b[0] = (uint8_t)(n >> 24);
    b[1] = (uint8_t)(n >> 16);
    b[2] = (uint8_t)(n >> 8);
    b[3] = (uint8_t)n;
    w_tag(w, TAG_ARBITRARY);
    w_raw(w, b, 4);
    w_raw(w, d, n);
}

static void w_sample_spec(Writer *w, uint8_t fmt, uint8_t ch, uint32_t rate) {
    uint8_t b[4];
    b[0] = (uint8_t)(rate >> 24);
    b[1] = (uint8_t)(rate >> 16);
    b[2] = (uint8_t)(rate >> 8);
    b[3] = (uint8_t)rate;
    w_tag(w, TAG_SAMPLE_SPEC);
    w_byte(w, fmt);
    w_byte(w, ch);
    w_raw(w, b, 4);
}

static void w_channel_map(Writer *w, const uint8_t *map, size_t n) {
    w_tag(w, TAG_CHANNEL_MAP);
    w_byte(w, (uint8_t)n);
    w_raw(w, map, n);
}

static void w_cvolume(Writer *w, uint8_t ch, uint32_t val) {
    uint8_t b[4];
    b[0] = (uint8_t)(val >> 24);
    b[1] = (uint8_t)(val >> 16);
    b[2] = (uint8_t)(val >> 8);
    b[3] = (uint8_t)val;
    w_tag(w, TAG_CVOLUME);
    w_byte(w, ch);
    for (unsigned i = 0; i < ch; i++)
        w_raw(w, b, 4);
}

static void w_proplist(Writer *w, const char *const *keys,
                       const uint8_t *const *vals, const size_t *lens, size_t n) {
    w_tag(w, TAG_PROPLIST);
    for (size_t i = 0; i < n; i++) {
        w_string(w, keys[i]);
        w_u32(w, (uint32_t)lens[i]);
        w_arbitrary(w, vals[i], lens[i]);
    }
    w_string_null(w);
}

static void w_boolean(Writer *w, int b) {
    w_tag(w, b ? TAG_BOOL_TRUE : TAG_BOOL_FALSE);
}

static uint8_t r_byte(Reader *r) {
    if (r->pos >= r->len) {
        r->failed = 1;
        return 0;
    }
    return r->data[r->pos++];
}

static int r_tag(Reader *r, uint8_t t) {
    if (r_byte(r) != t) {
        r->failed = 1;
        return 0;
    }
    return 1;
}

static int r_boolean(Reader *r, int *out) {
    uint8_t b = r_byte(r);
    if (b == TAG_BOOL_TRUE) {
        *out = 1;
        return 1;
    }
    if (b == TAG_BOOL_FALSE) {
        *out = 0;
        return 1;
    }
    r->failed = 1;
    return 0;
}

static const uint8_t *r_take(Reader *r, size_t n) {
    if (r->pos + n > r->len) {
        r->failed = 1;
        return NULL;
    }
    const uint8_t *p = r->data + r->pos;
    r->pos += n;
    return p;
}

static uint32_t r_u32(Reader *r) {
    if (!r_tag(r, TAG_U32))
        return 0;
    const uint8_t *b = r_take(r, 4);
    if (!b)
        return 0;
    return ((uint32_t)b[0] << 24) | ((uint32_t)b[1] << 16) | ((uint32_t)b[2] << 8) | b[3];
}

static int r_string(Reader *r, char *out, size_t outn) {
    if (!r_tag(r, TAG_STRING))
        return 0;
    size_t start = r->pos;
    while (r->pos < r->len && r->data[r->pos] != 0)
        r->pos++;
    if (r->pos >= r->len) {
        r->failed = 1;
        return 0;
    }
    size_t n = r->pos - start;
    if (n >= outn)
        n = outn - 1;
    memcpy(out, r->data + start, n);
    out[n] = 0;
    r->pos++;
    return 1;
}

static int r_skip_sample_spec(Reader *r) {
    if (!r_tag(r, TAG_SAMPLE_SPEC))
        return 0;
    return r_take(r, 6) != NULL;
}

static int r_skip_channel_map(Reader *r) {
    if (!r_tag(r, TAG_CHANNEL_MAP))
        return 0;
    size_t n = r_byte(r);
    return r_take(r, n) != NULL;
}

static int r_skip_cvolume(Reader *r) {
    if (!r_tag(r, TAG_CVOLUME))
        return 0;
    size_t n = r_byte(r);
    return r_take(r, 4 * n) != NULL;
}

static ssize_t read_interruptible(int fd, uint8_t *out, size_t n,
                                  const volatile int *stop, char *err, size_t errn) {
    size_t got = 0;
    struct pollfd pfd;
    pfd.fd = fd;
    pfd.events = POLLIN;
    while (got < n) {
        if (stop && *stop)
            break;
        pfd.revents = 0;
        int r;
        do {
            r = poll(&pfd, 1, 50);
        } while (r < 0 && errno == EINTR);
        if (r < 0) {
            snprintf(err, errn, "pulse: poll: %s", strerror(errno));
            return -1;
        }
        if (stop && *stop)
            break;
        ssize_t k = read(fd, out + got, n - got);
        if (k == 0) {
            snprintf(err, errn, "pulse: server closed the connection");
            return -1;
        }
        if (k < 0) {
            if (errno == EAGAIN || errno == EINTR)
                continue;
            snprintf(err, errn, "pulse: read: %s", strerror(errno));
            return -1;
        }
        got += (size_t)k;
    }
    return (ssize_t)got;
}

static void load_cookie(uint8_t *cookie) {
    memset(cookie, 0, COOKIE_LEN);
    char paths[3][512];
    int n = 0;
    const char *rt = getenv("XDG_RUNTIME_DIR");
    if (rt && n < 3)
        snprintf(paths[n++], sizeof paths[0], "%s/pulse/cookie", rt);
    const char *home = getenv("HOME");
    if (home) {
        if (n < 3)
            snprintf(paths[n++], sizeof paths[0], "%s/.config/pulse/cookie", home);
        if (n < 3)
            snprintf(paths[n++], sizeof paths[0], "%s/.pulse-cookie", home);
    }
    for (int i = 0; i < n; i++) {
        FILE *f = fopen(paths[i], "r");
        if (f) {
            fread(cookie, 1, COOKIE_LEN, f);
            fclose(f);
            break;
        }
    }
}

static int send_msg(Pulse *p, uint32_t cmd, uint32_t tag, Writer *body,
                    char *err, size_t errn) {
    Writer pay = {0};
    w_tag(&pay, TAG_U32);
    uint8_t b[4];
    b[0] = (uint8_t)(cmd >> 24);
    b[1] = (uint8_t)(cmd >> 16);
    b[2] = (uint8_t)(cmd >> 8);
    b[3] = (uint8_t)cmd;
    w_raw(&pay, b, 4);
    w_tag(&pay, TAG_U32);
    b[0] = (uint8_t)(tag >> 24);
    b[1] = (uint8_t)(tag >> 16);
    b[2] = (uint8_t)(tag >> 8);
    b[3] = (uint8_t)tag;
    w_raw(&pay, b, 4);
    w_raw(&pay, body->data, body->len);
    uint8_t hdr[20];
    uint32_t len = (uint32_t)pay.len;
    hdr[0] = (uint8_t)(len >> 24);
    hdr[1] = (uint8_t)(len >> 16);
    hdr[2] = (uint8_t)(len >> 8);
    hdr[3] = (uint8_t)len;
    hdr[4] = 0xFF;
    hdr[5] = 0xFF;
    hdr[6] = 0xFF;
    hdr[7] = 0xFF;
    memset(hdr + 8, 0, 12);
    size_t off = 0;
    while (off < sizeof hdr) {
        ssize_t k = write(p->fd, hdr + off, sizeof hdr - off);
        if (k <= 0) {
            if (k < 0 && errno == EINTR)
                continue;
            snprintf(err, errn, "pulse: write: %s", strerror(errno));
            free(pay.data);
            return 0;
        }
        off += (size_t)k;
    }
    off = 0;
    while (off < pay.len) {
        ssize_t k = write(p->fd, pay.data + off, pay.len - off);
        if (k <= 0) {
            if (k < 0 && errno == EINTR)
                continue;
            snprintf(err, errn, "pulse: write: %s", strerror(errno));
            free(pay.data);
            return 0;
        }
        off += (size_t)k;
    }
    free(pay.data);
    return 1;
}

static int read_frame(int fd, const volatile int *stop, uint8_t **payload,
                      size_t *paylen, uint32_t *channel, char *err, size_t errn) {
    uint8_t desc[20];
    ssize_t g = read_interruptible(fd, desc, sizeof desc, stop, err, errn);
    if (g < 0)
        return -1;
    if (g < 20)
        return 0;
    uint32_t len = ((uint32_t)desc[0] << 24) | ((uint32_t)desc[1] << 16) |
                   ((uint32_t)desc[2] << 8) | desc[3];
    *channel = ((uint32_t)desc[4] << 24) | ((uint32_t)desc[5] << 16) |
               ((uint32_t)desc[6] << 8) | desc[7];
    if (len == 0 || len > 16 * 1024 * 1024) {
        snprintf(err, errn, "pulse: bogus frame size %u", len);
        return -1;
    }
    uint8_t *p = malloc(len);
    if (!p)
        abort();
    g = read_interruptible(fd, p, len, stop, err, errn);
    if (g < 0) {
        free(p);
        return -1;
    }
    if ((size_t)g < len) {
        free(p);
        return 0;
    }
    *payload = p;
    *paylen = len;
    return 1;
}

static int reply_for(Pulse *p, uint32_t want, uint8_t **body, size_t *bodylen,
                     char *err, size_t errn) {
    for (;;) {
        uint8_t *payload = NULL;
        size_t paylen = 0;
        uint32_t channel = 0;
        int r = read_frame(p->fd, NULL, &payload, &paylen, &channel, err, errn);
        if (r <= 0)
            return r;
        if (channel != 0xFFFFFFFFu) {
            free(payload);
            continue;
        }
        Reader rd = {payload, paylen, 0, 0};
        uint32_t cmd = r_u32(&rd);
        uint32_t tag = r_u32(&rd);
        if (rd.failed || tag != want) {
            free(payload);
            continue;
        }
        if (cmd == CMD_REPLY) {
            *bodylen = paylen - rd.pos;
            *body = malloc(*bodylen ? *bodylen : 1);
            if (!*body)
                abort();
            memcpy(*body, payload + rd.pos, *bodylen);
            free(payload);
            return 1;
        }
        if (cmd == CMD_ERROR) {
            uint32_t code = r_u32(&rd);
            snprintf(err, errn, "pulse: server error %u", code);
            free(payload);
            return -1;
        }
        free(payload);
    }
}

Pulse *pulse_connect(char *err, size_t errn) {
    char cands[6][512];
    int nc = 0;
    const char *srv = getenv("PULSE_SERVER");
    if (srv) {
        char *dup = strdup(srv);
        char *save = NULL;
        char *tok = strtok_r(dup, " ", &save);
        while (tok && nc < 6) {
            while (*tok == ' ' || *tok == '\t')
                tok++;
            if (*tok) {
                char item[112];
                snprintf(item, sizeof item, "%s", tok);
                if (!strncmp(item, "unix:", 5))
                    snprintf(cands[nc++], sizeof cands[0], "%s", item + 5);
                else if (strncmp(item, "tcp:", 4))
                    snprintf(cands[nc++], sizeof cands[0], "%s", item);
            }
            tok = strtok_r(NULL, " ", &save);
        }
        free(dup);
    }
    const char *rt = getenv("XDG_RUNTIME_DIR");
    if (rt && nc < 6)
        snprintf(cands[nc++], sizeof cands[0], "%s/pulse/native", rt);
    {
        unsigned uid = (unsigned)geteuid();
        if (nc < 6)
            snprintf(cands[nc++], sizeof cands[0], "/run/user/%u/pulse/native", uid);
        if (nc < 6)
            snprintf(cands[nc++], sizeof cands[0], "/tmp/pulse-%u/native", uid);
    }
    char last_err[768];
    snprintf(last_err, sizeof last_err, "no pulse server socket found");
    for (int i = 0; i < nc; i++) {
        size_t cl = strlen(cands[i]);
        char short_path[128];
        if (cl < sizeof short_path)
            memcpy(short_path, cands[i], cl + 1);
        else {
            memcpy(short_path, cands[i], sizeof short_path - 4);
            memcpy(short_path + sizeof short_path - 4, "...", 4);
        }
        int fd = socket(AF_UNIX, SOCK_STREAM, 0);
        if (fd < 0)
            continue;
        struct sockaddr_un addr;
        memset(&addr, 0, sizeof addr);
        addr.sun_family = AF_UNIX;
        if (cl >= sizeof addr.sun_path)
            continue;
        memcpy(addr.sun_path, cands[i], cl + 1);
        if (connect(fd, (struct sockaddr *)&addr, sizeof addr) != 0) {
            snprintf(last_err, sizeof last_err, "pulse: %s: %s", short_path, strerror(errno));
            close(fd);
            continue;
        }
        Pulse *p = calloc(1, sizeof *p);
        if (!p)
            abort();
        p->fd = fd;
        p->tag = 0;
        char e2[256];
        uint32_t tag = ++p->tag;
        Writer body = {0};
        w_u32(&body, PA_PROTOCOL_VERSION);
        uint8_t cookie[COOKIE_LEN];
        load_cookie(cookie);
        w_arbitrary(&body, cookie, COOKIE_LEN);
        int ok = send_msg(p, CMD_AUTH, tag, &body, e2, sizeof e2);
        free(body.data);
        if (ok) {
            uint8_t *rb = NULL;
            size_t rbl = 0;
            if (reply_for(p, tag, &rb, &rbl, e2, sizeof e2) > 0)
                free(rb);
            else
                ok = 0;
        }
        if (ok) {
            tag = ++p->tag;
            Writer b2 = {0};
            const char *keys[] = {"application.name", "application.process.binary"};
            const uint8_t *vv[] = {(const uint8_t *)"sharkvis", (const uint8_t *)"sharkvis"};
            const size_t ll[] = {8, 8};
            w_proplist(&b2, keys, vv, ll, 2);
            ok = send_msg(p, CMD_SET_CLIENT_NAME, tag, &b2, e2, sizeof e2);
            free(b2.data);
            if (ok) {
                uint8_t *rb = NULL;
                size_t rbl = 0;
                if (reply_for(p, tag, &rb, &rbl, e2, sizeof e2) > 0)
                    free(rb);
                else
                    ok = 0;
            }
        }
        if (ok)
            return p;
        snprintf(last_err, sizeof last_err, "%s", e2);
        close(fd);
        free(p);
    }
    snprintf(err, errn, "%s", last_err);
    return NULL;
}

void pulse_free(Pulse *p) {
    if (!p)
        return;
    close(p->fd);
    free(p);
}

int pulse_default_monitor(Pulse *p, char *out, size_t outn, char *err, size_t errn) {
    uint32_t tag = ++p->tag;
    Writer body = {0};
    w_u32(&body, PA_INVALID_INDEX);
    w_string_null(&body);
    if (!send_msg(p, CMD_GET_SINK_INFO, tag, &body, err, errn)) {
        free(body.data);
        return 0;
    }
    free(body.data);
    uint8_t *rb = NULL;
    size_t rbl = 0;
    if (reply_for(p, tag, &rb, &rbl, err, errn) <= 0)
        return 0;
    Reader r = {rb, rbl, 0, 0};
    char tmp[512];
    int b;
    r_u32(&r);
    if (r.data[r.pos] == TAG_STRING_NULL) {
        r.pos++;
    } else if (!r_string(&r, tmp, sizeof tmp)) {
        r.failed = 1;
    }
    if (!r.failed) {
        if (r.data[r.pos] == TAG_STRING_NULL) {
            r.pos++;
        } else if (!r_string(&r, tmp, sizeof tmp)) {
            r.failed = 1;
        }
    }
    if (!r.failed)
        r_skip_sample_spec(&r);
    if (!r.failed)
        r_skip_channel_map(&r);
    if (!r.failed)
        r_u32(&r);
    if (!r.failed)
        r_skip_cvolume(&r);
    if (!r.failed)
        r_boolean(&r, &b);
    if (!r.failed)
        r_u32(&r);
    int have_mon = 0;
    if (!r.failed) {
        if (r.pos < r.len && r.data[r.pos] == TAG_STRING_NULL) {
            r.pos++;
        } else if (r_string(&r, tmp, sizeof tmp)) {
            have_mon = tmp[0] != 0;
        } else {
            r.failed = 1;
        }
    }
    if (r.failed || !have_mon) {
        snprintf(err, errn, "pulse: default sink has no monitor source");
        free(rb);
        return 0;
    }
    snprintf(out, outn, "%s", tmp);
    free(rb);
    return 1;
}

Record *pulse_record(Pulse *p, const char *device, unsigned rate,
                     unsigned channels, unsigned fragsize, char *err, size_t errn) {
    uint8_t map[2];
    size_t mapn;
    if (channels >= 2) {
        map[0] = 1;
        map[1] = 2;
        mapn = 2;
    } else {
        map[0] = 0;
        mapn = 1;
    }
    uint32_t tag = ++p->tag;
    Writer body = {0};
    w_sample_spec(&body, PA_SAMPLE_S16NE, (uint8_t)channels, rate);
    w_channel_map(&body, map, mapn);
    w_u32(&body, PA_INVALID_INDEX);
    w_string(&body, device);
    w_u32(&body, 0xFFFFFFFFu);
    w_boolean(&body, 0);
    w_u32(&body, fragsize);
    for (int i = 0; i < 9; i++)
        w_boolean(&body, 0);
    const char *keys[] = {"media.name", "application.name", "application.process.binary"};
    const uint8_t *vv[] = {(const uint8_t *)"sharkvis spectrum",
                           (const uint8_t *)"sharkvis", (const uint8_t *)"sharkvis"};
    const size_t ll[] = {18, 8, 8};
    w_proplist(&body, keys, vv, ll, 3);
    w_u32(&body, PA_INVALID_INDEX);
    w_boolean(&body, 0);
    w_boolean(&body, 0);
    w_boolean(&body, 0);
    w_u8(&body, 0);
    w_cvolume(&body, (uint8_t)channels, PA_VOLUME_NORM);
    for (int i = 0; i < 5; i++)
        w_boolean(&body, 0);
    if (!send_msg(p, CMD_CREATE_RECORD_STREAM, tag, &body, err, errn)) {
        free(body.data);
        return NULL;
    }
    free(body.data);
    uint8_t *rb = NULL;
    size_t rbl = 0;
    if (reply_for(p, tag, &rb, &rbl, err, errn) <= 0)
        return NULL;
    Reader r = {rb, rbl, 0, 0};
    uint32_t stream_index = r_u32(&r);
    if (r.failed) {
        snprintf(err, errn, "pulse: bad record reply");
        free(rb);
        return NULL;
    }
    free(rb);
    Record *rec = calloc(1, sizeof *rec);
    if (!rec)
        abort();
    rec->fd = p->fd;
    rec->stream_index = stream_index;
    free(p);
    return rec;
}

void record_free(Record *r) {
    if (!r)
        return;
    close(r->fd);
    free(r->pending);
    free(r);
}

long record_read(Record *r, uint8_t *out, size_t cap,
                 const volatile int *stop, char *err, size_t errn) {
    size_t filled = 0;
    while (filled < cap) {
        if (r->off >= r->plen) {
            free(r->pending);
            r->pending = NULL;
            r->plen = 0;
            r->off = 0;
            if (stop && *stop)
                break;
            uint8_t *payload = NULL;
            size_t paylen = 0;
            uint32_t channel = 0;
            int fr = read_frame(r->fd, stop, &payload, &paylen, &channel, err, errn);
            if (fr < 0)
                return -1;
            if (fr == 0) {
                free(payload);
                break;
            }
            if (channel == 0xFFFFFFFFu) {
                Reader rd = {payload, paylen, 0, 0};
                uint32_t cmd = r_u32(&rd);
                r_u32(&rd);
                if (cmd == CMD_ERROR) {
                    uint32_t code = r_u32(&rd);
                    snprintf(err, errn, "pulse: server error %u", code);
                    free(payload);
                    return -1;
                }
                if (cmd == CMD_RECORD_STREAM_KILLED) {
                    snprintf(err, errn, "pulse: record stream was killed");
                    free(payload);
                    return -1;
                }
                free(payload);
            } else {
                if (channel == r->stream_index) {
                    r->pending = payload;
                    r->plen = paylen;
                } else {
                    free(payload);
                }
            }
            continue;
        }
        size_t avail = r->plen - r->off;
        size_t n = avail < cap - filled ? avail : cap - filled;
        memcpy(out + filled, r->pending + r->off, n);
        r->off += n;
        filled += n;
    }
    return (long)filled;
}
