use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct Track {
    pub present: bool,
    pub player: String,
    pub artist: String,
    pub title: String,
    pub position: f64,
    pub duration: f64,
    pub url: String,
}

impl Track {
    pub fn key(&self) -> String {
        if !self.present {
            return String::new();
        }
        if !self.title.trim().is_empty() {
            return format!("{}|{}", self.artist.trim(), self.title.trim());
        }
        if !self.url.trim().is_empty() {
            return format!("url|{}", self.url.trim());
        }
        String::new()
    }
}

pub(crate) fn cmd_out(cmd: &str, args: &[&str], timeout_ms: u64) -> Option<String> {
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let pid = child.id() as libc::pid_t;
    let out = std::thread::scope(|s| {
        let h = s.spawn(|| {
            use std::io::Read;
            let mut buf = Vec::new();
            if let Some(o) = child.stdout.take() {
                let _ = o.take(64 * 1024).read_to_end(&mut buf);
            }
            let _ = child.wait();
            buf
        });
        let start = Instant::now();
        loop {
            if h.is_finished() {
                break;
            }
            if start.elapsed() > Duration::from_millis(timeout_ms) {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        h.join().ok()
    })?;
    String::from_utf8(out).ok().map(|s| s.trim().to_string())
}

fn playing_player(allow: &[String]) -> Option<String> {
    let list = cmd_out("playerctl", &["-l"], 500)?;
    for line in list.lines() {
        let p = line.trim();
        if p.is_empty() {
            continue;
        }
        if !allow.is_empty() && !allow.iter().any(|a| p == a || p.starts_with(a)) {
            continue;
        }
        if let Some(st) = cmd_out("playerctl", &["-p", p, "status"], 500) {
            if st.eq_ignore_ascii_case("playing") {
                return Some(p.to_string());
            }
        }
    }
    None
}

pub fn poll_track(allow: &[String]) -> Track {
    let mut t = Track::default();
    let Some(player) = playing_player(allow) else {
        return t;
    };
    t.player = player.clone();
    let meta = cmd_out(
        "playerctl",
        &["-p", &player, "metadata", "--format", "{{artist}}|{{title}}|{{mpris:length}}|{{xesam:url}}"],
        500,
    )
    .unwrap_or_default();
    let mut parts = meta.splitn(4, '|');
    t.artist = parts.next().unwrap_or("").trim().to_string();
    t.title = parts.next().unwrap_or("").trim().to_string();
    t.duration = parts
        .next()
        .unwrap_or("")
        .trim()
        .parse::<f64>()
        .map(|us| us / 1_000_000.0)
        .unwrap_or(0.0);
    t.url = parts.next().unwrap_or("").trim().to_string();
    if t.title.is_empty() && t.url.is_empty() {
        return t;
    }
    t.position = poll_position(&t.player).unwrap_or(0.0);
    t.present = true;
    t
}

pub fn poll_position(player: &str) -> Option<f64> {
    if player.is_empty() {
        return None;
    }
    cmd_out("playerctl", &["-p", player, "position"], 300)
        .and_then(|s| s.parse::<f64>().ok())
}
