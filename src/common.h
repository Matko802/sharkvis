#ifndef SHARKVIS_COMMON_H
#define SHARKVIS_COMMON_H

#include <stddef.h>
#include <stdint.h>

typedef struct {
    uint8_t *data;
    size_t len;
    size_t cap;
} SvBuf;

void sv_buf_reserve(SvBuf *b, size_t extra);
void sv_buf_put(SvBuf *b, const void *p, size_t n);
void sv_buf_put_u32(SvBuf *b, unsigned v);
void sv_buf_free(SvBuf *b);

uint64_t sv_now_ms(void);
uint64_t sv_now_ns(void);

#endif
