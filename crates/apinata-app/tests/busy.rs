use apinata_app::busy::{BUSY_AFTER, BusyState, BusyTracker};
use std::time::{Duration, Instant};

const MS: Duration = Duration::from_millis(1);

#[test]
fn nothing_pending_is_idle() {
    let mut b = BusyTracker::default();
    assert_eq!(b.observe(0, 0, Instant::now()), BusyState::Idle);
}

#[test]
fn a_pending_command_waits_then_turns_busy() {
    let t0 = Instant::now();
    let mut b = BusyTracker::default();
    let until = t0 + BUSY_AFTER;
    assert_eq!(b.observe(2, 5, t0), BusyState::Waiting { until });
    assert_eq!(b.observe(2, 5, until - MS), BusyState::Waiting { until });
    assert_eq!(b.observe(2, 5, until), BusyState::Busy);
}

#[test]
fn progress_restarts_the_clock() {
    let t0 = Instant::now();
    let mut b = BusyTracker::default();
    b.observe(3, 5, t0);
    let t1 = t0 + 400 * MS;
    assert_eq!(
        b.observe(2, 6, t1),
        BusyState::Waiting {
            until: t1 + BUSY_AFTER
        }
    );
    assert_eq!(
        b.observe(2, 6, t0 + 600 * MS),
        BusyState::Waiting {
            until: t1 + BUSY_AFTER
        }
    );
}

#[test]
fn draining_the_queue_clears_busy() {
    let t0 = Instant::now();
    let mut b = BusyTracker::default();
    b.observe(1, 5, t0);
    assert_eq!(b.observe(1, 5, t0 + BUSY_AFTER), BusyState::Busy);
    assert_eq!(b.observe(0, 6, t0 + BUSY_AFTER + MS), BusyState::Idle);
    let t2 = t0 + 2000 * MS;
    assert_eq!(
        b.observe(1, 6, t2),
        BusyState::Waiting {
            until: t2 + BUSY_AFTER
        }
    );
}
