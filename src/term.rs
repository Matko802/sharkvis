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
pub const KEY_MOUSE: i32 = 0x1008;

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

pub fn term_cell_aspect(fd: RawFd) -> f64 {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } != 0
        || ws.ws_row == 0
        || ws.ws_col == 0
        || ws.ws_xpixel == 0
        || ws.ws_ypixel == 0
    {
        return 2.0;
    }
    let cw = ws.ws_xpixel as f64 / ws.ws_col as f64;
    let ch = ws.ws_ypixel as f64 / ws.ws_row as f64;
    if !(cw > 0.0) || !(ch > 0.0) {
        return 2.0;
    }
    (ch / cw).clamp(0.5, 3.0)
}

pub fn term_raw_enter(fd: RawFd) -> bool {
    if HAVE_SAVED.load(Ordering::Acquire) {
        return true;
    }
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
    if HAVE_SAVED.swap(false, Ordering::SeqCst) {
        unsafe {
            let p = std::ptr::addr_of!(SAVED).cast::<libc::termios>();
            libc::tcsetattr(fd, libc::TCSANOW, p);
        }
    }
}

pub fn term_mouse_enter() {
    unsafe {
        libc::write(1, b"\x1b[?1000h\x1b[?1006h".as_ptr() as *const libc::c_void, 16);
    }
}

pub fn term_mouse_leave() {
    unsafe {
        libc::write(1, b"\x1b[?1000l\x1b[?1006l".as_ptr() as *const libc::c_void, 16);
    }
}

pub fn mouse_decode(cp: &[u8]) -> Option<(u8, u16, u16)> {
    if cp.len() < 5 {
        return None;
    }
    let x = u16::from_le_bytes([cp[1], cp[2]]);
    let y = u16::from_le_bytes([cp[3], cp[4]]);
    if x == 0 || y == 0 {
        return None;
    }
    Some((cp[0], x, y))
}

fn read_sgr_mouse(fd: RawFd, out: &mut [u8; 8]) -> (i32, usize) {
    let mut body = [0u8; 16];
    let mut n = 0usize;
    loop {
        if n >= body.len() {
            break;
        }
        if !poll_readable(fd, 30) {
            break;
        }
        match read_byte(fd) {
            Some(b) => {
                body[n] = b;
                n += 1;
                if b == b'M' || b == b'm' {
                    break;
                }
            }
            None => break,
        }
    }
    if n == 0 || (body[n - 1] != b'M' && body[n - 1] != b'm') {
        return (KEY_ESC, 0);
    }
    if body[n - 1] == b'm' {
        return (KEY_NONE, 0);
    }
    let mut nums = [0u32; 3];
    let mut ni = 0usize;
    let mut cur = 0u32;
    let mut digits = 0u32;
    let mut i = 0usize;
    while i + 1 < n {
        let b = body[i];
        if b.is_ascii_digit() {
            cur = cur.saturating_mul(10).saturating_add((b - b'0') as u32);
            digits += 1;
        } else if b == b';' {
            if ni < 3 {
                nums[ni] = cur;
                ni += 1;
            }
            cur = 0;
            digits = 0;
        } else {
            return (KEY_ESC, 0);
        }
        i += 1;
    }
    if digits == 0 || ni != 2 {
        return (KEY_ESC, 0);
    }
    nums[ni] = cur;
    let btn = nums[0];
    let x = nums[1].clamp(1, 65535);
    let y = nums[2].clamp(1, 65535);
    if btn & 64 != 0 {
        if btn & 1 != 0 {
            return (KEY_DOWN, 0);
        }
        return (KEY_UP, 0);
    }
    out[0] = (btn & 0xff) as u8;
    out[1] = (x & 0xff) as u8;
    out[2] = ((x >> 8) & 0xff) as u8;
    out[3] = (y & 0xff) as u8;
    out[4] = ((y >> 8) & 0xff) as u8;
    (KEY_MOUSE, 5)
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
                        if c3 == b'<' {
                            return read_sgr_mouse(fd, out);
                        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_aspect_defaults_without_pixels() {
        assert_eq!(term_cell_aspect(-1), 2.0);
    }
}