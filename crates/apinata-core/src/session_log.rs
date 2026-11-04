//! The session log: a bounded, redacted record of console lines and network entries.

mod cap;
mod capture;
mod entry;
mod hold;
mod redact;
mod retained;
#[cfg(test)]
mod test_support;

pub(crate) use hold::SendHold;
pub(crate) use retained::Logged;
pub(crate) use {capture::Exchange, redact::Redactor};

use crate::{ConsoleLevel, HttpVersion, ScriptPhase};
use cap::Cut;
use retained::Retained;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

/// How much the session log retains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LogLimits {
    /// The most console lines kept; at least one is always kept, so 0 acts as 1.
    pub console: usize,
    /// The most network entries kept; at least one is always kept, so 0 acts as 1.
    pub network: usize,
    /// The byte cap on each stored request or response body.
    pub body_bytes: usize,
    /// The accounted bytes both kinds may hold together.
    pub budget_bytes: usize,
}

impl Default for LogLimits {
    fn default() -> LogLimits {
        LogLimits {
            console: 1000,
            network: 500,
            body_bytes: 64 * 1024,
            budget_bytes: 16 * 1024 * 1024,
        }
    }
}

/// What made a network exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SendSource {
    /// A single request sent on its own.
    Request,
    /// A request sent by a collection or folder run.
    Runner,
    /// A request sent from a script.
    Script,
    /// An OAuth 2 token exchange.
    OAuth2,
    /// A GraphQL schema introspection.
    GraphqlSchema,
}

impl SendSource {
    /// The source's stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            SendSource::Request => "request",
            SendSource::Runner => "runner",
            SendSource::Script => "script",
            SendSource::OAuth2 => "oauth2",
            SendSource::GraphqlSchema => "graphql-schema",
        }
    }
}

/// One redacted, capped line a script wrote to the console.
#[derive(Clone)]
#[non_exhaustive]
pub struct ConsoleLine {
    /// The log-wide sequence number.
    pub seq: u64,
    /// When the line was collected.
    pub at: SystemTime,
    /// The request's display name.
    pub request: String,
    /// The script phase that wrote the line, or `None` for a message from the host app.
    pub phase: Option<ScriptPhase>,
    /// The console method called.
    pub level: ConsoleLevel,
    /// The line's text.
    pub text: String,
    // Where each capped field's content ends if it was cut, so re-redaction never
    // reads a marker back out of stored text.
    request_cut: Option<Cut>,
    text_cut: Option<Cut>,
}

/// One redacted, capped network exchange, from first hop to final response or failure.
#[derive(Clone)]
#[non_exhaustive]
pub struct NetworkEntry {
    /// The log-wide sequence number.
    pub seq: u64,
    /// When the send began.
    pub at: SystemTime,
    /// What made the exchange.
    pub source: SendSource,
    /// The request method.
    pub method: String,
    /// The URL as first sent.
    pub url: String,
    /// The last hop's status; `None` on failure.
    pub status: Option<u16>,
    /// The failure's text; `None` on success.
    pub error: Option<String>,
    /// The wall time of the whole send.
    pub duration: Duration,
    /// The request headers as resolved, before any hop.
    pub request_headers: Vec<(String, String)>,
    /// The bytes the first hop sent.
    pub request_body: LoggedBody,
    /// The last hop's decoded body.
    pub response_body: LoggedBody,
    /// Completed hops, redirects and digest retries included.
    pub hops: Vec<LoggedHop>,
    /// How many hops were dropped from the middle of `hops`.
    pub hops_omitted: usize,
    /// This send's number and the most sends the request could make, as in "attempt 2 of 3";
    /// `None` when the request was not retrying.
    pub attempt: Option<(u32, u32)>,
    method_cut: Option<Cut>,
    url_cut: Option<Cut>,
    error_cut: Option<Cut>,
    // How many headers the synthetic last pair of `request_headers` stands for; 0 if none.
    request_folded: usize,
}

/// One completed hop of a network exchange.
#[derive(Clone)]
#[non_exhaustive]
pub struct LoggedHop {
    /// The hop's method.
    pub method: String,
    /// The hop's URL.
    pub url: String,
    /// The hop's response status.
    pub status: u16,
    /// The headers the hop sent.
    pub request_headers: Vec<(String, String)>,
    /// The headers the hop received.
    pub response_headers: Vec<(String, String)>,
    /// The hop's round-trip time.
    pub elapsed: Duration,
    /// The PAC route or fallback cause, as the timeline's "PAC:" line; `None` outside PAC.
    pub pac: Option<String>,
    /// The host override the hop used, as `name -> address`; `None` when DNS answered.
    pub override_from: Option<String>,
    /// Where the hop's time went; `None` when it was not measured.
    pub timing: Option<crate::HopTiming>,
    version: Option<HttpVersion>,
    remote_addr: Option<std::net::SocketAddr>,
    via_proxy: bool,
    method_cut: Option<Cut>,
    url_cut: Option<Cut>,
    pac_cut: Option<Cut>,
    override_cut: Option<Cut>,
    request_folded: usize,
    response_folded: usize,
}

impl LoggedHop {
    /// The HTTP version the hop was answered over; `None` only for one outside [`HttpVersion`].
    pub fn version(&self) -> Option<HttpVersion> {
        self.version
    }

    /// The peer address the hop reached; through a proxy it is the proxy's.
    pub fn remote_addr(&self) -> Option<std::net::SocketAddr> {
        self.remote_addr
    }

    /// Whether the hop went through a proxy, so the address is the proxy's.
    pub fn via_proxy(&self) -> bool {
        self.via_proxy
    }

    /// The peer as shown to a person: `127.0.0.1:8080` or `proxy 127.0.0.1:3128`; `None` when
    /// unknown.
    pub fn peer_label(&self) -> Option<String> {
        let addr = self.remote_addr?;
        Some(if self.via_proxy {
            format!("proxy {addr}")
        } else {
            addr.to_string()
        })
    }
}

/// A request or response body as stored.
#[derive(Clone)]
#[non_exhaustive]
pub struct LoggedBody {
    /// Lossy UTF-8, capped, ending in the truncation marker when cut.
    pub text: String,
    /// The bytes actually sent or received.
    pub size: u64,
    /// Whether `text` was cut.
    pub truncated: bool,
    cut: Option<Cut>,
}

/// A shared, bounded log of console lines and network entries; clones share one ring.
#[derive(Clone)]
pub struct SessionLog {
    inner: Arc<Mutex<Inner>>,
}

/// The ring, its sequence counter and the limits it was made with, under one lock.
struct Inner {
    next: u64,
    limits: LogLimits,
    retained: Retained,
}

/// The entries of one kind from a sequence number on, with where to resume.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct LogSlice<T> {
    /// Entries of this kind whose seq is at least the asked one, oldest first.
    pub entries: Vec<Arc<T>>,
    /// The seq to pass back to get only newer entries.
    pub next: u64,
    /// The oldest seq of this kind still retained, or `next` when none is.
    pub first: u64,
}

impl Default for SessionLog {
    fn default() -> SessionLog {
        SessionLog::new()
    }
}

impl SessionLog {
    /// An empty log under the default limits.
    pub fn new() -> SessionLog {
        SessionLog::with_limits(LogLimits::default())
    }

    /// An empty log under `limits`.
    pub fn with_limits(limits: LogLimits) -> SessionLog {
        SessionLog {
            inner: Arc::new(Mutex::new(Inner {
                next: 0,
                limits,
                retained: Retained::new(limits),
            })),
        }
    }

    /// The limits the log was made with.
    pub(crate) fn limits(&self) -> LogLimits {
        self.lock().limits
    }

    /// Console lines with seq at least `seq`, oldest first.
    pub fn console_since(&self, seq: u64) -> LogSlice<ConsoleLine> {
        let inner = self.lock();
        LogSlice {
            entries: inner.retained.console_from(seq),
            next: inner.next,
            first: inner.retained.first_console().unwrap_or(inner.next),
        }
    }

    /// Network entries with seq at least `seq`, oldest first.
    pub fn network_since(&self, seq: u64) -> LogSlice<NetworkEntry> {
        let inner = self.lock();
        LogSlice {
            entries: inner.retained.network_from(seq),
            next: inner.next,
            first: inner.retained.first_network().unwrap_or(inner.next),
        }
    }

    /// Drops every retained console line.
    pub fn clear_console(&self) {
        self.lock().retained.clear_console();
    }

    /// Drops every retained network entry.
    pub fn clear_network(&self) {
        self.lock().retained.clear_network();
    }

    /// The accounted bytes both kinds currently hold.
    pub fn retained_bytes(&self) -> usize {
        self.lock().retained.bytes()
    }

    /// Records a message from the host app, not a script, under the name `source`. The line
    /// carries no script phase, and its text is capped like any console line. Returns the seq given.
    pub fn note(&self, source: &str, level: ConsoleLevel, text: &str) -> u64 {
        let line = ConsoleLine::capture(
            SystemTime::now(),
            source,
            None,
            level,
            text,
            &Redactor::none(),
        );
        self.push(Logged::Console(Arc::new(line)))
    }

    /// Records a console line, overwriting its seq, and returns the seq given.
    #[cfg(test)]
    pub(crate) fn push_console(&self, line: ConsoleLine) -> u64 {
        self.push(Logged::Console(Arc::new(line)))
    }

    /// Records a network entry, overwriting its seq, and returns the seq given.
    pub(crate) fn push_network(&self, entry: NetworkEntry) -> u64 {
        self.push(Logged::Network(Arc::new(entry)))
    }

    /// Records a staged item under the next seq. The seq is assigned and the item retained
    /// under one lock, so a reader never sees a gap that fills in later.
    pub(crate) fn push(&self, item: Logged) -> u64 {
        let mut inner = self.lock();
        let seq = inner.next;
        let item = match item {
            Logged::Console(l) => {
                let mut l = Arc::unwrap_or_clone(l);
                l.seq = seq;
                Logged::Console(Arc::new(l))
            }
            Logged::Network(e) => {
                let mut e = Arc::unwrap_or_clone(e);
                e.seq = seq;
                Logged::Network(Arc::new(e))
            }
        };
        inner.next += 1;
        inner.retained.push(seq, item);
        seq
    }

    /// A panic elsewhere never leaves the ring half-updated, so a poisoned lock is safe to reuse.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::redact::Redactor;
    use super::retained::{Logged, Retained};
    use super::test_support::{entry_with, line_with};
    use super::*;

    fn line(text: &str) -> ConsoleLine {
        line_with(&Redactor::none(), text)
    }
    fn entry() -> NetworkEntry {
        entry_with(&Redactor::none(), "v", b"body", 64 * 1024)
    }
    fn seqs<T>(slice: &LogSlice<T>, seq: impl Fn(&T) -> u64) -> Vec<u64> {
        slice.entries.iter().map(|e| seq(e.as_ref())).collect()
    }

    #[test]
    fn a_note_lands_in_the_console_with_its_source() {
        let log = SessionLog::new();
        log.note("Themes", ConsoleLevel::Warn, "skipped a.json");
        let lines = log.console_since(0).entries;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].request, "Themes");
        assert_eq!(lines[0].text, "skipped a.json");
        assert_eq!(lines[0].level, ConsoleLevel::Warn);
    }

    #[test]
    fn a_log_reports_the_limits_it_was_made_with() {
        let limits = LogLimits {
            body_bytes: 100,
            ..LogLimits::default()
        };
        assert_eq!(SessionLog::with_limits(limits).limits(), limits);
        assert_eq!(SessionLog::new().limits(), LogLimits::default());
    }

    #[test]
    fn a_fresh_log_is_empty_at_zero() {
        let log = SessionLog::new();
        let s = log.console_since(0);
        assert!(s.entries.is_empty());
        assert_eq!((s.next, s.first), (0, 0));
        assert_eq!(log.retained_bytes(), 0);
    }

    #[test]
    fn both_kinds_share_one_increasing_seq() {
        let log = SessionLog::default();
        assert_eq!(log.push_console(line("a")), 0);
        assert_eq!(log.push_network(entry()), 1);
        assert_eq!(log.push_console(line("b")), 2);
        let c = log.console_since(0);
        assert_eq!((seqs(&c, |l| l.seq), c.next, c.first), (vec![0, 2], 3, 0));
        let n = log.network_since(0);
        assert_eq!((seqs(&n, |e| e.seq), n.first), (vec![1], 1));
        assert_eq!(seqs(&log.console_since(2), |l| l.seq), [2]);
        let none = log.console_since(3);
        assert!(none.entries.is_empty());
        assert_eq!(none.next, 3);
    }

    #[test]
    fn count_caps_keep_the_newest() {
        let log = SessionLog::new();
        for i in 0..1200 {
            log.push_console(line(&i.to_string()));
        }
        let c = log.console_since(0);
        assert_eq!((c.entries.len(), c.first), (1000, 200));
        for _ in 0..600 {
            log.push_network(entry());
        }
        assert_eq!(log.network_since(0).entries.len(), 500);
    }

    #[test]
    fn clear_moves_first_to_next() {
        let log = SessionLog::new();
        for _ in 0..3 {
            log.push_console(line("x"));
        }
        log.push_network(entry());
        log.clear_console();
        let c = log.console_since(0);
        assert!(c.entries.is_empty());
        assert_eq!((c.first, c.next), (4, 4));
        assert_eq!(log.network_since(0).entries.len(), 1);
        log.clear_network();
        assert_eq!(log.retained_bytes(), 0);
    }

    #[test]
    fn the_budget_holds_after_every_publish() {
        let limits = LogLimits {
            budget_bytes: 1024 * 1024,
            ..LogLimits::default()
        };
        let log = SessionLog::with_limits(limits);
        let body = vec![b'b'; 200 * 1024];
        let mut pushed = 0;
        while pushed < 3 * limits.budget_bytes {
            let e = entry_with(&Redactor::none(), "v", &body, limits.body_bytes);
            pushed += e.accounted_bytes();
            let last = log.push_network(e);
            log.push_console(line("between"));
            assert!(log.retained_bytes() <= limits.budget_bytes);
            let c = log.console_since(0);
            let n = log.network_since(0);
            let recount: usize = c.entries.iter().map(|l| l.accounted_bytes()).sum::<usize>()
                + n.entries.iter().map(|e| e.accounted_bytes()).sum::<usize>();
            assert_eq!(log.retained_bytes(), recount);
            let mut all: Vec<u64> = seqs(&c, |l| l.seq);
            all.extend(seqs(&n, |e| e.seq));
            all.sort();
            assert_eq!(all, (all[0]..=last + 1).collect::<Vec<_>>());
        }
        assert!(log.network_since(0).first > 0);
    }

    #[test]
    fn a_twenty_mib_console_line_fits_the_budget() {
        let log = SessionLog::new();
        log.push_console(line(&"y".repeat(20 * 1024 * 1024)));
        let c = log.console_since(0);
        assert!(c.entries[0].text.len() < 16 * 1024 + 64);
        assert!(log.retained_bytes() <= LogLimits::default().budget_bytes);
    }

    #[test]
    fn a_held_send_publishing_late_is_seen_exactly_once() {
        let log = SessionLog::new();
        let mut hold = Retained::new(LogLimits::default());
        let mut seen = Vec::new();
        let mut next = 0;
        let mut poll = |log: &SessionLog, seen: &mut Vec<u64>| {
            let c = log.console_since(next);
            let n = log.network_since(next);
            seen.extend(seqs(&c, |l| l.seq));
            seen.extend(seqs(&n, |e| e.seq));
            next = c.next;
        };
        hold.push(0, Logged::Console(Arc::new(line("held a"))));
        log.push_network(entry());
        poll(&log, &mut seen);
        hold.push(1, Logged::Network(Arc::new(entry())));
        log.push_console(line("direct"));
        poll(&log, &mut seen);
        for item in hold.into_ordered() {
            log.push(item);
        }
        poll(&log, &mut seen);
        poll(&log, &mut seen);
        assert_eq!(seen, [0, 1, 2, 3]);
    }

    #[test]
    fn clones_share_one_ring_and_a_poisoned_lock_recovers() {
        let log = SessionLog::new();
        let other = log.clone();
        other.push_console(line("shared"));
        assert_eq!(log.console_since(0).entries.len(), 1);
        let poisoner = log.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.inner.lock().unwrap();
            panic!("poison the log");
        })
        .join();
        assert_eq!(log.push_console(line("after")), 1);
        assert_eq!(log.console_since(0).entries.len(), 2);
    }
}
