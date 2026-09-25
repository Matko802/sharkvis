#ifndef SHARKVIS_TERM_H
#define SHARKVIS_TERM_H

#include <stddef.h>
#include <stdint.h>

#define KEY_NONE -1
#define KEY_ESC 0x1b
#define KEY_UP 0x1001
#define KEY_DOWN 0x1002
#define KEY_LEFT 0x1003
#define KEY_RIGHT 0x1004
#define KEY_ENTER 0x1005
#define KEY_BACKSPACE 0x1006
#define KEY_CHAR 0x1007

int term_winsize(int fd, unsigned *rows, unsigned *cols);
double term_cell_aspect(int fd);
int term_raw_enter(int fd);
void term_raw_restore(int fd);
int term_read_key(int fd);
int term_read_codepoint(int fd, uint8_t out8[8], size_t *n);

#endif
