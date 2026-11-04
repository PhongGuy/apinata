//! Which targets a reload replaced or a rename moved, and when: the ledger the actor consults
//! to hold back edits typed against replaced content and to redirect commands to moved items.

use std::collections::HashMap;
use std::mem;

use apinata_core::ItemPath;

use crate::command::Target;

/// The most reload records the ledger keeps, items and subtrees together.
pub const MAX_RELOADS: usize = 256;

/// What a reload replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reloaded {
    /// One document.
    Item(Target),
    /// Every request and folder at or under this directory.
    Subtree(ItemPath),
}

/// The sequence of the first snapshot showing each reload and each move.
#[derive(Debug, Default)]
pub struct StaleLedger {
    items: HashMap<Target, u64>,
    subtrees: HashMap<ItemPath, u64>,
    /// The latest evicted record's snapshot: every edit built before it is held, whatever
    /// its target, so eviction never lets through what a dropped record would have held.
    floor: u64,
    /// In the order they happened: `(from, to, at)`.
    moves: Vec<(Target, Target, u64)>,
}

impl StaleLedger {
    /// An empty ledger.
    pub fn new() -> StaleLedger {
        StaleLedger::default()
    }

    /// How many reload records (items and subtrees) the ledger holds.
    pub fn len(&self) -> usize {
        self.items.len() + self.subtrees.len()
    }

    /// Whether the ledger holds no reload records.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Note that `what` was reloaded, first shown by snapshot `at`; keeps the latest `at`.
    /// Past [`MAX_RELOADS`] records the oldest are evicted into the floor.
    pub fn record(&mut self, what: Reloaded, at: u64) {
        let slot = match what {
            Reloaded::Item(target) => self.items.entry(target).or_default(),
            Reloaded::Subtree(dir) => self.subtrees.entry(dir).or_default(),
        };
        *slot = (*slot).max(at);
        while self.len() > MAX_RELOADS {
            self.evict_oldest();
        }
    }

    // Drop the record with the smallest snapshot and raise the floor to it, which keeps
    // holding every edit that record held (and, rarely, an unrelated edit just as old).
    fn evict_oldest(&mut self) {
        let at = match (oldest(&self.items), oldest(&self.subtrees)) {
            (Some((_, i)), Some((dir, s))) if s < i => {
                self.subtrees.remove(&dir);
                s
            }
            (Some((target, i)), _) => {
                self.items.remove(&target);
                i
            }
            (None, Some((dir, s))) => {
                self.subtrees.remove(&dir);
                s
            }
            (None, None) => return,
        };
        self.floor = self.floor.max(at);
    }

    /// The latest reload covering `target`: its own record or any enclosing directory's.
    pub fn reloaded_at(&self, target: &Target) -> Option<u64> {
        let own = self.items.get(target).copied();
        let path = match target {
            Target::Request(p) | Target::Folder(p) => Some(p),
            Target::Collection | Target::Environment(_) => None,
        };
        let enclosing = path.into_iter().flat_map(|p| {
            self.subtrees
                .iter()
                .filter(|(dir, _)| is_within(p, dir))
                .map(|(_, at)| *at)
        });
        own.into_iter().chain(enclosing).max()
    }

    /// Whether an edit stamped with `seen` predates a reload of `target`, or predates the
    /// floor that evicted records left.
    pub fn is_stale(&self, target: &Target, seen: u64) -> bool {
        seen < self.floor || self.reloaded_at(target).is_some_and(|at| at > seen)
    }

    /// Note that `from` moved to `to`, first shown by snapshot `at`; reload records keyed at or
    /// under `from` move with it, so an edit remapped onto the new path is still held.
    pub fn record_move(&mut self, from: Target, to: Target, at: u64) {
        let items = mem::take(&mut self.items);
        for (target, when) in items {
            let key = moved(&target, &from, &to).unwrap_or(target);
            let slot = self.items.entry(key).or_default();
            *slot = (*slot).max(when);
        }
        if let (Target::Folder(f), Target::Folder(t)) = (&from, &to) {
            let subtrees = mem::take(&mut self.subtrees);
            for (dir, when) in subtrees {
                let key = rebase(&dir, f, t).unwrap_or(dir);
                let slot = self.subtrees.entry(key).or_default();
                *slot = (*slot).max(when);
            }
        }
        self.moves.push((from, to, at));
    }

    /// Where a command stamped with `seen` should address `target`: every move it had not
    /// seen applied in order, so chained moves compose.
    pub fn remap(&self, target: &Target, seen: u64) -> Target {
        let unseen = self.moves.iter().filter(|(_, _, at)| seen < *at);
        unseen.fold(target.clone(), |t, (from, to, _)| {
            moved(&t, from, to).unwrap_or(t)
        })
    }

    /// Forget the moves that every command stamped `seen` or later has seen. Reload records
    /// stay (an edit's basis may lag its `seen`); only the cap evicts them, into the floor.
    pub fn prune(&mut self, seen: u64) {
        self.moves.retain(|(_, _, at)| *at > seen);
    }
}

/// Where `target` lands when `from` moves to `to`; `None` when the move does not touch it. A
/// folder move carries every request and folder under it.
pub(crate) fn moved(target: &Target, from: &Target, to: &Target) -> Option<Target> {
    match (from, to, target) {
        (Target::Folder(f), Target::Folder(t), Target::Request(p)) => {
            rebase(p, f, t).map(Target::Request)
        }
        (Target::Folder(f), Target::Folder(t), Target::Folder(p)) => {
            rebase(p, f, t).map(Target::Folder)
        }
        (Target::Request(_), Target::Request(_), Target::Request(_))
        | (Target::Environment(_), Target::Environment(_), Target::Environment(_))
            if target == from =>
        {
            Some(to.clone())
        }
        _ => None,
    }
}

/// The record with the smallest snapshot in `records`.
fn oldest<K: Clone>(records: &HashMap<K, u64>) -> Option<(K, u64)> {
    let (key, at) = records.iter().min_by_key(|(_, at)| **at)?;
    Some((key.clone(), *at))
}

/// `path` with its `from` prefix replaced by `to`, when `path` is `from` or under it.
pub fn rebase(path: &ItemPath, from: &ItemPath, to: &ItemPath) -> Option<ItemPath> {
    if !is_within(path, from) {
        return None;
    }
    let rest = &path.as_str()[from.as_str().len()..];
    ItemPath::new(format!("{}{rest}", to.as_str()).trim_start_matches('/'))
}

/// Whether `path` is `dir` or below it, by whole segments.
pub(crate) fn is_within(path: &ItemPath, dir: &ItemPath) -> bool {
    let (p, d) = (path.as_str(), dir.as_str());
    p == d || p.strip_prefix(d).is_some_and(|rest| rest.starts_with('/'))
}
