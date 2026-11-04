//! The send hold: one top-level send's log items, staged and published together.

use super::SessionLog;
use super::redact::Redactor;
use super::retained::{Logged, Retained};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The most distinct values a hold learns; past it, what the hold publishes is withheld.
pub(crate) const LEARN_CAP: usize = 1024;

/// The whole text of a console line whose hold learned more values than it can redact with.
pub(crate) const WITHHELD: &str = "[withheld: too many secrets to redact]";

/// One top-level send's staging area; clones share it.
#[derive(Clone)]
pub(crate) struct SendHold {
    inner: Arc<Mutex<Staging>>,
}

/// Publishes the hold when dropped: at the send's end, on a cancel or in a panic.
pub(crate) struct HoldGuard(SendHold);

/// The staged items under the ring's own caps (`None` once published), their staging order,
/// the learned values every item is re-redacted with at publish, and whether more values
/// arrived than the cap allows.
struct Staging {
    log: SessionLog,
    staged: Option<Retained>,
    order: u64,
    learned: HashSet<String>,
    overflowed: bool,
}

/// What publishing needs from the staging area, taken out under its lock.
type Learned = (SessionLog, Vec<String>, bool);

impl SendHold {
    pub(crate) fn open(log: SessionLog) -> (SendHold, HoldGuard) {
        let staged = Some(Retained::new(log.limits()));
        let hold = SendHold {
            inner: Arc::new(Mutex::new(Staging {
                log,
                staged,
                order: 0,
                learned: HashSet::new(),
                overflowed: false,
            })),
        };
        (hold.clone(), HoldGuard(hold))
    }

    /// Stages under the log's caps and budget (oldest staged evicted first); after publish,
    /// pushes straight to the log, re-redacted with what was learned.
    pub(crate) fn stage(&self, item: Logged) {
        let mut staging = self.lock();
        let order = staging.order;
        if let Some(staged) = staging.staged.as_mut() {
            staged.push(order, item);
            staging.order += 1;
            return;
        }
        // The ring's lock is never taken while the hold's is held.
        let learned = staging.learned();
        drop(staging);
        publish_into(learned, vec![item]);
    }

    /// Values every staged item is re-redacted with at publish. Past [`LEARN_CAP`] distinct
    /// values a new one is not kept, and the hold withholds what it publishes instead.
    pub(crate) fn learn(&self, values: &[String]) {
        let mut staging = self.lock();
        for value in values {
            if value.is_empty() || staging.learned.contains(value) {
                continue;
            }
            if staging.learned.len() < LEARN_CAP {
                staging.learned.insert(value.clone());
            } else {
                staging.overflowed = true;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn learned_len(&self) -> usize {
        self.lock().learned.len()
    }

    #[cfg(test)]
    pub(crate) fn staged_bytes(&self) -> usize {
        self.lock().staged.as_ref().map_or(0, Retained::bytes)
    }

    /// Takes the staged items under the hold's lock, then re-redacts and pushes them outside it,
    /// so seqs are assigned only now and in staging order. Later calls find nothing staged.
    fn publish(&self) {
        let mut staging = self.lock();
        let Some(staged) = staging.staged.take() else {
            return;
        };
        let learned = staging.learned();
        drop(staging);
        publish_into(learned, staged.into_ordered());
    }

    /// A panic while staging never leaves the hold half-updated, so a poisoned lock is safe to reuse.
    fn lock(&self) -> MutexGuard<'_, Staging> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Staging {
    fn learned(&self) -> Learned {
        let values = self.learned.iter().cloned().collect();
        (self.log.clone(), values, self.overflowed)
    }
}

impl Drop for HoldGuard {
    fn drop(&mut self) {
        self.0.publish();
    }
}

/// Re-redacts each item with the learned values (taken out of its `Arc`, never changed in
/// place) and pushes it, which re-measures it against the ring's budget. After an overflow an
/// item may hold a value that was not kept, so every field it could echo one into is withheld.
fn publish_into((log, learned, overflowed): Learned, items: Vec<Logged>) {
    let redactor = (!learned.is_empty()).then(|| Redactor::new(&learned));
    let body_cap = log.limits().body_bytes;
    for item in items {
        let item = match (&redactor, item) {
            (None, item) => item,
            (Some(r), Logged::Console(line)) => {
                let mut line = Arc::unwrap_or_clone(line);
                line.rededact(r);
                if overflowed {
                    line.withhold(WITHHELD);
                }
                Logged::Console(Arc::new(line))
            }
            (Some(r), Logged::Network(entry)) => {
                let mut entry = Arc::unwrap_or_clone(entry);
                entry.rededact(r, body_cap);
                if overflowed {
                    entry.withhold(WITHHELD);
                }
                Logged::Network(Arc::new(entry))
            }
        };
        log.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_log::test_support::{entry_texts, entry_with, line_with};
    use crate::session_log::{LogLimits, redact::Redactor};
    use std::sync::Arc;

    const LATE: &str = "LATE-value-0042";

    fn line(text: &str) -> Logged {
        Logged::Console(Arc::new(line_with(&Redactor::none(), text)))
    }
    fn entry(value: &str, body: &[u8]) -> Logged {
        Logged::Network(Arc::new(entry_with(
            &Redactor::none(),
            value,
            body,
            64 * 1024,
        )))
    }

    #[test]
    fn nothing_is_published_until_the_guard_drops_then_all_in_staging_order() {
        let log = SessionLog::new();
        let (hold, guard) = SendHold::open(log.clone());
        hold.stage(line("a"));
        hold.clone().stage(entry("v", b"x"));
        hold.stage(line("b"));
        assert!(log.console_since(0).entries.is_empty() && log.network_since(0).entries.is_empty());
        drop(guard);
        let c = log.console_since(0);
        assert_eq!(
            c.entries
                .iter()
                .map(|l| (l.seq, l.text.as_str()))
                .collect::<Vec<_>>(),
            [(0, "a"), (2, "b")]
        );
        assert_eq!(log.network_since(0).entries[0].seq, 1);
    }

    #[test]
    fn a_learned_value_is_removed_from_every_field_even_across_the_body_cut() {
        let log = SessionLog::new();
        let (hold, guard) = SendHold::open(log.clone());
        let mut body = vec![b'a'; 64 * 1024 - 5];
        body.extend_from_slice(LATE.as_bytes());
        hold.stage(entry(LATE, &body));
        hold.stage(line(&format!("saw {LATE}")));
        hold.learn(&[LATE.to_owned()]);
        drop(guard);
        let e = &log.network_since(0).entries[0];
        for text in entry_texts(e) {
            assert!(
                !text
                    .to_ascii_uppercase()
                    .contains(&LATE.to_ascii_uppercase()),
                "{text}"
            );
        }
        let kept = e
            .response_body
            .text
            .split("\n[truncated:")
            .next()
            .unwrap_or_default();
        for k in 1..LATE.len() {
            assert!(!kept.ends_with(&LATE[..k]), "ends in {}", &LATE[..k]);
        }
        assert_eq!(log.console_since(0).entries[0].text, "saw [redacted]");
    }

    #[test]
    fn staging_keeps_the_budget_and_count_caps() {
        let limits = LogLimits {
            budget_bytes: 1024 * 1024,
            ..LogLimits::default()
        };
        let log = SessionLog::with_limits(limits);
        let (hold, guard) = SendHold::open(log.clone());
        let body = vec![b'b'; 200 * 1024];
        for _ in 0..40 {
            hold.stage(entry("v", &body));
            assert!(hold.staged_bytes() <= limits.budget_bytes);
        }
        drop(guard);
        assert!(log.retained_bytes() <= limits.budget_bytes);
        let log = SessionLog::new();
        let (hold, guard) = SendHold::open(log.clone());
        for _ in 0..600 {
            hold.stage(entry("v", b"x"));
        }
        drop(guard);
        let n = log.network_since(0);
        assert_eq!((n.entries.len(), n.entries[0].seq, n.next), (500, 0, 500));
    }

    #[test]
    fn a_stage_after_publish_goes_straight_to_the_log() {
        let log = SessionLog::new();
        let (hold, guard) = SendHold::open(log.clone());
        drop(guard);
        assert_eq!(hold.staged_bytes(), 0);
        hold.stage(line("late"));
        assert_eq!(log.console_since(0).entries.len(), 1);
    }

    #[test]
    fn a_stage_after_publish_is_redacted_with_what_was_learned() {
        let log = SessionLog::new();
        let (hold, guard) = SendHold::open(log.clone());
        hold.learn(&[LATE.to_owned()]);
        drop(guard);
        hold.stage(line(&format!("late {LATE}")));
        assert_eq!(log.console_since(0).entries[0].text, "late [redacted]");
    }

    #[test]
    fn learned_values_stay_within_the_cap_however_many_sends_learn() {
        let (hold, _guard) = SendHold::open(SessionLog::new());
        // Each simulated send places a fresh signature next to one static secret.
        for i in 0..3 * LEARN_CAP {
            hold.learn(&[format!("sig-{i:06}"), "static-secret".to_owned()]);
            assert!(hold.learned_len() <= LEARN_CAP);
        }
        assert_eq!(hold.learned_len(), LEARN_CAP);
    }

    #[test]
    fn past_the_cap_console_lines_are_withheld_and_no_learned_value_shows() {
        let log = SessionLog::new();
        let (hold, guard) = SendHold::open(log.clone());
        let values: Vec<String> = (0..LEARN_CAP + 5)
            .map(|i| format!("placed-{i:06}"))
            .collect();
        hold.stage(line(&format!("saw {}", values[0])));
        hold.stage(entry(&values[1], values[2].as_bytes()));
        hold.learn(&values);
        drop(guard);
        hold.stage(line(&format!("late {}", values[3])));
        let lines = log.console_since(0).entries;
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.text == WITHHELD), "{lines:?}");
        let e = &log.network_since(0).entries[0];
        for text in entry_texts(e) {
            for value in &values[..LEARN_CAP] {
                assert!(!text.contains(value.as_str()), "{text}");
            }
        }
    }

    #[test]
    fn a_saturated_hold_withholds_entry_values_it_could_not_learn() {
        let log = SessionLog::new();
        let (hold, guard) = SendHold::open(log.clone());
        let values: Vec<String> = (0..=LEARN_CAP).map(|i| format!("read-{i:06}")).collect();
        hold.learn(&values[..LEARN_CAP]);
        // Learned after the cap, so no redactor holds it; only withholding can hide it.
        let late = &values[LEARN_CAP];
        hold.learn(std::slice::from_ref(late));
        hold.stage(entry(late, format!("echo {late}").as_bytes()));
        drop(guard);
        hold.stage(entry(late, late.as_bytes()));
        let all = log.network_since(0).entries;
        assert_eq!(all.len(), 2);
        for e in &all {
            for text in entry_texts(e) {
                for value in &values {
                    assert!(!text.contains(value.as_str()), "{text}");
                }
            }
            assert_eq!(e.request_body.text, WITHHELD);
            assert_eq!(e.response_body.text, WITHHELD);
            assert_eq!(e.url, format!("https://api.test/items?{WITHHELD}"));
            let hop = &e.hops[0];
            assert_eq!(hop.url, e.url);
            for (_, v) in e
                .request_headers
                .iter()
                .chain(&hop.request_headers)
                .chain(&hop.response_headers)
            {
                assert_eq!(v, WITHHELD);
            }
        }
    }

    #[test]
    fn a_panic_publishes_what_was_staged() {
        let log = SessionLog::new();
        let inner = log.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let (hold, _guard) = SendHold::open(inner);
            hold.stage(line("before the panic"));
            panic!("send panicked");
        }));
        assert_eq!(log.console_since(0).entries[0].text, "before the panic");
    }
}
