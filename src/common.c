#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "common.h"

void sv_buf_reserve(SvBuf *b, size_t extra) {
    size_t need = b->len + extra;
    if (need <= b->cap)
        return;
    size_t cap = b->cap ? b->cap : 256;
    while (cap < need)
        cap *= 2;
    uint8_t *p = realloc(b->data, cap);
    if (!p)
        abort();
    b->data = p;
    b->cap = cap;
}

void sv_buf_put(SvBuf *b, const void *p, size_t n) {
    sv_buf_reserve(b, n);
    memcpy(b->data + b->len, p, n);
    b->len += n;
}

void sv_buf_put_u32(SvBuf *b, unsigned v) {
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
    sv_buf_put(b, d, (size_t)n);
}

void sv_buf_free(SvBuf *b) {
    free(b->data);
    b->data = NULL;
    b->len = 0;
    b->cap = 0;
}

uint64_t sv_now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000 + (uint64_t)ts.tv_nsec / 1000000;
}

uint64_t sv_now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000 + (uint64_t)ts.tv_nsec;
}
