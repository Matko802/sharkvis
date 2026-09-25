#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "common.h"
#include "config.h"
#include "render.h"
#include "unifont.h"

typedef struct {
    SvBuf *buf;
    size_t cap;
} Out;

typedef struct {
    int active;
    uint8_t *col;
    size_t len;
    size_t cap;
} ColorState;

typedef struct {
    int kind;
    uint8_t block[7];
    uint32_t cp;
    int w;
} Glyph;

static void out_s(Out *o, const void *p, size_t n) {
    size_t room = o->cap > o->buf->len ? o->cap - o->buf->len : 0;
    if (room == 0)
        return;
    if (n > room)
        n = room;
    sv_buf_put(o->buf, p, n);
}

static void out_u(Out *o, unsigned v) {
    char d[16];
    int n = 0;
    if (v == 0) {
        d[n++] = '0';
    } else {
        char tmp[16];
        int m = 0;
        while (v > 0) {
            tmp[m++] = (char)('0' + v % 10);
            v /= 10;
        }
        while (m > 0)
            d[n++] = tmp[--m];
    }
    out_s(o, d, (size_t)n);
}

static void seek_cell(unsigned r, unsigned c, Out *o) {
    out_s(o, "\x1b[", 2);
    out_u(o, r + 1);
    out_s(o, ";", 1);
    out_u(o, c + 1);
    out_s(o, "H", 1);
}

static size_t utf8_encode(uint32_t cp, uint8_t out[4]) {
    if (cp < 0x80) {
        out[0] = (uint8_t)cp;
        return 1;
    }
    if (cp < 0x800) {
        out[0] = (uint8_t)(0xC0 | (cp >> 6));
        out[1] = (uint8_t)(0x80 | (cp & 0x3F));
        return 2;
    }
    if (cp < 0x10000) {
        out[0] = (uint8_t)(0xE0 | (cp >> 12));
        out[1] = (uint8_t)(0x80 | ((cp >> 6) & 0x3F));
        out[2] = (uint8_t)(0x80 | (cp & 0x3F));
        return 3;
    }
    out[0] = (uint8_t)(0xF0 | (cp >> 18));
    out[1] = (uint8_t)(0x80 | ((cp >> 12) & 0x3F));
    out[2] = (uint8_t)(0x80 | ((cp >> 6) & 0x3F));
    out[3] = (uint8_t)(0x80 | (cp & 0x3F));
    return 4;
}

static size_t utf8_decode(const uint8_t *s, size_t n, uint32_t *cp) {
    if (n == 0)
        return 0;
    uint8_t c = s[0];
    if (c < 0x80) {
        *cp = c;
        return 1;
    }
    if ((c & 0xE0) == 0xC0) {
        if (n < 2)
            return 0;
        *cp = ((uint32_t)(c & 0x1F) << 6) | (s[1] & 0x3F);
        return 2;
    }
    if ((c & 0xF0) == 0xE0) {
        if (n < 3)
            return 0;
        *cp = ((uint32_t)(c & 0x0F) << 12) | ((uint32_t)(s[1] & 0x3F) << 6) | (s[2] & 0x3F);
        return 3;
    }
    if ((c & 0xF8) == 0xF0) {
        if (n < 4)
            return 0;
        *cp = ((uint32_t)(c & 0x07) << 18) | ((uint32_t)(s[1] & 0x3F) << 12) |
              ((uint32_t)(s[2] & 0x3F) << 6) | (s[3] & 0x3F);
        return 4;
    }
    *cp = c;
    return 1;
}

static uint32_t *decode_str(const char *s, size_t *n) {
    size_t cap = 64;
    uint32_t *out = malloc(cap * sizeof(uint32_t));
    size_t m = 0;
    size_t len = strlen(s);
    size_t i = 0;
    while (i < len && m < 512) {
        uint32_t cp;
        size_t k = utf8_decode((const uint8_t *)s + i, len - i, &cp);
        if (k == 0)
            break;
        if (m >= cap) {
            cap *= 2;
            uint32_t *p = realloc(out, cap * sizeof(uint32_t));
            if (!p)
                break;
            out = p;
        }
        out[m++] = cp;
        i += k;
    }
    *n = m;
    return out;
}

static void term_esc_buf(int code, int dim, uint8_t **out, size_t *len) {
    char tmp[24];
    int n;
    if (dim)
        n = snprintf(tmp, sizeof tmp, "\x1b[2;%dm", code);
    else
        n = snprintf(tmp, sizeof tmp, "\x1b[%dm", code);
    *out = malloc((size_t)n);
    memcpy(*out, tmp, (size_t)n);
    *len = (size_t)n;
}

static void block_glyph(uint32_t c, uint8_t r[7]) {
    uint32_t u = c;
    if (u < 128 && u >= 'a' && u <= 'z')
        u -= 32;
    switch (u) {
    case 'A': r[0]=0x0E;r[1]=0x11;r[2]=0x11;r[3]=0x1F;r[4]=0x11;r[5]=0x11;r[6]=0x11;break;
    case 'B': r[0]=0x1E;r[1]=0x11;r[2]=0x11;r[3]=0x1E;r[4]=0x11;r[5]=0x11;r[6]=0x1E;break;
    case 'C': r[0]=0x0E;r[1]=0x11;r[2]=0x10;r[3]=0x10;r[4]=0x10;r[5]=0x11;r[6]=0x0E;break;
    case 'D': r[0]=0x1E;r[1]=0x11;r[2]=0x11;r[3]=0x11;r[4]=0x11;r[5]=0x11;r[6]=0x1E;break;
    case 'E': r[0]=0x1F;r[1]=0x10;r[2]=0x10;r[3]=0x1E;r[4]=0x10;r[5]=0x10;r[6]=0x1F;break;
    case 'F': r[0]=0x1F;r[1]=0x10;r[2]=0x10;r[3]=0x1E;r[4]=0x10;r[5]=0x10;r[6]=0x10;break;
    case 'G': r[0]=0x0E;r[1]=0x11;r[2]=0x10;r[3]=0x17;r[4]=0x11;r[5]=0x11;r[6]=0x0F;break;
    case 'H': r[0]=0x11;r[1]=0x11;r[2]=0x11;r[3]=0x1F;r[4]=0x11;r[5]=0x11;r[6]=0x11;break;
    case 'I': r[0]=0x0E;r[1]=0x04;r[2]=0x04;r[3]=0x04;r[4]=0x04;r[5]=0x04;r[6]=0x0E;break;
    case 'J': r[0]=0x07;r[1]=0x02;r[2]=0x02;r[3]=0x02;r[4]=0x02;r[5]=0x12;r[6]=0x0C;break;
    case 'K': r[0]=0x11;r[1]=0x12;r[2]=0x14;r[3]=0x18;r[4]=0x14;r[5]=0x12;r[6]=0x11;break;
    case 'L': r[0]=0x10;r[1]=0x10;r[2]=0x10;r[3]=0x10;r[4]=0x10;r[5]=0x10;r[6]=0x1F;break;
    case 'M': r[0]=0x11;r[1]=0x1B;r[2]=0x15;r[3]=0x15;r[4]=0x11;r[5]=0x11;r[6]=0x11;break;
    case 'N': r[0]=0x11;r[1]=0x11;r[2]=0x19;r[3]=0x15;r[4]=0x13;r[5]=0x11;r[6]=0x11;break;
    case 'O': r[0]=0x0E;r[1]=0x11;r[2]=0x11;r[3]=0x11;r[4]=0x11;r[5]=0x11;r[6]=0x0E;break;
    case 'P': r[0]=0x1E;r[1]=0x11;r[2]=0x11;r[3]=0x1E;r[4]=0x10;r[5]=0x10;r[6]=0x10;break;
    case 'Q': r[0]=0x0E;r[1]=0x11;r[2]=0x11;r[3]=0x11;r[4]=0x15;r[5]=0x12;r[6]=0x0D;break;
    case 'R': r[0]=0x1E;r[1]=0x11;r[2]=0x11;r[3]=0x1E;r[4]=0x14;r[5]=0x12;r[6]=0x11;break;
    case 'S': r[0]=0x0F;r[1]=0x10;r[2]=0x10;r[3]=0x0E;r[4]=0x01;r[5]=0x01;r[6]=0x1E;break;
    case 'T': r[0]=0x1F;r[1]=0x04;r[2]=0x04;r[3]=0x04;r[4]=0x04;r[5]=0x04;r[6]=0x04;break;
    case 'U': r[0]=0x11;r[1]=0x11;r[2]=0x11;r[3]=0x11;r[4]=0x11;r[5]=0x11;r[6]=0x0E;break;
    case 'V': r[0]=0x11;r[1]=0x11;r[2]=0x11;r[3]=0x11;r[4]=0x11;r[5]=0x0A;r[6]=0x04;break;
    case 'W': r[0]=0x11;r[1]=0x11;r[2]=0x11;r[3]=0x15;r[4]=0x15;r[5]=0x1B;r[6]=0x11;break;
    case 'X': r[0]=0x11;r[1]=0x11;r[2]=0x0A;r[3]=0x04;r[4]=0x0A;r[5]=0x11;r[6]=0x11;break;
    case 'Y': r[0]=0x11;r[1]=0x11;r[2]=0x0A;r[3]=0x04;r[4]=0x04;r[5]=0x04;r[6]=0x04;break;
    case 'Z': r[0]=0x1F;r[1]=0x01;r[2]=0x02;r[3]=0x04;r[4]=0x08;r[5]=0x10;r[6]=0x1F;break;
    case '0': r[0]=0x0E;r[1]=0x11;r[2]=0x13;r[3]=0x15;r[4]=0x19;r[5]=0x11;r[6]=0x0E;break;
    case '1': r[0]=0x04;r[1]=0x0C;r[2]=0x04;r[3]=0x04;r[4]=0x04;r[5]=0x04;r[6]=0x0E;break;
    case '2': r[0]=0x0E;r[1]=0x11;r[2]=0x01;r[3]=0x02;r[4]=0x04;r[5]=0x08;r[6]=0x1F;break;
    case '3': r[0]=0x1F;r[1]=0x02;r[2]=0x04;r[3]=0x02;r[4]=0x01;r[5]=0x11;r[6]=0x0E;break;
    case '4': r[0]=0x02;r[1]=0x06;r[2]=0x0A;r[3]=0x12;r[4]=0x1F;r[5]=0x02;r[6]=0x02;break;
    case '5': r[0]=0x1F;r[1]=0x10;r[2]=0x1E;r[3]=0x01;r[4]=0x01;r[5]=0x11;r[6]=0x0E;break;
    case '6': r[0]=0x06;r[1]=0x08;r[2]=0x10;r[3]=0x1E;r[4]=0x11;r[5]=0x11;r[6]=0x0E;break;
    case '7': r[0]=0x1F;r[1]=0x01;r[2]=0x02;r[3]=0x04;r[4]=0x08;r[5]=0x08;r[6]=0x08;break;
    case '8': r[0]=0x0E;r[1]=0x11;r[2]=0x11;r[3]=0x0E;r[4]=0x11;r[5]=0x11;r[6]=0x0E;break;
    case '9': r[0]=0x0E;r[1]=0x11;r[2]=0x11;r[3]=0x0F;r[4]=0x01;r[5]=0x02;r[6]=0x0C;break;
    case '-': r[0]=0x00;r[1]=0x00;r[2]=0x00;r[3]=0x1F;r[4]=0x00;r[5]=0x00;r[6]=0x00;break;
    case '.': r[0]=0x00;r[1]=0x00;r[2]=0x00;r[3]=0x00;r[4]=0x00;r[5]=0x0C;r[6]=0x0C;break;
    case ',': r[0]=0x00;r[1]=0x00;r[2]=0x00;r[3]=0x00;r[4]=0x0C;r[5]=0x04;r[6]=0x08;break;
    case '!': r[0]=0x04;r[1]=0x04;r[2]=0x04;r[3]=0x04;r[4]=0x04;r[5]=0x00;r[6]=0x04;break;
    case '?': r[0]=0x0E;r[1]=0x11;r[2]=0x01;r[3]=0x02;r[4]=0x04;r[5]=0x00;r[6]=0x04;break;
    case ':': r[0]=0x00;r[1]=0x0C;r[2]=0x0C;r[3]=0x00;r[4]=0x0C;r[5]=0x0C;r[6]=0x00;break;
    case '/': r[0]=0x01;r[1]=0x01;r[2]=0x02;r[3]=0x04;r[4]=0x08;r[5]=0x10;r[6]=0x10;break;
    case '(': r[0]=0x02;r[1]=0x04;r[2]=0x08;r[3]=0x08;r[4]=0x08;r[5]=0x04;r[6]=0x02;break;
    case ')': r[0]=0x08;r[1]=0x04;r[2]=0x02;r[3]=0x02;r[4]=0x02;r[5]=0x04;r[6]=0x08;break;
    case '+': r[0]=0x00;r[1]=0x04;r[2]=0x04;r[3]=0x1F;r[4]=0x04;r[5]=0x04;r[6]=0x00;break;
    case '*': r[0]=0x00;r[1]=0x04;r[2]=0x15;r[3]=0x0E;r[4]=0x15;r[5]=0x04;r[6]=0x00;break;
    case '#': r[0]=0x0A;r[1]=0x0A;r[2]=0x1F;r[3]=0x0A;r[4]=0x1F;r[5]=0x0A;r[6]=0x0A;break;
    case '_': r[0]=0x00;r[1]=0x00;r[2]=0x00;r[3]=0x00;r[4]=0x00;r[5]=0x00;r[6]=0x1F;break;
    case '\'': r[0]=0x04;r[1]=0x04;r[2]=0x08;r[3]=0x00;r[4]=0x00;r[5]=0x00;r[6]=0x00;break;
    case '%': r[0]=0x19;r[1]=0x1A;r[2]=0x02;r[3]=0x04;r[4]=0x08;r[5]=0x0B;r[6]=0x13;break;
    case '=': r[0]=0x00;r[1]=0x00;r[2]=0x1F;r[3]=0x00;r[4]=0x1F;r[5]=0x00;r[6]=0x00;break;
    case '[': r[0]=0x0E;r[1]=0x08;r[2]=0x08;r[3]=0x08;r[4]=0x08;r[5]=0x08;r[6]=0x0E;break;
    case ']': r[0]=0x0E;r[1]=0x02;r[2]=0x02;r[3]=0x02;r[4]=0x02;r[5]=0x02;r[6]=0x0E;break;
    case '|': r[0]=0x04;r[1]=0x04;r[2]=0x04;r[3]=0x04;r[4]=0x04;r[5]=0x04;r[6]=0x04;break;
    case '"': r[0]=0x0A;r[1]=0x0A;r[2]=0x0A;r[3]=0x00;r[4]=0x00;r[5]=0x00;r[6]=0x00;break;
    case '$': r[0]=0x04;r[1]=0x0F;r[2]=0x14;r[3]=0x0E;r[4]=0x05;r[5]=0x1E;r[6]=0x04;break;
    case '&': r[0]=0x0C;r[1]=0x12;r[2]=0x14;r[3]=0x08;r[4]=0x15;r[5]=0x12;r[6]=0x0D;break;
    case ';': r[0]=0x00;r[1]=0x0C;r[2]=0x0C;r[3]=0x00;r[4]=0x0C;r[5]=0x0C;r[6]=0x08;break;
    case '<': r[0]=0x02;r[1]=0x04;r[2]=0x08;r[3]=0x10;r[4]=0x08;r[5]=0x04;r[6]=0x02;break;
    case '>': r[0]=0x08;r[1]=0x04;r[2]=0x02;r[3]=0x01;r[4]=0x02;r[5]=0x04;r[6]=0x08;break;
    case '@': r[0]=0x0E;r[1]=0x11;r[2]=0x1B;r[3]=0x15;r[4]=0x15;r[5]=0x10;r[6]=0x0E;break;
    case '\\': r[0]=0x10;r[1]=0x10;r[2]=0x08;r[3]=0x04;r[4]=0x02;r[5]=0x01;r[6]=0x01;break;
    case '^': r[0]=0x04;r[1]=0x0A;r[2]=0x11;r[3]=0x00;r[4]=0x00;r[5]=0x00;r[6]=0x00;break;
    case '`': r[0]=0x08;r[1]=0x04;r[2]=0x02;r[3]=0x00;r[4]=0x00;r[5]=0x00;r[6]=0x00;break;
    case '{': r[0]=0x02;r[1]=0x04;r[2]=0x04;r[3]=0x08;r[4]=0x04;r[5]=0x04;r[6]=0x02;break;
    case '}': r[0]=0x08;r[1]=0x04;r[2]=0x04;r[3]=0x02;r[4]=0x04;r[5]=0x04;r[6]=0x08;break;
    case '~': r[0]=0x00;r[1]=0x00;r[2]=0x0A;r[3]=0x15;r[4]=0x00;r[5]=0x00;r[6]=0x00;break;
    case 0x2022: r[0]=0x00;r[1]=0x00;r[2]=0x0C;r[3]=0x0C;r[4]=0x0C;r[5]=0x00;r[6]=0x00;break;
    case 0x00B0: r[0]=0x0E;r[1]=0x0A;r[2]=0x0E;r[3]=0x00;r[4]=0x00;r[5]=0x00;r[6]=0x00;break;
    default: r[0]=0;r[1]=0;r[2]=0;r[3]=0;r[4]=0;r[5]=0;r[6]=0;break;
    }
}

static int base_latin(uint32_t c, uint32_t *out) {
    switch (c) {
    case 0xC0: case 0xC1: case 0xC2: case 0xC3: case 0xC4: case 0xC5:
    case 0xE0: case 0xE1: case 0xE2: case 0xE3: case 0xE4: case 0xE5:
    case 0x100: case 0x101: case 0x102: case 0x103: case 0x104: case 0x105:
        *out = 'A'; return 1;
    case 0xC7: case 0xE7: case 0x106: case 0x107: case 0x108: case 0x109:
    case 0x10A: case 0x10B: case 0x10C: case 0x10D:
        *out = 'C'; return 1;
    case 0xD0: case 0xF0: case 0x10E: case 0x10F: case 0x110: case 0x111:
        *out = 'D'; return 1;
    case 0xC8: case 0xC9: case 0xCA: case 0xCB: case 0xE8: case 0xE9:
    case 0xEA: case 0xEB: case 0x112: case 0x113: case 0x114: case 0x115:
    case 0x116: case 0x117: case 0x118: case 0x119: case 0x11A: case 0x11B:
        *out = 'E'; return 1;
    case 0x11C: case 0x11D: case 0x11E: case 0x11F: case 0x120: case 0x121:
    case 0x122: case 0x123:
        *out = 'G'; return 1;
    case 0x124: case 0x125: case 0x126: case 0x127:
        *out = 'H'; return 1;
    case 0xCC: case 0xCD: case 0xCE: case 0xCF: case 0xEC: case 0xED:
    case 0xEE: case 0xEF: case 0x128: case 0x129: case 0x12A: case 0x12B:
    case 0x12C: case 0x12D: case 0x12E: case 0x12F: case 0x130:
        *out = 'I'; return 1;
    case 0x134: case 0x135:
        *out = 'J'; return 1;
    case 0x136: case 0x137: case 0x138:
        *out = 'K'; return 1;
    case 0x139: case 0x13A: case 0x13B: case 0x13C: case 0x13D: case 0x13E:
    case 0x13F: case 0x140: case 0x141: case 0x142:
        *out = 'L'; return 1;
    case 0xD1: case 0xF1: case 0x143: case 0x144: case 0x145: case 0x146:
    case 0x147: case 0x148:
        *out = 'N'; return 1;
    case 0xD2: case 0xD3: case 0xD4: case 0xD5: case 0xD6: case 0xD8:
    case 0xF2: case 0xF3: case 0xF4: case 0xF5: case 0xF6: case 0xF8:
    case 0x14C: case 0x14D: case 0x14E: case 0x14F: case 0x150: case 0x151:
        *out = 'O'; return 1;
    case 0x154: case 0x155: case 0x156: case 0x157: case 0x158: case 0x159:
        *out = 'R'; return 1;
    case 0x15A: case 0x15B: case 0x15C: case 0x15D: case 0x15E: case 0x15F:
    case 0x160: case 0x161:
        *out = 'S'; return 1;
    case 0x162: case 0x163: case 0x164: case 0x165: case 0x166: case 0x167:
        *out = 'T'; return 1;
    case 0xD9: case 0xDA: case 0xDB: case 0xDC: case 0xF9: case 0xFA:
    case 0xFB: case 0xFC: case 0x168: case 0x169: case 0x16A: case 0x16B:
    case 0x16C: case 0x16D: case 0x16E: case 0x16F: case 0x170: case 0x171:
    case 0x172: case 0x173:
        *out = 'U'; return 1;
    case 0x174: case 0x175:
        *out = 'W'; return 1;
    case 0xDD: case 0xFD: case 0xFF: case 0x176: case 0x177:
        *out = 'Y'; return 1;
    case 0x179: case 0x17A: case 0x17B: case 0x17C: case 0x17D: case 0x17E:
        *out = 'Z'; return 1;
    case 0xDE: case 0xFE:
        *out = 'P'; return 1;
    case 0xDF: case 0x1E9E:
        *out = 'S'; return 1;
    case 0xE6: case 0xC6:
        *out = 'A'; return 1;
    case 0x153: case 0x152:
        *out = 'O'; return 1;
    case 0xA1:
        *out = '!'; return 1;
    case 0xBF:
        *out = '?'; return 1;
    case 0x201C: case 0x201D: case 0x201E: case 0xAB: case 0xBB:
    case 0x2039: case 0x203A:
        *out = '"'; return 1;
    case 0x2018: case 0x2019: case 0x201A:
        *out = '\''; return 1;
    case 0x2013: case 0x2014: case 0x2010: case 0x2011:
        *out = '-'; return 1;
    case 0x2026: case 0xB7:
        *out = '.'; return 1;
    case 0xA0: case 0x09: case 0x0D:
    case 0x2000: case 0x2001: case 0x2002: case 0x2003: case 0x2004:
    case 0x2005: case 0x2006: case 0x2007: case 0x2008: case 0x2009:
    case 0x200A: case 0x200B: case 0x202F: case 0x205F: case 0x3000:
        *out = ' '; return 1;
    default:
        return 0;
    }
}

static void resolve_glyph(uint32_t c, Glyph *g, int *w, int *h) {
    uint8_t b[7];
    block_glyph(c, b);
    int any = 0;
    for (int i = 0; i < 7; i++) {
        if (b[i]) {
            any = 1;
            break;
        }
    }
    if (c == ' ' || any) {
        g->kind = 0;
        memcpy(g->block, b, 7);
        *w = 5;
        *h = 7;
        return;
    }
    uint32_t base;
    if (base_latin(c, &base)) {
        block_glyph(base, b);
        g->kind = 0;
        memcpy(g->block, b, 7);
        *w = 5;
        *h = 7;
        return;
    }
    int uw, uh;
    if (c <= 0x10FFFF && unifont_get_size(c, &uw, &uh)) {
        g->kind = 1;
        g->cp = c;
        g->w = uw;
        *w = uw;
        *h = 16;
        return;
    }
    g->kind = 2;
    *w = 5;
    *h = 7;
}

static int glyph_on(const Glyph *g, int x, int y, int h) {
    if (g->kind == 0) {
        int oy = (h - 7) / 2;
        if (oy < 0)
            oy = 0;
        if (y < oy || y >= oy + 7 || x >= 5)
            return 0;
        return (g->block[y - oy] >> (4 - x)) & 1;
    }
    if (g->kind == 1)
        return unifont_get_pixel(g->cp, x, y);
    return 0;
}

static int cell_width(uint32_t c) {
    if (c < 0x80)
        return 1;
    if ((c >= 0x1100 && c <= 0x115F) || (c >= 0x2E80 && c <= 0xA4CF) ||
        (c >= 0xAC00 && c <= 0xD7A3) || (c >= 0xF900 && c <= 0xFAFF) ||
        (c >= 0xFE30 && c <= 0xFE4F) || (c >= 0xFF00 && c <= 0xFF60) ||
        (c >= 0xFFE0 && c <= 0xFFE6) || (c >= 0x20000 && c <= 0x3FFFD))
        return 2;
    return 1;
}

static const uint8_t *render_glyph(const Renderer *r, int gi) {
    if (gi <= 0 || r->glyph_count == 0)
        return (const uint8_t *)" ";
    int n = (int)r->glyph_count;
    int idx = (int)(((gi - 1) * (double)(n - 1) / 7.0) + 0.5);
    if (idx < 0)
        idx = 0;
    if (idx >= n)
        idx = n - 1;
    return r->glyphs[idx];
}

static void bar_color_buf(const Renderer *r, unsigned from_bottom, unsigned rows,
                          uint8_t **out, size_t *len) {
    if (r->grad_lo_term >= 0 && r->grad_hi_term >= 0) {
        double frac = rows > 1 ? (double)from_bottom / (double)(rows - 1) : 0.0;
        term_esc_buf(frac < 0.5 ? r->grad_lo_term : r->grad_hi_term, 0, out, len);
        return;
    }
    unsigned lo_r = (r->grad_lo >> 16) & 0xff;
    unsigned lo_g = (r->grad_lo >> 8) & 0xff;
    unsigned lo_b = r->grad_lo & 0xff;
    unsigned hi_r = (r->grad_hi >> 16) & 0xff;
    unsigned hi_g = (r->grad_hi >> 8) & 0xff;
    unsigned hi_b = r->grad_hi & 0xff;
    double frac = rows > 1 ? (double)from_bottom / (double)(rows - 1) : 0.0;
    unsigned cr = (unsigned)(lo_r + (hi_r - lo_r) * frac + 0.5);
    unsigned cg = (unsigned)(lo_g + (hi_g - lo_g) * frac + 0.5);
    unsigned cb = (unsigned)(lo_b + (hi_b - lo_b) * frac + 0.5);
    if (cr > 255)
        cr = 255;
    if (cg > 255)
        cg = 255;
    if (cb > 255)
        cb = 255;
    char tmp[32];
    int n;
    if (r->color_256) {
        unsigned idx = 16 + 36 * ((cr * 6) / 256) + 6 * ((cg * 6) / 256) + (cb * 6) / 256;
        n = snprintf(tmp, sizeof tmp, "\x1b[38;5;%um", idx);
    } else {
        n = snprintf(tmp, sizeof tmp, "\x1b[38;2;%u;%u;%um", cr, cg, cb);
    }
    *out = malloc((size_t)n);
    memcpy(*out, tmp, (size_t)n);
    *len = (size_t)n;
}

static void row_colors(Renderer *r) {
    if (r->gs_lo == r->grad_lo && r->gs_hi == r->grad_hi &&
        r->gs_256 == r->color_256 && r->gs_rows == r->rows &&
        r->gs_lo_term == r->grad_lo_term && r->gs_hi_term == r->grad_hi_term)
        return;
    r->gs_lo = r->grad_lo;
    r->gs_hi = r->grad_hi;
    r->gs_256 = r->color_256;
    r->gs_rows = r->rows;
    r->gs_lo_term = r->grad_lo_term;
    r->gs_hi_term = r->grad_hi_term;
    for (size_t y = 0; y < r->rows; y++) {
        free(r->row_col[y]);
        bar_color_buf(r, (unsigned)(r->rows - 1 - y), (unsigned)r->rows,
                      &r->row_col[y], &r->row_col_len[y]);
    }
}

static void emit_color_state(ColorState *st, const uint8_t *pre, size_t prelen, Out *o) {
    if (st->active && st->len == prelen && !memcmp(st->col, pre, prelen))
        return;
    out_s(o, pre, prelen);
    if (st->cap < prelen) {
        free(st->col);
        st->col = malloc(prelen ? prelen : 1);
        st->cap = prelen;
    }
    memcpy(st->col, pre, prelen);
    st->len = prelen;
    st->active = 1;
}

static void emit_cell(Renderer *r, size_t y, size_t x, int gi, ColorState *st, Out *o) {
    size_t idx = y * r->cols + x;
    if (r->prev[idx] == (uint8_t)gi)
        return;
    r->prev[idx] = (uint8_t)gi;
    seek_cell((unsigned)y, (unsigned)x, o);
    if (gi == 0) {
        out_s(o, " ", 1);
    } else {
        emit_color_state(st, r->row_col[y], r->row_col_len[y], o);
        const uint8_t *g = render_glyph(r, gi);
        out_s(o, g, strlen((const char *)g));
    }
}

Renderer *renderer_new(size_t rows, size_t cols, size_t bar_width,
                       size_t bar_spacing, size_t num_bars) {
    Renderer *r = calloc(1, sizeof *r);
    if (!r)
        return NULL;
    r->rows = rows;
    r->cols = cols;
    r->bar_width = bar_width ? bar_width : 1;
    r->bar_spacing = bar_spacing;
    r->num_bars = num_bars;
    r->grad_lo = 0xff0000u;
    r->grad_hi = 0x00ff00u;
    r->grad_lo_term = -1;
    r->grad_hi_term = -1;
    r->mode = RM_BARS;
    r->gs_lo = 1;
    r->gs_hi = 0;
    r->gs_256 = -1;
    r->gs_rows = (size_t)-1;
    r->gs_lo_term = -2;
    r->gs_hi_term = -2;
    size_t ncells = rows * cols;
    if (ncells == 0)
        ncells = 1;
    r->prev = malloc(ncells);
    r->rowbuf = malloc(cols ? cols : 1);
    r->osc_glow = malloc(ncells);
    r->row_col = calloc(rows ? rows : 1, sizeof(uint8_t *));
    r->row_col_len = calloc(rows ? rows : 1, sizeof(size_t));
    if (!r->prev || !r->rowbuf || !r->osc_glow || !r->row_col || !r->row_col_len) {
        renderer_free(r);
        return NULL;
    }
    memset(r->prev, 0xFF, rows * cols);
    r->db_x1 = cols > 0 ? cols - 1 : 0;
    r->db_y1 = rows > 0 ? rows - 1 : 0;
    r->wave_rate = 48000;
    r->wave_spc = 1;
    r->osc_spc = 1;
    const char *init = "SHARKVIS";
    size_t n = strlen(init);
    r->text = malloc(n * sizeof(uint32_t));
    for (size_t i = 0; i < n; i++)
        r->text[i] = (uint32_t)init[i];
    r->text_len = n;
    r->text_size = 1;
    r->yscale = 1;
    renderer_set_glyphs(r, NULL, 0);
    return r;
}

void renderer_free(Renderer *r) {
    if (!r)
        return;
    free(r->prev);
    free(r->rowbuf);
    free(r->osc_glow);
    for (size_t i = 0; i < r->rows; i++)
        free(r->row_col[i]);
    free(r->row_col);
    free(r->row_col_len);
    for (int i = 0; i < 9; i++)
        free(r->barstr[i]);
    free(r->spacestr);
    for (size_t i = 0; i < r->glyph_count; i++)
        free(r->glyphs[i]);
    free(r->glyphs);
    free(r->glyph_len);
    free(r->wave_buf);
    free(r->osc_l);
    free(r->osc_r);
    free(r->sc_yrow);
    free(r->sc_lo);
    free(r->sc_hi);
    free(r->sc_lo2);
    free(r->sc_hi2);
    free(r->text);
    free(r->text_dim);
    free(r);
}

void renderer_resize(Renderer *r, size_t rows, size_t cols, size_t num_bars) {
    size_t old_rows = r->rows;
    r->rows = rows;
    r->cols = cols;
    r->num_bars = num_bars;
    r->barstr_bw = 0;
    free(r->prev);
    free(r->osc_glow);
    if (r->row_col) {
        for (size_t i = 0; i < old_rows; i++)
            free(r->row_col[i]);
    }
    free(r->row_col);
    free(r->row_col_len);
    r->row_col = calloc(rows ? rows : 1, sizeof(uint8_t *));
    r->row_col_len = calloc(rows ? rows : 1, sizeof(size_t));
    size_t ncells = rows * cols;
    if (ncells == 0)
        ncells = 1;
    r->prev = malloc(ncells);
    r->osc_glow = malloc(ncells);
    if (r->prev)
        memset(r->prev, 0xFF, rows * cols);
    if (r->osc_glow)
        memset(r->osc_glow, 0, rows * cols);
    free(r->rowbuf);
    r->rowbuf = malloc(cols ? cols : 1);
    r->gs_rows = (size_t)-1;
    r->db_x0 = 0;
    r->db_y0 = 0;
    r->db_x1 = cols > 0 ? cols - 1 : 0;
    r->db_y1 = rows > 0 ? rows - 1 : 0;
}

void renderer_set_offset(Renderer *r, size_t x_off) {
    if (r->x_off == x_off)
        return;
    r->x_off = x_off;
    memset(r->prev, 0xFF, r->rows * r->cols);
    memset(r->osc_glow, 0, r->rows * r->cols);
    r->db_x0 = 0;
    r->db_y0 = 0;
    r->db_x1 = r->cols > 0 ? r->cols - 1 : 0;
    r->db_y1 = r->rows > 0 ? r->rows - 1 : 0;
}

void renderer_set_mode(Renderer *r, RenderMode m) {
    if (r->mode == m)
        return;
    r->mode = m;
    renderer_clear(r);
    if (m == RM_SCOPE)
        memset(r->osc_glow, 0, r->rows * r->cols);
}

RenderMode mode_parse(const char *name) {
    if (!strcmp(name, "wave"))
        return RM_WAVE;
    if (!strcmp(name, "oscilloscope") || !strcmp(name, "lissajous"))
        return RM_SCOPE;
    if (!strcmp(name, "lyrics") || !strcmp(name, "text"))
        return RM_LYRICS;
    return RM_BARS;
}

void renderer_set_text(Renderer *r, const char *s) {
    size_t n = 0;
    uint32_t *v = decode_str(s, &n);
    int same = n == r->text_len && !memcmp(v, r->text, n * sizeof(uint32_t));
    if (!same) {
        free(r->text);
        r->text = v;
        r->text_len = n;
        r->focus = n > 0 ? n - 1 : 0;
        free(r->text_dim);
        r->text_dim = NULL;
        renderer_clear(r);
    } else {
        free(v);
    }
}

void renderer_set_rich(Renderer *r, const char **strs, const char *cur, size_t n) {
    size_t cap = 64;
    uint32_t *text = malloc(cap * sizeof(uint32_t));
    char *dim = malloc(cap);
    size_t m = 0;
    size_t focus = 0;
    int found = 0;
    for (size_t i = 0; i < n; i++) {
        if (i > 0) {
            if (m >= cap) {
                cap *= 2;
                text = realloc(text, cap * sizeof(uint32_t));
                dim = realloc(dim, cap);
            }
            text[m] = '\n';
            dim[m] = 0;
            m++;
        }
        if (cur[i] && !found) {
            focus = m;
            found = 1;
        }
        size_t cn = 0;
        uint32_t *cv = decode_str(strs[i], &cn);
        for (size_t k = 0; k < cn; k++) {
            if (m >= cap) {
                cap *= 2;
                text = realloc(text, cap * sizeof(uint32_t));
                dim = realloc(dim, cap);
            }
            text[m] = cv[k];
            dim[m] = cur[i] ? 0 : 1;
            m++;
        }
        free(cv);
    }
    int same = m == r->text_len && !memcmp(text, r->text, m * sizeof(uint32_t));
    if (!same) {
        free(r->text);
        free(r->text_dim);
        r->text = text;
        r->text_dim = dim;
        r->text_len = m;
        r->focus = focus;
        renderer_clear(r);
    } else {
        free(text);
        free(dim);
    }
}

void renderer_set_wave(Renderer *r, unsigned sample_rate) {
    unsigned rate = sample_rate ? sample_rate : 48000;
    r->wave_rate = rate;
    size_t cap = sample_rate > 0 ? (size_t)sample_rate * 2 / 3 : 48000 * 2 / 3;
    if (cap < 4096)
        cap = 4096;
    size_t spc = (size_t)sample_rate / 2000;
    if (spc < 1)
        spc = 1;
    size_t osc_spc = (size_t)sample_rate / 800;
    if (osc_spc < 1)
        osc_spc = 1;
    size_t osc_win = (size_t)sample_rate / 20;
    if (osc_win < 256)
        osc_win = 256;
    if (r->wave_cap == cap) {
        r->wave_spc = spc;
        r->osc_spc = osc_spc;
        r->osc_win = osc_win;
        return;
    }
    free(r->wave_buf);
    free(r->osc_l);
    free(r->osc_r);
    r->wave_buf = calloc(cap, sizeof(double));
    r->osc_l = calloc(cap, sizeof(double));
    r->osc_r = calloc(cap, sizeof(double));
    r->wave_cap = cap;
    r->wave_pos = 0;
    r->wave_filled = 0;
    r->wave_spc = spc;
    r->osc_cap = cap;
    r->osc_pos = 0;
    r->osc_filled = 0;
    r->osc_spc = osc_spc;
    r->osc_win = osc_win;
}

void renderer_feed(Renderer *r, const double *left, const double *right, size_t n) {
    if (r->mode != RM_WAVE && r->mode != RM_SCOPE)
        return;
    if (r->wave_cap == 0 || n == 0)
        return;
    r->stereo_in = right != NULL;
    if (!left)
        return;
    for (size_t i = 0; i < n; i++) {
        double v = left[i];
        if (right)
            v = (v + right[i]) * 0.5;
        r->wave_buf[r->wave_pos] = v;
        r->osc_l[r->osc_pos] = left[i];
        r->osc_r[r->osc_pos] = right ? right[i] : left[i];
        r->wave_pos = (r->wave_pos + 1) % r->wave_cap;
        if (r->wave_filled < r->wave_cap)
            r->wave_filled++;
        r->osc_pos = (r->osc_pos + 1) % r->osc_cap;
        if (r->osc_filled < r->osc_cap)
            r->osc_filled++;
    }
}

void renderer_clear(Renderer *r) {
    memset(r->prev, 0xFF, r->rows * r->cols);
    r->db_x0 = 0;
    r->db_y0 = 0;
    r->db_x1 = r->cols > 0 ? r->cols - 1 : 0;
    r->db_y1 = r->rows > 0 ? r->rows - 1 : 0;
}

void renderer_set_glyphs(Renderer *r, const uint8_t *src, size_t len) {
    if (!src || len == 0) {
        src = (const uint8_t *)SV_DEFAULT_CHARS;
        len = SV_DEFAULT_CHARS_LEN;
    }
    for (size_t i = 0; i < r->glyph_count; i++)
        free(r->glyphs[i]);
    free(r->glyphs);
    free(r->glyph_len);
    r->glyphs = NULL;
    r->glyph_len = NULL;
    r->glyph_count = 0;
    size_t cap = 0;
    size_t p = 0;
    while (p < len && r->glyph_count < 64) {
        uint8_t c = src[p];
        size_t seq;
        if (c < 0x80)
            seq = 1;
        else if ((c & 0xE0) == 0xC0)
            seq = 2;
        else if ((c & 0xF0) == 0xE0)
            seq = 3;
        else
            seq = 4;
        if (r->glyph_count >= cap) {
            cap = cap ? cap * 2 : 16;
            r->glyphs = realloc(r->glyphs, cap * sizeof(uint8_t *));
            r->glyph_len = realloc(r->glyph_len, cap * sizeof(size_t));
        }
        uint8_t *dst = malloc(8);
        size_t m = 0;
        size_t i = 0;
        while (i < seq && i < 7 && p < len) {
            dst[m++] = src[p++];
            i++;
        }
        dst[m] = 0;
        r->glyphs[r->glyph_count] = dst;
        r->glyph_len[r->glyph_count] = m;
        r->glyph_count++;
    }
    if (r->glyph_count == 0) {
        r->glyphs = malloc(sizeof(uint8_t *));
        r->glyph_len = malloc(sizeof(size_t));
        r->glyphs[0] = (uint8_t *)strdup(" ");
        r->glyph_len[0] = 1;
        r->glyph_count = 1;
    }
    r->barstr_bw = 0;
}

static void letter_color_buf(const Renderer *r, double xfrac, double v,
                             uint8_t **out, size_t *len) {
    if (r->grad_lo_term >= 0 && r->grad_hi_term >= 0) {
        double t = xfrac;
        if (t < 0.0)
            t = 0.0;
        if (t > 1.0)
            t = 1.0;
        double vv = v;
        if (vv < 0.0)
            vv = 0.0;
        if (vv > 1.0)
            vv = 1.0;
        term_esc_buf(t < 0.5 ? r->grad_lo_term : r->grad_hi_term, vv < 0.6, out, len);
        return;
    }
    double lo_r = (double)((r->grad_lo >> 16) & 0xff);
    double lo_g = (double)((r->grad_lo >> 8) & 0xff);
    double lo_b = (double)(r->grad_lo & 0xff);
    double hi_r = (double)((r->grad_hi >> 16) & 0xff);
    double hi_g = (double)((r->grad_hi >> 8) & 0xff);
    double hi_b = (double)(r->grad_hi & 0xff);
    double t = xfrac;
    if (t < 0.0)
        t = 0.0;
    if (t > 1.0)
        t = 1.0;
    double vv = v;
    if (vv < 0.0)
        vv = 0.0;
    if (vv > 1.0)
        vv = 1.0;
    double b = 0.10 + 0.90 * vv;
    double cr = (lo_r + (hi_r - lo_r) * t) * b + 0.5;
    double cg = (lo_g + (hi_g - lo_g) * t) * b + 0.5;
    double cb = (lo_b + (hi_b - lo_b) * t) * b + 0.5;
    if (cr < 0.0)
        cr = 0.0;
    if (cr > 255.0)
        cr = 255.0;
    if (cg < 0.0)
        cg = 0.0;
    if (cg > 255.0)
        cg = 255.0;
    if (cb < 0.0)
        cb = 0.0;
    if (cb > 255.0)
        cb = 255.0;
    char tmp[32];
    int n;
    if (r->color_256) {
        unsigned idx = 16 + 36 * (((unsigned)cr * 6) / 256) +
                       6 * (((unsigned)cg * 6) / 256) + ((unsigned)cb * 6) / 256;
        n = snprintf(tmp, sizeof tmp, "\x1b[38;5;%um", idx);
    } else {
        n = snprintf(tmp, sizeof tmp, "\x1b[38;2;%u;%u;%um",
                     (unsigned)cr, (unsigned)cg, (unsigned)cb);
    }
    *out = malloc((size_t)n);
    memcpy(*out, tmp, (size_t)n);
    *len = (size_t)n;
}

typedef struct {
    size_t *idx;
    size_t n;
    size_t cap;
} IdxLine;

typedef struct {
    IdxLine *lines;
    size_t n;
    size_t cap;
} LineVec;

static void idxline_push(IdxLine *l, size_t v) {
    if (l->n >= l->cap) {
        l->cap = l->cap ? l->cap * 2 : 16;
        l->idx = realloc(l->idx, l->cap * sizeof(size_t));
    }
    l->idx[l->n++] = v;
}

static IdxLine *linevec_new_line(LineVec *v) {
    if (v->n >= v->cap) {
        v->cap = v->cap ? v->cap * 2 : 8;
        v->lines = realloc(v->lines, v->cap * sizeof(IdxLine));
    }
    IdxLine *l = &v->lines[v->n++];
    l->idx = NULL;
    l->n = 0;
    l->cap = 0;
    return l;
}

static void linevec_free(LineVec *v) {
    for (size_t i = 0; i < v->n; i++)
        free(v->lines[i].idx);
    free(v->lines);
    v->lines = NULL;
    v->n = 0;
    v->cap = 0;
}

static void wrap_chars(const uint32_t *pc, size_t pclen, const int *widths,
                       size_t base, size_t cap_px, LineVec *out) {
    if (cap_px < 1)
        cap_px = 1;
    IdxLine cur = {0};
    size_t used = 0;
    size_t i = 0;
    while (i < pclen) {
        int has_sep = 0;
        size_t sep = 0;
        while (i < pclen && pc[i] == ' ') {
            sep = i;
            has_sep = 1;
            i++;
        }
        if (i >= pclen)
            break;
        size_t j = i;
        while (j < pclen && pc[j] != ' ')
            j++;
        size_t ww = 0;
        for (size_t t = i; t < j; t++)
            ww += (size_t)widths[base + t] + 1;
        if (ww > 0)
            ww--;
        size_t sep_w = has_sep ? (size_t)widths[base + sep] : 0;
        size_t need = cur.n == 0 ? ww : 1 + sep_w + ww;
        if (cur.n > 0 && used + need > cap_px) {
            IdxLine *nl = linevec_new_line(out);
            *nl = cur;
            cur.idx = NULL;
            cur.n = 0;
            cur.cap = 0;
            used = 0;
        }
        if (cur.n == 0) {
            if (ww <= cap_px) {
                for (size_t t = i; t < j; t++)
                    idxline_push(&cur, base + t);
                used = ww;
            } else {
                size_t k = i;
                while (k < j) {
                    size_t cw = 0;
                    size_t e = k;
                    while (e < j) {
                        size_t add = (size_t)widths[base + e] + (e > k ? 1 : 0);
                        if (cw + add > cap_px)
                            break;
                        cw += add;
                        e++;
                    }
                    if (e == k) {
                        e = k + 1;
                        cw = (size_t)widths[base + k];
                    }
                    for (size_t t = k; t < e; t++)
                        idxline_push(&cur, base + t);
                    k = e;
                    if (k < j) {
                        IdxLine *nl = linevec_new_line(out);
                        *nl = cur;
                        cur.idx = NULL;
                        cur.n = 0;
                        cur.cap = 0;
                        used = 0;
                    } else {
                        used = cw;
                    }
                }
            }
        } else if (has_sep) {
            idxline_push(&cur, base + sep);
            used += 1 + sep_w;
            for (size_t t = i; t < j; t++)
                idxline_push(&cur, base + t);
            used += 1 + ww;
        } else {
            for (size_t t = i; t < j; t++)
                idxline_push(&cur, base + t);
            used += 1 + ww;
        }
        i = j;
    }
    if (cur.n > 0) {
        IdxLine *nl = linevec_new_line(out);
        *nl = cur;
    } else {
        free(cur.idx);
    }
}

static size_t line_height(const IdxLine *line, const int *heights) {
    size_t h = 7;
    for (size_t i = 0; i < line->n; i++) {
        if ((size_t)heights[line->idx[i]] > h)
            h = (size_t)heights[line->idx[i]];
    }
    return h;
}

static size_t layout_text(const uint32_t *chars, size_t m, size_t region_w,
                          size_t rows, size_t focus, size_t max_s, size_t yscale,
                          LineVec *out) {
    size_t ys = yscale > 1 ? yscale : 1;
    out->lines = NULL;
    out->n = 0;
    out->cap = 0;
    if (m == 0 || region_w == 0 || rows == 0)
        return 1;
    size_t auto_s = rows / (7 * ys);
    if (auto_s < 1)
        auto_s = 1;
    size_t nparas = 1;
    for (size_t i = 0; i < m; i++) {
        if (chars[i] == '\n')
            nparas++;
    }
    size_t *pbases = malloc(nparas * sizeof(size_t));
    size_t *plens = malloc(nparas * sizeof(size_t));
    size_t pi = 0;
    size_t start = 0;
    for (size_t i = 0; i <= m; i++) {
        if (i == m || chars[i] == '\n') {
            pbases[pi] = start;
            plens[pi] = i - start;
            pi++;
            start = i + 1;
        }
    }
    int *widths = malloc(m * sizeof(int));
    int *heights = malloc(m * sizeof(int));
    for (size_t i = 0; i < m; i++) {
        Glyph g;
        int w, h;
        resolve_glyph(chars[i], &g, &w, &h);
        widths[i] = w;
        heights[i] = h;
    }
    size_t top_s = max_s == 0 ? auto_s : max_s < auto_s ? max_s : auto_s;
    if (top_s < 1)
        top_s = 1;
    size_t result_s = 1;
    if (max_s != 0) {
        size_t s = top_s;
        size_t sy = s * ys;
        size_t cap_px = region_w / s;
        if (cap_px < 1)
            cap_px = 1;
        for (size_t p = 0; p < nparas; p++) {
            LineVec tmp = {0};
            wrap_chars(chars + pbases[p], plens[p], widths, pbases[p], cap_px, &tmp);
            for (size_t l = 0; l < tmp.n; l++) {
                IdxLine *nl = linevec_new_line(out);
                *nl = tmp.lines[l];
            }
            free(tmp.lines);
        }
        if (out->n == 0) {
            free(pbases);
            free(plens);
            free(widths);
            free(heights);
            return s;
        }
        size_t total = out->n - 1;
        total *= sy;
        for (size_t l = 0; l < out->n; l++)
            total += line_height(&out->lines[l], heights) * sy;
        if (total > rows) {
            size_t fi = 0;
            for (size_t l = 0; l < out->n; l++) {
                for (size_t k = 0; k < out->lines[l].n; k++) {
                    if (out->lines[l].idx[k] == focus) {
                        fi = l;
                        break;
                    }
                }
            }
            size_t lo = fi, hi = fi;
            size_t used = line_height(&out->lines[fi], heights) * sy;
            for (;;) {
                int grew = 0;
                if (hi + 1 < out->n) {
                    size_t h = line_height(&out->lines[hi + 1], heights) * sy;
                    if (used + sy + h <= rows) {
                        hi++;
                        used += sy + h;
                        grew = 1;
                    }
                }
                if (lo > 0) {
                    size_t h = line_height(&out->lines[lo - 1], heights) * sy;
                    if (used + sy + h <= rows) {
                        lo--;
                        used += sy + h;
                        grew = 1;
                    }
                }
                if (!grew)
                    break;
            }
            LineVec win = {0};
            for (size_t l = lo; l <= hi; l++) {
                IdxLine *nl = linevec_new_line(&win);
                *nl = out->lines[l];
            }
            free(out->lines);
            *out = win;
        }
        result_s = s;
        free(pbases);
        free(plens);
        free(widths);
        free(heights);
        return result_s;
    }
    for (size_t s = top_s; s >= 1; s--) {
        size_t cap_px = region_w / s;
        if (cap_px < 1)
            cap_px = 1;
        LineVec tmp = {0};
        for (size_t p = 0; p < nparas; p++) {
            LineVec one = {0};
            wrap_chars(chars + pbases[p], plens[p], widths, pbases[p], cap_px, &one);
            for (size_t l = 0; l < one.n; l++) {
                IdxLine *nl = linevec_new_line(&tmp);
                *nl = one.lines[l];
            }
            free(one.lines);
        }
        if (tmp.n == 0) {
            linevec_free(&tmp);
            if (s == 1)
                break;
            continue;
        }
        size_t sy = s * ys;
        size_t need_h = tmp.n - 1;
        need_h *= sy;
        for (size_t l = 0; l < tmp.n; l++)
            need_h += line_height(&tmp.lines[l], heights) * sy;
        if (need_h <= rows) {
            *out = tmp;
            result_s = s;
            free(pbases);
            free(plens);
            free(widths);
            free(heights);
            return result_s;
        }
        linevec_free(&tmp);
        if (s == 1)
            break;
    }
    {
        size_t cap_px = region_w > 1 ? region_w : 1;
        for (size_t p = 0; p < nparas; p++) {
            LineVec one = {0};
            wrap_chars(chars + pbases[p], plens[p], widths, pbases[p], cap_px, &one);
            for (size_t l = 0; l < one.n; l++) {
                IdxLine *nl = linevec_new_line(out);
                *nl = one.lines[l];
            }
            free(one.lines);
        }
        size_t keep = rows / (8 * ys);
        if (keep < 1)
            keep = 1;
        if (out->n > keep) {
            size_t fi = 0;
            for (size_t l = 0; l < out->n; l++) {
                for (size_t k = 0; k < out->lines[l].n; k++) {
                    if (out->lines[l].idx[k] == focus) {
                        fi = l;
                        break;
                    }
                }
            }
            size_t st = fi >= keep - 1 ? fi - (keep - 1) : 0;
            size_t en = st + keep;
            if (en > out->n)
                en = out->n;
            LineVec win = {0};
            for (size_t l = st; l < en; l++) {
                IdxLine *nl = linevec_new_line(&win);
                *nl = out->lines[l];
            }
            for (size_t l = 0; l < st; l++)
                free(out->lines[l].idx);
            for (size_t l = en; l < out->n; l++)
                free(out->lines[l].idx);
            free(out->lines);
            *out = win;
        }
    }
    free(pbases);
    free(plens);
    free(widths);
    free(heights);
    return 1;
}

static size_t spin_frame(uint64_t elapsed_ms) {
    return (size_t)(elapsed_ms / 120 % 8);
}

static void draw_spinner(Renderer *r, size_t x_start, size_t region_w, Out *o) {
    size_t rows = r->rows;
    size_t cols = r->cols;
    if (rows == 0 || region_w == 0)
        return;
    static const size_t PX[8] = {0, 1, 2, 2, 2, 1, 0, 0};
    static const size_t PY[8] = {0, 0, 0, 1, 2, 2, 2, 1};
    uint64_t now = sv_now_ms();
    if (!r->spin_active) {
        r->spin_active = 1;
        r->spin_t0_ms = now;
    }
    size_t active = spin_frame(now - r->spin_t0_ms);
    size_t ys = r->yscale > 1 ? r->yscale : 1;
    size_t bw = 4;
    size_t bh = 2 * ys;
    size_t px = bw + 2;
    size_t py = bh + ys;
    size_t sub = 2 * px + bw;
    size_t ox = x_start + (region_w > sub ? (region_w - sub) / 2 : 0);
    size_t sub2 = 2 * py + bh;
    size_t oy = rows > sub2 ? (rows - sub2) / 2 : 0;
    size_t x_end = x_start + region_w;
    if (x_end > cols)
        x_end = cols;
    const uint8_t *full = render_glyph(r, 8);
    size_t fulllen = strlen((const char *)full);
    uint8_t *esc_on;
    size_t esc_on_len;
    uint8_t *esc_off;
    size_t esc_off_len;
    letter_color_buf(r, 0.5, 1.0, &esc_on, &esc_on_len);
    letter_color_buf(r, 0.5, 0.12, &esc_off, &esc_off_len);
    size_t bx[8], by[8];
    for (size_t i = 0; i < 8; i++) {
        int on = i == active;
        uint8_t marker = on ? 79 : 66;
        bx[i] = ox + PX[i] * px;
        by[i] = oy + PY[i] * py;
        int changed = 0;
        for (size_t dy = 0; dy < bh && !changed; dy++) {
            for (size_t dx = 0; dx < bw; dx++) {
                size_t x = bx[i] + dx;
                size_t y = by[i] + dy;
                if (x >= x_end || y >= rows)
                    continue;
                if (r->prev[y * cols + x] != marker) {
                    changed = 1;
                    break;
                }
            }
        }
        if (!changed)
            continue;
        for (size_t dy = 0; dy < bh; dy++) {
            size_t y = by[i] + dy;
            if (y >= rows)
                break;
            seek_cell((unsigned)y, (unsigned)bx[i], o);
            if (on)
                out_s(o, esc_on, esc_on_len);
            else
                out_s(o, esc_off, esc_off_len);
            for (size_t dx = 0; dx < bw; dx++) {
                size_t x = bx[i] + dx;
                if (x >= x_end)
                    break;
                r->prev[y * cols + x] = marker;
                out_s(o, full, fulllen);
            }
        }
    }
    for (size_t y = 0; y < rows; y++) {
        for (size_t x = x_start; x < x_end; x++) {
            int in_box = 0;
            for (size_t i = 0; i < 8; i++) {
                if (y >= by[i] && y < by[i] + bh && x >= bx[i] && x < bx[i] + bw) {
                    in_box = 1;
                    break;
                }
            }
            if (in_box)
                continue;
            size_t idx = y * cols + x;
            if (r->prev[idx] != 0) {
                seek_cell((unsigned)y, (unsigned)x, o);
                out_s(o, " ", 1);
                r->prev[idx] = 0;
            }
        }
    }
    free(esc_on);
    free(esc_off);
}

static void clear_text_region(Renderer *r, size_t x_start, size_t region_w, Out *o) {
    size_t x_end = x_start + region_w;
    if (x_end > r->cols)
        x_end = r->cols;
    for (size_t y = 0; y < r->rows; y++) {
        for (size_t x = x_start; x < x_end; x++) {
            size_t idx = y * r->cols + x;
            if (r->prev[idx] != 0) {
                seek_cell((unsigned)y, (unsigned)x, o);
                out_s(o, " ", 1);
                r->prev[idx] = 0;
            }
        }
    }
}

static void draw_small_text(Renderer *r, size_t x_start, size_t region_w, Out *o) {
    size_t rows = r->rows;
    size_t cols = r->cols;
    if (rows == 0 || region_w == 0)
        return;
    if (r->text_len == 0) {
        clear_text_region(r, x_start, region_w, o);
        return;
    }
    uint8_t marker = 79;
    uint8_t *esc;
    size_t esc_len;
    letter_color_buf(r, 0.5, 1.0, &esc, &esc_len);
    uint8_t *grey_esc;
    size_t grey_len;
    if (r->grad_lo_term >= 0 && r->grad_hi_term >= 0) {
        term_esc_buf(90, 0, &grey_esc, &grey_len);
    } else if (r->color_256) {
        grey_esc = (uint8_t *)strdup("\x1b[38;5;240m");
        grey_len = 11;
    } else {
        grey_esc = (uint8_t *)strdup("\x1b[38;2;96;96;96m");
        grey_len = 16;
    }
    size_t nparas = 1;
    for (size_t i = 0; i < r->text_len; i++) {
        if (r->text[i] == '\n')
            nparas++;
    }
    typedef struct {
        size_t s;
        size_t e;
        int grey;
    } Span;
    Span *spans = NULL;
    size_t nspans = 0, scap = 0;
    size_t start = 0;
    for (size_t pi = 0; pi < nparas; pi++) {
        size_t pe = start;
        while (pe < r->text_len && r->text[pe] != '\n')
            pe++;
        if (pe <= start) {
            if (nspans >= scap) {
                scap = scap ? scap * 2 : 8;
                spans = realloc(spans, scap * sizeof(Span));
            }
            spans[nspans].s = start;
            spans[nspans].e = start;
            spans[nspans].grey = 0;
            nspans++;
        } else {
            size_t k = start;
            while (k < pe) {
                size_t cells = 0;
                size_t end = k;
                while (end < pe) {
                    size_t cw = (size_t)cell_width(r->text[end]);
                    if (end > k && cells + cw > region_w)
                        break;
                    cells += cw;
                    end++;
                }
                if (end == k)
                    end = k + 1;
                int grey = 1;
                for (size_t t = k; t < end; t++) {
                    char d = t < r->text_len && r->text_dim ? r->text_dim[t] : 0;
                    if (!d) {
                        grey = 0;
                        break;
                    }
                }
                if (nspans >= scap) {
                    scap = scap ? scap * 2 : 8;
                    spans = realloc(spans, scap * sizeof(Span));
                }
                spans[nspans].s = k;
                spans[nspans].e = end;
                spans[nspans].grey = grey;
                nspans++;
                k = end;
            }
        }
        start = pe + 1;
    }
    if (nspans > rows)
        nspans = rows;
    size_t x_end = x_start + region_w;
    if (x_end > cols)
        x_end = cols;
    size_t top = rows > nspans ? (rows - nspans) / 2 : 0;
    size_t *boxx = malloc(nspans * sizeof(size_t));
    size_t *boxy = malloc(nspans * sizeof(size_t));
    size_t *boxw = malloc(nspans * sizeof(size_t));
    for (size_t li = 0; li < nspans; li++) {
        size_t s = spans[li].s;
        size_t e = spans[li].e;
        size_t y = top + li;
        if (y >= rows)
            break;
        size_t cells = 0;
        for (size_t t = s; t < e; t++)
            cells += (size_t)cell_width(r->text[t]);
        size_t x0;
        if (r->text_left) {
            x0 = x_start;
        } else {
            size_t sub = region_w > cells ? region_w - cells : 0;
            x0 = x_start + sub / 2;
        }
        size_t avail = x0 < x_end ? x_end - x0 : 0;
        boxx[li] = x0;
        boxy[li] = y;
        boxw[li] = cells < avail ? cells : avail;
        int changed = 0;
        size_t cx = x0;
        for (size_t t = s; t < e && !changed; t++) {
            for (int q = 0; q < cell_width(r->text[t]); q++) {
                if (cx >= x_end)
                    break;
                if (r->prev[y * cols + cx] != marker) {
                    changed = 1;
                    break;
                }
                cx++;
            }
            if (changed || cx >= x_end)
                break;
        }
        if (!changed)
            continue;
        seek_cell((unsigned)y, (unsigned)x0, o);
        if (spans[li].grey)
            out_s(o, grey_esc, grey_len);
        else
            out_s(o, esc, esc_len);
        cx = x0;
        uint8_t enc[4];
        for (size_t t = s; t < e; t++) {
            if (cx >= x_end)
                break;
            for (int q = 0; q < cell_width(r->text[t]); q++) {
                if (cx >= x_end)
                    break;
                r->prev[y * cols + cx] = marker;
                cx++;
            }
            size_t el = utf8_encode(r->text[t], enc);
            out_s(o, enc, el);
        }
    }
    for (size_t y = 0; y < rows; y++) {
        for (size_t x = x_start; x < x_end; x++) {
            int in_box = 0;
            for (size_t li = 0; li < nspans; li++) {
                if (y == boxy[li] && x >= boxx[li] && x < boxx[li] + boxw[li]) {
                    in_box = 1;
                    break;
                }
            }
            if (in_box)
                continue;
            size_t idx = y * cols + x;
            if (r->prev[idx] != 0) {
                seek_cell((unsigned)y, (unsigned)x, o);
                out_s(o, " ", 1);
                r->prev[idx] = 0;
            }
        }
    }
    free(spans);
    free(boxx);
    free(boxy);
    free(boxw);
    free(esc);
    free(grey_esc);
}

static void draw_text(Renderer *r, size_t x_start, size_t region_w, Out *o) {
    size_t rows = r->rows;
    size_t cols = r->cols;
    if (rows == 0 || region_w == 0)
        return;
    size_t m = r->text_len;
    if (m == 0) {
        clear_text_region(r, x_start, region_w, o);
        return;
    }
    size_t ys = r->yscale > 1 ? r->yscale : 1;
    LineVec lv = {0};
    size_t s = layout_text(r->text, m, region_w, rows, r->focus, r->text_size, ys, &lv);
    if (lv.n == 0) {
        linevec_free(&lv);
        return;
    }
    size_t sy = s * ys;
    int *heights = malloc(m * sizeof(int));
    for (size_t i = 0; i < m; i++) {
        Glyph g;
        int w, h;
        resolve_glyph(r->text[i], &g, &w, &h);
        heights[i] = h;
    }
    size_t total_h = lv.n - 1;
    total_h *= sy;
    for (size_t l = 0; l < lv.n; l++)
        total_h += line_height(&lv.lines[l], heights) * sy;
    size_t top = rows > total_h ? (rows - total_h) / 2 : 0;
    size_t x_end = x_start + region_w;
    if (x_end > cols)
        x_end = cols;
    const uint8_t *full = render_glyph(r, 8);
    size_t fulllen = strlen((const char *)full);
    size_t *boxx = malloc(lv.n * 64 * sizeof(size_t));
    size_t *boxy = malloc(lv.n * 64 * sizeof(size_t));
    size_t *boxw = malloc(lv.n * 64 * sizeof(size_t));
    size_t *boxh = malloc(lv.n * 64 * sizeof(size_t));
    size_t nboxes = 0;
    size_t y0 = top;
    for (size_t li = 0; li < lv.n; li++) {
        IdxLine *line = &lv.lines[li];
        if (line->n == 0)
            continue;
        Glyph *gs = malloc(line->n * sizeof(Glyph));
        int *gw = malloc(line->n * sizeof(int));
        size_t lw = 0;
        size_t lh = 7;
        for (size_t k = 0; k < line->n; k++) {
            int w, h;
            resolve_glyph(r->text[line->idx[k]], &gs[k], &w, &h);
            gw[k] = w;
            lw += (size_t)w + 1;
            if ((size_t)h > lh)
                lh = (size_t)h;
        }
        if (lw > 0)
            lw--;
        size_t wline = lw * s;
        size_t lead = 0;
        if (!r->text_left)
            lead = region_w > wline ? (region_w - wline) / 2 : 0;
        size_t x0 = x_start + lead;
        for (size_t k = 0; k < line->n; k++) {
            size_t ci = line->idx[k];
            int w = gw[k];
            size_t box_w = (k + 1 < line->n) ? ((size_t)w + 1) * s : (size_t)w * s;
            char dd = (r->text_dim && ci < m) ? r->text_dim[ci] : 0;
            double v = dd ? 0.35 : 1.0;
            uint8_t marker = (uint8_t)(64 + (v * 15.0 + 0.5));
            double xfrac = ((double)k + 0.5) / (double)line->n;
            uint8_t *esc;
            size_t esc_len;
            letter_color_buf(r, xfrac, v, &esc, &esc_len);
            boxx[nboxes] = x0;
            boxy[nboxes] = y0;
            boxw[nboxes] = box_w;
            boxh[nboxes] = lh;
            nboxes++;
            int changed = 0;
            for (size_t gr = 0; gr < lh && !changed; gr++) {
                for (size_t pr = 0; pr < sy; pr++) {
                    size_t y = y0 + gr * sy + pr;
                    if (y >= rows)
                        break;
                    for (size_t pxx = 0; pxx < box_w; pxx++) {
                        size_t x = x0 + pxx;
                        if (x >= x_end)
                            break;
                        int on = glyph_on(&gs[k], (int)(pxx / s), (int)gr, (int)lh);
                        uint8_t want = on ? marker : 0;
                        if (r->prev[y * cols + x] != want) {
                            changed = 1;
                            break;
                        }
                    }
                    if (changed)
                        break;
                }
            }
            if (changed) {
                for (size_t gr = 0; gr < lh; gr++) {
                    for (size_t pr = 0; pr < sy; pr++) {
                        size_t y = y0 + gr * sy + pr;
                        if (y >= rows)
                            break;
                        seek_cell((unsigned)y, (unsigned)x0, o);
                        out_s(o, esc, esc_len);
                        for (size_t pxx = 0; pxx < box_w; pxx++) {
                            size_t x = x0 + pxx;
                            if (x >= x_end)
                                break;
                            int on = glyph_on(&gs[k], (int)(pxx / s), (int)gr, (int)lh);
                            uint8_t want = on ? marker : 0;
                            r->prev[y * cols + x] = want;
                            if (on)
                                out_s(o, full, fulllen);
                            else
                                out_s(o, " ", 1);
                        }
                    }
                }
            }
            free(esc);
            x0 += box_w;
        }
        free(gs);
        free(gw);
        y0 += (lh + 1) * sy;
    }
    for (size_t y = 0; y < rows; y++) {
        for (size_t x = x_start; x < x_end; x++) {
            int in_box = 0;
            for (size_t bi = 0; bi < nboxes; bi++) {
                if (y >= boxy[bi] && y < boxy[bi] + boxh[bi] * sy &&
                    x >= boxx[bi] && x < boxx[bi] + boxw[bi]) {
                    in_box = 1;
                    break;
                }
            }
            if (in_box)
                continue;
            size_t idx = y * cols + x;
            if (r->prev[idx] != 0) {
                seek_cell((unsigned)y, (unsigned)x, o);
                out_s(o, " ", 1);
                r->prev[idx] = 0;
            }
        }
    }
    free(boxx);
    free(boxy);
    free(boxw);
    free(boxh);
    free(heights);
    linevec_free(&lv);
}

static void draw_lyrics_mode(Renderer *r, size_t x_start, size_t region_w, Out *o) {
    if (r->loading) {
        draw_spinner(r, x_start, region_w, o);
    } else {
        r->spin_active = 0;
        if (r->text_small)
            draw_small_text(r, x_start, region_w, o);
        else
            draw_text(r, x_start, region_w, o);
    }
}

static void build_barstrings(Renderer *r) {
    size_t bw = r->bar_width ? r->bar_width : 1;
    if (bw > 8)
        bw = 8;
    if (r->barstr_bw == bw)
        return;
    r->barstr_bw = bw;
    for (int gi = 0; gi <= 8; gi++) {
        free(r->barstr[gi]);
        const uint8_t *g = render_glyph(r, gi);
        size_t gl = strlen((const char *)g);
        uint8_t *s = malloc(bw * gl + 1);
        size_t p = 0;
        for (size_t k = 0; k < bw; k++) {
            memcpy(s + p, g, gl);
            p += gl;
        }
        s[p] = 0;
        r->barstr[gi] = s;
        r->barstr_len[gi] = p;
    }
    free(r->spacestr);
    r->spacestr = malloc(bw + 1);
    memset(r->spacestr, ' ', bw);
    r->spacestr[bw] = 0;
    r->spacestr_len = bw;
}

static void draw_bars(Renderer *r, const double *left, const double *right,
                      size_t nbars, size_t per_ch_l, size_t x_start,
                      size_t region_w, Out *o) {
    size_t rows = r->rows;
    size_t cols = r->cols;
    if (rows == 0 || region_w == 0)
        return;
    size_t bw = r->bar_width ? r->bar_width : 1;
    size_t step = bw + r->bar_spacing;
    if (step == 0)
        step = 1;
    size_t used = nbars * step;
    size_t lead = used < region_w ? (region_w - used) / 2 : 0;
    size_t region_end = x_start + region_w;
    if (region_end > cols)
        region_end = cols;
    build_barstrings(r);
    ColorState st = {0};
    for (size_t y = 0; y < rows; y++) {
        size_t fb = rows - 1 - y;
        size_t skip = 0;
        int wrote = 0;
        int color_on = 0;
        for (size_t b = 0; b < nbars; b++) {
            size_t col = x_start + lead + b * step;
            if (col >= region_end)
                break;
            const double *src;
            size_t vi;
            if (b < per_ch_l) {
                src = left;
                vi = per_ch_l - 1 - b;
            } else {
                if (!right)
                    continue;
                src = right;
                vi = b - per_ch_l;
            }
            double v = src[vi];
            if (!(v > 0.0))
                v = 0.0;
            else if (v > 1.0)
                v = 1.0;
            double h = v * (double)rows;
            double frac = h - (double)fb;
            if (!(frac > 0.0))
                frac = 0.0;
            else if (frac > 1.0)
                frac = 1.0;
            int gi = (int)(frac * 8.0 + 0.9999);
            if (gi < 0)
                gi = 0;
            if (gi > 8)
                gi = 8;
            size_t idx = y * cols + col;
            if (r->prev[idx] == (uint8_t)gi) {
                skip += step;
                continue;
            }
            r->prev[idx] = (uint8_t)gi;
            size_t wvis = region_end - col;
            if (wvis > bw)
                wvis = bw;
            for (size_t w = 1; w < wvis; w++)
                r->prev[idx + w] = (uint8_t)gi;
            if (!wrote) {
                seek_cell((unsigned)y, (unsigned)col, o);
                wrote = 1;
            } else if (skip > 0) {
                out_s(o, "\x1b[", 2);
                out_u(o, (unsigned)skip);
                out_s(o, "C", 1);
            }
            skip = 0;
            if (gi > 0) {
                if (!color_on) {
                    emit_color_state(&st, r->row_col[y], r->row_col_len[y], o);
                    color_on = 1;
                }
                if (wvis == bw) {
                    out_s(o, r->barstr[gi], r->barstr_len[gi]);
                } else {
                    const uint8_t *g = render_glyph(r, gi);
                    size_t gl = strlen((const char *)g);
                    for (size_t w = 0; w < wvis; w++)
                        out_s(o, g, gl);
                }
            } else {
                if (wvis == bw) {
                    out_s(o, r->spacestr, r->spacestr_len);
                } else {
                    for (size_t w = 0; w < wvis; w++)
                        out_s(o, " ", 1);
                }
            }
            if (r->bar_spacing != 0 && b + 1 < nbars && col + step < region_end) {
                if (r->bar_spacing == 1) {
                    out_s(o, " ", 1);
                } else {
                    out_s(o, "\x1b[", 2);
                    out_u(o, (unsigned)r->bar_spacing);
                    out_s(o, "C", 1);
                }
            }
        }
    }
    for (size_t col = 0; col < region_w; col++) {
        int in_bar = 0;
        if (col >= lead) {
            size_t t = col - lead;
            if (t / step < nbars && t % step < bw)
                in_bar = 1;
        }
        if (in_bar)
            continue;
        size_t abs_col = x_start + col;
        for (size_t y = 0; y < rows; y++) {
            size_t idx = y * cols + abs_col;
            if (r->prev[idx] == 0xFF)
                continue;
            emit_cell(r, y, abs_col, 0, &st, o);
        }
    }
    free(st.col);
}

static const uint8_t *glyph_for(const Renderer *r, int gi) {
    return render_glyph(r, gi);
}

static void emit_row_raw(uint8_t *prev, size_t cols, const uint8_t *row_col,
                         size_t row_col_len, const Renderer *r, size_t y,
                         size_t x_start, size_t region_w, const uint8_t *tgt,
                         ColorState *st, Out *o) {
    size_t skip = 0;
    int wrote = 0;
    int color_on = 0;
    for (size_t c = 0; c < region_w; c++) {
        uint8_t gi = tgt[c];
        size_t idx = y * cols + x_start + c;
        if (gi == prev[idx]) {
            skip += 1;
            continue;
        }
        prev[idx] = gi;
        if (!wrote) {
            seek_cell((unsigned)y, (unsigned)(x_start + c), o);
            wrote = 1;
        } else if (skip > 0) {
            out_s(o, "\x1b[", 2);
            out_u(o, (unsigned)skip);
            out_s(o, "C", 1);
        }
        skip = 0;
        if (gi > 0) {
            if (!color_on) {
                emit_color_state(st, row_col, row_col_len, o);
                color_on = 1;
            }
            const uint8_t *g = glyph_for(r, (int)gi);
            out_s(o, g, strlen((const char *)g));
        } else {
            out_s(o, " ", 1);
        }
    }
}

static long long rllround(double v) {
    return (long long)(v >= 0 ? v + 0.5 : v - 0.5);
}

static void wave_clip_esc_buf(const Renderer *r, uint8_t **out, size_t *len) {
    if (r->grad_lo_term >= 0 && r->grad_hi_term >= 0) {
        term_esc_buf(31, 0, out, len);
        return;
    }
    if (r->color_256) {
        *out = (uint8_t *)strdup("\x1b[38;5;196m");
        *len = 11;
        return;
    }
    *out = (uint8_t *)strdup("\x1b[38;2;255;64;64m");
    *len = 17;
}

static void draw_wave(Renderer *r, size_t x_start, size_t region_w, Out *o) {
    if (r->wave_cap == 0 || r->rows < 3 || region_w == 0)
        return;
    size_t ncol = region_w < 4096 ? region_w : 4096;
    if (ncol == 0)
        return;
    {
        long long *ny = realloc(r->sc_yrow, ncol * sizeof(long long));
        long long *nl = realloc(r->sc_lo, ncol * sizeof(long long));
        long long *nh = realloc(r->sc_hi, ncol * sizeof(long long));
        long long *nl2 = realloc(r->sc_lo2, ncol * sizeof(long long));
        long long *nh2 = realloc(r->sc_hi2, ncol * sizeof(long long));
        if (!ny || !nl || !nh || !nl2 || !nh2) {
            free(ny);
            free(nl);
            free(nh);
            free(nl2);
            free(nh2);
            return;
        }
        r->sc_yrow = ny;
        r->sc_lo = nl;
        r->sc_hi = nh;
        r->sc_lo2 = nl2;
        r->sc_hi2 = nh2;
    }
    size_t rate = r->wave_rate ? r->wave_rate : 48000;
    size_t snap = rate * 5 / 1000;
    if (snap < 64)
        snap = 64;
    if (snap > 4096)
        snap = 4096;
    size_t avail = r->wave_filled < r->wave_cap ? r->wave_filled : r->wave_cap;
    if (avail < 16)
        return;
    size_t look = snap < avail ? snap : avail;
    if (look < 2)
        return;
    size_t base = (r->wave_pos + r->wave_cap - look) % r->wave_cap;
    size_t cap = r->wave_cap;
    int stereo = r->stereo_in;
    double center = (double)(r->rows - 1) * 0.5;
    double height = (double)r->rows - 6.0;
    if (height < 4.0)
        height = 4.0;
    height /= 3.0;
    double denom = (double)(look - 1);
    double xdenom = (double)(ncol > 1 ? ncol - 1 : 1);
    for (size_t c = 0; c < ncol; c++) {
        r->sc_lo[c] = (long long)9223372036854775807LL;
        r->sc_hi[c] = (long long)-9223372036854775807LL - 1;
        r->sc_lo2[c] = (long long)9223372036854775807LL;
        r->sc_hi2[c] = (long long)-9223372036854775807LL - 1;
    }
    double peak = 0.0;
    for (size_t i = 0; i < look; i++) {
        double a = r->osc_l[(base + i) % cap];
        double b = stereo ? r->osc_r[(base + i) % cap] : a;
        double pa = a < 0 ? -a : a;
        double pb = b < 0 ? -b : b;
        if (pa > peak)
            peak = pa;
        if (pb > peak)
            peak = pb;
    }
    int clipped = peak > 0.99;
    int have_prev = 0;
    double prev_xf = 0;
    long long prev_yl = 0, prev_yr = 0;
    for (size_t i = 0; i < look; i++) {
        double xf = (double)i * xdenom / denom;
        double a = r->osc_l[(base + i) % cap];
        double b = stereo ? r->osc_r[(base + i) % cap] : a;
        if (a < -1.0)
            a = -1.0;
        if (a > 1.0)
            a = 1.0;
        if (b < -1.0)
            b = -1.0;
        if (b > 1.0)
            b = 1.0;
        long long yl = (long long)(center - a * height + 0.5);
        long long yr = (long long)(center - b * height + 0.5);
        size_t yidx = (size_t)(xf + 0.5) % ncol;
        r->sc_yrow[yidx] = yl;
        if (have_prev) {
            long long x0 = rllround(prev_xf);
            long long x1 = rllround(xf);
            long long xa = x0 < x1 ? x0 : x1;
            long long xb = x0 < x1 ? x1 : x0;
            for (long long cx = xa; cx <= xb; cx++) {
                if (cx < 0 || cx >= (long long)ncol)
                    continue;
                size_t cc = (size_t)cx;
                double t;
                long long d = x1 - x0;
                if (d < 0)
                    d = -d;
                if (d < 1)
                    t = 0.0;
                else
                    t = (double)(cx - x0) / (double)(x1 - x0);
                long long yyl = rllround((double)prev_yl + (double)(yl - prev_yl) * t);
                long long yyr = rllround((double)prev_yr + (double)(yr - prev_yr) * t);
                if (yyl < r->sc_lo[cc])
                    r->sc_lo[cc] = yyl;
                if (yyl > r->sc_hi[cc])
                    r->sc_hi[cc] = yyl;
                if (yyr < r->sc_lo2[cc])
                    r->sc_lo2[cc] = yyr;
                if (yyr > r->sc_hi2[cc])
                    r->sc_hi2[cc] = yyr;
            }
        } else {
            long long cc = rllround(xf);
            if (cc < 0)
                cc = 0;
            if (cc >= (long long)ncol)
                cc = (long long)ncol - 1;
            r->sc_lo[cc] = yl;
            r->sc_hi[cc] = yl;
            r->sc_lo2[cc] = yr;
            r->sc_hi2[cc] = yr;
        }
        have_prev = 1;
        prev_xf = xf;
        prev_yl = yl;
        prev_yr = yr;
    }
    for (size_t c = 0; c < ncol; c++) {
        if (r->sc_lo[c] == (long long)9223372036854775807LL) {
            r->sc_lo[c] = 0;
            r->sc_hi[c] = -1;
        }
        if (r->sc_lo2[c] == (long long)9223372036854775807LL) {
            r->sc_lo2[c] = 0;
            r->sc_hi2[c] = -1;
        }
    }
    uint8_t *clip_esc = NULL;
    size_t clip_len = 0;
    if (clipped)
        wave_clip_esc_buf(r, &clip_esc, &clip_len);
    ColorState st = {0};
    size_t cy0 = r->rows;
    size_t cy1 = 0;
    for (size_t c = 0; c < ncol; c++) {
        if (r->sc_hi[c] >= 0 && r->sc_lo[c] <= r->sc_hi[c]) {
            size_t l = (size_t)r->sc_lo[c];
            size_t h = (size_t)r->sc_hi[c];
            if (l < cy0)
                cy0 = l;
            if (h > cy1)
                cy1 = h;
        }
        if (r->sc_hi2[c] >= 0 && r->sc_lo2[c] <= r->sc_hi2[c]) {
            size_t l = (size_t)r->sc_lo2[c];
            size_t h = (size_t)r->sc_hi2[c];
            if (l < cy0)
                cy0 = l;
            if (h > cy1)
                cy1 = h;
        }
    }
    size_t uy0 = cy0 < r->db_y0 ? cy0 : r->db_y0;
    size_t uy1 = cy1 > r->db_y1 ? cy1 : r->db_y1;
    if (uy1 >= uy0) {
        size_t cols = r->cols;
        for (size_t y = uy0; y <= uy1; y++) {
            for (size_t c = 0; c < ncol; c++) {
                long long yy = (long long)y;
                if ((yy >= r->sc_lo[c] && yy <= r->sc_hi[c]) ||
                    (yy >= r->sc_lo2[c] && yy <= r->sc_hi2[c]))
                    r->rowbuf[c] = 8;
                else
                    r->rowbuf[c] = 0;
            }
            const uint8_t *re;
            size_t rel;
            if (clip_esc) {
                re = clip_esc;
                rel = clip_len;
            } else {
                re = r->row_col[y];
                rel = r->row_col_len[y];
            }
            emit_row_raw(r->prev, cols, re, rel, r, y, x_start, ncol, r->rowbuf, &st, o);
        }
    }
    r->db_y0 = cy0;
    r->db_y1 = cy1;
    free(st.col);
    free(clip_esc);
}

static void set_beam(Renderer *r, long long x, long long y) {
    if (x >= 0 && y >= 0 && (size_t)x < r->cols && (size_t)y < r->rows)
        r->osc_glow[(size_t)y * r->cols + (size_t)x] = 255;
}

static void beam_line(Renderer *r, long long x0, long long y0, long long x1, long long y1) {
    long long dx = x1 > x0 ? x1 - x0 : x0 - x1;
    long long dy = y1 > y0 ? y1 - y0 : y0 - y1;
    long long sx = x0 < x1 ? 1 : -1;
    long long sy = y0 < y1 ? 1 : -1;
    long long err = dx - dy;
    for (;;) {
        set_beam(r, x0, y0);
        if (x0 == x1 && y0 == y1)
            break;
        long long e2 = 2 * err;
        if (e2 > -dy) {
            err -= dy;
            x0 += sx;
        }
        if (e2 < dx) {
            err += dx;
            y0 += sy;
        }
    }
}

static void draw_oscilloscope(Renderer *r, size_t x_start, size_t region_w, Out *o) {
    if (r->osc_cap == 0 || r->rows < 3 || region_w < 4)
        return;
    size_t rows = r->rows;
    size_t cols = r->cols;
    memset(r->osc_glow, 0, rows * cols);
    size_t cx0 = cols;
    size_t cy0 = rows;
    size_t cx1 = 0;
    size_t cy1 = 0;
    size_t n = r->osc_filled;
    if (n > r->osc_win)
        n = r->osc_win;
    if (n > 1) {
        size_t delay = r->osc_spc ? r->osc_spc : 1;
        double cx = (double)x_start + (double)(region_w - 1) * 0.5;
        double cy = (double)(rows - 1) * 0.5;
        double sxc = (double)(region_w - 1) * 0.5;
        double syc = (double)(rows - 1) * 0.5;
        long long px = -1;
        long long py = -1;
        for (size_t i = 0; i < n; i++) {
            size_t idx = (r->osc_pos + r->osc_cap - n + i) % r->osc_cap;
            double l = r->osc_l[idx];
            double rr = r->osc_r[idx];
            if (!r->stereo_in) {
                size_t idx2 = (idx + r->osc_cap - delay) % r->osc_cap;
                rr = r->osc_l[idx2];
            }
            if (l < -1.0)
                l = -1.0;
            else if (l > 1.0)
                l = 1.0;
            if (rr < -1.0)
                rr = -1.0;
            else if (rr > 1.0)
                rr = 1.0;
            long long xx = (long long)(cx + l * sxc + 0.5);
            long long yy = (long long)(cy - rr * syc + 0.5);
            if (xx < (long long)x_start || xx >= (long long)(x_start + region_w) ||
                yy < 0 || yy >= (long long)rows) {
                px = -1;
                py = -1;
                continue;
            }
            if (px >= 0 && py >= 0)
                beam_line(r, px, py, xx, yy);
            else
                set_beam(r, xx, yy);
            size_t uxx = (size_t)xx;
            size_t uyy = (size_t)yy;
            if (uxx < cx0)
                cx0 = uxx;
            if (uxx > cx1)
                cx1 = uxx;
            if (uyy < cy0)
                cy0 = uyy;
            if (uyy > cy1)
                cy1 = uyy;
            px = xx;
            py = yy;
        }
    }
    ColorState st = {0};
    size_t ux0 = cx0 < r->db_x0 ? cx0 : r->db_x0;
    size_t ux1 = cx1 > r->db_x1 ? cx1 : r->db_x1;
    size_t uy0 = cy0 < r->db_y0 ? cy0 : r->db_y0;
    size_t uy1 = cy1 > r->db_y1 ? cy1 : r->db_y1;
    if (ux1 >= ux0 && uy1 >= uy0) {
        size_t w = ux1 - ux0 + 1;
        for (size_t y = uy0; y <= uy1; y++) {
            for (size_t x = ux0; x <= ux1; x++)
                r->rowbuf[x - ux0] = r->osc_glow[y * cols + x] ? 8 : 0;
            emit_row_raw(r->prev, cols, r->row_col[y], r->row_col_len[y], r, y, ux0, w,
                         r->rowbuf, &st, o);
        }
    }
    r->db_x0 = cx0;
    r->db_x1 = cx1;
    r->db_y0 = cy0;
    r->db_y1 = cy1;
    free(st.col);
}

void renderer_draw(Renderer *r, const double *values, SvBuf *out, size_t cap) {
    if (r->cols <= r->x_off)
        return;
    size_t region = r->cols - r->x_off;
    if (region == 0)
        return;
    Out o = {out, cap};
    row_colors(r);
    if (r->mode == RM_WAVE)
        draw_wave(r, r->x_off, region, &o);
    else if (r->mode == RM_SCOPE)
        draw_oscilloscope(r, r->x_off, region, &o);
    else if (r->mode == RM_LYRICS)
        draw_lyrics_mode(r, r->x_off, region, &o);
    else
        draw_bars(r, values, NULL, r->num_bars, r->num_bars, r->x_off, region, &o);
}

void renderer_draw_stereo(Renderer *r, const double *left, const double *right,
                          size_t per_ch_l, size_t per_ch_r, SvBuf *out, size_t cap) {
    (void)per_ch_r;
    if (r->cols <= r->x_off)
        return;
    size_t region = r->cols - r->x_off;
    if (region == 0)
        return;
    Out o = {out, cap};
    row_colors(r);
    if (r->mode == RM_WAVE)
        draw_wave(r, r->x_off, region, &o);
    else if (r->mode == RM_SCOPE)
        draw_oscilloscope(r, r->x_off, region, &o);
    else if (r->mode == RM_LYRICS)
        draw_lyrics_mode(r, r->x_off, region, &o);
    else
        draw_bars(r, left, right, r->num_bars, per_ch_l, r->x_off, region, &o);
}
