#ifndef SHARKVIS_STATE_H
#define SHARKVIS_STATE_H

#include <stddef.h>
#include <stdint.h>

typedef struct StateWriter StateWriter;

StateWriter *state_writer_new(void);
void state_writer_free(StateWriter *w);
void state_writer_update(StateWriter *w, double energy, double bass,
                         double left, double right,
                         uint8_t lr, uint8_t lg, uint8_t lb,
                         uint8_t hr, uint8_t hg, uint8_t hb);

void beat_step(double energy, double *avg, double *peak, double *prev,
               double *beat, double dt);
void lerp_rgb(uint8_t lr, uint8_t lg, uint8_t lb, uint8_t hr, uint8_t hg,
              uint8_t hb, float t, uint8_t *r, uint8_t *g, uint8_t *b);

void state_path(char *buf, size_t n);
uint64_t session_started_ms(void);
void clear_state_file(void);
void remove_stale_state(void);

#endif
