use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};

const PA_PROTOCOL_VERSION: u32 = 35;
const PA_PROTOCOL_VERSION_MASK: u32 = 0x0000FFFF;
const PA_INVALID_INDEX: u32 = 0xFFFF_FFFF;

const TAG_STRING: u8 = b't';
const TAG_STRING_NULL: u8 = b'N';
const TAG_U32: u8 = b'L';
const TAG_U8: u8 = b'B';
const TAG_SAMPLE_SPEC: u8 = b'a';
const TAG_ARBITRARY: u8 = b'x';
const TAG_BOOLEAN_TRUE: u8 = b'1';
const TAG_BOOLEAN_FALSE: u8 = b'0';
const TAG_CHANNEL_MAP: u8 = b'm';
const TAG_CVOLUME: u8 = b'v';
const TAG_PROPLIST: u8 = b'P';

const CMD_ERROR: u32 = 0;
const CMD_REPLY: u32 = 2;
const CMD_CREATE_RECORD_STREAM: u32 = 5;
const CMD_AUTH: u32 = 8;
const CMD_SET_CLIENT_NAME: u32 = 9;
const CMD_GET_SINK_INFO: u32 = 21;
const CMD_RECORD_STREAM_KILLED: u32 = 65;

const COOKIE_LEN: usize = 256;
const PA_SAMPLE_S16NE: u8 = 3;
const PA_VOLUME_NORM: u32 = 0x10000;

struct Writer {
    raw: Vec<u8>,
}

impl Writer {
    fn new() -> Writer {
        Writer { raw: Vec::new() }
    }
    fn tag(&mut self, t: u8) {
        self.raw.push(t);
    }
    fn u32(&mut self, v: u32) {
        self.tag(TAG_U32);
        self.raw.extend_from_slice(&v.to_be_bytes());
    }
    fn u8(&mut self, v: u8) {
        self.tag(TAG_U8);
        self.raw.push(v);
    }
    fn string(&mut self, s: &str) {
        self.tag(TAG_STRING);
        self.raw.extend_from_slice(s.as_bytes());
        self.raw.push(0);
    }
    fn string_null(&mut self) {
        self.tag(TAG_STRING_NULL);
    }
    fn arbitrary(&mut self, data: &[u8]) {
        self.tag(TAG_ARBITRARY);
        self.raw.extend_from_slice(&(data.len() as u32).to_be_bytes());
        self.raw.extend_from_slice(data);
    }
    fn sample_spec(&mut self, format: u8, channels: u8, rate: u32) {
        self.tag(TAG_SAMPLE_SPEC);
        self.raw.push(format);
        self.raw.push(channels);
        self.raw.extend_from_slice(&rate.to_be_bytes());
    }
    fn channel_map(&mut self, map: &[u8]) {
        self.tag(TAG_CHANNEL_MAP);
        self.raw.push(map.len() as u8);
        self.raw.extend_from_slice(map);
    }
    fn cvolume(&mut self, channels: u8, val: u32) {
        self.tag(TAG_CVOLUME);
        self.raw.push(channels);
        for _ in 0..channels {
            self.raw.extend_from_slice(&val.to_be_bytes());
        }
    }
    fn proplist(&mut self, props: &[(&str, &[u8])]) {
        self.tag(TAG_PROPLIST);
        for (k, v) in props {
            self.string(k);
            self.u32(v.len() as u32);
            self.arbitrary(v);
        }
        self.string_null();
    }
    fn boolean(&mut self, b: bool) {
        self.tag(if b { TAG_BOOLEAN_TRUE } else { TAG_BOOLEAN_FALSE });
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }
    fn byte(&mut self) -> Result<u8, String> {
        let b = self.data.get(self.pos).copied().ok_or("pulse: truncated message")?;
        self.pos += 1;
        Ok(b)
    }
    fn tag(&mut self, t: u8) -> Result<(), String> {
        let got = self.byte()?;
        if got != t {
            return Err(format!("pulse: expected tag {} but got {}", t as char, got as char));
        }
        Ok(())
    }
    fn boolean(&mut self) -> Result<bool, String> {
        match self.byte()? {
            TAG_BOOLEAN_TRUE => Ok(true),
            TAG_BOOLEAN_FALSE => Ok(false),
            b => Err(format!("pulse: expected boolean tag, got {}", b as char)),
        }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.data.len() {
            return Err("pulse: truncated message".into());
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        self.tag(TAG_U32)?;
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn string(&mut self) -> Result<String, String> {
        self.tag(TAG_STRING)?;
        let start = self.pos;
        loop {
            match self.data.get(self.pos) {
                Some(&0) => break,
                Some(_) => self.pos += 1,
                None => return Err("pulse: unterminated string".into()),
            }
        }
        let s = std::str::from_utf8(&self.data[start..self.pos]).map_err(|_| "pulse: bad string".to_string())?;
        self.pos += 1;
        Ok(s.to_string())
    }
    fn string_or_null(&mut self) -> Result<Option<String>, String> {
        if self.data.get(self.pos) == Some(&TAG_STRING_NULL) {
            self.pos += 1;
            Ok(None)
        } else {
            self.string().map(Some)
        }
    }
    fn skip_sample_spec(&mut self) -> Result<(), String> {
        self.tag(TAG_SAMPLE_SPEC)?;
        self.take(6).map(|_| ())
    }
    fn skip_channel_map(&mut self) -> Result<(), String> {
        self.tag(TAG_CHANNEL_MAP)?;
        let n = self.byte()? as usize;
        self.take(n).map(|_| ())
    }
    fn skip_cvolume(&mut self) -> Result<(), String> {
        self.tag(TAG_CVOLUME)?;
        let n = self.byte()? as usize;
        self.take(4 * n).map(|_| ())
    }
}

fn parse_header(payload: &[u8]) -> Result<(u32, u32, &[u8]), String> {
    let mut r = Reader::new(payload);
    let cmd = r.u32()?;
    let tag = r.u32()?;
    Ok((cmd, tag, &payload[r.pos..]))
}

fn read_interruptible(mut sock: &UnixStream, out: &mut [u8], stop: &AtomicBool) -> Result<usize, String> {
    let mut got = 0;
    let mut pfd = libc::pollfd {
        fd: sock.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    while got < out.len() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        loop {
            let r = unsafe { libc::poll(&mut pfd, 1, 50) };
            if r >= 0 {
                break;
            }
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("pulse: poll: {e}"));
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match sock.read(&mut out[got..]) {
            Ok(0) => return Err("pulse: server closed the connection".into()),
            Ok(n) => got += n,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(format!("pulse: read: {e}"));
            }
        }
    }
    Ok(got)
}

fn load_cookie() -> Vec<u8> {
    let mut cookie = vec![0u8; COOKIE_LEN];
    let mut paths = Vec::new();
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        paths.push(format!("{rt}/pulse/cookie"));
    }
    if let Ok(home) = std::env::var("HOME") {
        paths.push(format!("{home}/.config/pulse/cookie"));
        paths.push(format!("{home}/.pulse-cookie"));
    }
    for p in paths {
        if let Ok(mut f) = std::fs::File::open(&p) {
            if f.read(&mut cookie).is_ok() {
                break;
            }
        }
    }
    cookie
}

fn server_candidates() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if let Ok(srv) = std::env::var("PULSE_SERVER") {
        for part in srv.split(' ') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some(rest) = part.strip_prefix("unix:") {
                v.push(rest.to_string());
            } else if !part.starts_with("tcp:") {
                v.push(part.to_string());
            }
        }
    }
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        v.push(format!("{rt}/pulse/native"));
    }
    let uid = unsafe { libc::geteuid() };
    v.push(format!("/run/user/{uid}/pulse/native"));
    v.push(format!("/tmp/pulse-{uid}/native"));
    v
}

enum Frame {
    Packet(Vec<u8>),
    Data(u32, Vec<u8>),
}

pub struct Pulse {
    sock: UnixStream,
    tag: u32,
}

impl Pulse {
    pub fn connect() -> Result<Pulse, String> {
        let mut last_err = "no pulse server socket found".to_string();
        for path in server_candidates() {
            match UnixStream::connect(&path) {
                Ok(sock) => {
                    let mut p = Pulse { sock, tag: 0 };
                    p.auth()?;
                    p.set_client_name()?;
                    return Ok(p);
                }
                Err(e) => last_err = format!("pulse: {path}: {e}"),
            }
        }
        Err(last_err)
    }

    fn next_tag(&mut self) -> u32 {
        self.tag += 1;
        self.tag
    }

    fn send(&mut self, cmd: u32, tag: u32, body: &Writer) -> Result<(), String> {
        let mut payload = Vec::with_capacity(body.raw.len() + 10);
        payload.push(TAG_U32);
        payload.extend_from_slice(&cmd.to_be_bytes());
        payload.push(TAG_U32);
        payload.extend_from_slice(&tag.to_be_bytes());
        payload.extend_from_slice(&body.raw);
        let mut frame = Vec::with_capacity(20 + payload.len());
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&u32::MAX.to_be_bytes());
        frame.extend_from_slice(&[0; 12]);
        frame.extend_from_slice(&payload);
        self.sock.write_all(&frame).map_err(|e| format!("pulse: write: {e}"))
    }

    fn read_frame(&mut self, stop: &AtomicBool) -> Result<Option<Frame>, String> {
        let mut desc = [0u8; 20];
        if read_interruptible(&self.sock, &mut desc, stop)? < 20 {
            return Ok(None);
        }
        let len = u32::from_be_bytes(desc[0..4].try_into().unwrap()) as usize;
        let channel = u32::from_be_bytes(desc[4..8].try_into().unwrap());
        if len == 0 || len > 16 * 1024 * 1024 {
            return Err(format!("pulse: bogus frame size {len}"));
        }
        let mut payload = vec![0u8; len];
        if read_interruptible(&self.sock, &mut payload, stop)? < len {
            return Ok(None);
        }
        if channel == u32::MAX {
            Ok(Some(Frame::Packet(payload)))
        } else {
            Ok(Some(Frame::Data(channel, payload)))
        }
    }

    fn reply_for(&mut self, want: u32) -> Result<Vec<u8>, String> {
        let idle = AtomicBool::new(false);
        loop {
            match self.read_frame(&idle)? {
                Some(Frame::Packet(payload)) => {
                    let (cmd, tag, rest) = parse_header(&payload)?;
                    if tag == want {
                        match cmd {
                            CMD_REPLY => return Ok(rest.to_vec()),
                            CMD_ERROR => {
                                let code = Reader::new(rest).u32().unwrap_or(0);
                                return Err(format!("pulse: server error {code}"));
                            }
                            _ => {}
                        }
                    }
                }
                Some(Frame::Data(_, _)) | None => {}
            }
        }
    }

    fn auth(&mut self) -> Result<(), String> {
        let tag = self.next_tag();
        let mut body = Writer::new();
        body.u32(PA_PROTOCOL_VERSION);
        body.arbitrary(&load_cookie());
        self.send(CMD_AUTH, tag, &body)?;
        let reply = self.reply_for(tag)?;
        let mut r = Reader::new(&reply);
        let version = r.u32().unwrap_or(PA_PROTOCOL_VERSION);
        let _ = version & PA_PROTOCOL_VERSION_MASK;
        Ok(())
    }

    fn set_client_name(&mut self) -> Result<(), String> {
        let tag = self.next_tag();
        let mut body = Writer::new();
        body.proplist(&[
            ("application.name", b"sharkvis".as_slice()),
            ("application.process.binary", b"sharkvis".as_slice()),
        ]);
        self.send(CMD_SET_CLIENT_NAME, tag, &body)?;
        self.reply_for(tag).map(|_| ())
    }

    pub fn default_monitor(&mut self) -> Result<String, String> {
        let tag = self.next_tag();
        let mut body = Writer::new();
        body.u32(PA_INVALID_INDEX);
        body.string_null();
        self.send(CMD_GET_SINK_INFO, tag, &body)?;
        let reply = self.reply_for(tag)?;
        let mut r = Reader::new(&reply);
        let _index = r.u32()?;
        let _name = r.string_or_null()?;
        let _desc = r.string_or_null()?;
        r.skip_sample_spec()?;
        r.skip_channel_map()?;
        let _module = r.u32()?;
        r.skip_cvolume()?;
        let _mute = r.boolean()?;
        let _monitor_index = r.u32()?;
        let mon = r.string_or_null()?;
        match mon {
            Some(m) if !m.is_empty() => Ok(m),
            _ => Err("pulse: default sink has no monitor source".into()),
        }
    }

    pub fn record(mut self, device: &str, rate: u32, channels: u8, fragsize: u32) -> Result<Record, String> {
        let map: Vec<u8> = if channels >= 2 {
            vec![1, 2]
        } else {
            vec![0]
        };
        let tag = self.next_tag();
        let mut body = Writer::new();
        body.sample_spec(PA_SAMPLE_S16NE, channels, rate);
        body.channel_map(&map);
        body.u32(PA_INVALID_INDEX);
        body.string(device);
        body.u32(u32::MAX);
        body.boolean(false);
        body.u32(fragsize);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        body.proplist(&[
            ("media.name", b"sharkvis spectrum".as_slice()),
            ("application.name", b"sharkvis".as_slice()),
            ("application.process.binary", b"sharkvis".as_slice()),
        ]);
        body.u32(PA_INVALID_INDEX);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        body.u8(0);
        body.cvolume(channels, PA_VOLUME_NORM);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        body.boolean(false);
        self.send(CMD_CREATE_RECORD_STREAM, tag, &body)?;
        let reply = self.reply_for(tag)?;
        let mut r = Reader::new(&reply);
        let stream_index = r.u32()?;
        Ok(Record {
            sock: self.sock,
            stream_index,
            pending: Vec::new(),
            offset: 0,
        })
    }
}

pub struct Record {
    sock: UnixStream,
    stream_index: u32,
    pending: Vec<u8>,
    offset: usize,
}

impl Record {
    fn read_frame(&mut self, stop: &AtomicBool) -> Result<Option<Frame>, String> {
        let mut desc = [0u8; 20];
        if read_interruptible(&self.sock, &mut desc, stop)? < 20 {
            return Ok(None);
        }
        let len = u32::from_be_bytes(desc[0..4].try_into().unwrap()) as usize;
        let channel = u32::from_be_bytes(desc[4..8].try_into().unwrap());
        if len == 0 || len > 16 * 1024 * 1024 {
            return Err(format!("pulse: bogus frame size {len}"));
        }
        let mut payload = vec![0u8; len];
        if read_interruptible(&self.sock, &mut payload, stop)? < len {
            return Ok(None);
        }
        if channel == u32::MAX {
            Ok(Some(Frame::Packet(payload)))
        } else {
            Ok(Some(Frame::Data(channel, payload)))
        }
    }

    pub fn read(&mut self, out: &mut [u8], stop: &AtomicBool) -> Result<usize, String> {
        let mut filled = 0;
        while filled < out.len() {
            if self.offset >= self.pending.len() {
                self.pending.clear();
                self.offset = 0;
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                match self.read_frame(stop)? {
                    Some(Frame::Data(channel, data)) => {
                        if channel == self.stream_index {
                            self.pending = data;
                        }
                    }
                    Some(Frame::Packet(payload)) => {
                        let mut r = Reader::new(&payload);
                        let cmd = r.u32()?;
                        let _tag = r.u32()?;
                        match cmd {
                            CMD_ERROR => {
                                let code = r.u32().unwrap_or(0);
                                return Err(format!("pulse: server error {code}"));
                            }
                            CMD_RECORD_STREAM_KILLED => {
                                return Err("pulse: record stream was killed".into());
                            }
                            _ => {}
                        }
                    }
                    None => break,
                }
                continue;
            }
            let avail = self.pending.len() - self.offset;
            let n = avail.min(out.len() - filled);
            out[filled..filled + n].copy_from_slice(&self.pending[self.offset..self.offset + n]);
            self.offset += n;
            filled += n;
        }
        Ok(filled)
    }
}