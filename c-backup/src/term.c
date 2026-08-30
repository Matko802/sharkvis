#include "term.h"

#include <poll.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

static struct termios saved;
static bool have_saved = false;

bool term_winsize(int fd, unsigned *rows, unsigned *cols) {
    struct winsize ws;
    if (ioctl(fd, TIOCGWINSZ, &ws) != 0 || ws.ws_row == 0 || ws.ws_col == 0)
        return false;
    *rows = ws.ws_row;
    *cols = ws.ws_col;
    return true;
}

bool term_raw_enter(int fd) {
    struct termios t;
    if (tcgetattr(fd, &t) != 0)
        return false;
    saved = t;
    have_saved = true;
    t.c_lflag &= ~(ICANON | ECHO | ISIG);
    t.c_iflag &= ~(IXON | ICRNL);
    t.c_cc[VMIN] = 0;
    t.c_cc[VTIME] = 0;
    return tcsetattr(fd, TCSANOW, &t) == 0;
}

void term_raw_restore(int fd) {
    if (have_saved)
        tcsetattr(fd, TCSANOW, &saved);
}

int term_read_key(int fd) {
    unsigned char c;
    if (read(fd, &c, 1) != 1)
        return KEY_NONE;
    if (c != 0x1b)
        return c;
    struct pollfd pfd = { .fd = fd, .events = POLLIN };
    if (poll(&pfd, 1, 30) > 0) {
        unsigned char c2;
        if (read(fd, &c2, 1) == 1 && c2 == '[') {
            if (poll(&pfd, 1, 30) > 0) {
                unsigned char c3;
                if (read(fd, &c3, 1) == 1) {
                    switch (c3) {
                    case 'A': return KEY_UP;
                    case 'B': return KEY_DOWN;
                    case 'C': return KEY_RIGHT;
                    case 'D': return KEY_LEFT;
                    default: return c3;
                    }
                }
            }
        }
    }
    return KEY_ESC;
}

int term_read_codepoint(int fd, char *out, size_t cap) {
    if (cap == 0)
        return KEY_NONE;
    unsigned char c;
    if (read(fd, &c, 1) != 1)
        return KEY_NONE;
    if (c == 0x1b) {
        struct pollfd pfd = { .fd = fd, .events = POLLIN };
        if (poll(&pfd, 1, 30) > 0) {
            unsigned char c2;
            if (read(fd, &c2, 1) == 1 && c2 == '[') {
                if (poll(&pfd, 1, 30) > 0) {
                    unsigned char c3;
                    if (read(fd, &c3, 1) == 1) {
                        switch (c3) {
                        case 'A': return KEY_UP;
                        case 'B': return KEY_DOWN;
                        case 'C': return KEY_RIGHT;
                        case 'D': return KEY_LEFT;
                        default:  return KEY_ESC;
                        }
                    }
                }
            }
        }
        return KEY_ESC;
    }
    if (c == 0x0d || c == 0x0a)
        return KEY_ENTER;
    if (c == 0x7f || c == 0x08)
        return KEY_BACKSPACE;
    if (c < 0x20)
        return c; /* raw control code, e.g. 0x03 = Ctrl+C */

    size_t need;
    if (c < 0x80)
        need = 1;
    else if ((c & 0xE0) == 0xC0)
        need = 2;
    else if ((c & 0xF0) == 0xE0)
        need = 3;
    else if ((c & 0xF8) == 0xF0)
        need = 4;
    else
        return KEY_NONE;

    if (need > cap)
        need = cap;
    size_t got = 1;
    out[0] = (char)c;
    while (got < need) {
        struct pollfd pfd = { .fd = fd, .events = POLLIN };
        if (poll(&pfd, 1, 50) <= 0)
            break;
        unsigned char cc;
        if (read(fd, &cc, 1) != 1)
            break;
        out[got++] = (char)cc;
    }
    out[got] = '\0';
    return KEY_CHAR;
}
