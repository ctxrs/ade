#[derive(Debug, Clone)]
pub struct SseEvent {
    pub data: String,
}

#[derive(Default)]
struct SseEventBuilder {
    data_lines: Vec<String>,
}

impl SseEventBuilder {
    fn push_line(&mut self, line: &str) {
        if let Some(rest) = line.strip_prefix("data:") {
            self.data_lines.push(rest.trim_start().to_string());
            return;
        }
    }

    fn finalize(&mut self) -> Option<SseEvent> {
        if self.data_lines.is_empty() {
            self.clear();
            return None;
        }
        let event = SseEvent {
            data: self.data_lines.join("\n"),
        };
        self.clear();
        Some(event)
    }

    fn clear(&mut self) {
        self.data_lines.clear();
    }
}

#[derive(Default)]
pub struct SseDecoder {
    buffer: String,
    current: SseEventBuilder,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        let chunk = String::from_utf8_lossy(bytes);
        self.buffer.push_str(&chunk);
        self.drain_buffer()
    }

    fn drain_buffer(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        while let Some(pos) = self.buffer.find('\n') {
            let mut line = self.buffer[..pos].to_string();
            self.buffer.drain(..=pos);
            if line.ends_with('\r') {
                line.pop();
            }
            if line.is_empty() {
                if let Some(event) = self.current.finalize() {
                    events.push(event);
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            self.current.push_line(&line);
        }
        events
    }

    pub fn flush(&mut self) -> Option<SseEvent> {
        if let Some(event) = self.current.finalize() {
            return Some(event);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_event() {
        let mut decoder = SseDecoder::new();
        let events = decoder.push_bytes(b"data: {\"text\":\"hi\"}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "{\"text\":\"hi\"}");
    }

    #[test]
    fn parses_multi_line_event() {
        let mut decoder = SseDecoder::new();
        let events = decoder.push_bytes(b"data: first\n");
        assert!(events.is_empty());
        let events = decoder.push_bytes(b"data: second\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "first\nsecond");
    }
}
