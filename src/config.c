#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>

#include "config.h"

const char *SV_PALETTE_NAMES[] = {
    "white", "red", "green", "blue", "yellow", "magenta", "cyan",
    "orange", "purple", "lime", "teal", "pink", "gray", "black"
};

const char *SV_PALETTE_HEX[] = {
    "ffffff", "ff0000", "00ff00", "0000ff", "ffff00", "ff00ff", "00ffff",
    "ff8800", "8800ff", "88ff00", "00ff88", "ff0088", "888888", "000000"
};

int SV_PALETTE_COUNT = 14;

const char SV_DEFAULT_CHARS[] = "\xe2\x96\x81\xe2\x96\x82\xe2\x96\x83\xe2\x96\x84"
                                "\xe2\x96\x85\xe2\x96\x86\xe2\x96\x87\xe2\x96\x88";
unsigned SV_DEFAULT_CHARS_LEN = 24;

void config_default(SvConfig *c) {
    memset(c, 0, sizeof *c);
    c->bars = 0;
    c->bar_width = 2;
    c->bar_spacing = 1;
    c->framerate = 60;
    c->sensitivity = 100.0;
    c->autosens = 1;
    c->lower_cutoff = 50;
    c->higher_cutoff = 8000;
    c->noise_reduction = 0.2;
    strcpy(c->source, "auto");
    c->sample_rate = 48000;
    c->channels = 2;
    c->color_256 = 0;
    strcpy(c->gradient_low, "white");
    strcpy(c->gradient_high, "white");
    strcpy(c->mode, "bars");
    strcpy(c->text_align, "center");
    c->text_size = 1;
    strcpy(c->text_style, "big ahh");
    strcpy(c->provider, "auto");
    c->lyric_offset_ms = 0;
    c->lyrics_folder[0] = 0;
    c->mpris_players[0] = 0;
    memcpy(c->chars, SV_DEFAULT_CHARS, SV_DEFAULT_CHARS_LEN);
    c->chars_len = SV_DEFAULT_CHARS_LEN;
}

static int ci_eq(const char *a, const char *b) {
    while (*a && *b) {
        if (tolower((unsigned char)*a) != tolower((unsigned char)*b))
            return 0;
        a++;
        b++;
    }
    return *a == *b;
}

static void ci_lower(const char *s, char *out, size_t n) {
    size_t i = 0;
    while (s[i] && i + 1 < n) {
        out[i] = (char)tolower((unsigned char)s[i]);
        i++;
    }
    out[i] = 0;
}

int palette_ansi(const char *name) {
    char l[64];
    ci_lower(name, l, sizeof l);
    if (!strcmp(l, "black"))
        return 30;
    if (!strcmp(l, "red"))
        return 31;
    if (!strcmp(l, "green"))
        return 32;
    if (!strcmp(l, "yellow"))
        return 33;
    if (!strcmp(l, "blue"))
        return 34;
    if (!strcmp(l, "magenta") || !strcmp(l, "purple"))
        return 35;
    if (!strcmp(l, "cyan"))
        return 36;
    if (!strcmp(l, "white"))
        return 37;
    if (!strcmp(l, "gray") || !strcmp(l, "grey") || !strcmp(l, "bright_black"))
        return 90;
    if (!strcmp(l, "orange") || !strcmp(l, "bright_red"))
        return 91;
    if (!strcmp(l, "lime") || !strcmp(l, "bright_green"))
        return 92;
    if (!strcmp(l, "bright_yellow"))
        return 93;
    if (!strcmp(l, "bright_blue"))
        return 94;
    if (!strcmp(l, "pink") || !strcmp(l, "bright_magenta"))
        return 95;
    if (!strcmp(l, "teal") || !strcmp(l, "bright_cyan"))
        return 96;
    if (!strcmp(l, "bright_white"))
        return 97;
    return -1;
}

int color_to_ansi(const char *s) {
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')
        s++;
    return palette_ansi(s);
}

static int hexval(char c) {
    if (c >= '0' && c <= '9')
        return c - '0';
    if (c >= 'a' && c <= 'f')
        return c - 'a' + 10;
    if (c >= 'A' && c <= 'F')
        return c - 'A' + 10;
    return -1;
}

static int parse_hex_rgb(const char *s, unsigned *r, unsigned *g, unsigned *b) {
    if (*s == '#')
        s++;
    if (strlen(s) != 6)
        return 0;
    int v[6];
    for (int i = 0; i < 6; i++) {
        v[i] = hexval(s[i]);
        if (v[i] < 0)
            return 0;
    }
    *r = (unsigned)((v[0] << 4) | v[1]);
    *g = (unsigned)((v[2] << 4) | v[3]);
    *b = (unsigned)((v[4] << 4) | v[5]);
    return 1;
}

int color_to_rgb(const char *s, unsigned *r, unsigned *g, unsigned *b) {
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')
        s++;
    char t[64];
    size_t n = 0;
    while (s[n] && s[n] != ' ' && s[n] != '\t' && s[n] != '\n' && s[n] != '\r' && n + 1 < sizeof t) {
        t[n] = s[n];
        n++;
    }
    t[n] = 0;
    for (int i = 0; i < SV_PALETTE_COUNT; i++) {
        if (ci_eq(t, SV_PALETTE_NAMES[i]))
            return parse_hex_rgb(SV_PALETTE_HEX[i], r, g, b);
    }
    char l[64];
    ci_lower(t, l, sizeof l);
    const char *approx = NULL;
    if (!strcmp(l, "grey"))
        approx = "888888";
    else if (!strcmp(l, "bright_black"))
        approx = "808080";
    else if (!strcmp(l, "bright_red"))
        approx = "ff5555";
    else if (!strcmp(l, "bright_green"))
        approx = "55ff55";
    else if (!strcmp(l, "bright_yellow"))
        approx = "ffff55";
    else if (!strcmp(l, "bright_blue"))
        approx = "5555ff";
    else if (!strcmp(l, "bright_magenta"))
        approx = "ff55ff";
    else if (!strcmp(l, "bright_cyan"))
        approx = "55ffff";
    else if (!strcmp(l, "bright_white"))
        approx = "ffffff";
    if (approx)
        return parse_hex_rgb(approx, r, g, b);
    return parse_hex_rgb(t, r, g, b);
}

const char *color_name(const char *s) {
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')
        s++;
    char t[64];
    size_t n = 0;
    while (s[n] && s[n] != ' ' && s[n] != '\t' && s[n] != '\n' && s[n] != '\r' && n + 1 < sizeof t) {
        t[n] = s[n];
        n++;
    }
    t[n] = 0;
    for (int i = 0; i < SV_PALETTE_COUNT; i++) {
        if (ci_eq(t, SV_PALETTE_NAMES[i]))
            return SV_PALETTE_NAMES[i];
    }
    if (ci_eq(t, "grey"))
        return "gray";
    static const char *aliases[] = {
        "bright_black", "bright_red", "bright_green", "bright_yellow",
        "bright_blue", "bright_magenta", "bright_cyan", "bright_white"
    };
    static const char *targets[] = {
        "gray", "orange", "lime", "yellow", "blue", "pink", "teal", "white"
    };
    for (int i = 0; i < 8; i++) {
        if (ci_eq(t, aliases[i]))
            return targets[i];
    }
    for (int i = 0; i < SV_PALETTE_COUNT; i++) {
        if (ci_eq(t, SV_PALETTE_HEX[i]))
            return SV_PALETTE_NAMES[i];
    }
    return NULL;
}

int color_index(const char *s) {
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')
        s++;
    char t[64];
    size_t n = 0;
    while (s[n] && s[n] != ' ' && s[n] != '\t' && s[n] != '\n' && s[n] != '\r' && n + 1 < sizeof t) {
        t[n] = s[n];
        n++;
    }
    t[n] = 0;
    for (int i = 0; i < SV_PALETTE_COUNT; i++) {
        if (ci_eq(t, SV_PALETTE_NAMES[i]))
            return i;
    }
    for (int i = 0; i < SV_PALETTE_COUNT; i++) {
        if (ci_eq(t, SV_PALETTE_HEX[i]))
            return i;
    }
    return -1;
}

void config_default_path(char *buf, size_t n) {
    const char *env = getenv("SHARKVIS_CONFIG");
    if (env && *env) {
        snprintf(buf, n, "%s", env);
        return;
    }
    /* JSONC is the preferred format (same as jefetch); TOML still loads for
     * backward compatibility. An explicit existing file always wins. */
    static const char *cands[] = {
        NULL, /* $HOME/.config/sharkvis/config.jsonc */
        NULL, /* $HOME/.config/sharkvis/config.toml */
        "./config.jsonc",
        "./config.toml",
    };
    char home_json[1024] = "";
    char home_toml[1024] = "";
    const char *home = getenv("HOME");
    if (home && *home) {
        snprintf(home_json, sizeof home_json, "%s/.config/sharkvis/config.jsonc", home);
        snprintf(home_toml, sizeof home_toml, "%s/.config/sharkvis/config.toml", home);
        cands[0] = home_json;
        cands[1] = home_toml;
    }
    struct stat st;
    for (size_t i = home && *home ? 0 : 2; i < 4; i++) {
        if (stat(cands[i], &st) == 0) {
            snprintf(buf, n, "%s", cands[i]);
            return;
        }
    }
    /* Nothing exists yet: create JSONC going forward. */
    if (home && *home)
        snprintf(buf, n, "%s", home_json);
    else
        snprintf(buf, n, "./config.jsonc");
}

static long geti(const char *v, long def) {
    while (*v == ' ' || *v == '\t')
        v++;
    long sign = 1;
    if (*v == '-' || *v == '+') {
        if (*v == '-')
            sign = -1;
        v++;
    }
    if (*v < '0' || *v > '9')
        return def;
    long val = 0;
    while (*v >= '0' && *v <= '9') {
        val = val * 10 + (*v - '0');
        v++;
    }
    return sign * val;
}

static double getf(const char *v, double def) {
    char *end;
    double d = strtod(v, &end);
    if (end == v)
        return def;
    return d;
}

static void copy_str(char *dst, size_t n, const char *src) {
    if (n == 0)
        return;
    strncpy(dst, src, n - 1);
    dst[n - 1] = 0;
}

/* ---- JSONC support (same format family as jefetch's config.jsonc) ----
 * sharkvis loads both `config.jsonc` and legacy `config.toml`; new files are
 * created as JSONC. Line and block comments are allowed in JSONC. */

static void mkdir_p(const char *path);

typedef enum {
    SJ_NULL,
    SJ_BOOL,
    SJ_NUM,
    SJ_STR,
    SJ_ARR,
    SJ_OBJ
} SjType;

typedef struct SjNode SjNode;
struct SjNode {
    SjType type;
    int boolean;
    double num;
    char *str;
    char **keys;
    SjNode **vals;
    size_t nm;
    SjNode **items;
    size_t n;
};

typedef struct {
    const char *text;
    size_t len;
    size_t pos;
    SjNode **nodes;
    size_t nnodes;
    size_t capnodes;
    int fail;
    int depth;
} SjParser;

static SjNode *sj_alloc(SjParser *p) {
    SjNode *nd = calloc(1, sizeof *nd);
    if (!nd) {
        p->fail = 1;
        return NULL;
    }
    if (p->nnodes >= p->capnodes) {
        size_t nc = p->capnodes ? p->capnodes * 2 : 64;
        SjNode **nn = realloc(p->nodes, nc * sizeof *nn);
        if (!nn) {
            free(nd);
            p->fail = 1;
            return NULL;
        }
        p->nodes = nn;
        p->capnodes = nc;
    }
    p->nodes[p->nnodes++] = nd;
    return nd;
}

static void sj_free_all(SjParser *p) {
    for (size_t i = 0; i < p->nnodes; i++) {
        SjNode *nd = p->nodes[i];
        free(nd->str);
        for (size_t k = 0; k < nd->nm; k++)
            free(nd->keys[k]);
        free(nd->keys);
        free(nd->vals);
        free(nd->items);
        free(nd);
    }
    free(p->nodes);
    p->nodes = NULL;
    p->nnodes = p->capnodes = 0;
}

static void sj_skip(SjParser *p) {
    while (p->pos < p->len) {
        char c = p->text[p->pos];
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r') {
            p->pos++;
        } else if (c == '/' && p->pos + 1 < p->len && p->text[p->pos + 1] == '/') {
            p->pos += 2;
            while (p->pos < p->len && p->text[p->pos] != '\n')
                p->pos++;
        } else if (c == '/' && p->pos + 1 < p->len && p->text[p->pos + 1] == '*') {
            p->pos += 2;
            while (p->pos + 1 < p->len &&
                   !(p->text[p->pos] == '*' && p->text[p->pos + 1] == '/'))
                p->pos++;
            p->pos += 2;
        } else {
            break;
        }
    }
}

static int sj_hex(const char *s, unsigned *out) {
    unsigned v = 0;
    for (int i = 0; i < 4; i++) {
        char c = s[i];
        v <<= 4;
        if (c >= '0' && c <= '9')
            v |= (unsigned)(c - '0');
        else if (c >= 'a' && c <= 'f')
            v |= (unsigned)(c - 'a' + 10);
        else if (c >= 'A' && c <= 'F')
            v |= (unsigned)(c - 'A' + 10);
        else
            return 0;
    }
    *out = v;
    return 1;
}

/* Encode cp as UTF-8 into buf (returns bytes written, 0 on invalid). */
static int sj_utf8(unsigned cp, char *buf) {
    if (cp < 0x80) {
        buf[0] = (char)cp;
        return 1;
    }
    if (cp < 0x800) {
        buf[0] = (char)(0xC0 | (cp >> 6));
        buf[1] = (char)(0x80 | (cp & 0x3F));
        return 2;
    }
    if (cp < 0x10000) {
        if (cp >= 0xD800 && cp <= 0xDFFF)
            return 0;
        buf[0] = (char)(0xE0 | (cp >> 12));
        buf[1] = (char)(0x80 | ((cp >> 6) & 0x3F));
        buf[2] = (char)(0x80 | (cp & 0x3F));
        return 3;
    }
    if (cp <= 0x10FFFF) {
        buf[0] = (char)(0xF0 | (cp >> 18));
        buf[1] = (char)(0x80 | ((cp >> 12) & 0x3F));
        buf[2] = (char)(0x80 | ((cp >> 6) & 0x3F));
        buf[3] = (char)(0x80 | (cp & 0x3F));
        return 4;
    }
    return 0;
}

static SjNode *sj_parse_value(SjParser *p);

static char *sj_parse_string(SjParser *p) {
    /* Assumes current char is '"'. Returns decoded malloc'd string. */
    size_t cap = 64, len = 0;
    char *out = malloc(cap);
    if (!out) {
        p->fail = 1;
        return NULL;
    }
    p->pos++; /* opening quote */
    while (p->pos < p->len) {
        char c = p->text[p->pos];
        if (c == '"') {
            p->pos++;
            out[len] = 0;
            return out;
        }
        if (c == '\\') {
            p->pos++;
            if (p->pos >= p->len)
                break;
            char e = p->text[p->pos++];
            char wb[4];
            int wl = 0;
            char simple = 0;
            switch (e) {
            case '"':
                simple = '"';
                break;
            case '\\':
                simple = '\\';
                break;
            case '/':
                simple = '/';
                break;
            case 'b':
                simple = '\b';
                break;
            case 'f':
                simple = '\f';
                break;
            case 'n':
                simple = '\n';
                break;
            case 'r':
                simple = '\r';
                break;
            case 't':
                simple = '\t';
                break;
            case 'u': {
                unsigned cp = 0;
                if (p->pos + 4 > p->len || !sj_hex(p->text + p->pos, &cp)) {
                    p->fail = 1;
                    free(out);
                    return NULL;
                }
                p->pos += 4;
                if (cp >= 0xD800 && cp <= 0xDBFF && p->pos + 6 <= p->len &&
                    p->text[p->pos] == '\\' && p->text[p->pos + 1] == 'u') {
                    unsigned lo = 0;
                    if (sj_hex(p->text + p->pos + 2, &lo) && lo >= 0xDC00 && lo <= 0xDFFF) {
                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                        p->pos += 6;
                    }
                }
                wl = sj_utf8(cp, wb);
                if (!wl) {
                    p->fail = 1;
                    free(out);
                    return NULL;
                }
                break;
            }
            default:
                p->fail = 1;
                free(out);
                return NULL;
            }
            size_t need = (size_t)wl ? (size_t)wl : 1;
            while (len + need + 1 > cap) {
                cap *= 2;
                char *no = realloc(out, cap);
                if (!no) {
                    free(out);
                    p->fail = 1;
                    return NULL;
                }
                out = no;
            }
            if (wl)
                memcpy(out + len, wb, (size_t)wl);
            else
                out[len] = simple;
            len += need;
            continue;
        }
        if ((unsigned char)c < 0x20) {
            p->fail = 1;
            free(out);
            return NULL;
        }
        if (len + 2 > cap) {
            cap *= 2;
            char *no = realloc(out, cap);
            if (!no) {
                free(out);
                p->fail = 1;
                return NULL;
            }
            out = no;
        }
        out[len++] = c;
        p->pos++;
    }
    p->fail = 1;
    free(out);
    return NULL;
}

static SjNode *sj_parse_value(SjParser *p) {
    if (p->depth > 32) {
        p->fail = 1;
        return NULL;
    }
    sj_skip(p);
    if (p->pos >= p->len) {
        p->fail = 1;
        return NULL;
    }
    char c = p->text[p->pos];
    SjNode *nd = sj_alloc(p);
    if (!nd)
        return NULL;
    if (c == '{') {
        nd->type = SJ_OBJ;
        p->pos++;
        p->depth++;
        sj_skip(p);
        if (p->pos < p->len && p->text[p->pos] == '}') {
            p->pos++;
            p->depth--;
            return nd;
        }
        for (;;) {
            sj_skip(p);
            if (p->pos >= p->len || p->text[p->pos] != '"') {
                p->fail = 1;
                return NULL;
            }
            char *key = sj_parse_string(p);
            if (!key)
                return NULL;
            sj_skip(p);
            if (p->pos >= p->len || p->text[p->pos] != ':') {
                free(key);
                p->fail = 1;
                return NULL;
            }
            p->pos++;
            SjNode *val = sj_parse_value(p);
            if (!val) {
                free(key);
                return NULL;
            }
            char **nk = realloc(nd->keys, (nd->nm + 1) * sizeof *nk);
            SjNode **nv = realloc(nd->vals, (nd->nm + 1) * sizeof *nv);
            if (!nk || !nv) {
                free(nk);
                free(nv);
                free(key);
                p->fail = 1;
                return NULL;
            }
            nd->keys = nk;
            nd->vals = nv;
            nd->keys[nd->nm] = key;
            nd->vals[nd->nm] = val;
            nd->nm++;
            sj_skip(p);
            if (p->pos < p->len && p->text[p->pos] == ',') {
                p->pos++;
                continue;
            }
            if (p->pos < p->len && p->text[p->pos] == '}') {
                p->pos++;
                p->depth--;
                return nd;
            }
            p->fail = 1;
            return NULL;
        }
    }
    if (c == '[') {
        nd->type = SJ_ARR;
        p->pos++;
        p->depth++;
        sj_skip(p);
        if (p->pos < p->len && p->text[p->pos] == ']') {
            p->pos++;
            p->depth--;
            return nd;
        }
        for (;;) {
            SjNode *val = sj_parse_value(p);
            if (!val)
                return NULL;
            SjNode **ni = realloc(nd->items, (nd->n + 1) * sizeof *ni);
            if (!ni) {
                p->fail = 1;
                return NULL;
            }
            nd->items = ni;
            nd->items[nd->n++] = val;
            sj_skip(p);
            if (p->pos < p->len && p->text[p->pos] == ',') {
                p->pos++;
                continue;
            }
            if (p->pos < p->len && p->text[p->pos] == ']') {
                p->pos++;
                p->depth--;
                return nd;
            }
            p->fail = 1;
            return NULL;
        }
    }
    if (c == '"') {
        nd->type = SJ_STR;
        nd->str = sj_parse_string(p);
        if (!nd->str)
            return NULL;
        return nd;
    }
    if (!strncmp(p->text + p->pos, "true", 4) &&
        (p->pos + 4 >= p->len || strchr(" \t\n\r,}]", p->text[p->pos + 4]))) {
        nd->type = SJ_BOOL;
        nd->boolean = 1;
        p->pos += 4;
        return nd;
    }
    if (!strncmp(p->text + p->pos, "false", 5) &&
        (p->pos + 5 >= p->len || strchr(" \t\n\r,}]", p->text[p->pos + 5]))) {
        nd->type = SJ_BOOL;
        nd->boolean = 0;
        p->pos += 5;
        return nd;
    }
    if (!strncmp(p->text + p->pos, "null", 4) &&
        (p->pos + 4 >= p->len || strchr(" \t\n\r,}]", p->text[p->pos + 4]))) {
        nd->type = SJ_NULL;
        p->pos += 4;
        return nd;
    }
    /* Number: validate with strtod, keep raw text. */
    {
        char *end = NULL;
        double d = strtod(p->text + p->pos, &end);
        if (end == p->text + p->pos) {
            p->fail = 1;
            return NULL;
        }
        size_t raw = (size_t)(end - (p->text + p->pos));
        nd->type = SJ_NUM;
        nd->num = d;
        nd->str = malloc(raw + 1);
        if (!nd->str) {
            p->fail = 1;
            return NULL;
        }
        memcpy(nd->str, p->text + p->pos, raw);
        nd->str[raw] = 0;
        p->pos += raw;
        return nd;
    }
}

static const SjNode *sj_get(const SjNode *o, const char *key) {
    if (!o || o->type != SJ_OBJ)
        return NULL;
    for (size_t i = 0; i < o->nm; i++) {
        if (!strcmp(o->keys[i], key))
            return o->vals[i];
    }
    return NULL;
}

/* Typed getters accept number/bool/string forms so hand-written configs are
 * forgiving (e.g. "autosens": 1 or "framerate": "60"). */
static long sj_geti(const SjNode *v, long def) {
    if (!v)
        return def;
    if (v->type == SJ_BOOL)
        return v->boolean ? 1 : 0;
    if (v->type == SJ_NUM)
        return (long)v->num;
    if (v->type == SJ_STR)
        return geti(v->str, def);
    return def;
}

static double sj_getf(const SjNode *v, double def) {
    if (!v)
        return def;
    if (v->type == SJ_BOOL)
        return v->boolean ? 1.0 : 0.0;
    if (v->type == SJ_NUM)
        return v->num;
    if (v->type == SJ_STR)
        return getf(v->str, def);
    return def;
}

static int sj_getb(const SjNode *v, int def) {
    if (!v)
        return def;
    if (v->type == SJ_BOOL)
        return v->boolean;
    if (v->type == SJ_NUM)
        return v->num != 0.0;
    if (v->type == SJ_STR) {
        const char *s = v->str;
        while (*s == ' ' || *s == '\t')
            s++;
        char l[16];
        size_t i = 0;
        while (s[i] && i + 1 < sizeof l) {
            l[i] = (char)tolower((unsigned char)s[i]);
            i++;
        }
        l[i] = 0;
        if (!strcmp(l, "true") || !strcmp(l, "yes") || !strcmp(l, "on"))
            return 1;
        if (!strcmp(l, "false") || !strcmp(l, "no") || !strcmp(l, "off"))
            return 0;
        return geti(s, def) != 0;
    }
    return def;
}

static void sj_gets(const SjNode *v, char *dst, size_t n, const char *def) {
    if (n == 0)
        return;
    if (!v || v->type == SJ_NULL) {
        snprintf(dst, n, "%s", def ? def : "");
        return;
    }
    if (v->type == SJ_STR) {
        snprintf(dst, n, "%s", v->str);
        return;
    }
    if (v->type == SJ_BOOL) {
        snprintf(dst, n, "%s", v->boolean ? "true" : "false");
        return;
    }
    if (v->type == SJ_NUM) {
        snprintf(dst, n, "%s", v->str ? v->str : "");
        return;
    }
    snprintf(dst, n, "%s", def ? def : "");
}

/* Escape a string for JSON output. */
static void json_write_str(FILE *f, const char *s) {
    fputc('"', f);
    for (const unsigned char *p = (const unsigned char *)s; *p; p++) {
        switch (*p) {
        case '"':
            fputs("\\\"", f);
            break;
        case '\\':
            fputs("\\\\", f);
            break;
        case '\b':
            fputs("\\b", f);
            break;
        case '\f':
            fputs("\\f", f);
            break;
        case '\n':
            fputs("\\n", f);
            break;
        case '\r':
            fputs("\\r", f);
            break;
        case '\t':
            fputs("\\t", f);
            break;
        default:
            if (*p < 0x20)
                fprintf(f, "\\u%04x", *p);
            else
                fputc(*p, f);
            break;
        }
    }
    fputc('"', f);
}

static int config_save_jsonc(const SvConfig *c, const char *path) {
    mkdir_p(path);
    FILE *f = fopen(path, "w");
    if (!f)
        return 0;
    fprintf(f, "{\n");
    fprintf(f, "    // Sharkvis config (JSONC — // and /* */ comments allowed).\n");
    fprintf(f, "    // TOML `config.toml` still loads; this file takes precedence.\n");
    fprintf(f, "    \"general\": {\n");
    fprintf(f, "        \"bars\": %zu,\n", c->bars);
    fprintf(f, "        \"bar_width\": %zu,\n", c->bar_width);
    fprintf(f, "        \"bar_spacing\": %zu,\n", c->bar_spacing);
    fprintf(f, "        \"framerate\": %u,\n", c->framerate);
    fprintf(f, "        \"sensitivity\": %.0f,\n", c->sensitivity);
    fprintf(f, "        \"autosens\": %s,\n", c->autosens ? "true" : "false");
    fprintf(f, "        \"lower_cutoff_freq\": %u,\n", c->lower_cutoff);
    fprintf(f, "        \"higher_cutoff_freq\": %u\n", c->higher_cutoff);
    fprintf(f, "    },\n");
    fprintf(f, "    \"smoothing\": {\n");
    fprintf(f, "        \"noise_reduction\": %.2f\n", c->noise_reduction);
    fprintf(f, "    },\n");
    fprintf(f, "    \"input\": {\n");
    fprintf(f, "        \"method\": \"pulse\",\n");
    fprintf(f, "        \"source\": ");
    json_write_str(f, c->source);
    fprintf(f, ",\n");
    fprintf(f, "        \"sample_rate\": %u,\n", c->sample_rate);
    fprintf(f, "        \"channels\": %u\n", c->channels);
    fprintf(f, "    },\n");
    fprintf(f, "    \"color\": {\n");
    fprintf(f, "        \"color_mode\": \"%s\",\n", c->color_256 ? "256" : "24bit");
    fprintf(f, "        \"gradient_low\": ");
    json_write_str(f, c->gradient_low);
    fprintf(f, ",\n");
    fprintf(f, "        \"gradient_high\": ");
    json_write_str(f, c->gradient_high);
    fprintf(f, "\n");
    fprintf(f, "    },\n");
    fprintf(f, "    \"visualizer\": {\n");
    fprintf(f, "        \"mode\": ");
    json_write_str(f, c->mode);
    fprintf(f, ",\n");
    fprintf(f, "        \"text_align\": ");
    json_write_str(f, c->text_align);
    fprintf(f, ",\n");
    fprintf(f, "        \"text_size\": %u,\n", c->text_size);
    fprintf(f, "        \"text_style\": ");
    json_write_str(f, c->text_style);
    fprintf(f, ",\n");
    fprintf(f, "        \"provider\": ");
    json_write_str(f, c->provider);
    fprintf(f, ",\n");
    fprintf(f, "        \"offset_ms\": %ld,\n", c->lyric_offset_ms);
    fprintf(f, "        \"chars\": \"");
    for (size_t i = 0; i < c->chars_len; i++) {
        unsigned char ch = c->chars[i];
        if (ch == '"' || ch == '\\')
            fputc('\\', f);
        if (ch >= 0x20 || ch >= 0x80)
            fputc(ch, f);
    }
    fprintf(f, "\"\n");
    fprintf(f, "    },\n");
    fprintf(f, "    \"lyrics\": {\n");
    fprintf(f, "        \"folder\": ");
    json_write_str(f, c->lyrics_folder);
    fprintf(f, "\n");
    fprintf(f, "    },\n");
    fprintf(f, "    \"mpris\": {\n");
    fprintf(f, "        \"players\": ");
    json_write_str(f, c->mpris_players);
    fprintf(f, "\n");
    fprintf(f, "    }\n");
    fprintf(f, "}\n");
    fclose(f);
    return 1;
}

static void config_apply_jsonc(SvConfig *c, const SjNode *root) {
    const SjNode *general = sj_get(root, "general");
    if (general) {
        c->bars = (size_t)sj_geti(sj_get(general, "bars"), (long)c->bars);
        c->bar_width = (size_t)sj_geti(sj_get(general, "bar_width"), (long)c->bar_width);
        c->bar_spacing =
            (size_t)sj_geti(sj_get(general, "bar_spacing"), (long)c->bar_spacing);
        c->framerate = (unsigned)sj_geti(sj_get(general, "framerate"), (long)c->framerate);
        c->sensitivity = sj_getf(sj_get(general, "sensitivity"), c->sensitivity);
        c->autosens = sj_getb(sj_get(general, "autosens"), c->autosens ? 1 : 0) != 0;
        c->lower_cutoff =
            (unsigned)sj_geti(sj_get(general, "lower_cutoff_freq"), (long)c->lower_cutoff);
        c->higher_cutoff =
            (unsigned)sj_geti(sj_get(general, "higher_cutoff_freq"), (long)c->higher_cutoff);
    }
    const SjNode *smoothing = sj_get(root, "smoothing");
    if (smoothing)
        c->noise_reduction =
            sj_getf(sj_get(smoothing, "noise_reduction"), c->noise_reduction);
    const SjNode *input = sj_get(root, "input");
    if (input) {
        const SjNode *method = sj_get(input, "method");
        if (method && method->type == SJ_STR && *method->str && strcmp(method->str, "pulse") &&
            strcmp(method->str, "pipewire") && strcmp(method->str, "auto"))
            fprintf(stderr, "sharkvis: input method '%s' not supported, using pulse\n",
                    method->str);
        char tmp[256];
        sj_gets(sj_get(input, "source"), tmp, sizeof tmp, c->source);
        if (*tmp)
            copy_str(c->source, sizeof c->source, tmp);
        c->sample_rate =
            (unsigned)sj_geti(sj_get(input, "sample_rate"), (long)c->sample_rate);
        c->channels = (unsigned)sj_geti(sj_get(input, "channels"), (long)c->channels);
    }
    const SjNode *lyrics = sj_get(root, "lyrics");
    if (lyrics) {
        char tmp[512];
        sj_gets(sj_get(lyrics, "folder"), tmp, sizeof tmp, c->lyrics_folder);
        copy_str(c->lyrics_folder, sizeof c->lyrics_folder, tmp);
    }
    const SjNode *mpris = sj_get(root, "mpris");
    if (mpris) {
        char tmp[512];
        sj_gets(sj_get(mpris, "players"), tmp, sizeof tmp, c->mpris_players);
        copy_str(c->mpris_players, sizeof c->mpris_players, tmp);
    }
    const SjNode *color = sj_get(root, "color");
    if (color) {
        char tmp[64];
        sj_gets(sj_get(color, "color_mode"), tmp, sizeof tmp, "");
        if (*tmp) {
            if (!strcmp(tmp, "256") || !strcmp(tmp, "indexed"))
                c->color_256 = 1;
            else if (!strcmp(tmp, "24bit") || !strcmp(tmp, "truecolor"))
                c->color_256 = 0;
            else
                c->color_256 = geti(tmp, 0) != 0;
        }
        sj_gets(sj_get(color, "gradient_low"), tmp, sizeof tmp, "");
        if (*tmp) {
            unsigned r, g, b;
            if (color_to_rgb(tmp, &r, &g, &b))
                copy_str(c->gradient_low, sizeof c->gradient_low, tmp);
        }
        sj_gets(sj_get(color, "gradient_high"), tmp, sizeof tmp, "");
        if (*tmp) {
            unsigned r, g, b;
            if (color_to_rgb(tmp, &r, &g, &b))
                copy_str(c->gradient_high, sizeof c->gradient_high, tmp);
        }
    }
    const SjNode *vis = sj_get(root, "visualizer");
    if (vis) {
        char tmp[512];
        sj_gets(sj_get(vis, "mode"), tmp, sizeof tmp, "");
        if (*tmp) {
            if (!strcmp(tmp, "bars") || !strcmp(tmp, "wave") || !strcmp(tmp, "oscilloscope") ||
                !strcmp(tmp, "lissajous"))
                copy_str(c->mode, sizeof c->mode, tmp);
            else if (!strcmp(tmp, "lyrics"))
                strcpy(c->mode, "lyrics");
            else if (!strcmp(tmp, "text"))
                strcpy(c->mode, "lyrics");
        }
        sj_gets(sj_get(vis, "text_align"), tmp, sizeof tmp, "");
        if ((*tmp == 'l' || *tmp == 'c') &&
            (!strcmp(tmp, "left") || !strcmp(tmp, "center")))
            copy_str(c->text_align, sizeof c->text_align, tmp);
        const SjNode *tsz = sj_get(vis, "text_size");
        if (tsz) {
            long n = sj_geti(tsz, (long)c->text_size);
            if (n > 5)
                n = 5;
            c->text_size = (unsigned)n;
        }
        sj_gets(sj_get(vis, "text_style"), tmp, sizeof tmp, "");
        if (*tmp) {
            if (!strcmp(tmp, "big ahh") || !strcmp(tmp, "normal"))
                copy_str(c->text_style, sizeof c->text_style, tmp);
            else if (!strcmp(tmp, "big"))
                strcpy(c->text_style, "big ahh");
            else if (!strcmp(tmp, "small"))
                strcpy(c->text_style, "normal");
        }
        sj_gets(sj_get(vis, "provider"), tmp, sizeof tmp, "");
        if (*tmp && (!strcmp(tmp, "auto") || !strcmp(tmp, "musixmatch") ||
                     !strcmp(tmp, "lrclib")))
            copy_str(c->provider, sizeof c->provider, tmp);
        const SjNode *off = sj_get(vis, "offset_ms");
        if (off) {
            long n = sj_geti(off, c->lyric_offset_ms);
            if (n < -10000)
                n = -10000;
            if (n > 10000)
                n = 10000;
            c->lyric_offset_ms = n;
        }
        const SjNode *ch = sj_get(vis, "chars");
        if (ch && ch->type == SJ_STR) {
            size_t n = strlen(ch->str);
            if (n > sizeof c->chars)
                n = sizeof c->chars;
            memcpy(c->chars, ch->str, n);
            c->chars_len = n;
        }
    }
}

static int config_load_jsonc_text(SvConfig *c, const char *text, size_t len) {
    SjParser p;
    memset(&p, 0, sizeof p);
    p.text = text;
    p.len = len;
    SjNode *root = sj_parse_value(&p);
    int ok = 0;
    if (root && !p.fail) {
        sj_skip(&p);
        if (p.pos == p.len && root->type == SJ_OBJ) {
            config_apply_jsonc(c, root);
            ok = 1;
        }
    }
    sj_free_all(&p);
    return ok;
}

static int path_is_jsonc(const char *path) {
    size_t n = strlen(path);
    if (n < 6)
        return 0;
    const char *e = path + n - 6;
    return e[0] == '.' && (e[1] == 'j' || e[1] == 'J') && (e[2] == 's' || e[2] == 'S') &&
           (e[3] == 'o' || e[3] == 'O') && (e[4] == 'n' || e[4] == 'N') &&
           (e[5] == 'c' || e[5] == 'C');
}

int config_load(SvConfig *c, const char *path) {
    /* JSONC when the file looks like JSON (leading '{'); the .jsonc
     * extension alone is not trusted so a misnamed TOML still loads. */
    FILE *probe = fopen(path, "r");
    if (probe) {
        int ch = 0, jsonc = 0;
        do {
            ch = fgetc(probe);
            if (ch == 0xEF) { /* skip UTF-8 BOM */
                if (fgetc(probe) != 0xBB || fgetc(probe) != 0xBF)
                    break;
            } else if (ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r') {
                continue;
            } else if (ch == '{') {
                jsonc = 1;
            }
            break;
        } while (ch != EOF);
        fclose(probe);
        if (jsonc) {
            FILE *f = fopen(path, "r");
            if (!f)
                return 0;
            size_t cap = 65536, len = 0;
            char *buf = malloc(cap);
            if (!buf) {
                fclose(f);
                return 0;
            }
            size_t k = 0;
            while ((k = fread(buf + len, 1, cap - len - 1, f)) > 0) {
                len += k;
                if (len + 1 >= cap) {
                    if (cap >= (size_t)1024 * 1024)
                        break;
                    cap *= 2;
                    char *nb = realloc(buf, cap);
                    if (!nb) {
                        free(buf);
                        fclose(f);
                        return 0;
                    }
                    buf = nb;
                }
            }
            fclose(f);
            buf[len] = 0;
            int ok = config_load_jsonc_text(c, buf, len);
            free(buf);
            return ok;
        }
    }
    FILE *f = fopen(path, "r");
    if (!f)
        return 0;
    char line[1024];
    char section[64] = "general";
    while (fgets(line, sizeof line, f)) {
        size_t len = strlen(line);
        while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r'))
            line[--len] = 0;
        char *s = line;
        while (*s == ' ' || *s == '\t')
            s++;
        char *e = s + strlen(s);
        while (e > s && (e[-1] == ' ' || e[-1] == '\t'))
            *--e = 0;
        if (*s == 0 || *s == ';' || *s == '#')
            continue;
        if (*s == '[') {
            char *end = strchr(s, ']');
            if (end)
                *end = 0;
            char *name = s + 1;
            while (*name == ' ' || *name == '\t')
                name++;
            char *ne = name + strlen(name);
            while (ne > name && (ne[-1] == ' ' || ne[-1] == '\t'))
                *--ne = 0;
            size_t i = 0;
            while (name[i] && i + 1 < sizeof section) {
                section[i] = (char)tolower((unsigned char)name[i]);
                i++;
            }
            section[i] = 0;
            continue;
        }
        char *eq = strchr(s, '=');
        if (!eq)
            continue;
        *eq = 0;
        char *key = s;
        char *ke = key + strlen(key);
        while (ke > key && (ke[-1] == ' ' || ke[-1] == '\t'))
            *--ke = 0;
        char keyl[64];
        size_t ki = 0;
        while (key[ki] && ki + 1 < sizeof keyl) {
            keyl[ki] = (char)tolower((unsigned char)key[ki]);
            ki++;
        }
        keyl[ki] = 0;
        char *val = eq + 1;
        while (*val == ' ' || *val == '\t')
            val++;
        char *ve = val + strlen(val);
        while (ve > val && (ve[-1] == ' ' || ve[-1] == '\t'))
            *--ve = 0;
        if (strcmp(keyl, "chars") != 0) {
            char *semi = strchr(val, ';');
            if (semi) {
                *semi = 0;
                ve = semi;
                while (ve > val && (ve[-1] == ' ' || ve[-1] == '\t'))
                    *--ve = 0;
            }
        }
        if (!strcmp(section, "general")) {
            if (!strcmp(keyl, "bars"))
                c->bars = (size_t)geti(val, (long)c->bars);
            else if (!strcmp(keyl, "bar_width"))
                c->bar_width = (size_t)geti(val, (long)c->bar_width);
            else if (!strcmp(keyl, "bar_spacing"))
                c->bar_spacing = (size_t)geti(val, (long)c->bar_spacing);
            else if (!strcmp(keyl, "framerate"))
                c->framerate = (unsigned)geti(val, (long)c->framerate);
            else if (!strcmp(keyl, "sensitivity"))
                c->sensitivity = getf(val, c->sensitivity);
            else if (!strcmp(keyl, "autosens"))
                c->autosens = geti(val, 1) != 0;
            else if (!strcmp(keyl, "lower_cutoff_freq"))
                c->lower_cutoff = (unsigned)geti(val, (long)c->lower_cutoff);
            else if (!strcmp(keyl, "higher_cutoff_freq"))
                c->higher_cutoff = (unsigned)geti(val, (long)c->higher_cutoff);
        } else if (!strcmp(section, "smoothing")) {
            if (!strcmp(keyl, "noise_reduction"))
                c->noise_reduction = getf(val, c->noise_reduction);
        } else if (!strcmp(section, "input")) {
            if (!strcmp(keyl, "method")) {
                if (*val && strcmp(val, "pulse") && strcmp(val, "pipewire") && strcmp(val, "auto"))
                    fprintf(stderr, "sharkvis: input method '%s' not supported, using pulse\n", val);
            } else if (!strcmp(keyl, "source")) {
                copy_str(c->source, sizeof c->source, val);
            } else if (!strcmp(keyl, "sample_rate")) {
                c->sample_rate = (unsigned)geti(val, (long)c->sample_rate);
            } else if (!strcmp(keyl, "channels")) {
                c->channels = (unsigned)geti(val, (long)c->channels);
            }
        } else if (!strcmp(section, "lyrics")) {
            if (!strcmp(keyl, "folder"))
                copy_str(c->lyrics_folder, sizeof c->lyrics_folder, val);
        } else if (!strcmp(section, "mpris")) {
            if (!strcmp(keyl, "players"))
                copy_str(c->mpris_players, sizeof c->mpris_players, val);
        } else if (!strcmp(section, "color")) {
            if (!strcmp(keyl, "color_mode")) {
                if (!strcmp(val, "256") || !strcmp(val, "indexed"))
                    c->color_256 = 1;
                else if (!strcmp(val, "24bit") || !strcmp(val, "truecolor"))
                    c->color_256 = 0;
                else
                    c->color_256 = geti(val, 0) != 0;
            } else if (!strcmp(keyl, "gradient_low")) {
                unsigned r, g, b;
                if (color_to_rgb(val, &r, &g, &b))
                    copy_str(c->gradient_low, sizeof c->gradient_low, val);
            } else if (!strcmp(keyl, "gradient_high")) {
                unsigned r, g, b;
                if (color_to_rgb(val, &r, &g, &b))
                    copy_str(c->gradient_high, sizeof c->gradient_high, val);
            }
        } else if (!strcmp(section, "visualizer")) {
            if (!strcmp(keyl, "mode")) {
                if (!strcmp(val, "bars") || !strcmp(val, "wave") ||
                    !strcmp(val, "oscilloscope") || !strcmp(val, "lissajous")) {
                    copy_str(c->mode, sizeof c->mode, val);
                } else if (!strcmp(val, "lyrics")) {
                    strcpy(c->mode, "lyrics");
                } else if (!strcmp(val, "text")) {
                    strcpy(c->mode, "lyrics");
                }
            } else if (!strcmp(keyl, "text") || !strcmp(keyl, "text_source")) {
            } else if (!strcmp(keyl, "text_align")) {
                if (!strcmp(val, "left") || !strcmp(val, "center"))
                    copy_str(c->text_align, sizeof c->text_align, val);
            } else if (!strcmp(keyl, "text_size")) {
                char *end;
                long n = strtol(val, &end, 10);
                if (end != val) {
                    if (n > 5)
                        n = 5;
                    c->text_size = (unsigned)n;
                }
            } else if (!strcmp(keyl, "text_style")) {
                if (!strcmp(val, "big ahh") || !strcmp(val, "normal"))
                    copy_str(c->text_style, sizeof c->text_style, val);
                else if (!strcmp(val, "big"))
                    strcpy(c->text_style, "big ahh");
                else if (!strcmp(val, "small"))
                    strcpy(c->text_style, "normal");
            } else if (!strcmp(keyl, "provider")) {
                if (!strcmp(val, "auto") || !strcmp(val, "musixmatch") || !strcmp(val, "lrclib"))
                    copy_str(c->provider, sizeof c->provider, val);
            } else if (!strcmp(keyl, "offset_ms")) {
                char *end;
                long n = strtol(val, &end, 10);
                if (end != val) {
                    if (n < -10000)
                        n = -10000;
                    if (n > 10000)
                        n = 10000;
                    c->lyric_offset_ms = n;
                }
            } else if (!strcmp(keyl, "chars")) {
                size_t n = strlen(val);
                if (n > sizeof c->chars)
                    n = sizeof c->chars;
                memcpy(c->chars, val, n);
                c->chars_len = n;
            }
        }
    }
    fclose(f);
    return 1;
}

static void mkdir_p(const char *path) {
    char tmp[1024];
    snprintf(tmp, sizeof tmp, "%s", path);
    char *slash = strrchr(tmp, '/');
    if (!slash)
        return;
    *slash = 0;
    char cur[1024];
    size_t n = 0;
    if (tmp[0] == '/')
        cur[n++] = '/';
    char *p = tmp + (tmp[0] == '/' ? 1 : 0);
    char *tok = strtok(p, "/");
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
        tok = strtok(NULL, "/");
    }
}

int config_save(const SvConfig *c, const char *path) {
    /* Save in the format the path asks for; JSONC is the default for new
     * files (see config_default_path). */
    if (path_is_jsonc(path))
        return config_save_jsonc(c, path);
    mkdir_p(path);
    FILE *f = fopen(path, "w");
    if (!f)
        return 0;
    fprintf(f, "[general]\n");
    fprintf(f, "bars = %zu\n", c->bars);
    fprintf(f, "bar_width = %zu\n", c->bar_width);
    fprintf(f, "bar_spacing = %zu\n", c->bar_spacing);
    fprintf(f, "framerate = %u\n", c->framerate);
    fprintf(f, "sensitivity = %.0f\n", c->sensitivity);
    fprintf(f, "autosens = %d\n", c->autosens ? 1 : 0);
    fprintf(f, "lower_cutoff_freq = %u\n", c->lower_cutoff);
    fprintf(f, "higher_cutoff_freq = %u\n", c->higher_cutoff);
    fprintf(f, "\n[smoothing]\n");
    fprintf(f, "noise_reduction = %.2f\n", c->noise_reduction);
    fprintf(f, "\n[input]\n");
    fprintf(f, "method = pulse\n");
    fprintf(f, "source = %s\n", c->source);
    fprintf(f, "sample_rate = %u\n", c->sample_rate);
    fprintf(f, "channels = %u\n", c->channels);
    fprintf(f, "\n[color]\n");
    fprintf(f, "color_mode = %s\n", c->color_256 ? "256" : "24bit");
    fprintf(f, "gradient_low = %s\n", c->gradient_low);
    fprintf(f, "gradient_high = %s\n", c->gradient_high);
    fprintf(f, "\n[visualizer]\n");
    fprintf(f, "mode = %s\n", c->mode);
    fprintf(f, "text_align = %s\n", c->text_align);
    fprintf(f, "text_size = %u\n", c->text_size);
    fprintf(f, "text_style = %s\n", c->text_style);
    fprintf(f, "provider = %s\n", c->provider);
    fprintf(f, "offset_ms = %ld\n", c->lyric_offset_ms);
    fprintf(f, "chars = ");
    fwrite(c->chars, 1, c->chars_len, f);
    fprintf(f, "\n");
    fprintf(f, "\n[lyrics]\n");
    fprintf(f, "folder = %s\n", c->lyrics_folder);
    fprintf(f, "\n[mpris]\n");
    fprintf(f, "players = %s\n", c->mpris_players);
    fclose(f);
    return 1;
}
