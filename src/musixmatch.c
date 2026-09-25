#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#include "lyrics.h"
#include "mpris.h"
#include "musixmatch.h"

#define UA "Musixmatch/2025120901 CFNetwork/3860.300.31 Darwin/25.2.0"
#define APP_ID "mac-ios-v2.0"

typedef enum { J_NULL, J_BOOL, J_NUM, J_STR, J_ARR, J_OBJ } JType;

typedef struct Json Json;
typedef struct {
    char *key;
    Json *val;
} JMember;

struct Json {
    JType type;
    int boolean;
    double num;
    char *str;
    Json **items;
    size_t n;
    JMember *members;
    size_t nm;
};

typedef struct {
    const uint8_t *b;
    size_t len;
    size_t pos;
} Parser;

static void skip_ws(Parser *p) {
    while (p->pos < p->len) {
        uint8_t c = p->b[p->pos];
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r')
            p->pos++;
        else
            break;
    }
}

static Json *json_new(JType t) {
    Json *j = calloc(1, sizeof *j);
    if (j)
        j->type = t;
    return j;
}

static void json_free(Json *j) {
    if (!j)
        return;
    free(j->str);
    for (size_t i = 0; i < j->n; i++)
        json_free(j->items[i]);
    free(j->items);
    for (size_t i = 0; i < j->nm; i++) {
        free(j->members[i].key);
        json_free(j->members[i].val);
    }
    free(j->members);
    free(j);
}

static int parse_string_raw(Parser *p, char **out) {
    if (p->pos >= p->len || p->b[p->pos] != '"')
        return 0;
    p->pos++;
    size_t cap = 64;
    size_t n = 0;
    char *o = malloc(cap);
    if (!o)
        return 0;
    while (p->pos < p->len) {
        uint8_t c = p->b[p->pos];
        if (c == '"') {
            p->pos++;
            o[n] = 0;
            *out = o;
            return 1;
        }
        if (c == '\\') {
            p->pos++;
            if (p->pos >= p->len) {
                free(o);
                return 0;
            }
            uint8_t e = p->b[p->pos];
            char tmp[8];
            size_t tn = 0;
            if (e == 'n')
                tmp[tn++] = '\n';
            else if (e == 'r')
                tmp[tn++] = '\r';
            else if (e == 't')
                tmp[tn++] = '\t';
            else if (e == '"' || e == '\\' || e == '/')
                tmp[tn++] = (char)e;
            else if (e == 'u') {
                if (p->pos + 4 >= p->len) {
                    free(o);
                    return 0;
                }
                char h[5];
                memcpy(h, p->b + p->pos + 1, 4);
                h[4] = 0;
                char *end;
                unsigned long cp = strtoul(h, &end, 16);
                if (end != h + 4 || cp > 0x10FFFF) {
                    free(o);
                    return 0;
                }
                if (cp < 0x80) {
                    tmp[tn++] = (char)cp;
                } else if (cp < 0x800) {
                    tmp[tn++] = (char)(0xC0 | (cp >> 6));
                    tmp[tn++] = (char)(0x80 | (cp & 0x3F));
                } else if (cp < 0x10000) {
                    tmp[tn++] = (char)(0xE0 | (cp >> 12));
                    tmp[tn++] = (char)(0x80 | ((cp >> 6) & 0x3F));
                    tmp[tn++] = (char)(0x80 | (cp & 0x3F));
                } else {
                    tmp[tn++] = (char)(0xF0 | (cp >> 18));
                    tmp[tn++] = (char)(0x80 | ((cp >> 12) & 0x3F));
                    tmp[tn++] = (char)(0x80 | ((cp >> 6) & 0x3F));
                    tmp[tn++] = (char)(0x80 | (cp & 0x3F));
                }
                p->pos += 4;
            } else {
                free(o);
                return 0;
            }
            while (n + tn + 1 > cap) {
                cap *= 2;
                o = realloc(o, cap);
            }
            memcpy(o + n, tmp, tn);
            n += tn;
            p->pos++;
        } else {
            if (n + 2 > cap) {
                cap *= 2;
                o = realloc(o, cap);
            }
            o[n++] = (char)c;
            p->pos++;
        }
        if (n > 1000000) {
            free(o);
            return 0;
        }
    }
    free(o);
    return 0;
}

static Json *parse_value(Parser *p, int depth);

static Json *parse_number(Parser *p) {
    size_t start = p->pos;
    if (p->pos < p->len && p->b[p->pos] == '-')
        p->pos++;
    int any = 0;
    while (p->pos < p->len && p->b[p->pos] >= '0' && p->b[p->pos] <= '9') {
        p->pos++;
        any = 1;
    }
    if (p->pos < p->len && p->b[p->pos] == '.') {
        p->pos++;
        while (p->pos < p->len && p->b[p->pos] >= '0' && p->b[p->pos] <= '9') {
            p->pos++;
            any = 1;
        }
    }
    if (p->pos < p->len && (p->b[p->pos] == 'e' || p->b[p->pos] == 'E')) {
        p->pos++;
        if (p->pos < p->len && (p->b[p->pos] == '+' || p->b[p->pos] == '-'))
            p->pos++;
        while (p->pos < p->len && p->b[p->pos] >= '0' && p->b[p->pos] <= '9') {
            p->pos++;
            any = 1;
        }
    }
    if (!any)
        return NULL;
    char *tmp = malloc(p->pos - start + 1);
    memcpy(tmp, p->b + start, p->pos - start);
    tmp[p->pos - start] = 0;
    char *end;
    double v = strtod(tmp, &end);
    free(tmp);
    if (end == tmp)
        return NULL;
    Json *j = json_new(J_NUM);
    j->num = v;
    return j;
}

static Json *parse_value(Parser *p, int depth) {
    if (depth > 64)
        return NULL;
    skip_ws(p);
    if (p->pos >= p->len)
        return NULL;
    uint8_t c = p->b[p->pos];
    if (c == 'n' && p->pos + 4 <= p->len && !memcmp(p->b + p->pos, "null", 4)) {
        p->pos += 4;
        return json_new(J_NULL);
    }
    if (c == 't' && p->pos + 4 <= p->len && !memcmp(p->b + p->pos, "true", 4)) {
        p->pos += 4;
        Json *j = json_new(J_BOOL);
        j->boolean = 1;
        return j;
    }
    if (c == 'f' && p->pos + 5 <= p->len && !memcmp(p->b + p->pos, "false", 5)) {
        p->pos += 5;
        Json *j = json_new(J_BOOL);
        j->boolean = 0;
        return j;
    }
    if (c == '"') {
        char *s = NULL;
        if (!parse_string_raw(p, &s))
            return NULL;
        Json *j = json_new(J_STR);
        j->str = s;
        return j;
    }
    if (c == '[') {
        p->pos++;
        Json *j = json_new(J_ARR);
        for (;;) {
            skip_ws(p);
            if (p->pos < p->len && p->b[p->pos] == ']') {
                p->pos++;
                break;
            }
            Json *v = parse_value(p, depth + 1);
            if (!v) {
                json_free(j);
                return NULL;
            }
            j->items = realloc(j->items, (j->n + 1) * sizeof(Json *));
            j->items[j->n++] = v;
            if (j->n > 4096) {
                json_free(j);
                return NULL;
            }
            skip_ws(p);
            if (p->pos >= p->len) {
                json_free(j);
                return NULL;
            }
            if (p->b[p->pos] == ',') {
                p->pos++;
            } else if (p->b[p->pos] == ']') {
                continue;
            } else {
                json_free(j);
                return NULL;
            }
        }
        return j;
    }
    if (c == '{') {
        p->pos++;
        Json *j = json_new(J_OBJ);
        for (;;) {
            skip_ws(p);
            if (p->pos < p->len && p->b[p->pos] == '}') {
                p->pos++;
                break;
            }
            char *k = NULL;
            if (!parse_string_raw(p, &k)) {
                json_free(j);
                return NULL;
            }
            skip_ws(p);
            if (p->pos >= p->len || p->b[p->pos] != ':') {
                free(k);
                json_free(j);
                return NULL;
            }
            p->pos++;
            Json *v = parse_value(p, depth + 1);
            if (!v) {
                free(k);
                json_free(j);
                return NULL;
            }
            j->members = realloc(j->members, (j->nm + 1) * sizeof(JMember));
            j->members[j->nm].key = k;
            j->members[j->nm].val = v;
            j->nm++;
            if (j->nm > 1024) {
                json_free(j);
                return NULL;
            }
            skip_ws(p);
            if (p->pos >= p->len) {
                json_free(j);
                return NULL;
            }
            if (p->b[p->pos] == ',') {
                p->pos++;
            } else if (p->b[p->pos] == '}') {
                continue;
            } else {
                json_free(j);
                return NULL;
            }
        }
        return j;
    }
    if (c == '-' || (c >= '0' && c <= '9'))
        return parse_number(p);
    return NULL;
}

static Json *parse_json(const char *text) {
    Parser p = {(const uint8_t *)text, strlen(text), 0};
    Json *v = parse_value(&p, 0);
    if (!v)
        return NULL;
    skip_ws(&p);
    if (p.pos != p.len) {
        json_free(v);
        return NULL;
    }
    return v;
}

static const Json *json_get(const Json *j, const char *key) {
    if (!j || j->type != J_OBJ)
        return NULL;
    for (size_t i = 0; i < j->nm; i++) {
        if (!strcmp(j->members[i].key, key))
            return j->members[i].val;
    }
    return NULL;
}

static const Json *json_pointer(const Json *j, const char *path) {
    const Json *cur = j;
    const char *p = path;
    while (*p) {
        while (*p == '/')
            p++;
        if (!*p)
            break;
        const char *e = p;
        while (*e && *e != '/')
            e++;
        char part[128];
        size_t n = (size_t)(e - p);
        if (n >= sizeof part)
            return NULL;
        memcpy(part, p, n);
        part[n] = 0;
        char *end;
        long idx = strtol(part, &end, 10);
        if (end != part && *end == 0) {
            if (!cur || cur->type != J_ARR || idx < 0 || (size_t)idx >= cur->n)
                return NULL;
            cur = cur->items[idx];
        } else {
            cur = json_get(cur, part);
            if (!cur)
                return NULL;
        }
        p = e;
    }
    return cur;
}

static const char *json_str(const Json *j) {
    return (j && j->type == J_STR) ? j->str : NULL;
}

static int json_f64(const Json *j, double *out) {
    if (!j || j->type != J_NUM)
        return 0;
    *out = j->num;
    return 1;
}

static int json_i64(const Json *j, long long *out) {
    double v;
    if (!json_f64(j, &v))
        return 0;
    *out = (long long)v;
    return 1;
}

static char *api_get(const char *url, const char *guid) {
    const char *args[12];
    int n = 0;
    char cookie[128];
    char xcookie[192];
    args[n++] = "-fsSL";
    args[n++] = "-m";
    args[n++] = "20";
    args[n++] = "-A";
    args[n++] = UA;
    if (guid) {
        snprintf(cookie, sizeof cookie, "x-mxm-token-guid=%s", guid);
        snprintf(xcookie, sizeof xcookie, "X-Cookie: %s", cookie);
        args[n++] = "--cookie";
        args[n++] = cookie;
        args[n++] = "-H";
        args[n++] = xcookie;
    }
    args[n++] = url;
    args[n++] = NULL;
    return cmd_out("curl", args, 25000);
}

static int token_path(char *out, size_t n) {
    const char *base = getenv("XDG_CACHE_HOME");
    char tmp[1024];
    if (base && *base) {
        const char *s = base;
        while (*s == ' ' || *s == '\t')
            s++;
        if (*s)
            snprintf(tmp, sizeof tmp, "%s", base);
        else
            base = NULL;
    }
    if (!base || !*base) {
        const char *home = getenv("HOME");
        if (!home || !*home)
            return 0;
        snprintf(tmp, sizeof tmp, "%s/.cache", home);
    }
    snprintf(out, n, "%s/sharkvis/musixmatch_token.json", tmp);
    return 1;
}

static int is_valid_token(const char *token) {
    while (*token == ' ' || *token == '\t' || *token == '\n' || *token == '\r')
        token++;
    size_t len = strlen(token);
    while (len > 0 && (token[len - 1] == ' ' || token[len - 1] == '\t' ||
                       token[len - 1] == '\n' || token[len - 1] == '\r'))
        len--;
    if (len == 0)
        return 0;
    char t[1024];
    if (len >= sizeof t)
        len = sizeof t - 1;
    memcpy(t, token, len);
    t[len] = 0;
    if (strstr(t, "UpgradeOnly"))
        return 0;
    int allzero = 1;
    for (size_t i = 0; t[i]; i++) {
        if (t[i] != '0' && t[i] != '-') {
            allzero = 0;
            break;
        }
    }
    return !allzero;
}

static char *load_disk_token(void) {
    char path[1100];
    if (!token_path(path, sizeof path))
        return NULL;
    FILE *f = fopen(path, "r");
    if (!f)
        return NULL;
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (sz < 0 || sz > 100000) {
        fclose(f);
        return NULL;
    }
    char *content = malloc((size_t)sz + 1);
    if (fread(content, 1, (size_t)sz, f) != (size_t)sz) {
        free(content);
        fclose(f);
        return NULL;
    }
    content[sz] = 0;
    fclose(f);
    Json *json = parse_json(content);
    free(content);
    if (!json)
        return NULL;
    const char *tok = json_str(json_get(json, "user_token"));
    char *out = NULL;
    if (tok) {
        while (*tok == ' ' || *tok == '\t')
            tok++;
        if (is_valid_token(tok))
            out = strdup(tok);
    }
    json_free(json);
    return out;
}

static void json_escape_into(const char *s, char *out, size_t n) {
    size_t m = 0;
    for (size_t i = 0; s[i] && m + 2 < n; i++) {
        unsigned char c = (unsigned char)s[i];
        if (c == '"' || c == '\\') {
            out[m++] = '\\';
            out[m++] = (char)c;
        } else if (c == '\n') {
            out[m++] = '\\';
            out[m++] = 'n';
        } else if (c < 0x20) {
            if (m + 7 < n) {
                m += (size_t)snprintf(out + m, n - m, "\\u%04x", c);
            }
        } else {
            out[m++] = (char)c;
        }
    }
    out[m] = 0;
}

static void save_disk_token(const char *token) {
    char path[1100];
    if (!token_path(path, sizeof path))
        return;
    char *slash = strrchr(path, '/');
    if (slash) {
        *slash = 0;
        char cur[1100];
        size_t n = 0;
        if (path[0] == '/')
            cur[n++] = '/';
        char *save = NULL;
        char *tok = strtok_r(path + (path[0] == '/' ? 1 : 0), "/", &save);
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
        *slash = '/';
        char *s2 = strrchr(path, '/');
        (void)s2;
    }
    char esc[2048];
    json_escape_into(token, esc, sizeof esc);
    char tmp[1200];
    snprintf(tmp, sizeof tmp, "%s.tmp", path);
    FILE *f = fopen(tmp, "w");
    if (!f)
        return;
    fprintf(f, "{\"user_token\":\"%s\"}", esc);
    fclose(f);
    rename(tmp, path);
}

static void clear_disk_token(void) {
    char path[1100];
    if (token_path(path, sizeof path))
        remove(path);
}

static void new_guid(char *out, size_t n) {
    uint8_t bytes[16];
    memset(bytes, 0, sizeof bytes);
    FILE *f = fopen("/dev/urandom", "r");
    if (f) {
        fread(bytes, 1, 16, f);
        fclose(f);
    }
    int allzero = 1;
    for (int i = 0; i < 16; i++) {
        if (bytes[i]) {
            allzero = 0;
            break;
        }
    }
    if (allzero) {
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        unsigned long long t = (unsigned long long)ts.tv_sec * 1000000000ull +
                                (unsigned long long)ts.tv_nsec;
        unsigned pid = (unsigned)getpid();
        memcpy(bytes, &t, 8);
        memcpy(bytes + 8, &pid, 4);
        memcpy(bytes + 12, &pid, 4);
    }
    snprintf(out, n, "%02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-%02x%02x%02x%02x%02x%02x",
             bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
             bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11],
             bytes[12], bytes[13], bytes[14], bytes[15]);
}

static char *fetch_fresh_token(void) {
    char guid[64];
    new_guid(guid, sizeof guid);
    char url[512];
    snprintf(url, sizeof url,
             "https://apic-appmobile.musixmatch.com/ws/1.1/token.get?app_id=%s&guid=%s",
             APP_ID, guid);
    char *body = api_get(url, guid);
    if (!body)
        return NULL;
    Json *json = parse_json(body);
    free(body);
    if (!json)
        return NULL;
    long long code = 0;
    const Json *sc = json_pointer(json, "/message/header/status_code");
    int ok = sc && json_i64(sc, &code) && code == 200;
    char *out = NULL;
    if (ok) {
        const char *tok = json_str(json_pointer(json, "/message/body/user_token"));
        if (tok) {
            while (*tok == ' ' || *tok == '\t')
                tok++;
            if (is_valid_token(tok)) {
                out = strdup(tok);
                save_disk_token(out);
            }
        }
    }
    json_free(json);
    return out;
}

static void macro_url(const char *token, const char *artist, const char *title,
                      const char *album, double duration, char *out, size_t n) {
    char ea[1024], et[1024], eal[1024], etok[2048];
    url_encode(artist, ea, sizeof ea);
    url_encode(title, et, sizeof et);
    size_t m = snprintf(out, n,
        "https://apic-appmobile.musixmatch.com/ws/1.1/macro.subtitles.get"
        "?format=json&namespace=lyrics_richsynched&subtitle_format=mxm"
        "&optional_calls=track.richsync&app_id=%s&richsync_compact_type=words"
        "&q_artist=%s&q_artists=%s&q_track=%s",
        APP_ID, ea, ea, et);
    if (album) {
        const char *a = album;
        while (*a == ' ' || *a == '\t')
            a++;
        if (*a && m < n) {
            url_encode(album, eal, sizeof eal);
            m += (size_t)snprintf(out + m, n - m, "&q_album=%s", eal);
        }
    }
    if (duration > 0.0 && m < n)
        m += (size_t)snprintf(out + m, n - m, "&q_duration=%lld", (long long)llround(duration));
    if (m < n) {
        url_encode(token, etok, sizeof etok);
        snprintf(out + m, n - m, "&usertoken=%s", etok);
    }
}

typedef struct {
    LyricLine *lines;
    size_t n;
} LineVec2;

static void linevec2_push(LineVec2 *v, double t, const char *text) {
    v->lines = realloc(v->lines, (v->n + 1) * sizeof(LyricLine));
    v->lines[v->n].t = t;
    v->lines[v->n].text = strdup(text);
    v->lines[v->n].words = NULL;
    v->lines[v->n].nwords = 0;
    v->n++;
}

static void linevec2_word(LineVec2 *v, double t, const char *text) {
    LyricLine *l = &v->lines[v->n - 1];
    l->words = realloc(l->words, (l->nwords + 1) * sizeof(LyricWord));
    l->words[l->nwords].t = t;
    l->words[l->nwords].text = strdup(text);
    l->nwords++;
}

static char *trim_dup(const char *s) {
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')
        s++;
    size_t len = strlen(s);
    while (len > 0 && (s[len - 1] == ' ' || s[len - 1] == '\t' ||
                       s[len - 1] == '\n' || s[len - 1] == '\r'))
        len--;
    char *o = malloc(len + 1);
    memcpy(o, s, len);
    o[len] = 0;
    return o;
}

static int parse_richsync_words(const Json *line, double ls, double le, LineVec2 *v) {
    const Json *arr = json_get(line, "words");
    size_t before = v->lines[v->n - 1].nwords;
    if (arr && arr->type == J_ARR) {
        size_t k = 0;
        for (size_t i = 0; i < arr->n && k < 64; i++, k++) {
            const Json *w = arr->items[i];
            double start = ls, end;
            const Json *sj = json_get(w, "start");
            if (sj)
                json_f64(sj, &start);
            const Json *ej = json_get(w, "end");
            if (ej && json_f64(ej, &end)) {
                if (end <= start)
                    end = le;
            } else {
                end = start;
                if (end <= start)
                    end = le;
            }
            const char *tx = json_str(json_get(w, "text"));
            char *t = trim_dup(tx ? tx : "");
            if (!t[0]) {
                free(t);
                continue;
            }
            if (end - start < 0.039)
                end = start + 0.039;
            linevec2_word(v, start, t);
            free(t);
        }
        if (v->lines[v->n - 1].nwords > before)
            return 1;
    }
    const Json *arr2 = json_get(line, "l");
    if (arr2 && arr2->type == J_ARR) {
        double t = ls;
        double step = arr2->n ? (le - ls) / (double)arr2->n : 0.0;
        size_t k = 0;
        for (size_t i = 0; i < arr2->n && k < 128; i++, k++) {
            const char *tx = json_str(json_get(arr2->items[i], "c"));
            char *t2 = trim_dup(tx ? tx : "");
            if (!t2[0]) {
                free(t2);
                continue;
            }
            linevec2_word(v, t, t2);
            free(t2);
            t += step;
        }
        if (v->lines[v->n - 1].nwords > before)
            return 1;
    }
    return 0;
}

static LineVec2 richsync_lines(const Json *calls) {
    LineVec2 v = {0};
    long long code = 0;
    const Json *sc = json_pointer(calls, "/track.richsync.get/message/header/status_code");
    if (!sc || !json_i64(sc, &code) || code != 200)
        return v;
    const char *text = json_str(json_pointer(calls,
        "/track.richsync.get/message/body/richsync/richsync_body"));
    if (!text)
        return v;
    Json *json = parse_json(text);
    if (!json || json->type != J_ARR) {
        json_free(json);
        return v;
    }
    size_t k = 0;
    for (size_t i = 0; i < json->n && k < 400; i++, k++) {
        const Json *line = json->items[i];
        double start = 0.0, end;
        const Json *ts = json_pointer(line, "/ts");
        if (ts)
            json_f64(ts, &start);
        const Json *te = json_pointer(line, "/te");
        if (te && json_f64(te, &end)) {
        } else {
            end = start + 3.0;
        }
        const char *tx = json_str(json_get(line, "x"));
        if (!tx)
            tx = json_str(json_get(line, "text"));
        char *t = trim_dup(tx ? tx : "");
        if (!t[0]) {
            free(t);
            continue;
        }
        linevec2_push(&v, start, t);
        free(t);
        parse_richsync_words(line, start, end, &v);
    }
    json_free(json);
    return v;
}

static LineVec2 subtitles_lines(const Json *calls) {
    LineVec2 v = {0};
    long long code = 0;
    const Json *sc = json_pointer(calls, "/track.subtitles.get/message/header/status_code");
    if (!sc || !json_i64(sc, &code) || code != 200)
        return v;
    const char *text = json_str(json_pointer(calls,
        "/track.subtitles.get/message/body/subtitle_list/0/subtitle/subtitle_body"));
    if (!text)
        return v;
    Json *json = parse_json(text);
    if (!json || json->type != J_ARR) {
        json_free(json);
        return v;
    }
    size_t k = 0;
    for (size_t i = 0; i < json->n && k < 400; i++, k++) {
        const Json *line = json->items[i];
        double t = 0.0;
        const Json *tt = json_pointer(line, "/time/total");
        if (tt)
            json_f64(tt, &t);
        const char *tx = json_str(json_get(line, "text"));
        char *t2 = trim_dup(tx ? tx : "");
        if (!t2[0]) {
            free(t2);
            continue;
        }
        linevec2_push(&v, t, t2);
        free(t2);
    }
    json_free(json);
    return v;
}

static int is_instrumental(const Json *calls) {
    long long v = 0;
    const Json *j = json_pointer(calls, "/matcher.track.get/message/body/track/instrumental");
    return j && json_i64(j, &v) && v == 1;
}

typedef enum { MO_LINES, MO_TOKEN, MO_FAIL } MacroOutcome;

static MacroOutcome try_macro(const char *token, const char *artist,
                              const char *title, double duration, LineVec2 *out) {
    char url[4096];
    macro_url(token, artist, title, "", duration, url, sizeof url);
    char *body = api_get(url, "");
    if (!body)
        return MO_FAIL;
    Json *json = parse_json(body);
    free(body);
    if (!json)
        return MO_FAIL;
    long long code = -1;
    const Json *sc = json_pointer(json, "/message/header/status_code");
    if (sc)
        json_i64(sc, &code);
    MacroOutcome r = MO_FAIL;
    if (code == 401) {
        clear_disk_token();
        r = MO_TOKEN;
    } else if (code == 402 || code == 403 || code == 429) {
        r = MO_TOKEN;
    } else if (code == 200) {
        const Json *calls = json_pointer(json, "/message/body/macro_calls");
        if (calls) {
            if (is_instrumental(calls)) {
                linevec2_push(out, 0.0, "\xe2\x99\xaa Instrumental \xe2\x99\xaa");
                r = MO_LINES;
            } else {
                LineVec2 v = richsync_lines(calls);
                if (v.n > 0) {
                    *out = v;
                    r = MO_LINES;
                } else {
                    v = subtitles_lines(calls);
                    if (v.n > 0) {
                        *out = v;
                        r = MO_LINES;
                    }
                }
            }
        }
    }
    json_free(json);
    return r;
}

LyricLine *musix_fetch(const char *artist, const char *title,
                       double duration, size_t *nlines) {
    *nlines = 0;
    char *tokens[64];
    size_t ntok = 0;
    const char *env = getenv("MUSIXMATCH_USERTOKEN");
    if (env) {
        char *dup = strdup(env);
        char *save = NULL;
        char *tok = strtok_r(dup, ",", &save);
        while (tok && ntok < 64) {
            while (*tok == ' ' || *tok == '\t')
                tok++;
            char *e = tok + strlen(tok);
            while (e > tok && (e[-1] == ' ' || e[-1] == '\t'))
                *--e = 0;
            if (is_valid_token(tok))
                tokens[ntok++] = strdup(tok);
            tok = strtok_r(NULL, ",", &save);
        }
        free(dup);
    }
    if (ntok == 0) {
        char *t = load_disk_token();
        if (t)
            tokens[ntok++] = t;
    }
    size_t head = 0;
    int tried_fresh = 0;
    for (;;) {
        if (head >= ntok) {
            if (tried_fresh)
                break;
            tried_fresh = 1;
            char *t = fetch_fresh_token();
            if (t) {
                tokens[ntok++] = t;
                continue;
            }
            break;
        }
        char *token = tokens[head++];
        LineVec2 v = {0};
        MacroOutcome mo = try_macro(token, artist, title, duration, &v);
        if (mo == MO_LINES) {
            int need_save = head < ntok;
            if (!need_save) {
                char *d = load_disk_token();
                need_save = d == NULL;
                free(d);
            }
            if (need_save)
                save_disk_token(token);
            for (size_t i = 0; i < ntok; i++)
                free(tokens[i]);
            *nlines = v.n;
            return v.lines;
        }
        if (mo == MO_TOKEN) {
            clear_disk_token();
            free(v.lines);
            continue;
        }
        free(v.lines);
        for (size_t i = 0; i < ntok; i++)
            free(tokens[i]);
        break;
    }
    for (size_t i = 0; i < ntok; i++)
        free(tokens[i]);
    return NULL;
}
