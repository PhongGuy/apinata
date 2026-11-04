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

/// A named pipe at `path`; opening it for reading waits for a writer that never comes.
#[cfg(unix)]
pub(crate) fn mkfifo(path: &std::path::Path) {
    let made = std::process::Command::new("mkfifo").arg(path).status();
    assert!(made.is_ok_and(|s| s.success()), "mkfifo failed");
}

/// Run `body` on its own thread and fail if it is still stuck on `pipe` after 5 seconds. The
/// pipe is then opened read-write, which never waits and releases an open stuck either way.
#[cfg(unix)]
pub(crate) fn off_the_pipe<T: Send + 'static>(
    pipe: &std::path::Path,
    body: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(body());
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| {
        let release = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe);
        drop(release);
        panic!("blocked on the pipe at {}", pipe.display())
    })
}
