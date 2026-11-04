//! Test helper: turn a hang into a failure.

use std::sync::mpsc;
use std::time::Duration;

/// How long a guarded body may run; far beyond any limit under test, so only a hang trips it.
const GUARD: Duration = Duration::from_secs(30);

/// Run `body` on its own thread and panic if it has not finished within `GUARD`. A body stuck
/// in a loop its limit missed leaks that thread, which is acceptable in a failing test.
pub(crate) fn guarded<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(body());
    });
    rx.recv_timeout(GUARD)
        .unwrap_or_else(|_| panic!("hung: no result within {GUARD:?}, its limit did not fire"))
}
