//! The growing view of a response that is still arriving: a bounded tail of
//! text lines and, for event streams, the parsed events.

use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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
    /// When each of `events` arrived, as an offset from `started`.
    arrivals: Vec<Duration>,
    /// Events already compacted away from the front of `events`.
    compacted: u64,
    started: Instant,
    serial: u64,
}

// Tells one live stream from the next, so a cache keyed on a stream never outlives it.
static SERIAL: AtomicU64 = AtomicU64::new(0);

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
            arrivals: Vec::new(),
            compacted: 0,
            started: Instant::now(),
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Adds body bytes: splits them into lines and, for event streams, events.
    pub fn push(&mut self, bytes: &[u8]) {
        self.push_at(bytes, Instant::now());
    }

    /// Adds body bytes as `push` does, stamping the events they complete as arrived at `at`.
    pub fn push_at(&mut self, bytes: &[u8], at: Instant) {
        self.bytes += bytes.len() as u64;
        if self.event_stream {
            let done = self.parser.push(bytes);
            let offset = at.saturating_duration_since(self.started);
            self.arrivals
                .extend(std::iter::repeat_n(offset, done.len()));
            self.events.extend(done);
            // Compact lazily so the front drain is amortised, not per event.
            if self.events.len() >= EVENT_CAP * 2 {
                let excess = self.events.len() - EVENT_CAP;
                self.events.drain(..excess);
                self.arrivals.drain(..excess);
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

    /// When the response's headers arrived, which event times count from.
    pub fn started(&self) -> Instant {
        self.started
    }

    /// The arrival offsets of `events()`, index for index.
    pub fn arrivals(&self) -> &[Duration] {
        &self.arrivals[self.arrivals.len().saturating_sub(EVENT_CAP)..]
    }

    /// The arrival offsets of the kept events, by their index in the whole stream.
    pub fn times(&self) -> EventTimes {
        EventTimes {
            first: self.dropped_events(),
            times: self.arrivals().to_vec(),
        }
    }

    /// Tells this stream apart from every other one in the process.
    pub fn serial(&self) -> u64 {
        self.serial
    }
}

/// Arrival offsets of a stream's events, by the event's index in the whole stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventTimes {
    /// The stream index of the first kept time; earlier events were dropped.
    first: u64,
    times: Vec<Duration>,
}

impl EventTimes {
    /// The arrival offset of stream event `ix`, when it was stamped and kept.
    pub fn get(&self, ix: usize) -> Option<Duration> {
        let at = (ix as u64).checked_sub(self.first)?;
        self.times.get(usize::try_from(at).ok()?).copied()
    }

    /// The stream index of the first kept time.
    pub fn first(&self) -> u64 {
        self.first
    }
}

/// An arrival offset as the Events tab shows it: seconds with milliseconds, minutes once a
/// minute has passed, whole seconds past an hour; blank when unknown.
pub fn time_label(at: Option<Duration>) -> String {
    let Some(at) = at else {
        return String::new();
    };
    let ms = at.as_millis();
    let (secs, milli) = (ms / 1000, ms % 1000);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    match secs {
        0..60 => format!("+{s}.{milli:03}s"),
        60..3600 => format!("+{m}:{s:02}.{milli:03}"),
        _ => format!("+{h}:{m:02}:{s:02}"),
    }
}

/// Whether `event` holds `needle` (already lowercase) in its name, id or data.
pub fn event_matches(event: &SseEvent, needle: &str) -> bool {
    let name = event.event.as_deref().filter(|n| !n.is_empty());
    [
        name.unwrap_or("message"),
        event.id.as_deref().unwrap_or_default(),
        event.data.as_str(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(needle))
}

/// Where a filtered event list comes from: one live stream, or one finished result's events.
#[derive(Clone)]
pub enum FilterSource {
    /// A live stream, by its serial.
    Live(u64),
    /// A finished result's parsed or seeded events.
    Done(Rc<Vec<SseEvent>>),
}

impl FilterSource {
    fn same(&self, other: &FilterSource) -> bool {
        match (self, other) {
            (FilterSource::Live(a), FilterSource::Live(b)) => a == b,
            (FilterSource::Done(a), FilterSource::Done(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// The filter's matches over a growing list, scanned once per event: a live stream only
/// appends, so each call looks at the events that arrived since the last one.
#[derive(Default)]
pub struct EventFilter {
    needle: String,
    source: Option<FilterSource>,
    /// Stream index up to which events were scanned.
    scanned: u64,
    /// Stream indices of the matches, ascending.
    hits: Vec<u64>,
}

impl EventFilter {
    /// The positions in `events` that hold `needle` (lowercase), where `events[0]` is stream
    /// event `first`.
    pub fn matches(
        &mut self,
        needle: &str,
        source: FilterSource,
        events: &[SseEvent],
        first: u64,
    ) -> Vec<usize> {
        let end = first + events.len() as u64;
        let same = self.source.as_ref().is_some_and(|s| s.same(&source));
        if !same || self.needle != needle || self.scanned > end {
            *self = EventFilter {
                needle: needle.to_owned(),
                source: Some(source),
                scanned: first,
                hits: Vec::new(),
            };
        }
        let from = self.scanned.max(first);
        for ix in from..end {
            if event_matches(&events[(ix - first) as usize], needle) {
                self.hits.push(ix);
            }
        }
        self.scanned = end;
        // Matches that fell off the front of a live list go with their events.
        let gone = self.hits.partition_point(|&ix| ix < first);
        self.hits.drain(..gone);
        self.hits.iter().map(|&ix| (ix - first) as usize).collect()
    }
}

/// Characters kept of the data's first line in an event's row.
pub const PREVIEW_CHARS: usize = 240;

/// One row of the Events tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    /// The event name, `message` when the event has none.
    pub name: String,
    /// The event id, when it sent one.
    pub id: Option<String>,
    /// The reconnect delay in milliseconds, when the event sent one.
    pub retry: Option<u64>,
    /// The data's first line, cut short with an ellipsis.
    pub preview: String,
    /// When the event arrived after the response started; unknown for a kept response.
    pub time: Option<Duration>,
}

/// The first line of `data`, cut at `PREVIEW_CHARS`; an ellipsis marks that anything was
/// left out, a later line included.
pub fn preview(data: &str) -> String {
    let line = data.split('\n').next().unwrap_or_default();
    let mut text = match line.char_indices().nth(PREVIEW_CHARS) {
        Some((at, _)) => line[..at].to_owned(),
        None => line.to_owned(),
    };
    if text.len() < data.len() {
        text.push('\u{2026}');
    }
    text
}

/// The muted note for an event's reconnect delay; none when it sent no `retry`.
pub fn retry_label(retry: Option<u64>) -> Option<String> {
    retry.map(|ms| format!("retry {ms} ms"))
}

/// The rows for `events`, in order, with no arrival times.
pub fn event_rows(events: &[SseEvent]) -> Vec<EventRow> {
    events.iter().map(|e| event_row(e, None)).collect()
}

/// The row for one event that arrived at `time`.
pub fn event_row(e: &SseEvent, time: Option<Duration>) -> EventRow {
    EventRow {
        name: e
            .event
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "message".to_owned()),
        id: e.id.clone(),
        retry: e.retry,
        preview: preview(&e.data),
        time,
    }
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
