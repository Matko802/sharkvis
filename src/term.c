#include <poll.h>
#include <string.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

#include "term.h"

static struct termios saved_term;
static int have_saved = 0;

int term_winsize(int fd, unsigned *rows, unsigned *cols) {
    struct winsize ws;
    memset(&ws, 0, sizeof ws);
    if (ioctl(fd, TIOCGWINSZ, &ws) != 0)
        return 0;
    if (ws.ws_row == 0 || ws.ws_col == 0)
        return 0;
    *rows = ws.ws_row;
    *cols = ws.ws_col;
    return 1;
}

double term_cell_aspect(int fd) {
    struct winsize ws;
    memset(&ws, 0, sizeof ws);
    if (ioctl(fd, TIOCGWINSZ, &ws) != 0)
        return 2.0;
    if (ws.ws_row == 0 || ws.ws_col == 0 || ws.ws_xpixel == 0 || ws.ws_ypixel == 0)
        return 2.0;
    double cw = (double)ws.ws_xpixel / (double)ws.ws_col;
    double ch = (double)ws.ws_ypixel / (double)ws.ws_row;
    if (!(cw > 0.0) || !(ch > 0.0))
        return 2.0;
    double r = ch / cw;
    if (r < 0.5)
        r = 0.5;
    if (r > 3.0)
        r = 3.0;
    return r;
}

int term_raw_enter(int fd) {
    struct termios t;
    if (tcgetattr(fd, &t) != 0)
        return 0;
    saved_term = t;
    have_saved = 1;
    t.c_lflag &= (unsigned)(~(ICANON | ECHO | ISIG));
    t.c_iflag &= (unsigned)(~(IXON | ICRNL));
    t.c_cc[VMIN] = 0;
    t.c_cc[VTIME] = 0;
    return tcsetattr(fd, TCSANOW, &t) == 0;
}

void term_raw_restore(int fd) {
    if (have_saved)
        tcsetattr(fd, TCSANOW, &saved_term);
}

static int poll_readable(int fd, int timeout_ms) {
    struct pollfd pfd;
    pfd.fd = fd;
    pfd.events = POLLIN;
    pfd.revents = 0;
    return poll(&pfd, 1, timeout_ms) > 0;
}

static int read_byte(int fd, uint8_t *out) {
    uint8_t c;
    if (read(fd, &c, 1) == 1) {
        *out = c;
        return 1;
    }
    return 0;
}

int term_read_key(int fd) {
    uint8_t c;
    if (!read_byte(fd, &c))
        return KEY_NONE;
    if (c != 0x1b)
        return (int)c;
    if (poll_readable(fd, 30)) {
        uint8_t c2;
        if (read_byte(fd, &c2)) {
            if (c2 == '[' && poll_readable(fd, 30)) {
                uint8_t c3;
                if (read_byte(fd, &c3)) {
                    switch (c3) {
                    case 'A':
                        return KEY_UP;
                    case 'B':
                        return KEY_DOWN;
                    case 'C':
                        return KEY_RIGHT;
                    case 'D':
                        return KEY_LEFT;
                    default:
                        return (int)c3;
                    }
                }
            }
        }
    }
    return KEY_ESC;
}

int term_read_codepoint(int fd, uint8_t out8[8], size_t *n) {
    uint8_t c;
    if (!read_byte(fd, &c)) {
        *n = 0;
        return KEY_NONE;
    }
    if (c == 0x1b) {
        if (poll_readable(fd, 30)) {
            uint8_t c2;
            if (read_byte(fd, &c2)) {
                if (c2 == '[' && poll_readable(fd, 30)) {
                    uint8_t c3;
                    if (read_byte(fd, &c3)) {
                        *n = 0;
                        switch (c3) {
                        case 'A':
                            return KEY_UP;
                        case 'B':
                            return KEY_DOWN;
                        case 'C':
                            return KEY_RIGHT;
                        case 'D':
                            return KEY_LEFT;
                        default:
                            return KEY_ESC;
                        }
                    }
                }
            }
        }
        *n = 0;
        return KEY_ESC;
    }
    if (c == 0x0d || c == 0x0a) {
        *n = 0;
        return KEY_ENTER;
    }
    if (c == 0x7f || c == 0x08) {
        *n = 0;
        return KEY_BACKSPACE;
    }
    if (c < 0x20) {
        *n = 0;
        return (int)c;
    }
    size_t need;
    if (c < 0x80)
        need = 1;
    else if ((c & 0xE0) == 0xC0)
        need = 2;
    else if ((c & 0xF0) == 0xE0)
        need = 3;
    else if ((c & 0xF8) == 0xF0)
        need = 4;
    else {
        *n = 0;
        return KEY_NONE;
    }
    out8[0] = c;
    size_t got = 1;
    while (got < need) {
        if (!poll_readable(fd, 50))
            break;
        uint8_t cc;
        if (!read_byte(fd, &cc))
            break;
        out8[got++] = cc;
    }
    *n = got;
    return KEY_CHAR;
}
