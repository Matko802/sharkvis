use std::time::{Duration, Instant};

use crate::lyrics::json_string;
use crate::mpris::cmd_out;

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

fn strip_tags(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => o.push(c),
            _ => {}
        }
    }
    o.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn web_search(query: &str) -> Option<String> {
    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        crate::lyrics::url_encode(query)
    );
    let body = cmd_out("curl", &["-fsSL", "-m", "10", "-A", "Mozilla/5.0", &url], 12000)?;
    let mut out = Vec::new();
    let mut rest = body.as_str();
    while let Some(a) = rest.find("result__snippet") {
        rest = &rest[a..];
        let Some(gt) = rest.find('>') else { break };
        rest = &rest[gt + 1..];
        let Some(end) = rest.find("</a>") else { break };
        let snippet = strip_tags(&rest[..end]);
        rest = &rest[end + 4..];
        if snippet.len() > 40 {
            out.push(snippet);
        }
        if out.len() >= 3 {
            break;
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(out.join(" "))
}

fn ask_ollama(host: &str, model: &str, question: &str, context: &str) -> Option<String> {
    let prompt = if context.is_empty() {
        format!(
            "Answer very briefly in at most 25 words. Plain text only, no quotes, no emoji. Question: {}",
            question.replace('\'', "")
        )
    } else {
        format!(
            "Answer very briefly in at most 25 words using this web context: {}. Plain text only, no quotes, no emoji. Question: {}",
            context.replace('\'', ""),
            question.replace('\'', "")
        )
    };
    let body = format!(
        "{{\"model\":\"{}\",\"prompt\":\"{}\",\"stream\":false}}",
        json_escape(model),
        json_escape(&prompt)
    );
    let url = format!("{}/api/generate", host.trim_end_matches('/'));
    let out = cmd_out(
        "curl",
        &[
            "-fsSL", "-m", "120", "-X", "POST", &url, "-H", "Content-Type: application/json",
            "-d", &body,
        ],
        125000,
    )?;
    let mut reply = json_string(&out, "response")?;
    reply = reply.trim().to_string();
    if reply.is_empty() {
        return None;
    }
    if reply.len() > 300 {
        reply.truncate(300);
    }
    Some(reply)
}

fn speak(text: &str) {
    let _ = cmd_out("spd-say", &["-C"], 2000);
    let _ = cmd_out("spd-say", &["-w", text], 8000);
}

pub struct Assistant {
    reply: Option<String>,
    rx: Option<std::sync::mpsc::Receiver<Option<String>>>,
    busy: bool,
    scroll: usize,
    last_scroll: Instant,
}

impl Assistant {
    pub fn new() -> Self {
        Assistant {
            reply: None,
            rx: None,
            busy: false,
            scroll: 0,
            last_scroll: Instant::now(),
        }
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn ask(&mut self, model: &str, host: &str, question: &str, web: bool, speech: bool) {
        if self.busy {
            return;
        }
        let q = question.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.busy = true;
        self.reply = None;
        self.scroll = 0;
        self.last_scroll = Instant::now();
        let model = model.to_string();
        let host = host.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let ctx = if web { web_search(&q).unwrap_or_default() } else { String::new() };
            let reply = ask_ollama(&host, &model, &q, &ctx);
            if speech {
                if let Some(text) = reply.clone() {
                    speak(&text);
                }
            }
            let _ = tx.send(reply);
        });
    }

    pub fn poll(&mut self) {
        let incoming = self.rx.as_ref().and_then(|r| r.try_recv().ok());
        if let Some(reply) = incoming {
            self.rx = None;
            self.busy = false;
            self.reply = reply.clone();
            self.scroll = 0;
            self.last_scroll = Instant::now();
            if let Some(text) = reply {
                if let Ok(path) = std::env::var("SHARKVIS_DEBUG_ASK") {
                    let _ = std::fs::write(&path, &text);
                }
            }
        }
    }

    pub fn has_reply(&self) -> bool {
        self.reply.is_some()
    }

    pub fn view(&mut self, width_chars: usize) -> String {
        if self.busy && self.reply.is_none() {
            return "THINKING".to_string();
        }
        let Some(reply) = &self.reply else {
            return String::new();
        };
        let words: Vec<&str> = reply.split_whitespace().collect();
        if words.is_empty() {
            return String::new();
        }
        let win = width_chars.max(10);
        if self.scroll >= words.len() {
            self.scroll = 0;
        }
        let mut end = self.scroll;
        let mut len = 0usize;
        while end < words.len() {
            let wlen = words[end].len() + if end > self.scroll { 1 } else { 0 };
            if len + wlen > win {
                break;
            }
            len += wlen;
            end += 1;
        }
        if end == self.scroll && end < words.len() {
            end += 1;
        }
        if self.last_scroll.elapsed() >= Duration::from_millis(500) {
            self.last_scroll = Instant::now();
            if end < words.len() && self.scroll + 1 < words.len() {
                self.scroll += 1;
            }
        }
        words[self.scroll..end.min(words.len())].join(" ")
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

    #[test]
    fn strips_html_tags() {
        assert_eq!(strip_tags("<a class=\"x\">hello <b>world</b></a>"), "hello world");
        assert_eq!(strip_tags("plain text"), "plain text");
    }
}
