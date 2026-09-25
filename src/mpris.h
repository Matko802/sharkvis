#ifndef SHARKVIS_MPRIS_H
#define SHARKVIS_MPRIS_H

#include <stddef.h>

typedef struct Track {
    int present;
    char player[256];
    char artist[512];
    char title[512];
    double position;
    double duration;
    char url[1024];
} Track;

void track_key(const Track *t, char *out, size_t outn);

char *cmd_out(const char *cmd, const char *const *args, unsigned timeout_ms);

char **player_list(size_t *n);
void player_list_free(char **p, size_t n);
int any_active_player(void);
Track poll_track(const char *const *allow, size_t nallow);
Track poll_named(const char *player);
int poll_position(const char *player, double *pos);

#endif
