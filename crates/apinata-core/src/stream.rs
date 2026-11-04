//! Types for reading a response body incrementally: limits, a chunk sink and how a stream ended.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Bounds that end a stream on their own; each is off until set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StreamLimits {
    pub(crate) max_duration: Option<Duration>,
    pub(crate) max_events: Option<u64>,
    pub(crate) max_bytes: Option<u64>,
}

impl StreamLimits {
    /// No limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ends the stream this long after the final response's headers arrive.
    pub fn max_duration(mut self, d: Option<Duration>) -> Self {
        self.max_duration = d;
        self
    }

    /// Ends the stream once this many events have been received.
    pub fn max_events(mut self, n: Option<u64>) -> Self {
        self.max_events = n;
        self
    }

    /// Ends the stream once this many body bytes have been received.
    pub fn max_bytes(mut self, n: Option<u64>) -> Self {
        self.max_bytes = n;
        self
    }
}

/// What a sink is told while a streamed response arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamEvent {
    /// The final response's headers arrived.
    Started {
        /// The status code.
        status: u16,
        /// The response headers, lowercase names.
        headers: Vec<(String, String)>,
        /// Whether the body is a Server-Sent Events stream.
        event_stream: bool,
    },
    /// The next piece of the body. With a body file set it holds only the bytes kept in memory.
    Chunk(Vec<u8>),
    /// How much of the body has arrived; sent per chunk while a body file is being written.
    Progress {
        /// Decoded body bytes received so far, kept or not.
        received: u64,
        /// The body's length when the response states it and is not compressed.
        total: Option<u64>,
    },
}

/// Which bound ended a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamBound {
    /// The duration limit.
    Time,
    /// The event-count limit.
    Events,
}

/// How a streamed body ended.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamEnd {
    /// The server finished the body.
    Closed,
    /// The user pressed Stop.
    Stopped,
    /// A limit was reached.
    Bound(StreamBound),
    /// No data arrived within the request timeout.
    IdleTimeout,
    /// The body reached the size cap.
    BodyCap,
    /// The connection failed mid-body.
    Error(String),
}

impl StreamEnd {
    /// A stable lowercase name for reports and tool results.
    pub fn label(&self) -> &'static str {
        match self {
            StreamEnd::Closed => "closed",
            StreamEnd::Stopped => "stopped",
            StreamEnd::Bound(StreamBound::Time) => "time_limit",
            StreamEnd::Bound(StreamBound::Events) => "event_limit",
            StreamEnd::IdleTimeout => "idle_timeout",
            StreamEnd::BodyCap => "body_cap",
            StreamEnd::Error(_) => "error",
        }
    }
}

/// The type behind a sink: called for each `StreamEvent` as it happens.
pub(crate) type Sink = Arc<dyn Fn(StreamEvent) + Send + Sync>;

/// Turns streaming on for one send: its limits and an optional sink for live events.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct StreamOptions {
    pub(crate) limits: StreamLimits,
    pub(crate) sink: Option<Sink>,
    pub(crate) body_file: Option<PathBuf>,
}

impl StreamOptions {
    /// Streaming with no limits and no sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the limits.
    pub fn limits(mut self, l: StreamLimits) -> Self {
        self.limits = l;
        self
    }

    /// Sets the sink called for each event as the response arrives.
    pub fn sink(mut self, f: impl Fn(StreamEvent) + Send + Sync + 'static) -> Self {
        self.sink = Some(Arc::new(f));
        self
    }

    /// Writes the final hop's whole decoded body to `path`, however large, while the response
    /// keeps only the first `max_body_bytes`. The file appears only when the body ends well.
    pub fn body_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.body_file = Some(path.into());
        self
    }
}

impl fmt::Debug for StreamOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamOptions")
            .field("limits", &self.limits)
            .field("sink", &self.sink.is_some())
            .field("body_file", &self.body_file)
            .finish()
    }
}
