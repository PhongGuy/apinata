//! The growing view of a response that is still arriving: a bounded tail of
//! text lines and, for event streams, the parsed events.

use std::collections::VecDeque;

use apinata_core::sse::{SseEvent, SseParser};

/// Complete lines kept for display; older ones are counted and dropped.
pub const TAIL_LINES: usize = 2000;
/// Events kept for display; older ones are counted in `dropped_events`.
pub const EVENT_CAP: usize = 10_000;
/// A longer line is cut into rows of this many bytes.
pub const MAX_LINE: usize = 64 * 1024;

/// Line tail, byte count and (for event streams) events of one live response.
pub struct LiveStream {
    status: u16,
    headers: Vec<(String, String)>,
    event_stream: bool,
    bytes: u64,
    /// Complete rows, oldest first, at most `TAIL_LINES`.
    lines: VecDeque<String>,
    /// Bytes of the line still waiting for its terminator.
    open: Vec<u8>,
    skipped: u64,
    parser: SseParser,
    events: Vec<SseEvent>,
    /// Events already compacted away from the front of `events`.
    compacted: u64,
}

impl LiveStream {
    /// An empty buffer for a response whose headers have just arrived.
    pub fn new(status: u16, headers: Vec<(String, String)>, event_stream: bool) -> LiveStream {
        LiveStream {
            status,
            headers,
            event_stream,
            bytes: 0,
            lines: VecDeque::new(),
            open: Vec::new(),
            skipped: 0,
            parser: SseParser::new(),
            events: Vec::new(),
            compacted: 0,
        }
    }

    /// Adds body bytes: splits them into lines and, for event streams, events.
    pub fn push(&mut self, bytes: &[u8]) {
        self.bytes += bytes.len() as u64;
        if self.event_stream {
            self.events.extend(self.parser.push(bytes));
            // Compact lazily so the front drain is amortised, not per event.
            if self.events.len() >= EVENT_CAP * 2 {
                let excess = self.events.len() - EVENT_CAP;
                self.events.drain(..excess);
                self.compacted += excess as u64;
            }
        }
        for &b in bytes {
            if b == b'\n' {
                // Decode only whole lines: UTF-8 never contains 0x0A, so a
                // split multibyte char is never cut here.
                if self.open.last() == Some(&b'\r') {
                    self.open.pop();
                }
                self.emit_line(self.open.len());
            } else {
                // One endless line must not grow the open buffer without
                // bound. A CR right at the limit may still pair with an LF.
                let cr_fits = b == b'\r' && self.open.len() == MAX_LINE;
                if self.open.len() >= MAX_LINE && !cr_fits {
                    self.emit_line(self.cut_point());
                }
                self.open.push(b);
            }
        }
    }

    /// Where to cut the open line at the limit: before an incomplete
    /// trailing multibyte char, so the char stays whole in the next row.
    fn cut_point(&self) -> usize {
        let end = self.open.len();
        let mut i = end;
        while i > 0 && end - i < 3 && self.open[i - 1] & 0xC0 == 0x80 {
            i -= 1;
        }
        if i > 0 {
            let lead = self.open[i - 1];
            let need = match lead {
                0xF0..=0xF7 => 4,
                0xE0..=0xEF => 3,
                0xC0..=0xDF => 2,
                _ => 1,
            };
            if need > end - (i - 1) {
                return i - 1;
            }
        }
        end
    }

    /// Moves the first `upto` bytes of the open line into the tail.
    fn emit_line(&mut self, upto: usize) {
        let line = String::from_utf8_lossy(&self.open[..upto]).into_owned();
        self.open.drain(..upto);
        if self.lines.len() == TAIL_LINES {
            self.lines.pop_front();
            self.skipped += 1;
        }
        self.lines.push_back(line);
    }

    /// The open line as shown: a CR still waiting for its LF is hidden.
    fn open_view(&self) -> &[u8] {
        self.open.strip_suffix(b"\r").unwrap_or(&self.open)
    }

    /// The response status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The response headers as received.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Whether the body is parsed as server-sent events.
    pub fn is_event_stream(&self) -> bool {
        self.event_stream
    }

    /// Every byte pushed, including those whose lines fell off the tail.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Events completed so far; empty for a plain body.
    pub fn events(&self) -> &[SseEvent] {
        &self.events[self.events.len().saturating_sub(EVENT_CAP)..]
    }

    /// Events that fell off the front of the kept `EVENT_CAP`.
    pub fn dropped_events(&self) -> u64 {
        self.compacted + self.events.len().saturating_sub(EVENT_CAP) as u64
    }

    /// Tail rows, the open last line included when non-empty.
    pub fn row_count(&self) -> usize {
        self.lines.len() + usize::from(!self.open_view().is_empty())
    }

    /// Row `ix` of the tail, or `None` past the end.
    pub fn row(&self, ix: usize) -> Option<String> {
        match ix.cmp(&self.lines.len()) {
            std::cmp::Ordering::Less => self.lines.get(ix).cloned(),
            std::cmp::Ordering::Equal if !self.open_view().is_empty() => {
                Some(String::from_utf8_lossy(self.open_view()).into_owned())
            }
            _ => None,
        }
    }

    /// Lines that fell off the front of the tail.
    pub fn skipped_lines(&self) -> u64 {
        self.skipped
    }
}

/// Lines of an event's data shown in its row.
pub const PREVIEW_LINES: usize = 3;
/// Characters kept of each previewed line.
pub const PREVIEW_CHARS: usize = 240;

/// One row of the Events tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    /// The event name, `message` when the event has none.
    pub name: String,
    /// The event id, when it sent one.
    pub id: Option<String>,
    /// The first lines of the data, cut short with an ellipsis.
    pub preview: String,
}

/// The first `PREVIEW_LINES` lines of `data`, each cut at `PREVIEW_CHARS`; an ellipsis marks
/// that anything was left out.
pub fn preview(data: &str) -> String {
    let mut cut = false;
    let mut lines = data.split('\n');
    let mut kept: Vec<String> = Vec::new();
    for line in lines.by_ref().take(PREVIEW_LINES) {
        match line.char_indices().nth(PREVIEW_CHARS) {
            Some((at, _)) => {
                kept.push(line[..at].to_owned());
                cut = true;
            }
            None => kept.push(line.to_owned()),
        }
    }
    cut |= lines.next().is_some();
    let mut text = kept.join("\n");
    if cut {
        text.push('\u{2026}');
    }
    text
}

/// The rows for `events`, in order.
pub fn event_rows(events: &[SseEvent]) -> Vec<EventRow> {
    events
        .iter()
        .map(|e| EventRow {
            name: e
                .event
                .clone()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "message".to_owned()),
            id: e.id.clone(),
            preview: preview(&e.data),
        })
        .collect()
}

/// A short word for why a stream ended, shown beside the size.
pub fn end_label(end: &apinata_core::StreamEnd) -> &'static str {
    use apinata_core::StreamEnd;
    match end {
        StreamEnd::Closed => "closed",
        StreamEnd::Stopped => "stopped",
        StreamEnd::Bound(_) => "limit",
        StreamEnd::IdleTimeout => "idle timeout",
        StreamEnd::BodyCap => "size limit",
        StreamEnd::Error(_) => "error",
        _ => "ended",
    }
}
