#ifndef SHARKVIS_PULSE_H
#define SHARKVIS_PULSE_H

#include <stddef.h>
#include <stdint.h>

typedef struct Pulse Pulse;
typedef struct Record Record;

Pulse *pulse_connect(char *err, size_t errn);
int pulse_default_monitor(Pulse *p, char *out, size_t outn, char *err, size_t errn);
Record *pulse_record(Pulse *p, const char *device, unsigned rate,
                     unsigned channels, unsigned fragsize, char *err, size_t errn);
void pulse_free(Pulse *p);

long record_read(Record *r, uint8_t *out, size_t cap,
                 const volatile int *stop, char *err, size_t errn);
void record_free(Record *r);

#endif
