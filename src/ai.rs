use crate::lyrics::json_string;
use crate::mpris::{cmd_out, Track};

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn ask_ollama(host: &str, model: &str, artist: &str, title: &str) -> Option<String> {
    let prompt = format!(
        "In one short punchy sentence of at most 15 words, hype up the song '{}' by '{}' for someone listening right now. Plain text only, no quotes, no emoji.",
        title.replace('\'', ""),
        artist.replace('\'', "")
    );
    let body = format!(
        "{{\"model\":\"{}\",\"prompt\":\"{}\",\"stream\":false}}",
        json_escape(model),
        json_escape(&prompt)
    );
    let url = format!("{}/api/generate", host.trim_end_matches('/'));
    let out = cmd_out("curl", &["-fsSL", "-m", "120", "-X", "POST", &url, "-H", "Content-Type: application/json", "-d", &body], 125000)?;
    let mut reply = json_string(&out, "response")?;
    reply = reply.trim().to_string();
    if reply.is_empty() {
        return None;
    }
    if reply.len() > 160 {
        reply.truncate(160);
    }
    Some(reply)
}

fn speak(text: &str) {
    let _ = cmd_out("spd-say", &["-C"], 2000);
    let _ = cmd_out("spd-say", &["-w", text], 5000);
}

pub struct AiWorker {
    last_key: String,
    reply: Option<String>,
    rx: Option<std::sync::mpsc::Receiver<(String, Option<String>)>>,
    busy_for: String,
}

impl AiWorker {
    pub fn new() -> Self {
        AiWorker {
            last_key: String::new(),
            reply: None,
            rx: None,
            busy_for: String::new(),
        }
    }

    pub fn update(&mut self, enabled: bool, track: &Track, model: &str, host: &str, speech: bool) {
        while let Some(Ok((k, reply))) = self.rx.as_ref().map(|r| r.try_recv()) {
            self.rx = None;
            self.busy_for = String::new();
            if k == self.last_key {
                if let Some(text) = reply {
                    if speech {
                        let t = text.clone();
                        std::thread::spawn(move || speak(&t));
                    }
                    self.reply = Some(text);
                }
            }
        }
        if !enabled {
            return;
        }
        let key = track.key();
        if key.is_empty() || key == self.last_key {
            return;
        }
        self.last_key = key.clone();
        self.reply = None;
        if self.busy_for == key {
            return;
        }
        self.busy_for = key.clone();
        let model = model.to_string();
        let host = host.to_string();
        let artist = track.artist.clone();
        let title = track.title.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let reply = ask_ollama(&host, &model, &artist, &title);
            let _ = tx.send((key, reply));
        });
    }

    pub fn display(&self) -> &str {
        self.reply.as_deref().unwrap_or("AI")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_escapes_cleanly() {
        let escaped = json_escape("say \"hi\"\nbye\\");
        assert_eq!(escaped, "say \\\"hi\\\"\\nbye\\\\");
    }
}
