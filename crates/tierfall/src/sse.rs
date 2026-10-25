//! Minimal Server-Sent Events parser and encoder.

use bytes::Bytes;

/// One SSE event. Comment lines are dropped; multiple `data:` lines are joined with `\n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

impl SseEvent {
    /// True for the OpenAI end-of-stream sentinel.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.data.trim() == "[DONE]"
    }

    /// Encodes the event in wire format.
    #[must_use]
    pub fn encode(&self) -> Bytes {
        let mut out = String::new();
        if let Some(event) = &self.event {
            out.push_str("event: ");
            out.push_str(event);
            out.push('\n');
        }
        for line in self.data.split('\n') {
            out.push_str("data: ");
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
        Bytes::from(out)
    }

    /// Builds a `data:`-only event.
    #[must_use]
    pub fn data(data: impl Into<String>) -> Self {
        Self {
            event: None,
            data: data.into(),
        }
    }
}

/// Incremental SSE parser that tolerates events and UTF-8 sequences split across chunks.
#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds bytes and returns all events completed by them.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if let Some(event) = self.handle_line(&line) {
                events.push(event);
            }
        }
        events
    }

    /// Flushes any trailing event that was not terminated by a blank line.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            let line = String::from_utf8_lossy(&rest)
                .trim_end_matches('\r')
                .to_string();
            if let Some(event) = self.handle_line(&line) {
                return Some(event);
            }
        }
        self.take_event()
    }

    fn handle_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.take_event();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "data" => self.data.push(value.to_string()),
            "event" => self.event = Some(value.to_string()),
            _ => {}
        }
        None
    }

    fn take_event(&mut self) -> Option<SseEvent> {
        if self.data.is_empty() {
            self.event = None;
            return None;
        }
        let data = std::mem::take(&mut self.data).join("\n");
        Some(SseEvent {
            event: self.event.take(),
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_split_events_and_comments() {
        let mut p = SseParser::new();
        let mut events = p.push(b": OPENROUTER PROCESSING\n\ndata: {\"a\"");
        assert!(events.is_empty());
        events.extend(p.push(b":1}\r\n\r\ndata: [DONE]\n\n"));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "{\"a\":1}");
        assert!(events[1].is_done());
    }

    #[test]
    fn handles_multiline_data_and_utf8_split() {
        let mut p = SseParser::new();
        let bytes = "event: x\ndata: line1\ndata: caf\u{e9}\n\n".as_bytes();
        let (a, b) = bytes.split_at(bytes.len() - 4);
        let mut events = p.push(a);
        events.extend(p.push(b));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("x"));
        assert_eq!(events[0].data, "line1\ncaf\u{e9}");
    }

    #[test]
    fn finish_flushes_unterminated_event() {
        let mut p = SseParser::new();
        assert!(p.push(b"data: tail").is_empty());
        assert_eq!(p.finish().unwrap().data, "tail");
    }

    #[test]
    fn encode_round_trip() {
        let ev = SseEvent::data("a\nb");
        let mut p = SseParser::new();
        let events = p.push(&ev.encode());
        assert_eq!(events, vec![ev]);
    }
}
