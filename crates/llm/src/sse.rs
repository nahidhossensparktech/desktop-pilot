//! Minimal server-sent-events parser for the Messages API stream (ADR-003).

/// One SSE event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Incremental parser: feed bytes as they arrive, get complete events out.
#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: String,
    data: String,
}

impl SseParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop(); // '\n'
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if line.is_empty() {
                if !self.data.is_empty() || !self.event.is_empty() {
                    out.push(SseEvent {
                        event: std::mem::take(&mut self.event),
                        data: std::mem::take(&mut self.data),
                    });
                }
                continue;
            }
            if line.starts_with(':') {
                continue; // comment
            }
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line.as_str(), ""),
            };
            match field {
                "event" => self.event = value.to_owned(),
                "data" => {
                    if !self.data.is_empty() {
                        self.data.push('\n');
                    }
                    self.data.push_str(value);
                }
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAM: &str = "event: message_start\ndata: {\"a\":1}\n\nevent: ping\ndata: {}\n\n: comment\nevent: content_block_delta\r\ndata: {\"b\":\r\ndata: 2}\r\n\r\n";

    #[test]
    fn parses_whole_stream() {
        let mut p = SseParser::default();
        let events = p.push(STREAM.as_bytes());
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event, "message_start");
        assert_eq!(events[0].data, "{\"a\":1}");
        assert_eq!(events[2].data, "{\"b\":\n2}");
    }

    #[test]
    fn split_at_every_byte_gives_same_events() {
        let whole = SseParser::default().push(STREAM.as_bytes());
        let mut p = SseParser::default();
        let mut pieces = Vec::new();
        for b in STREAM.as_bytes() {
            pieces.extend(p.push(std::slice::from_ref(b)));
        }
        assert_eq!(pieces, whole);
    }

    #[test]
    fn utf8_split_across_chunks() {
        let text = "event: x\ndata: héllo 🙂\n\n".as_bytes();
        let mut p = SseParser::default();
        let mut events = p.push(&text[..17]);
        events.extend(p.push(&text[17..]));
        assert_eq!(events[0].data, "héllo 🙂");
    }
}
