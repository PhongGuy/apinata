//! The busy rule: a collection whose actor holds pending commands without acknowledging
//! any of them for `BUSY_AFTER` is busy.

use std::time::{Duration, Instant};

/// How long pending commands may go unacknowledged before the collection is busy.
pub const BUSY_AFTER: Duration = Duration::from_millis(500);

/// The verdict of one observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyState {
    /// Nothing is pending.
    Idle,
    /// Commands are pending; the collection turns busy at `until` unless the actor progresses.
    Waiting {
        /// When to look again.
        until: Instant,
    },
    /// Commands have been pending without progress for `BUSY_AFTER`.
    Busy,
}

/// Remembers since when the actor has made no progress on its pending commands.
#[derive(Debug, Default)]
pub struct BusyTracker {
    since: Option<(u64, Instant)>,
}

impl BusyTracker {
    /// Judges the collection from its `pending` count and its snapshot's `acked`, at `now`.
    pub fn observe(&mut self, pending: u64, acked: u64, now: Instant) -> BusyState {
        if pending == 0 {
            self.since = None;
            return BusyState::Idle;
        }
        // The clock restarts whenever the queue was empty or the actor acknowledged more.
        let since = match self.since {
            Some((seen, since)) if seen == acked => since,
            _ => {
                self.since = Some((acked, now));
                now
            }
        };
        if now.saturating_duration_since(since) >= BUSY_AFTER {
            BusyState::Busy
        } else {
            BusyState::Waiting {
                until: since + BUSY_AFTER,
            }
        }
    }
}
