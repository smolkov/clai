//! Server-sent events: turns a byte stream into events.
//!
//! Chunks may end anywhere, also inside a line or a UTF-8 character.

use super::error::ProviderError;

/// Largest event (or unfinished line) accepted, so a broken server cannot
/// use up all memory.
pub const MAX_EVENT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field, if any.
    pub event: Option<String>,
    /// The `data:` lines, joined with `\n`.
    pub data: String,
}

#[derive(Debug, Default)]
struct Pending {
    event: Option<String>,
    data: String,
    has_data: bool,
}

#[derive(Debug)]
pub struct SseParser {
    buf: Vec<u8>,
    pending: Pending,
    max: usize,
}

impl Default for SseParser {
    fn default() -> Self {
        SseParser::new()
    }
}

impl SseParser {
    pub fn new() -> SseParser {
        SseParser::with_limit(MAX_EVENT_BYTES)
    }

    pub fn with_limit(max: usize) -> SseParser {
        SseParser {
            buf: Vec::new(),
            pending: Pending::default(),
            max,
        }
    }

    /// Adds a chunk and returns the events it completed.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, ProviderError> {
        self.buf.extend_from_slice(chunk);
        let mut events = Vec::new();
        let mut start = 0;
        while let Some(offset) = self
            .buf
            .get(start..)
            .and_then(|rest| rest.iter().position(|&b| b == b'\n'))
        {
            let end = start + offset;
            let line = self.buf.get(start..end).unwrap_or_default();
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let line = std::str::from_utf8(line)
                .map_err(|_| ProviderError::protocol("event stream is not UTF-8"))?;
            if let Some(event) = Self::line(&mut self.pending, line) {
                events.push(event);
            }
            if self.pending.data.len() > self.max {
                return Err(too_large());
            }
            start = end + 1;
        }
        self.buf.drain(..start);
        if self.buf.len() > self.max {
            return Err(too_large());
        }
        Ok(events)
    }

    /// Handles one line; returns an event at the blank line that ends it.
    fn line(pending: &mut Pending, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            let done = std::mem::take(pending);
            if !done.has_data {
                return None;
            }
            let mut data = done.data;
            if data.ends_with('\n') {
                data.pop();
            }
            return Some(SseEvent {
                event: done.event,
                data,
            });
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => pending.event = Some(value.to_owned()),
            "data" => {
                pending.data.push_str(value);
                pending.data.push('\n');
                pending.has_data = true;
            }
            _ => {}
        }
        None
    }
}

fn too_large() -> ProviderError {
    ProviderError::protocol("event stream: event too large")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_owned),
            data: data.to_owned(),
        }
    }

    fn parse_all(input: &[u8]) -> Vec<SseEvent> {
        SseParser::new().push(input).unwrap()
    }

    fn parse_bytewise(input: &[u8]) -> Vec<SseEvent> {
        let mut parser = SseParser::new();
        input
            .iter()
            .flat_map(|b| parser.push(std::slice::from_ref(b)).unwrap())
            .collect()
    }

    #[test]
    fn events_and_fields() {
        let input =
            b"event: message_start\ndata: {\"a\":1}\n\n: comment\ndata:x\ndata: y\nid: 3\n\n";
        let want = [ev(Some("message_start"), "{\"a\":1}"), ev(None, "x\ny")];
        assert_eq!(parse_all(input), want);
        assert_eq!(parse_bytewise(input), want);
    }

    #[test]
    fn crlf_and_utf8_split() {
        let input = "data: grüße 🦀\r\n\r\n".as_bytes();
        assert_eq!(parse_bytewise(input), [ev(None, "grüße 🦀")]);
    }

    #[test]
    fn event_without_data_is_dropped() {
        assert!(parse_all(b"event: ping\n\n").is_empty());
    }

    #[test]
    fn unfinished_event_waits() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: a\n").unwrap().is_empty());
        assert_eq!(parser.push(b"\n").unwrap(), [ev(None, "a")]);
    }

    #[test]
    fn invalid_utf8() {
        let err = SseParser::new().push(b"data: \xff\n\n").unwrap_err();
        assert!(err.message.contains("UTF-8"));
    }

    #[test]
    fn limit() {
        let mut parser = SseParser::with_limit(8);
        assert!(parser.push(b"data: 123456789").is_err());
        let mut parser = SseParser::with_limit(8);
        assert!(parser.push(b"data: 12345\ndata: 12345\n").is_err());
    }
}
