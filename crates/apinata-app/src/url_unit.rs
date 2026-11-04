//! The lock that keeps the URL and its params from overwriting each other's edits.

/// One of the two views of the URL's query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The URL text.
    Url,
    /// The params table.
    Params,
}

/// Remembers which side sent the last edit, so the other side stays read-only until the
/// actor has re-derived it; an edit from both at once would let one whole value replace
/// the other's unseen change.
#[derive(Debug, Default)]
pub struct UrlParamsUnit {
    last: Option<Side>,
}

impl UrlParamsUnit {
    /// Whether `side` is read-only: an edit is in flight and it was sent from the other side.
    pub fn locked(&self, side: Side, in_flight: bool) -> bool {
        in_flight && self.last.is_some_and(|last| last != side)
    }

    /// Asks to send an edit from `side`: `false` refuses it while that side is locked,
    /// `true` records `side` as the sender and the caller sends.
    pub fn edit(&mut self, side: Side, in_flight: bool) -> bool {
        if self.locked(side, in_flight) {
            return false;
        }
        self.last = Some(side);
        true
    }
}
