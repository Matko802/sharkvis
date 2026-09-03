use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};

pub const KEY_NONE: i32 = -1;
pub const KEY_ESC: i32 = 0x1b;
pub const KEY_UP: i32 = 0x1001;
pub const KEY_DOWN: i32 = 0x1002;
pub const KEY_LEFT: i32 = 0x1003;
pub const KEY_RIGHT: i32 = 0x1004;
pub const KEY_ENTER: i32 = 0x1005;
pub const KEY_BACKSPACE: i32 = 0x1006;
pub const KEY_CHAR: i32 = 0x1007;

static mut SAVED: std::mem::MaybeUninit<libc::termios> = std::mem::MaybeUninit::uninit();
static HAVE_SAVED: AtomicBool = AtomicBool::new(false);

pub fn term_winsize(fd: RawFd, rows: &mut u32, cols: &mut u32) -> bool {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } != 0 || ws.ws_row == 0 || ws.ws_col == 0 {
        return false;
    }
    *rows = ws.ws_row as u32;
    *cols = ws.ws_col as u32;
    true
}

/// Terminal pixel dimensions (total) from TIOCGWINSZ, if the terminal
/// reports them. Returns (0, 0) when unavailable.
pub fn term_winsize_px(fd: RawFd) -> (u32, u32) {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } != 0 {
        return (0, 0);
    }
    (ws.ws_xpixel as u32, ws.ws_ypixel as u32)
}

pub fn term_raw_enter(fd: RawFd) -> bool {
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut t) } != 0 {
        return false;
    }
    unsafe {
        std::ptr::addr_of_mut!(SAVED).cast::<libc::termios>().write(t);
    }
    HAVE_SAVED.store(true, Ordering::Release);
    t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
    t.c_iflag &= !(libc::IXON | libc::ICRNL);
    t.c_cc[libc::VMIN] = 0;
    t.c_cc[libc::VTIME] = 0;
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &t) == 0 }
}

pub fn term_raw_restore(fd: RawFd) {
    if HAVE_SAVED.load(Ordering::Acquire) {
        unsafe {
            let p = std::ptr::addr_of!(SAVED).cast::<libc::termios>();
            libc::tcsetattr(fd, libc::TCSANOW, p);
        }
    }
}

fn poll_readable(fd: RawFd, timeout_ms: i32) -> bool {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut pfd, 1, timeout_ms) > 0 }
}

fn read_byte(fd: RawFd) -> Option<u8> {
    let mut c: u8 = 0;
    if unsafe { libc::read(fd, &mut c as *mut u8 as *mut libc::c_void, 1) } == 1 {
        Some(c)
    } else {
        None
    }
}

#[allow(dead_code)]
pub fn term_read_key(fd: RawFd) -> i32 {
    let c = match read_byte(fd) {
        Some(c) => c,
        None => return KEY_NONE,
    };
    if c != 0x1b {
        return c as i32;
    }
    if poll_readable(fd, 30) {
        if let Some(c2) = read_byte(fd) {
            if c2 == b'[' && poll_readable(fd, 30) {
                if let Some(c3) = read_byte(fd) {
                    return match c3 {
                        b'A' => KEY_UP,
                        b'B' => KEY_DOWN,
                        b'C' => KEY_RIGHT,
                        b'D' => KEY_LEFT,
                        _ => c3 as i32,
                    };
                }
            }
        }
    }
    KEY_ESC
}

/// Reads a key press. Returns `(key, codepoint bytes, length of codepoint)`.
/// When `key == KEY_CHAR`, `cp[..len]` holds the UTF-8 sequence.
pub fn term_read_codepoint(fd: RawFd, out: &mut [u8; 8]) -> (i32, usize) {
    let c = match read_byte(fd) {
        Some(c) => c,
        None => return (KEY_NONE, 0),
    };
    if c == 0x1b {
        if poll_readable(fd, 30) {
            if let Some(c2) = read_byte(fd) {
                if c2 == b'[' && poll_readable(fd, 30) {
                    if let Some(c3) = read_byte(fd) {
                        return match c3 {
                            b'A' => (KEY_UP, 0),
                            b'B' => (KEY_DOWN, 0),
                            b'C' => (KEY_RIGHT, 0),
                            b'D' => (KEY_LEFT, 0),
                            _ => (KEY_ESC, 0),
                        };
                    }
                }
            }
        }
        return (KEY_ESC, 0);
    }
    if c == 0x0d || c == 0x0a {
        return (KEY_ENTER, 0);
    }
    if c == 0x7f || c == 0x08 {
        return (KEY_BACKSPACE, 0);
    }
    if c < 0x20 {
        return (c as i32, 0);
    }

    let need = if c < 0x80 {
        1
    } else if (c & 0xE0) == 0xC0 {
        2
    } else if (c & 0xF0) == 0xE0 {
        3
    } else if (c & 0xF8) == 0xF0 {
        4
    } else {
        return (KEY_NONE, 0);
    };

    let mut got = 1usize;
    out[0] = c;
    while got < need {
        if !poll_readable(fd, 50) {
            break;
        }
        match read_byte(fd) {
            Some(cc) => {
                out[got] = cc;
                got += 1;
            }
            None => break,
        }
    }
    (KEY_CHAR, got)
}