//! Server-Sent Events parsing following the WHATWG rules, in a one-shot and
//! an incremental form. Fields are reported as they appeared on the wire.

/// One dispatched event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SseEvent {
    /// The `id:` field of this event only.
    pub id: Option<String>,
    /// The `event:` field, the event name.
    pub event: Option<String>,
    /// The `data:` lines joined with LF.
    pub data: String,
    /// The `retry:` field in milliseconds.
    pub retry: Option<u64>,
}

impl SseEvent {
    /// An event carrying `data` and no other field.
    pub fn new(data: impl Into<String>) -> SseEvent {
        SseEvent {
            data: data.into(),
            ..SseEvent::default()
        }
    }

    /// Sets the event name.
    pub fn with_event(mut self, name: impl Into<String>) -> SseEvent {
        self.event = Some(name.into());
        self
    }

    /// Sets the id.
    pub fn with_id(mut self, id: impl Into<String>) -> SseEvent {
        self.id = Some(id.into());
        self
    }

    /// Sets the retry delay in milliseconds.
    pub fn with_retry(mut self, ms: u64) -> SseEvent {
        self.retry = Some(ms);
        self
    }
}

/// All complete events in `bytes`; a trailing incomplete event is dropped.
pub fn parse(bytes: &[u8]) -> Vec<SseEvent> {
    SseParser::new().push(bytes)
}

/// True when one `;`-separated part of a Content-Type equals
/// `text/event-stream`, trimmed, case-insensitive.
pub fn is_event_stream(content_type: &str) -> bool {
    content_type
        .split(';')
        .any(|part| part.trim().eq_ignore_ascii_case("text/event-stream"))
}

/// Longest line the parser buffers, in bytes. WHATWG sets no limit; a line
/// past this is dropped whole so an endless line cannot grow memory. One
/// event's accumulated `data` is held to the same size.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Incremental form: feed arbitrary slices, get the events completed by each.
#[derive(Debug)]
#[non_exhaustive]
pub struct SseParser {
    /// Longest line kept; a longer one is dropped up to its terminator.
    max_line: usize,
    /// The current line passed `max_line` and is being discarded.
    skipping: bool,
    /// Bytes of the line still waiting for its terminator.
    line: Vec<u8>,
    /// The previous byte was a CR, so a directly following LF belongs to it.
    after_cr: bool,
    /// Bytes seen so far, to strip a BOM only at the stream start.
    seen: usize,
    id: Option<String>,
    event: Option<String>,
    data: Option<String>,
    retry: Option<u64>,
    /// Number of `data` lines in the event being built.
    data_lines: usize,
}

impl Default for SseParser {
    fn default() -> SseParser {
        SseParser::with_max_line(MAX_LINE_BYTES)
    }
}

impl SseParser {
    /// A parser at the start of a stream.
    pub fn new() -> SseParser {
        SseParser::default()
    }

    /// A parser that drops any line longer than `max` bytes.
    pub fn with_max_line(max: usize) -> SseParser {
        SseParser {
            max_line: max,
            skipping: false,
            line: Vec::new(),
            after_cr: false,
            seen: 0,
            id: None,
            event: None,
            data: None,
            retry: None,
            data_lines: 0,
        }
    }

    /// Bytes of the unfinished line currently held.
    pub fn buffered(&self) -> usize {
        self.line.len()
    }

    /// Bytes of `data` gathered for the event being built.
    pub fn buffered_data(&self) -> usize {
        self.data.as_ref().map_or(0, String::len)
    }

    /// Feeds `bytes` and returns the events they completed.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        let mut out = Vec::new();
        for &b in bytes {
            self.seen += 1;
            // A line ends at LF, CR or CRLF; swallow the LF of a CRLF pair.
            let swallow = self.after_cr && b == b'\n';
            self.after_cr = false;
            if swallow {
                continue;
            }
            match b {
                b'\n' | b'\r' => {
                    self.after_cr = b == b'\r';
                    let line = std::mem::take(&mut self.line);
                    // A dropped line ends here without being interpreted.
                    if !std::mem::take(&mut self.skipping) {
                        self.line_done(&line, &mut out);
                    }
                }
                _ if self.skipping => {}
                _ if self.line.len() >= self.max_line => {
                    self.skipping = true;
                    self.line = Vec::new();
                }
                _ => {
                    self.line.push(b);
                    // The BOM is the first three bytes of the whole stream.
                    if self.seen == 3 && self.line == [0xEF, 0xBB, 0xBF] {
                        self.line.clear();
                    }
                }
            }
        }
        out
    }

    fn line_done(&mut self, raw: &[u8], out: &mut Vec<SseEvent>) {
        if raw.is_empty() {
            self.dispatch(out);
            return;
        }
        let line = String::from_utf8_lossy(raw);
        if line.starts_with(':') {
            return;
        }
        let (name, value) = match line.split_once(':') {
            Some((n, v)) => (n, v.strip_prefix(' ').unwrap_or(v)),
            None => (&*line, ""),
        };
        match name {
            "data" => {
                // Like an over-long line, a data line that would push the event
                // past the line cap is dropped whole, so many short lines
                // cannot grow one event without bound.
                let sep = usize::from(self.data_lines > 0);
                if self.buffered_data() + sep + value.len() > self.max_line {
                    return;
                }
                let d = self.data.get_or_insert_with(String::new);
                if self.data_lines > 0 {
                    d.push('\n');
                }
                d.push_str(value);
                self.data_lines += 1;
            }
            "event" => self.event = Some(value.to_string()),
            "id" if !value.contains('\0') => self.id = Some(value.to_string()),
            "retry" if !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()) => {
                self.retry = value.parse().ok();
            }
            _ => {}
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        let data = self.data.take();
        let id = self.id.take();
        let event = self.event.take();
        let retry = self.retry.take();
        self.data_lines = 0;
        if let Some(data) = data {
            out.push(SseEvent {
                id,
                event,
                data,
                retry,
            });
        }
    }
}
