//! The app-wide session log: one ring every executor the app builds records into, so the
//! console dock reads the same log across executor reinstalls. Also the dock's per-kind
//! feeds and its pure filters and labels.

use std::sync::Arc;
use std::time::SystemTime;

use apinata_core::{ConsoleLevel, ConsoleLine, NetworkEntry, SendSource, SessionLog};
use gpui_kit::{App, Global};

/// The global holding the app's one session log.
pub struct AppLog(pub SessionLog);

impl Global for AppLog {}

/// The app's session log, created on first call; every later call returns a clone of it.
pub fn ensure(cx: &mut App) -> SessionLog {
    if let Some(log) = shared(cx) {
        return log;
    }
    let log = SessionLog::new();
    cx.set_global(AppLog(log.clone()));
    log
}

/// The app's session log, or `None` before `ensure`.
pub fn shared(cx: &App) -> Option<SessionLog> {
    cx.try_global::<AppLog>().map(|g| g.0.clone())
}

/// An entry with a log-wide sequence number.
pub trait Sequenced {
    /// The entry's log-wide sequence number.
    fn seq(&self) -> u64;
}

impl Sequenced for ConsoleLine {
    fn seq(&self) -> u64 {
        self.seq
    }
}

impl Sequenced for NetworkEntry {
    fn seq(&self) -> u64 {
        self.seq
    }
}

/// One kind's entries as a reader holds them, with its own cursor.
pub struct Feed<T> {
    entries: Vec<Arc<T>>,
    next: u64,
    first: u64,
}

impl<T> Default for Feed<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            next: 0,
            first: 0,
        }
    }
}

impl<T: Sequenced> Feed<T> {
    /// The sequence number the next read starts from.
    pub fn next(&self) -> u64 {
        self.next
    }

    /// The held entries, oldest first.
    pub fn entries(&self) -> &[Arc<T>] {
        &self.entries
    }

    /// Appends `entries` with seq >= the feed's `next`, drops held entries below `first`,
    /// stores `next` and `first`, and returns whether `next` or `first` moved.
    pub fn absorb(&mut self, entries: Vec<Arc<T>>, next: u64, first: u64) -> bool {
        let fresh = self.next;
        self.entries
            .extend(entries.into_iter().filter(|e| e.seq() >= fresh));
        // Eviction and clear both surface as a raised `first`; anything below it is gone.
        self.entries.retain(|e| e.seq() >= first);
        let moved = next != self.next || first != self.first;
        self.next = next;
        self.first = first;
        moved
    }
}

/// Whether a console line passes the level filter and the case-insensitive search, which
/// matches the text or the request name; a blank search keeps every line.
pub fn console_keeps(
    level: ConsoleLevel,
    request: &str,
    text: &str,
    filter: Option<ConsoleLevel>,
    search: &str,
) -> bool {
    if filter.is_some_and(|f| f != level) {
        return false;
    }
    let query = search.trim().to_lowercase();
    query.is_empty()
        || text.to_lowercase().contains(&query)
        || request.to_lowercase().contains(&query)
}

/// Whether a network entry passes the source filter (`None` keeps all).
pub fn network_keeps(source: SendSource, filter: Option<SendSource>) -> bool {
    filter.is_none_or(|f| f == source)
}

/// The source filter's segments, in order: All, then each source.
pub const SOURCE_FILTERS: [Option<SendSource>; 6] = [
    None,
    Some(SendSource::Request),
    Some(SendSource::Runner),
    Some(SendSource::Script),
    Some(SendSource::OAuth2),
    Some(SendSource::GraphqlSchema),
];

/// The segment text of a source filter.
pub fn source_label(filter: Option<SendSource>) -> &'static str {
    match filter {
        None => "All",
        Some(SendSource::Request) => "Request",
        Some(SendSource::Runner) => "Runner",
        Some(SendSource::Script) => "Script",
        Some(SendSource::OAuth2) => "OAuth2",
        Some(SendSource::GraphqlSchema) => "GraphQL schema",
        Some(_) => "Other",
    }
}

/// The status column: the status code, or "error" when there is none.
pub fn status_label(status: Option<u16>) -> String {
    status.map_or_else(|| "error".to_string(), |s| s.to_string())
}

/// The note over a list that leaves out its `n` oldest items, e.g. "1 earlier line not shown".
pub fn omitted_label(n: usize, noun: &str) -> String {
    let plural = if n == 1 { "" } else { "s" };
    format!("{n} earlier {noun}{plural} not shown")
}

/// Local wall-clock time as "HH:MM:SS.mmm".
pub fn clock_label(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at)
        .format("%H:%M:%S%.3f")
        .to_string()
}
