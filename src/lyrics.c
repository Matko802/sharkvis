#include <ctype.h>
#include <dirent.h>
#include <math.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#include "common.h"
#include "lyrics.h"
#include "mpris.h"
#include "musixmatch.h"

void lyric_lines_free(LyricLine *lines, size_t n) {
    if (!lines)
        return;
    for (size_t i = 0; i < n; i++) {
        free(lines[i].text);
        for (size_t k = 0; k < lines[i].nwords; k++)
            free(lines[i].words[k].text);
        free(lines[i].words);
    }
    free(lines);
}

void lyric_display_free(LyricDisplay *d, size_t n) {
    if (!d)
        return;
    for (size_t i = 0; i < n; i++)
        free(d[i].text);
    free(d);
}

void url_encode(const char *s, char *out, size_t outn) {
    size_t m = 0;
    for (size_t i = 0; s[i] && m + 4 < outn; i++) {
        unsigned char b = (unsigned char)s[i];
        if ((b >= 'A' && b <= 'Z') || (b >= 'a' && b <= 'z') ||
            (b >= '0' && b <= '9') || b == '-' || b == '_' || b == '.' || b == '~') {
            out[m++] = (char)b;
        } else if (b == ' ') {
            out[m++] = '%';
            out[m++] = '2';
            out[m++] = '0';
        } else {
            m += (size_t)snprintf(out + m, outn - m, "%%%02X", b);
        }
    }
    out[m] = 0;
}

static uint32_t utf8_dec(const char *s, size_t *k) {
    const uint8_t *u = (const uint8_t *)s;
    if (u[0] < 0x80) {
        *k = 1;
        return u[0];
    }
    if ((u[0] & 0xE0) == 0xC0 && u[1]) {
        *k = 2;
        return ((uint32_t)(u[0] & 0x1F) << 6) | (u[1] & 0x3F);
    }
    if ((u[0] & 0xF0) == 0xE0 && u[1] && u[2]) {
        *k = 3;
        return ((uint32_t)(u[0] & 0x0F) << 12) | ((uint32_t)(u[1] & 0x3F) << 6) | (u[2] & 0x3F);
    }
    if (u[1] && u[2] && u[3]) {
        *k = 4;
        return ((uint32_t)(u[0] & 0x07) << 18) | ((uint32_t)(u[1] & 0x3F) << 12) |
               ((uint32_t)(u[2] & 0x3F) << 6) | (u[3] & 0x3F);
    }
    *k = 1;
    return u[0];
}

static void sanitize_query(const char *s, char *out, size_t n) {
    char tmp[1024];
    size_t m = 0;
    unsigned skip_b = 0, skip_p = 0;
    size_t i = 0;
    size_t len = strlen(s);
    while (i < len && m + 1 < sizeof tmp) {
        size_t k = 0;
        uint32_t c = utf8_dec(s + i, &k);
        if (c == '[') {
            skip_b++;
        } else if (c == ']') {
            if (skip_b > 0)
                skip_b--;
        } else if (c == '(') {
            skip_p++;
        } else if (c == ')') {
            if (skip_p > 0)
                skip_p--;
        } else if (skip_b == 0 && skip_p == 0) {
            if (m + k < sizeof tmp) {
                memcpy(tmp + m, s + i, k);
                m += k;
            }
        }
        i += k;
    }
    tmp[m] = 0;
    char *save = NULL;
    char *tok = strtok_r(tmp, " \t\n\r", &save);
    char joined[1024];
    joined[0] = 0;
    while (tok) {
        if (joined[0])
            strcat(joined, " ");
        strcat(joined, tok);
        tok = strtok_r(NULL, " \t\n\r", &save);
    }
    static const char *suffixes[] = {
        " - remaster", " - remastered", " - remastered version",
        " - live", " - acoustic", " - demo"
    };
    for (int si = 0; si < 6; si++) {
        size_t sl = strlen(suffixes[si]);
        size_t jl = strlen(joined);
        if (jl > sl) {
            int match = 1;
            for (size_t q = 0; q < sl; q++) {
                if (tolower((unsigned char)joined[jl - sl + q]) != suffixes[si][q]) {
                    match = 0;
                    break;
                }
            }
            if (match) {
                size_t bl = jl - sl;
                while (bl > 0 && (joined[bl - 1] == ' ' || joined[bl - 1] == '\t'))
                    bl--;
                if (bl > 0) {
                    joined[bl] = 0;
                    jl = bl;
                }
            }
        }
    }
    snprintf(out, n, "%s", joined);
}

static uint32_t canonical_char(uint32_t c) {
    if (c == 0x2018 || c == 0x2019 || c == 0x201B || c == '`')
        return '\'';
    if (c == 0x201C || c == 0x201D)
        return '"';
    if (c == 0x2013 || c == 0x2014 || c == 0x2015 || c == 0xFE58 ||
        c == 0xFE63 || c == 0xFF0D)
        return '-';
    if (c == 0xA0 || c == 0x2007 || c == 0x202F)
        return ' ';
    return c;
}

static void canonicalize(const char *s, char *out, size_t n) {
    size_t m = 0;
    size_t i = 0;
    size_t len = strlen(s);
    while (i < len && m + 5 < n) {
        size_t k = 0;
        uint32_t c = utf8_dec(s + i, &k);
        uint32_t d = canonical_char(c);
        if (d < 0x80) {
            out[m++] = (char)d;
        } else {
            memcpy(out + m, s + i, k);
            m += k;
        }
        i += k;
    }
    out[m] = 0;
}

static size_t levenshtein(const char *a, const char *b) {
    size_t al = strlen(a);
    size_t bl = strlen(b);
    char *al2 = malloc(al + 1);
    char *bl2 = malloc(bl + 1);
    for (size_t i = 0; i < al; i++)
        al2[i] = (char)tolower((unsigned char)a[i]);
    al2[al] = 0;
    for (size_t i = 0; i < bl; i++)
        bl2[i] = (char)tolower((unsigned char)b[i]);
    bl2[bl] = 0;
    if (al == 0) {
        free(al2);
        free(bl2);
        return bl;
    }
    if (bl == 0) {
        free(al2);
        free(bl2);
        return al;
    }
    size_t *prev = malloc((bl + 1) * sizeof(size_t));
    size_t *cur = malloc((bl + 1) * sizeof(size_t));
    for (size_t j = 0; j <= bl; j++)
        prev[j] = j;
    for (size_t i = 1; i <= al; i++) {
        cur[0] = i;
        for (size_t j = 1; j <= bl; j++) {
            size_t cost = al2[i - 1] == bl2[j - 1] ? 0 : 1;
            size_t v = prev[j] + 1;
            if (cur[j - 1] + 1 < v)
                v = cur[j - 1] + 1;
            if (prev[j - 1] + cost < v)
                v = prev[j - 1] + cost;
            cur[j] = v;
        }
        size_t *t = prev;
        prev = cur;
        cur = t;
    }
    size_t r = prev[bl];
    free(al2);
    free(bl2);
    free(prev);
    free(cur);
    return r;
}

static char *json_string_scan(const char *src, const char *key) {
    char pat[256];
    snprintf(pat, sizeof pat, "\"%s\":", key);
    const char *p = strstr(src, pat);
    if (!p)
        return NULL;
    p += strlen(pat);
    while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r')
        p++;
    if (!strncmp(p, "null", 4))
        return NULL;
    if (*p != '"')
        return NULL;
    p++;
    size_t cap = 128;
    size_t n = 0;
    char *o = malloc(cap);
    while (*p) {
        if (*p == '\\') {
            p++;
            char e = *p;
            if (!e) {
                free(o);
                return NULL;
            }
            char tmp[8];
            size_t tn = 0;
            if (e == 'n')
                tmp[tn++] = '\n';
            else if (e == 'r')
                tmp[tn++] = '\r';
            else if (e == 't')
                tmp[tn++] = '\t';
            else if (e == '"' || e == '\\' || e == '/')
                tmp[tn++] = e;
            else if (e == 'u') {
                char h[5];
                memcpy(h, p + 1, 4);
                h[4] = 0;
                char *end;
                unsigned long cp = strtoul(h, &end, 16);
                if (end != h + 4 || cp > 0x10FFFF) {
                    free(o);
                    return NULL;
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
                p += 4;
            } else {
                free(o);
                return NULL;
            }
            while (n + tn + 1 > cap) {
                cap *= 2;
                o = realloc(o, cap);
            }
            memcpy(o + n, tmp, tn);
            n += tn;
            p++;
        } else if (*p == '"') {
            o[n] = 0;
            return o;
        } else {
            if (n + 2 > cap) {
                cap *= 2;
                o = realloc(o, cap);
            }
            o[n++] = *p++;
        }
    }
    free(o);
    return NULL;
}

static int parse_time(const char *tag, double *out) {
    while (*tag == ' ' || *tag == '\t')
        tag++;
    const char *colon = strchr(tag, ':');
    if (!colon)
        return 0;
    char mbuf[32];
    size_t ml = (size_t)(colon - tag);
    if (ml >= sizeof mbuf)
        return 0;
    memcpy(mbuf, tag, ml);
    mbuf[ml] = 0;
    char *end;
    double min = strtod(mbuf, &end);
    if (end == mbuf)
        return 0;
    const char *rest = colon + 1;
    const char *dot = strchr(rest, '.');
    double sec, frac = 0.0;
    if (dot) {
        char sbuf[32];
        size_t sl = (size_t)(dot - rest);
        if (sl >= sizeof sbuf)
            return 0;
        memcpy(sbuf, rest, sl);
        sbuf[sl] = 0;
        sec = strtod(sbuf, &end);
        if (end == sbuf)
            return 0;
        const char *f = dot + 1;
        size_t fl = 0;
        while (f[fl] >= '0' && f[fl] <= '9')
            fl++;
        char fbuf[32];
        if (fl >= sizeof fbuf)
            fl = sizeof fbuf - 1;
        memcpy(fbuf, f, fl);
        fbuf[fl] = 0;
        double fv = strtod(fbuf, &end);
        if (end == fbuf)
            return 0;
        double scale = 1.0;
        for (size_t i = 0; i < fl; i++)
            scale *= 10.0;
        frac = fv / scale;
    } else {
        sec = strtod(rest, &end);
        if (end == rest)
            return 0;
    }
    *out = min * 60.0 + sec + frac;
    return 1;
}

typedef struct {
    double t;
    char *text;
} WordPair;

typedef struct {
    WordPair *pairs;
    size_t n;
    int saw;
} Pairs;

static int parse_vtt_time(const char *s, double *out) {
    while (*s == ' ' || *s == '\t')
        s++;
    char tmp[64];
    snprintf(tmp, sizeof tmp, "%s", s);
    char *e = tmp + strlen(tmp);
    while (e > tmp && (e[-1] == ' ' || e[-1] == '\t'))
        *--e = 0;
    double chunks[3];
    int nc = 0;
    char *save = NULL;
    char *tok = strtok_r(tmp, ":", &save);
    while (tok && nc < 3) {
        char *end;
        double v = strtod(tok, &end);
        if (end == tok)
            return 0;
        chunks[nc++] = v;
        tok = strtok_r(NULL, ":", &save);
    }
    if (nc < 2 || nc > 3)
        return 0;
    double secs = 0.0;
    double mult = 1.0;
    for (int i = nc - 1; i >= 0; i--) {
        secs += chunks[i] * mult;
        mult *= 60.0;
    }
    *out = secs;
    return 1;
}

static int parse_tag_time(const char *tag, double *out) {
    while (*tag == ' ' || *tag == '\t')
        tag++;
    if (!*tag)
        return 0;
    if (parse_vtt_time(tag, out))
        return 1;
    if (strchr(tag, ':'))
        return parse_time(tag, out);
    return 0;
}

static void pairs_push(Pairs *p, double t, const char *w) {
    p->pairs = realloc(p->pairs, (p->n + 1) * sizeof(WordPair));
    p->pairs[p->n].t = t;
    p->pairs[p->n].text = strdup(w);
    p->n++;
}

static Pairs split_inline_times(const char *s, double base) {
    Pairs out = {0};
    double cur = base;
    const char *rest = s;
    for (;;) {
        const char *lt = strchr(rest, '<');
        if (!lt)
            break;
        char head[2048];
        size_t hl = (size_t)(lt - rest);
        if (hl >= sizeof head)
            hl = sizeof head - 1;
        memcpy(head, rest, hl);
        head[hl] = 0;
        char *save = NULL;
        char *w = strtok_r(head, " \t\n\r", &save);
        while (w) {
            pairs_push(&out, cur, w);
            w = strtok_r(NULL, " \t\n\r", &save);
        }
        const char *after = lt + 1;
        const char *gt = strchr(after, '>');
        if (!gt) {
            rest = "";
            break;
        }
        char tag[128];
        size_t tl = (size_t)(gt - after);
        if (tl >= sizeof tag)
            tl = sizeof tag - 1;
        memcpy(tag, after, tl);
        tag[tl] = 0;
        double t;
        if (parse_tag_time(tag, &t)) {
            out.saw = 1;
            cur = t;
        }
        rest = gt + 1;
    }
    {
        char tail[2048];
        snprintf(tail, sizeof tail, "%s", rest);
        char *save = NULL;
        char *w = strtok_r(tail, " \t\n\r", &save);
        while (w) {
            pairs_push(&out, cur, w);
            w = strtok_r(NULL, " \t\n\r", &save);
        }
    }
    return out;
}

static void lines_push(LyricLine **lines, size_t *n, double t, const char *text,
                       WordPair *pairs, size_t npairs, int saw) {
    *lines = realloc(*lines, (*n + 1) * sizeof(LyricLine));
    LyricLine *l = &(*lines)[*n];
    l->t = t;
    l->text = strdup(text);
    l->words = NULL;
    l->nwords = 0;
    if (saw) {
        l->words = malloc(npairs * sizeof(LyricWord));
        for (size_t i = 0; i < npairs; i++) {
            l->words[i].t = pairs[i].t;
            l->words[i].text = strdup(pairs[i].text);
        }
        l->nwords = npairs;
    }
    (*n)++;
}

static int line_cmp(const void *a, const void *b) {
    double ta = ((const LyricLine *)a)->t;
    double tb = ((const LyricLine *)b)->t;
    if (ta < tb)
        return -1;
    if (ta > tb)
        return 1;
    return 0;
}

static char *trim_str(const char *s) {
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

static LyricLine *parse_lrc(const char *text, size_t *nlines) {
    LyricLine *out = NULL;
    size_t n = 0;
    double offset = 0.0;
    const char *p = text;
    while (*p) {
        const char *e = strchr(p, '\n');
        size_t ll = e ? (size_t)(e - p) : strlen(p);
        char *raw = malloc(ll + 1);
        memcpy(raw, p, ll);
        raw[ll] = 0;
        p = e ? e + 1 : p + ll;
        char *line = raw;
        while (*line == ' ' || *line == '\t')
            line++;
        if (*line != '[') {
            free(raw);
            continue;
        }
        double times[64];
        size_t nt = 0;
        for (;;) {
            while (*line == ' ' || *line == '\t')
                line++;
            if (*line != '[')
                break;
            char *end = strchr(line, ']');
            if (!end)
                break;
            *end = 0;
            char *tag = line + 1;
            line = end + 1;
            while (*line == ' ' || *line == '\t')
                line++;
            if (!strncmp(tag, "offset:", 7)) {
                offset = strtod(tag + 7, NULL) / 1000.0;
                continue;
            }
            if ((tag[0] >= 'A' && tag[0] <= 'Z') || (tag[0] >= 'a' && tag[0] <= 'z')) {
                if (!(tag[0] >= '0' && tag[0] <= '9'))
                    continue;
            }
            double t;
            if (parse_time(tag, &t) && nt < 64)
                times[nt++] = t;
        }
        char *txt = trim_str(line);
        free(raw);
        if (!txt[0]) {
            free(txt);
            continue;
        }
        for (size_t i = 0; i < nt; i++) {
            double t = times[i] + offset;
            Pairs pr = split_inline_times(txt, t);
            if (pr.n == 0) {
                for (size_t k = 0; k < pr.n; k++)
                    free(pr.pairs[k].text);
                free(pr.pairs);
                continue;
            }
            size_t need = 1;
            for (size_t k = 0; k < pr.n; k++)
                need += strlen(pr.pairs[k].text) + 1;
            char *clean = malloc(need);
            clean[0] = 0;
            for (size_t k = 0; k < pr.n; k++) {
                if (k > 0)
                    strcat(clean, " ");
                strcat(clean, pr.pairs[k].text);
            }
            if (!clean[0]) {
                free(clean);
                clean = strdup(txt);
            }
            lines_push(&out, &n, t, clean, pr.pairs, pr.n, pr.saw);
            free(clean);
            for (size_t k = 0; k < pr.n; k++)
                free(pr.pairs[k].text);
            free(pr.pairs);
        }
        free(txt);
    }
    qsort(out, n, sizeof(LyricLine), line_cmp);
    *nlines = n;
    return out;
}

static LyricLine *parse_vtt(const char *text, size_t *nlines) {
    LyricLine *out = NULL;
    size_t n = 0;
    int have_start = 0;
    double cur_start = 0;
    char cur_text[8192];
    cur_text[0] = 0;
    const char *p = text;
    while (1) {
        const char *e = strchr(p, '\n');
        size_t ll = e ? (size_t)(e - p) : strlen(p);
        int last = e ? 0 : 1;
        char raw[2048];
        if (ll >= sizeof raw)
            ll = sizeof raw - 1;
        memcpy(raw, p, ll);
        raw[ll] = 0;
        p = e ? e + 1 : p + ll;
        char *line = raw;
        while (*line == ' ' || *line == '\t')
            line++;
        char *le = line + strlen(line);
        while (le > line && (le[-1] == ' ' || le[-1] == '\t' || le[-1] == '\r'))
            *--le = 0;
        if (!line[0]) {
            if (have_start) {
                Pairs pr = split_inline_times(cur_text, cur_start);
                if (pr.n > 0) {
                    size_t need = 1;
                    for (size_t k = 0; k < pr.n; k++)
                        need += strlen(pr.pairs[k].text) + 1;
                    char *clean = malloc(need);
                    clean[0] = 0;
                    for (size_t k = 0; k < pr.n; k++) {
                        if (k > 0)
                            strcat(clean, " ");
                        strcat(clean, pr.pairs[k].text);
                    }
                    lines_push(&out, &n, cur_start, clean, pr.pairs, pr.n, pr.saw);
                    free(clean);
                }
                for (size_t k = 0; k < pr.n; k++)
                    free(pr.pairs[k].text);
                free(pr.pairs);
                cur_text[0] = 0;
            }
            have_start = 0;
            if (last)
                break;
            continue;
        }
        if (!strcmp(line, "WEBVTT") || !strncmp(line, "Kind:", 5) ||
            !strncmp(line, "Language:", 9)) {
            if (last)
                break;
            continue;
        }
        if (!strncmp(line, "NOTE", 4)) {
            have_start = 0;
            cur_text[0] = 0;
            if (last)
                break;
            continue;
        }
        char *arrow = strstr(line, "-->");
        if (arrow) {
            if (have_start) {
                Pairs pr = split_inline_times(cur_text, cur_start);
                if (pr.n > 0) {
                    size_t need = 1;
                    for (size_t k = 0; k < pr.n; k++)
                        need += strlen(pr.pairs[k].text) + 1;
                    char *clean = malloc(need);
                    clean[0] = 0;
                    for (size_t k = 0; k < pr.n; k++) {
                        if (k > 0)
                            strcat(clean, " ");
                        strcat(clean, pr.pairs[k].text);
                    }
                    lines_push(&out, &n, cur_start, clean, pr.pairs, pr.n, pr.saw);
                    free(clean);
                }
                for (size_t k = 0; k < pr.n; k++)
                    free(pr.pairs[k].text);
                free(pr.pairs);
                cur_text[0] = 0;
            }
            *arrow = 0;
            double t;
            if (parse_vtt_time(line, &t)) {
                cur_start = t;
                have_start = 1;
            } else {
                have_start = 0;
            }
            if (last)
                break;
            continue;
        }
        if (have_start) {
            if (cur_text[0])
                strcat(cur_text, " ");
            strcat(cur_text, line);
        }
        if (last)
            break;
    }
    if (have_start && cur_text[0]) {
        Pairs pr = split_inline_times(cur_text, cur_start);
        if (pr.n > 0) {
            size_t need = 1;
            for (size_t k = 0; k < pr.n; k++)
                need += strlen(pr.pairs[k].text) + 1;
            char *clean = malloc(need);
            clean[0] = 0;
            for (size_t k = 0; k < pr.n; k++) {
                if (k > 0)
                    strcat(clean, " ");
                strcat(clean, pr.pairs[k].text);
            }
            lines_push(&out, &n, cur_start, clean, pr.pairs, pr.n, pr.saw);
            free(clean);
        }
        for (size_t k = 0; k < pr.n; k++)
            free(pr.pairs[k].text);
        free(pr.pairs);
    }
    qsort(out, n, sizeof(LyricLine), line_cmp);
    *nlines = n;
    return out;
}

static void normalize_name(const char *s, char *out, size_t n) {
    char lower[2048];
    size_t m = 0;
    for (size_t i = 0; s[i] && m + 1 < sizeof lower; i++)
        lower[m++] = (char)tolower((unsigned char)s[i]);
    lower[m] = 0;
    char canon[2048];
    canonicalize(lower, canon, sizeof canon);
    char tmp[2048];
    size_t t = 0;
    for (size_t i = 0; canon[i] && t + 1 < sizeof tmp; i++) {
        unsigned char c = (unsigned char)canon[i];
        if ((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9'))
            tmp[t++] = (char)c;
        else
            tmp[t++] = ' ';
    }
    tmp[t] = 0;
    char *save = NULL;
    char *tok = strtok_r(tmp, " \t\n\r", &save);
    out[0] = 0;
    while (tok) {
        if (out[0])
            strcat(out, " ");
        if (strlen(out) + strlen(tok) + 1 < n)
            strcat(out, tok);
        tok = strtok_r(NULL, " \t\n\r", &save);
    }
}

static char *cached_folder = NULL;
static char **cached_paths = NULL;
static size_t cached_npaths = 0;
static time_t cached_mtime = 0;
static int cached_have = 0;

static char **list_lrc_files(const char *folder, size_t *n) {
    struct stat st;
    time_t mt = 0;
    if (stat(folder, &st) == 0)
        mt = st.st_mtime;
    if (cached_have && cached_folder && !strcmp(cached_folder, folder) &&
        cached_mtime == mt) {
        *n = cached_npaths;
        return cached_paths;
    }
    free(cached_folder);
    for (size_t i = 0; i < cached_npaths; i++)
        free(cached_paths[i]);
    free(cached_paths);
    cached_paths = NULL;
    cached_npaths = 0;
    cached_folder = strdup(folder);
    cached_mtime = mt;
    cached_have = 1;
    char *dirs[512];
    int depths[512];
    int nd = 0;
    dirs[nd] = strdup(folder);
    depths[nd] = 0;
    nd++;
    while (nd > 0) {
        nd--;
        char *dir = dirs[nd];
        int depth = depths[nd];
        if (depth > 4 || cached_npaths >= 20000) {
            free(dir);
            continue;
        }
        DIR *dp = opendir(dir);
        if (!dp) {
            free(dir);
            continue;
        }
        struct dirent *de;
        while ((de = readdir(dp)) != NULL) {
            if (!strcmp(de->d_name, ".") || !strcmp(de->d_name, ".."))
                continue;
            char full[2048];
            snprintf(full, sizeof full, "%s/%s", dir, de->d_name);
            struct stat s2;
            if (stat(full, &s2) != 0)
                continue;
            if (S_ISDIR(s2.st_mode)) {
                if (nd < 512) {
                    dirs[nd] = strdup(full);
                    depths[nd] = depth + 1;
                    nd++;
                }
            } else {
                const char *dot = strrchr(de->d_name, '.');
                if (dot && !strcasecmp(dot + 1, "lrc")) {
                    cached_paths = realloc(cached_paths,
                                           (cached_npaths + 1) * sizeof(char *));
                    cached_paths[cached_npaths++] = strdup(full);
                }
            }
        }
        closedir(dp);
        free(dir);
    }
    *n = cached_npaths;
    return cached_paths;
}

static unsigned fuzzy_score(const char *query, const char *stem) {
    if (!strcmp(stem, query))
        return 100;
    char q[512], s[512];
    snprintf(q, sizeof q, "%s", query);
    snprintf(s, sizeof s, "%s", stem);
    char *qt[128], *st[128];
    int nq = 0, ns = 0;
    char *save = NULL;
    char *tok = strtok_r(q, " ", &save);
    while (tok && nq < 128) {
        qt[nq++] = tok;
        tok = strtok_r(NULL, " ", &save);
    }
    tok = strtok_r(s, " ", &save);
    while (tok && ns < 128) {
        st[ns++] = tok;
        tok = strtok_r(NULL, " ", &save);
    }
    if (nq == 0)
        return 0;
    int hit = 0;
    for (int i = 0; i < nq; i++) {
        for (int j = 0; j < ns; j++) {
            size_t a = strlen(st[j]), b = strlen(qt[i]);
            if (!strcmp(st[j], qt[i]) ||
                (a >= b && !strncmp(st[j], qt[i], b)) ||
                (b >= a && !strncmp(qt[i], st[j], a))) {
                hit++;
                break;
            }
        }
    }
    unsigned score = (unsigned)hit * 60 / (unsigned)nq;
    if (strstr(stem, query))
        score += 25;
    if (score > 100)
        score = 100;
    return score;
}

static LyricLine *scan_local_lrc(const char *folder, const char *artist,
                                 const char *title, size_t *nlines) {
    *nlines = 0;
    char f[1024];
    if (folder[0] == '~' && folder[1] == '/') {
        const char *home = getenv("HOME");
        if (!home)
            return NULL;
        snprintf(f, sizeof f, "%s/%s", home, folder + 2);
    } else {
        snprintf(f, sizeof f, "%s", folder);
    }
    const char *t = f;
    while (*t == ' ' || *t == '\t')
        t++;
    if (!*t)
        return NULL;
    char q[512];
    {
        char combo[1024];
        snprintf(combo, sizeof combo, "%s %s", artist, title);
        normalize_name(combo, q, sizeof q);
    }
    {
        const char *u = q;
        while (*u == ' ' || *u == '\t')
            u++;
        if (!*u)
            return NULL;
    }
    size_t n = 0;
    char **paths = list_lrc_files(f, &n);
    unsigned best_score = 0;
    const char *best_path = NULL;
    int have_best = 0;
    for (size_t i = 0; i < n; i++) {
        const char *base = strrchr(paths[i], '/');
        base = base ? base + 1 : paths[i];
        const char *dot = strrchr(base, '.');
        size_t sl = dot ? (size_t)(dot - base) : strlen(base);
        char stem[512];
        if (sl >= sizeof stem)
            sl = sizeof stem - 1;
        memcpy(stem, base, sl);
        stem[sl] = 0;
        char ns[512];
        normalize_name(stem, ns, sizeof ns);
        unsigned score = fuzzy_score(q, ns);
        if (score >= 60 && (!have_best || score > best_score)) {
            best_score = score;
            best_path = paths[i];
            have_best = 1;
        }
    }
    if (!have_best)
        return NULL;
    FILE *fp = fopen(best_path, "r");
    if (!fp)
        return NULL;
    fseek(fp, 0, SEEK_END);
    long sz = ftell(fp);
    fseek(fp, 0, SEEK_SET);
    if (sz < 0 || sz > 1000000) {
        fclose(fp);
        return NULL;
    }
    char *text = malloc((size_t)sz + 1);
    if (fread(text, 1, (size_t)sz, fp) != (size_t)sz) {
        free(text);
        fclose(fp);
        return NULL;
    }
    text[sz] = 0;
    fclose(fp);
    LyricLine *lines = parse_lrc(text, nlines);
    free(text);
    if (*nlines == 0) {
        lyric_lines_free(lines, 0);
        return NULL;
    }
    return lines;
}

static void fmt_ts(double t, char *out, size_t n) {
    if (t < 0.0)
        t = 0.0;
    unsigned m = (unsigned)t / 60;
    double s = fmod(t, 60.0);
    snprintf(out, n, "[%02u:%05.2f]", m, s);
}

static char *serialize_lrc(const LyricLine *lines, size_t n) {
    size_t cap = 1024;
    size_t len = 0;
    char *o = malloc(cap);
    o[0] = 0;
    for (size_t i = 0; i < n; i++) {
        char ts[32];
        fmt_ts(lines[i].t, ts, sizeof ts);
        size_t need = strlen(ts) + strlen(lines[i].text) + 2;
        for (size_t k = 0; k < lines[i].nwords; k++)
            need += 16 + strlen(lines[i].words[k].text);
        while (len + need + 1 > cap) {
            cap *= 2;
            o = realloc(o, cap);
        }
        strcat(o, ts);
        len += strlen(ts);
        if (lines[i].nwords == 0) {
            strcat(o, lines[i].text);
            len += strlen(lines[i].text);
        } else {
            for (size_t k = 0; k < lines[i].nwords; k++) {
                char wts[32];
                fmt_ts(lines[i].words[k].t, wts, sizeof wts);
                char tag[40];
                snprintf(tag, sizeof tag, "<%s>", wts + 1);
                tag[strlen(tag) - 1] = 0;
                strcat(o, tag);
                strcat(o, lines[i].words[k].text);
                strcat(o, " ");
                len = strlen(o);
            }
            while (len > 0 && o[len - 1] == ' ')
                o[--len] = 0;
        }
        if (i + 1 < n) {
            strcat(o, "\n");
            len++;
        }
    }
    return o;
}

static int cache_path(const char *key, char *out, size_t n) {
    const char *home = getenv("HOME");
    if (!home)
        return 0;
    char name[128];
    size_t m = 0;
    for (size_t i = 0; key[i] && m + 1 < sizeof name; i++) {
        char c = key[i];
        if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
            (c >= '0' && c <= '9') || c == '-' || c == '_' || c == '.')
            name[m++] = c;
        else
            name[m++] = '_';
    }
    name[m] = 0;
    snprintf(out, n, "%s/.cache/sharkvis/lyrics/%s.lrc", home, name);
    return 1;
}

static uint64_t now_secs(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (uint64_t)ts.tv_sec;
}

static int dead_fresh(const char *path) {
    FILE *f = fopen(path, "r");
    if (!f)
        return 0;
    char buf[64];
    size_t n = fread(buf, 1, sizeof buf - 1, f);
    fclose(f);
    buf[n] = 0;
    char *end;
    unsigned long long when = strtoull(buf, &end, 10);
    if (end == buf)
        return 0;
    return when + 7ull * 24 * 3600 > now_secs();
}

static void norm_dup_str(const char *s, char *out, size_t n) {
    char lower[1024];
    size_t m = 0;
    for (size_t i = 0; s[i] && m + 1 < sizeof lower; i++)
        lower[m++] = (char)tolower((unsigned char)s[i]);
    lower[m] = 0;
    char *save = NULL;
    char *tok = strtok_r(lower, " \t\n\r", &save);
    out[0] = 0;
    while (tok) {
        if (out[0])
            strcat(out, " ");
        if (strlen(out) + strlen(tok) + 1 < n)
            strcat(out, tok);
        tok = strtok_r(NULL, " \t\n\r", &save);
    }
}

static LyricLine *dedup_rolling(LyricLine *lines, size_t n, size_t *nout) {
    LyricLine *out = NULL;
    size_t m = 0;
    for (size_t i = 0; i < n; i++) {
        if (m > 0) {
            char a[1024], b[1024];
            norm_dup_str(out[m - 1].text, a, sizeof a);
            norm_dup_str(lines[i].text, b, sizeof b);
            int dup = 0;
            if (a[0] && b[0]) {
                if (!strcmp(a, b))
                    dup = 1;
                else if ((strstr(a, b) == a || strstr(b, a) == b) &&
                         fabs(lines[i].t - out[m - 1].t) < 2.0)
                    dup = 1;
            }
            if (dup) {
                if (strlen(b) > strlen(a)) {
                    free(out[m - 1].text);
                    out[m - 1].text = strdup(lines[i].text);
                    for (size_t k = 0; k < out[m - 1].nwords; k++)
                        free(out[m - 1].words[k].text);
                    free(out[m - 1].words);
                    out[m - 1].words = lines[i].words;
                    out[m - 1].nwords = lines[i].nwords;
                    lines[i].words = NULL;
                    lines[i].nwords = 0;
                }
                free(lines[i].text);
                for (size_t k = 0; k < lines[i].nwords; k++)
                    free(lines[i].words[k].text);
                free(lines[i].words);
                continue;
            }
        }
        out = realloc(out, (m + 1) * sizeof(LyricLine));
        out[m++] = lines[i];
    }
    free(lines);
    *nout = m;
    return out;
}

typedef struct {
    char *artist;
    char *title;
    double duration;
    char *synced;
    char *plain;
} SearchHit;

static SearchHit *parse_search_hits(const char *body, size_t *n) {
    SearchHit *out = NULL;
    size_t m = 0;
    const char *p = body;
    while ((p = strstr(p, "{\"id\":")) != NULL) {
        p += 6;
        const char *next = strstr(p, "{\"id\":");
        size_t cl = next ? (size_t)(next - p) : strlen(p);
        char *chunk = malloc(cl + 1);
        memcpy(chunk, p, cl);
        chunk[cl] = 0;
        if (strstr(chunk, "\"instrumental\":true")) {
            free(chunk);
            continue;
        }
        char *artist = json_string_scan(chunk, "artistName");
        char *title = json_string_scan(chunk, "trackName");
        if ((!artist || !artist[0]) && (!title || !title[0])) {
            free(artist);
            free(title);
            free(chunk);
            continue;
        }
        double duration = 0.0;
        const char *dp = strstr(chunk, "\"duration\":");
        if (dp) {
            dp += 11;
            char *end;
            double v = strtod(dp, &end);
            if (end != dp)
                duration = v;
        }
        char *synced = json_string_scan(chunk, "syncedLyrics");
        if (synced) {
            char *t = synced;
            while (*t == ' ' || *t == '\t' || *t == '\n' || *t == '\r')
                t++;
            if (!*t) {
                free(synced);
                synced = NULL;
            }
        }
        char *plain = json_string_scan(chunk, "plainLyrics");
        if (plain) {
            char *t = plain;
            while (*t == ' ' || *t == '\t' || *t == '\n' || *t == '\r')
                t++;
            if (!*t) {
                free(plain);
                plain = NULL;
            }
        }
        out = realloc(out, (m + 1) * sizeof(SearchHit));
        out[m].artist = artist ? artist : strdup("");
        out[m].title = title ? title : strdup("");
        out[m].duration = duration;
        out[m].synced = synced;
        out[m].plain = plain;
        m++;
        free(chunk);
    }
    *n = m;
    return out;
}

static void hits_free(SearchHit *h, size_t n) {
    for (size_t i = 0; i < n; i++) {
        free(h[i].artist);
        free(h[i].title);
        free(h[i].synced);
        free(h[i].plain);
    }
    free(h);
}

static void score_hit(const char *qa, const char *qt, double qdur,
                      const SearchHit *hit, size_t *t0, unsigned long long *t1,
                      size_t *t2) {
    char qc[1024], hc[1024];
    {
        char combo[2048];
        snprintf(combo, sizeof combo, "%s %s", qa, qt);
        normalize_name(combo, qc, sizeof qc);
        snprintf(combo, sizeof combo, "%s %s", hit->artist, hit->title);
        normalize_name(combo, hc, sizeof hc);
    }
    size_t text = levenshtein(qc, hc);
    unsigned long long dur = 0;
    if (qdur > 0.0 && hit->duration > 0.0) {
        double d = qdur - hit->duration;
        if (d < 0)
            d = -d;
        dur = (unsigned long long)d;
    }
    *t0 = text / 3;
    *t1 = dur;
    *t2 = text;
}

static LyricLine *best_synced(SearchHit *hits, size_t nh, const char *qa,
                              const char *qt, double qdur, size_t *nlines) {
    *nlines = 0;
    int have = 0;
    size_t bs0 = 0, bs2 = 0;
    unsigned long long bs1 = 0;
    SearchHit *best = NULL;
    for (size_t i = 0; i < nh; i++) {
        if (!hits[i].synced)
            continue;
        size_t s0, s2;
        unsigned long long s1;
        score_hit(qa, qt, qdur, &hits[i], &s0, &s1, &s2);
        if (!have || s0 < bs0 || (s0 == bs0 && (s1 < bs1 || (s1 == bs1 && s2 < bs2)))) {
            have = 1;
            bs0 = s0;
            bs1 = s1;
            bs2 = s2;
            best = &hits[i];
        }
    }
    if (!best)
        return NULL;
    LyricLine *lines = parse_lrc(best->synced, nlines);
    if (*nlines == 0) {
        lyric_lines_free(lines, 0);
        return NULL;
    }
    return lines;
}

static LyricLine *distribute_plain(char **texts, size_t nt, double duration,
                                   size_t *nlines) {
    *nlines = 0;
    size_t m = 0;
    for (size_t i = 0; i < nt; i++) {
        if (texts[i][0])
            m++;
    }
    if (m < 2 || duration < 30.0)
        return NULL;
    LyricLine *out = malloc(m * sizeof(LyricLine));
    double step = duration / (double)m;
    size_t k = 0;
    for (size_t i = 0; i < nt; i++) {
        if (!texts[i][0])
            continue;
        out[k].t = (double)k * step;
        out[k].text = strdup(texts[i]);
        out[k].words = NULL;
        out[k].nwords = 0;
        k++;
    }
    *nlines = m;
    return out;
}

static LyricLine *best_plain(SearchHit *hits, size_t nh, const char *qa,
                             const char *qt, double qdur, size_t *nlines) {
    *nlines = 0;
    int have = 0;
    size_t bs0 = 0, bs2 = 0;
    unsigned long long bs1 = 0;
    SearchHit *best = NULL;
    for (size_t i = 0; i < nh; i++) {
        if (!hits[i].plain)
            continue;
        size_t s0, s2;
        unsigned long long s1;
        score_hit(qa, qt, qdur, &hits[i], &s0, &s1, &s2);
        if (!have || s0 < bs0 || (s0 == bs0 && (s1 < bs1 || (s1 == bs1 && s2 < bs2)))) {
            have = 1;
            bs0 = s0;
            bs1 = s1;
            bs2 = s2;
            best = &hits[i];
        }
    }
    if (!best)
        return NULL;
    char *copy = strdup(best->plain);
    char **texts = NULL;
    size_t nt = 0;
    char *save = NULL;
    char *line = strtok_r(copy, "\n", &save);
    while (line) {
        while (*line == ' ' || *line == '\t' || *line == '\r')
            line++;
        char *e = line + strlen(line);
        while (e > line && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\r'))
            *--e = 0;
        texts = realloc(texts, (nt + 1) * sizeof(char *));
        texts[nt++] = strdup(line);
        line = strtok_r(NULL, "\n", &save);
    }
    LyricLine *out = distribute_plain(texts, nt, qdur, nlines);
    for (size_t i = 0; i < nt; i++)
        free(texts[i]);
    free(texts);
    free(copy);
    return out;
}

static SearchHit *lrclib_search_hits(const char *artist, const char *title, size_t *n) {
    *n = 0;
    char sa[512], st[512];
    sanitize_query(artist, sa, sizeof sa);
    sanitize_query(title, st, sizeof st);
    if (!sa[0] && !st[0])
        return NULL;
    char ea[1024], et[1024];
    url_encode(sa, ea, sizeof ea);
    url_encode(st, et, sizeof et);
    char url[2300];
    snprintf(url, sizeof url, "https://lrclib.net/api/search?q=%s%%20%s", ea, et);
    const char *args[] = {"-fsSL", "-m", "15", url, NULL};
    char *body = cmd_out("curl", args, 20000);
    if (!body)
        return NULL;
    SearchHit *hits = parse_search_hits(body, n);
    free(body);
    return hits;
}

static LyricLine *fetch_synced(const char *artist, const char *title, size_t *nlines,
                               int *empty_ok) {
    *nlines = 0;
    *empty_ok = 0;
    char ea[1024], et[1024];
    url_encode(artist, ea, sizeof ea);
    url_encode(title, et, sizeof et);
    char url[2300];
    snprintf(url, sizeof url,
             "https://lrclib.net/api/get?artist_name=%s&track_name=%s", ea, et);
    const char *args[] = {"-fsSL", "-m", "15", url, NULL};
    char *body = cmd_out("curl", args, 20000);
    if (!body)
        return NULL;
    if (strstr(body, "\"instrumental\":true")) {
        free(body);
        *empty_ok = 1;
        return NULL;
    }
    char *synced = json_string_scan(body, "syncedLyrics");
    free(body);
    if (!synced)
        return NULL;
    char *t = synced;
    while (*t == ' ' || *t == '\t' || *t == '\n' || *t == '\r')
        t++;
    if (!*t) {
        free(synced);
        *empty_ok = 1;
        return NULL;
    }
    LyricLine *lines = parse_lrc(synced, nlines);
    free(synced);
    return lines;
}

static LyricLine *download_subs(const char *target, int *have_match,
                                 const char *match_filter, size_t *nlines) {
    *nlines = 0;
    const char *home = getenv("HOME");
    char dir[1024];
    if (home)
        snprintf(dir, sizeof dir, "%s/.cache/sharkvis/lyrics", home);
    else
        snprintf(dir, sizeof dir, "/tmp/sharkvis-lyrics");
    char cur[1024];
    size_t n = 0;
    if (dir[0] == '/')
        cur[n++] = '/';
    char *save = NULL;
    char *dup = strdup(dir + (dir[0] == '/' ? 1 : 0));
    char *tok = strtok_r(dup, "/", &save);
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
    free(dup);
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    unsigned long long ms = (unsigned long long)ts.tv_sec * 1000 +
                            (unsigned long long)ts.tv_nsec / 1000000;
    char stem[64];
    snprintf(stem, sizeof stem, "subs_%llu", ms);
    char out_tpl[1150];
    snprintf(out_tpl, sizeof out_tpl, "%s/%s.%%(id)s.%%(ext)s", dir, stem);
    const char *args[24];
    int na = 0;
    args[na++] = "--skip-download";
    args[na++] = "--no-playlist";
    args[na++] = "--socket-timeout";
    args[na++] = "15";
    args[na++] = "--write-auto-subs";
    args[na++] = "--write-subs";
    args[na++] = "--sub-langs";
    args[na++] = "en*";
    args[na++] = "--sub-format";
    args[na++] = "vtt/best";
    args[na++] = "-o";
    args[na++] = out_tpl;
    if (have_match && match_filter) {
        args[na++] = "--match-filter";
        args[na++] = match_filter;
    }
    args[na++] = target;
    args[na++] = NULL;
    char *o = cmd_out("yt-dlp", args, 90000);
    free(o);
    LyricLine *best = NULL;
    size_t best_n = 0;
    DIR *dp = opendir(dir);
    if (dp) {
        struct dirent *de;
        size_t sl = strlen(stem);
        while ((de = readdir(dp)) != NULL) {
            if (strncmp(de->d_name, stem, sl) != 0)
                continue;
            size_t dl = strlen(de->d_name);
            if (dl < 4 || strcmp(de->d_name + dl - 4, ".vtt") != 0)
                continue;
            char p[2048];
            snprintf(p, sizeof p, "%s/%s", dir, de->d_name);
            FILE *f = fopen(p, "r");
            if (!f) {
                remove(p);
                continue;
            }
            fseek(f, 0, SEEK_END);
            long sz = ftell(f);
            fseek(f, 0, SEEK_SET);
            if (sz < 0 || sz > 2000000) {
                fclose(f);
                remove(p);
                continue;
            }
            char *text = malloc((size_t)sz + 1);
            if (fread(text, 1, (size_t)sz, f) != (size_t)sz) {
                free(text);
                fclose(f);
                remove(p);
                continue;
            }
            text[sz] = 0;
            fclose(f);
            size_t vn = 0;
            LyricLine *lines = parse_vtt(text, &vn);
            free(text);
            size_t dn = 0;
            lines = dedup_rolling(lines, vn, &dn);
            if (dn > best_n) {
                lyric_lines_free(best, best_n);
                best = lines;
                best_n = dn;
            } else {
                lyric_lines_free(lines, dn);
            }
            remove(p);
        }
        closedir(dp);
    }
    *nlines = best_n;
    return best;
}

static LyricLine *fetch_subs(const char *url, size_t *nlines) {
    *nlines = 0;
    if (!strstr(url, "youtube.com/watch") && !strstr(url, "youtu.be/"))
        return NULL;
    return download_subs(url, NULL, NULL, nlines);
}

static LyricLine *fetch_search_subs(const char *artist, const char *title,
                                    double duration, size_t *nlines) {
    *nlines = 0;
    char ta[512], tt[512];
    {
        const char *s = artist;
        while (*s == ' ' || *s == '\t')
            s++;
        snprintf(ta, sizeof ta, "%s", s);
        s = title;
        while (*s == ' ' || *s == '\t')
            s++;
        snprintf(tt, sizeof tt, "%s", s);
    }
    char query[1100];
    snprintf(query, sizeof query, "ytsearch3:%s %s", ta, tt);
    if (duration > 30.0) {
        double lo = duration - 45.0;
        if (lo < 15.0)
            lo = 15.0;
        char filter[128];
        snprintf(filter, sizeof filter, "duration > %u & duration < %u",
                 (unsigned)lo, (unsigned)(duration + 90.0));
        int hm = 1;
        return download_subs(query, &hm, filter, nlines);
    }
    return download_subs(query, NULL, NULL, nlines);
}

static long long quality_bonus(const LyricLine *lines, size_t n, double duration) {
    if (n == 0)
        return -10000;
    long long s = (long long)(n < 60 ? n : 60);
    for (size_t i = 0; i < n; i++) {
        if (lines[i].nwords > 0) {
            s += 40;
            break;
        }
    }
    if (duration >= 30.0) {
        double last = 0.0;
        for (size_t i = 0; i < n; i++) {
            if (lines[i].t > last)
                last = lines[i].t;
        }
        double cov = last / duration;
        if (cov >= 0.5 && cov <= 1.1)
            s += 20;
        else if (cov < 0.2)
            s -= 30;
    }
    if (n >= 4) {
        double mean = 0.0;
        for (size_t i = 0; i + 1 < n; i++) {
            double g = lines[i + 1].t - lines[i].t;
            if (g < 0.0)
                g = 0.0;
            mean += g;
        }
        mean /= (double)(n - 1);
        if (mean > 0.0) {
            double var = 0.0;
            for (size_t i = 0; i + 1 < n; i++) {
                double g = lines[i + 1].t - lines[i].t;
                if (g < 0.0)
                    g = 0.0;
                var += (g - mean) * (g - mean);
            }
            var /= (double)(n - 1);
            if (sqrt(var) / mean < 0.03)
                s -= 25;
        }
    }
    return s;
}

static LyricLine *fetch_search_synced(const char *artist, const char *title,
                                      double duration, size_t *nlines) {
    size_t nh = 0;
    SearchHit *hits = lrclib_search_hits(artist, title, &nh);
    LyricLine *out = best_synced(hits, nh, artist, title, duration, nlines);
    hits_free(hits, nh);
    return out;
}

static LyricLine *fetch_search_plain(const char *artist, const char *title,
                                     double duration, size_t *nlines) {
    *nlines = 0;
    if (duration < 30.0)
        return NULL;
    size_t nh = 0;
    SearchHit *hits = lrclib_search_hits(artist, title, &nh);
    LyricLine *out = best_plain(hits, nh, artist, title, duration, nlines);
    hits_free(hits, nh);
    return out;
}

typedef struct {
    LyricLine *lines;
    size_t n;
    int contacted;
} FetchResult;

static FetchResult fetch_auto(const char *artist, const char *title, double duration) {
    FetchResult fr = {0};
    size_t en = 0;
    int empty_ok = 0;
    LyricLine *exact = fetch_synced(artist, title, &en, &empty_ok);
    if (exact || empty_ok)
        fr.contacted = 1;
    if (en > 0 && exact && 100 + quality_bonus(exact, en, duration) >= 160) {
        fr.lines = exact;
        fr.n = en;
        return fr;
    }
    long long best_score = -10000;
    if (en > 0 && exact)
        best_score = 100 + quality_bonus(exact, en, duration);
    else {
        lyric_lines_free(exact, en);
        exact = NULL;
        en = 0;
    }
    fr.lines = exact;
    fr.n = en;
    {
        size_t mn = 0;
        LyricLine *ml = musix_fetch(artist, title, duration, &mn);
        if (ml) {
            fr.contacted = 1;
            if (mn > 0) {
                long long s = 90 + quality_bonus(ml, mn, duration);
                if (s > best_score) {
                    best_score = s;
                    lyric_lines_free(fr.lines, fr.n);
                    fr.lines = ml;
                    fr.n = mn;
                } else {
                    lyric_lines_free(ml, mn);
                }
            } else {
                lyric_lines_free(ml, mn);
            }
        }
    }
    {
        size_t sn = 0;
        LyricLine *sl = fetch_search_synced(artist, title, duration, &sn);
        if (sl) {
            fr.contacted = 1;
            if (sn > 0) {
                long long s = 60 + quality_bonus(sl, sn, duration);
                if (s > best_score) {
                    best_score = s;
                    lyric_lines_free(fr.lines, fr.n);
                    fr.lines = sl;
                    fr.n = sn;
                } else {
                    lyric_lines_free(sl, sn);
                }
            } else {
                lyric_lines_free(sl, sn);
            }
        }
    }
    return fr;
}

static FetchResult fetch_lyrics(const char *artist, const char *title,
                                const char *url, double duration,
                                const FetchOpts *opts) {
    FetchResult fr = {0};
    const char *lf = opts->local_folder;
    while (*lf == ' ' || *lf == '\t')
        lf++;
    if (*lf) {
        size_t ln = 0;
        LyricLine *ll = scan_local_lrc(opts->local_folder, artist, title, &ln);
        if (ln > 0 && ll) {
            fr.lines = ll;
            fr.n = ln;
            fr.contacted = 1;
            return fr;
        }
        lyric_lines_free(ll, ln);
    }
    if (!strcmp(opts->provider, "auto")) {
        FetchResult r = fetch_auto(artist, title, duration);
        if (r.n > 0)
            return r;
        fr.contacted = r.contacted;
        lyric_lines_free(r.lines, r.n);
    } else {
        const char *order[2];
        if (!strcmp(opts->provider, "musixmatch")) {
            order[0] = "musixmatch";
            order[1] = "lrclib";
        } else {
            order[0] = "lrclib";
            order[1] = "musixmatch";
        }
        for (int oi = 0; oi < 2; oi++) {
            LyricLine *hit = NULL;
            size_t hn = 0;
            if (!strcmp(order[oi], "musixmatch")) {
                hit = musix_fetch(artist, title, duration, &hn);
            } else {
                int empty_ok = 0;
                hit = fetch_synced(artist, title, &hn, &empty_ok);
                if ((hn == 0 || !hit) && !empty_ok) {
                    lyric_lines_free(hit, hn);
                    hit = fetch_search_synced(artist, title, duration, &hn);
                } else if (hn > 0 && hit) {
                    FetchResult rr = {0};
                    rr.lines = hit;
                    rr.n = hn;
                    rr.contacted = 1;
                    return rr;
                } else {
                    lyric_lines_free(hit, hn);
                    hit = NULL;
                    hn = 0;
                }
            }
            if (hit)
                fr.contacted = 1;
            if (hn > 0 && hit) {
                fr.lines = hit;
                fr.n = hn;
                fr.contacted = 1;
                return fr;
            }
            lyric_lines_free(hit, hn);
        }
    }
    if (url && url[0]) {
        size_t sn = 0;
        LyricLine *subs = fetch_subs(url, &sn);
        if (sn > 0 && subs) {
            fr.lines = subs;
            fr.n = sn;
            fr.contacted = 1;
            return fr;
        }
        lyric_lines_free(subs, sn);
    }
    {
        const char *a = artist;
        while (*a == ' ' || *a == '\t')
            a++;
        const char *t = title;
        while (*t == ' ' || *t == '\t')
            t++;
        if (*a && *t) {
            size_t sn = 0;
            LyricLine *subs = fetch_search_subs(artist, title, duration, &sn);
            if (sn > 0 && subs) {
                fr.lines = subs;
                fr.n = sn;
                fr.contacted = 1;
                return fr;
            }
            lyric_lines_free(subs, sn);
            size_t pn = 0;
            LyricLine *pl = fetch_search_plain(artist, title, duration, &pn);
            if (pn > 0 && pl) {
                fr.lines = pl;
                fr.n = pn;
                fr.contacted = 1;
                return fr;
            }
            lyric_lines_free(pl, pn);
        }
    }
    return fr;
}

struct LyricWorker {
    char *key;
    LyricLine *lines;
    size_t nlines;
    pthread_mutex_t mu;
    int fetching;
    unsigned gen;
    char *res_key;
    LyricLine *res_lines;
    size_t res_n;
    int res_ready;
    unsigned res_gen;
    int have_attempt;
    uint64_t attempt_ms;
    char **dead;
    size_t ndead;
    size_t deadcap;
    double last_pos;
    double p0;
    double p1;
    uint64_t t0ms;
    uint64_t t1ms;
    char *last_track;
    char *man_a;
    char *man_t;
    int have_manual;
    long offset_ms;
    int follow;
    int have_frozen;
    double frozen;
};

LyricWorker *lyric_new(void) {
    LyricWorker *w = calloc(1, sizeof *w);
    if (!w)
        return NULL;
    w->key = strdup("");
    w->last_track = strdup("");
    pthread_mutex_init(&w->mu, NULL);
    uint64_t now = sv_now_ms();
    w->t0ms = now;
    w->t1ms = now;
    w->follow = 1;
    return w;
}

void lyric_free(LyricWorker *w) {
    if (!w)
        return;
    lyric_lines_free(w->lines, w->nlines);
    lyric_lines_free(w->res_lines, w->res_n);
    free(w->key);
    free(w->res_key);
    for (size_t i = 0; i < w->ndead; i++)
        free(w->dead[i]);
    free(w->dead);
    free(w->last_track);
    free(w->man_a);
    free(w->man_t);
    pthread_mutex_destroy(&w->mu);
    free(w);
}

void lyric_set_offset_ms(LyricWorker *w, long ms) {
    if (ms < -10000)
        ms = -10000;
    if (ms > 10000)
        ms = 10000;
    w->offset_ms = ms;
}

void lyric_set_follow(LyricWorker *w, int on, double pos) {
    w->follow = on;
    if (on) {
        w->have_frozen = 0;
    } else if (pos == pos && pos >= 0.0) {
        w->have_frozen = 1;
        w->frozen = pos;
    }
}

int lyric_following(const LyricWorker *w) {
    return w->follow;
}

void lyric_search_override(LyricWorker *w, const char *artist, const char *title) {
    while (*artist == ' ' || *artist == '\t')
        artist++;
    while (*title == ' ' || *title == '\t')
        title++;
    char a[512], t[512];
    snprintf(a, sizeof a, "%s", artist);
    snprintf(t, sizeof t, "%s", title);
    char *e = a + strlen(a);
    while (e > a && (e[-1] == ' ' || e[-1] == '\t'))
        *--e = 0;
    e = t + strlen(t);
    while (e > t && (e[-1] == ' ' || e[-1] == '\t'))
        *--e = 0;
    if (!a[0] && !t[0])
        return;
    free(w->man_a);
    free(w->man_t);
    w->man_a = strdup(a);
    w->man_t = strdup(t);
    w->have_manual = 1;
    lyric_lines_free(w->lines, w->nlines);
    w->lines = NULL;
    w->nlines = 0;
    pthread_mutex_lock(&w->mu);
    w->fetching = 0;
    w->gen++;
    w->res_ready = 0;
    lyric_lines_free(w->res_lines, w->res_n);
    w->res_lines = NULL;
    w->res_n = 0;
    free(w->res_key);
    w->res_key = NULL;
    pthread_mutex_unlock(&w->mu);
    w->have_attempt = 0;
    free(w->key);
    w->key = strdup("");
}

void lyric_force_reload(LyricWorker *w) {
    lyric_lines_free(w->lines, w->nlines);
    w->lines = NULL;
    w->nlines = 0;
    pthread_mutex_lock(&w->mu);
    w->fetching = 0;
    w->gen++;
    w->res_ready = 0;
    lyric_lines_free(w->res_lines, w->res_n);
    w->res_lines = NULL;
    w->res_n = 0;
    free(w->res_key);
    w->res_key = NULL;
    pthread_mutex_unlock(&w->mu);
    w->have_attempt = 0;
    for (size_t i = 0; i < w->ndead; i++) {
        if (!strcmp(w->dead[i], w->key)) {
            free(w->dead[i]);
            w->dead[i] = w->dead[w->ndead - 1];
            w->ndead--;
            break;
        }
    }
    if (w->key[0]) {
        char path[1100];
        if (cache_path(w->key, path, sizeof path)) {
            remove(path);
            char dp[1150];
            snprintf(dp, sizeof dp, "%s.none", path);
            remove(dp);
        }
    }
}

void lyric_poke(LyricWorker *w) {
    w->have_attempt = 0;
}

int lyric_loading(const LyricWorker *w) {
    pthread_mutex_lock((pthread_mutex_t *)&w->mu);
    int fetching = w->fetching;
    pthread_mutex_unlock((pthread_mutex_t *)&w->mu);
    if (!fetching || w->nlines > 0)
        return 0;
    if (!w->have_attempt)
        return 0;
    return sv_now_ms() - w->attempt_ms > 1500;
}

void lyric_reset(LyricWorker *w) {
    free(w->key);
    w->key = strdup("");
    lyric_lines_free(w->lines, w->nlines);
    w->lines = NULL;
    w->nlines = 0;
    pthread_mutex_lock(&w->mu);
    w->fetching = 0;
    w->gen++;
    w->res_ready = 0;
    lyric_lines_free(w->res_lines, w->res_n);
    w->res_lines = NULL;
    w->res_n = 0;
    free(w->res_key);
    w->res_key = NULL;
    pthread_mutex_unlock(&w->mu);
    w->have_attempt = 0;
    for (size_t i = 0; i < w->ndead; i++)
        free(w->dead[i]);
    w->ndead = 0;
    w->have_frozen = 0;
}

static int dead_contains(LyricWorker *w, const char *key) {
    for (size_t i = 0; i < w->ndead; i++) {
        if (!strcmp(w->dead[i], key))
            return 1;
    }
    return 0;
}

static void dead_insert(LyricWorker *w, const char *key) {
    if (dead_contains(w, key))
        return;
    if (w->ndead >= w->deadcap) {
        w->deadcap = w->deadcap ? w->deadcap * 2 : 16;
        w->dead = realloc(w->dead, w->deadcap * sizeof(char *));
    }
    w->dead[w->ndead++] = strdup(key);
}

typedef struct {
    LyricWorker *w;
    unsigned gen;
    char *key;
    char *artist;
    char *title;
    char *url;
    double duration;
    char *path;
    char *dpath;
    FetchOpts opts;
} FetchArgs;

static void mkdir_for(const char *path) {
    const char *slash = strrchr(path, '/');
    if (!slash)
        return;
    size_t dl = (size_t)(slash - path);
    char dir[1100];
    if (dl >= sizeof dir)
        dl = sizeof dir - 1;
    memcpy(dir, path, dl);
    dir[dl] = 0;
    char cur[1100];
    size_t n = 0;
    if (dir[0] == '/')
        cur[n++] = '/';
    char *save = NULL;
    char *dup = strdup(dir + (dir[0] == '/' ? 1 : 0));
    char *tok = strtok_r(dup, "/", &save);
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
    free(dup);
}

static void *fetch_thread(void *arg) {
    FetchArgs *a = arg;
    FetchResult fr = fetch_lyrics(a->artist, a->title, a->url, a->duration, &a->opts);
    if (fr.n > 0) {
        mkdir_for(a->path);
        char *ser = serialize_lrc(fr.lines, fr.n);
        FILE *f = fopen(a->path, "w");
        if (f) {
            fwrite(ser, 1, strlen(ser), f);
            fclose(f);
        }
        free(ser);
    } else if (fr.contacted) {
        FILE *f = fopen(a->dpath, "w");
        if (f) {
            fprintf(f, "%llu", (unsigned long long)now_secs());
            fclose(f);
        }
    }
    pthread_mutex_lock(&a->w->mu);
    if (a->gen == a->w->gen) {
        lyric_lines_free(a->w->res_lines, a->w->res_n);
        free(a->w->res_key);
        a->w->res_lines = fr.lines;
        a->w->res_n = fr.n;
        a->w->res_key = a->key;
        a->key = NULL;
        a->w->res_ready = 1;
        a->w->res_gen = a->gen;
        a->w->fetching = 0;
    } else {
        lyric_lines_free(fr.lines, fr.n);
    }
    pthread_mutex_unlock(&a->w->mu);
    free(a->key);
    free(a->artist);
    free(a->title);
    free(a->url);
    free(a->path);
    free(a->dpath);
    free(a);
    return NULL;
}

static void update_meta(LyricWorker *w, const Track *track, const FetchOpts *opts) {
    pthread_mutex_lock(&w->mu);
    if (w->res_ready && w->res_gen == w->gen) {
        w->res_ready = 0;
        w->fetching = 0;
        char *rk = w->res_key;
        w->res_key = NULL;
        LyricLine *rl = w->res_lines;
        size_t rn = w->res_n;
        w->res_lines = NULL;
        w->res_n = 0;
        pthread_mutex_unlock(&w->mu);
        if (rn == 0)
            dead_insert(w, rk);
        if (!strcmp(rk, w->key)) {
            lyric_lines_free(w->lines, w->nlines);
            w->lines = rl;
            w->nlines = rn;
        } else {
            lyric_lines_free(rl, rn);
        }
        free(rk);
    } else {
        pthread_mutex_unlock(&w->mu);
    }
    char tkey[2100];
    track_key(track, tkey, sizeof tkey);
    if (tkey[0] && w->last_track[0] && strcmp(tkey, w->last_track)) {
        free(w->last_track);
        w->last_track = strdup(tkey);
        free(w->man_a);
        free(w->man_t);
        w->man_a = NULL;
        w->man_t = NULL;
        w->have_manual = 0;
    } else if (tkey[0]) {
        free(w->last_track);
        w->last_track = strdup(tkey);
    }
    char key[2300];
    char artist[512], title[512];
    if (w->have_manual) {
        snprintf(key, sizeof key, "manual|%s|%s", w->man_a, w->man_t);
        snprintf(artist, sizeof artist, "%s", w->man_a);
        snprintf(title, sizeof title, "%s", w->man_t);
    } else {
        if (!tkey[0])
            return;
        snprintf(key, sizeof key, "%s", tkey);
        snprintf(artist, sizeof artist, "%s", track->artist);
        snprintf(title, sizeof title, "%s", track->title);
    }
    if (strcmp(key, w->key)) {
        free(w->key);
        w->key = strdup(key);
        lyric_lines_free(w->lines, w->nlines);
        w->lines = NULL;
        w->nlines = 0;
        pthread_mutex_lock(&w->mu);
        w->fetching = 0;
        w->gen++;
        w->res_ready = 0;
        lyric_lines_free(w->res_lines, w->res_n);
        w->res_lines = NULL;
        w->res_n = 0;
        free(w->res_key);
        w->res_key = NULL;
        pthread_mutex_unlock(&w->mu);
        w->have_attempt = 0;
    }
    pthread_mutex_lock(&w->mu);
    int fetching = w->fetching;
    pthread_mutex_unlock(&w->mu);
    if (w->nlines > 0 || fetching)
        return;
    if (w->have_attempt && sv_now_ms() - w->attempt_ms < 60000)
        return;
    char path[1100];
    if (!cache_path(key, path, sizeof path))
        return;
    FILE *cf = fopen(path, "r");
    if (cf) {
        fseek(cf, 0, SEEK_END);
        long sz = ftell(cf);
        fseek(cf, 0, SEEK_SET);
        if (sz > 0 && sz < 1000000) {
            char *text = malloc((size_t)sz + 1);
            if (fread(text, 1, (size_t)sz, cf) == (size_t)sz) {
                text[sz] = 0;
                char *t = text;
                while (*t == ' ' || *t == '\t' || *t == '\n' || *t == '\r')
                    t++;
                if (*t) {
                    size_t pn = 0;
                    LyricLine *pl = parse_lrc(text, &pn);
                    size_t dn = 0;
                    pl = dedup_rolling(pl, pn, &dn);
                    if (dn > 0) {
                        w->lines = pl;
                        w->nlines = dn;
                        free(text);
                        fclose(cf);
                        return;
                    }
                    lyric_lines_free(pl, dn);
                } else {
                    remove(path);
                }
            }
            free(text);
        }
        fclose(cf);
    }
    if (!w->have_manual) {
        if (dead_contains(w, key))
            return;
        char dpath[1150];
        snprintf(dpath, sizeof dpath, "%s.none", path);
        if (dead_fresh(dpath)) {
            dead_insert(w, key);
            return;
        }
        remove(dpath);
    }
    w->have_attempt = 1;
    w->attempt_ms = sv_now_ms();
    FetchArgs *a = calloc(1, sizeof *a);
    a->w = w;
    pthread_mutex_lock(&w->mu);
    w->gen++;
    a->gen = w->gen;
    w->fetching = 1;
    pthread_mutex_unlock(&w->mu);
    a->key = strdup(key);
    a->artist = strdup(artist);
    a->title = strdup(title);
    a->url = strdup(track->url);
    a->duration = track->duration;
    a->path = strdup(path);
    {
        char dpath[1150];
        snprintf(dpath, sizeof dpath, "%s.none", path);
        a->dpath = strdup(dpath);
    }
    a->opts = *opts;
    pthread_t th;
    pthread_attr_t at;
    pthread_attr_init(&at);
    pthread_attr_setdetachstate(&at, PTHREAD_CREATE_DETACHED);
    if (pthread_create(&th, &at, fetch_thread, a) != 0) {
        pthread_mutex_lock(&w->mu);
        w->fetching = 0;
        pthread_mutex_unlock(&w->mu);
        free(a->key);
        free(a->artist);
        free(a->title);
        free(a->url);
        free(a->path);
        free(a->dpath);
        free(a);
    }
    pthread_attr_destroy(&at);
}

void lyric_update(LyricWorker *w, const Track *track, const FetchOpts *opts) {
    update_meta(w, track, opts);
    if (track->present) {
        w->last_pos = track->position;
        if (w->p1 == 0.0 && track->position > 0.0)
            lyric_update_pos(w, track->position);
    }
}

void lyric_update_pos(LyricWorker *w, double pos) {
    if (pos == pos && pos >= 0.0) {
        w->p0 = w->p1;
        w->t0ms = w->t1ms;
        w->p1 = pos;
        w->t1ms = sv_now_ms();
        w->last_pos = pos;
    }
}

static double live_position(const LyricWorker *w) {
    double p0 = w->p0, p1 = w->p1;
    uint64_t t0 = w->t0ms, t1 = w->t1ms;
    if (p1 <= p0 || t1 <= t0)
        return p1;
    double rate = (p1 - p0) / ((double)(t1 - t0) / 1000.0);
    if (!(rate > 0.0) || !(rate < 4.0))
        return p1;
    uint64_t now = sv_now_ms();
    if (now < t1)
        return p1;
    double dt = (double)(now - t1) / 1000.0;
    if (dt > 2.0)
        return p1;
    return p1 + rate * dt;
}

static double live_pos(const LyricWorker *w, const Track *track) {
    if (w->nlines == 0) {
        if (track->present) {
            char tk[2100];
            track_key(track, tk, sizeof tk);
            if (!strcmp(tk, w->key))
                return track->position;
        }
        return w->last_pos;
    }
    return live_position(w);
}

static double cur_pos(const LyricWorker *w, const Track *track) {
    double pos;
    if (!w->follow)
        pos = w->have_frozen ? w->frozen : w->last_pos;
    else
        pos = live_pos(w, track);
    return pos + (double)w->offset_ms / 1000.0;
}

static int text_nonempty(const char *s) {
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')
        s++;
    return *s != 0;
}

LyricDisplay *lyric_display_lines(LyricWorker *w, const Track *track, size_t *n) {
    *n = 0;
    LyricDisplay *out = malloc(sizeof(LyricDisplay));
    if (!track->present || w->nlines == 0) {
        out[0].text = strdup("No Lyrics");
        out[0].cur = 0;
        *n = 1;
        return out;
    }
    double pos = cur_pos(w, track);
    const char *current = NULL;
    for (size_t i = 0; i < w->nlines; i++) {
        if (w->lines[i].t <= pos && text_nonempty(w->lines[i].text))
            current = w->lines[i].text;
    }
    if (current)
        out[0].text = strdup(current);
    else
        out[0].text = strdup("");
    out[0].cur = 1;
    *n = 1;
    return out;
}

LyricDisplay *lyric_display_context(LyricWorker *w, const Track *track, size_t *n) {
    *n = 0;
    LyricDisplay *out = NULL;
    if (!track->present || w->nlines == 0) {
        out = malloc(sizeof(LyricDisplay));
        out[0].text = strdup("No Lyrics");
        out[0].cur = 0;
        *n = 1;
        return out;
    }
    double pos = cur_pos(w, track);
    int idx = -1;
    for (size_t i = 0; i < w->nlines; i++) {
        if (w->lines[i].t <= pos && text_nonempty(w->lines[i].text))
            idx = (int)i;
    }
    size_t cap = 4;
    out = malloc(cap * sizeof(LyricDisplay));
    if (idx >= 0) {
        int p = idx;
        while (p > 0) {
            p--;
            if (text_nonempty(w->lines[p].text)) {
                out[*n].text = strdup(w->lines[p].text);
                out[*n].cur = 0;
                (*n)++;
                break;
            }
        }
        out[*n].text = strdup(w->lines[(size_t)idx].text);
        out[*n].cur = 1;
        (*n)++;
        for (size_t i = (size_t)idx + 1; i < w->nlines; i++) {
            if (text_nonempty(w->lines[i].text)) {
                out[*n].text = strdup(w->lines[i].text);
                out[*n].cur = 0;
                (*n)++;
                break;
            }
        }
    } else {
        out[*n].text = strdup("");
        out[*n].cur = 1;
        (*n)++;
        for (size_t i = 0; i < w->nlines; i++) {
            if (text_nonempty(w->lines[i].text)) {
                out[*n].text = strdup(w->lines[i].text);
                out[*n].cur = 0;
                (*n)++;
                break;
            }
        }
    }
    return out;
}
