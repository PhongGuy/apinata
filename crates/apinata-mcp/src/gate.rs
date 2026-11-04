//! Marks tool writes in flight so the file watcher does not snapshot a half-finished change.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Notify;

/// Counts the mutating tool calls in flight; shared by every clone of the server.
#[derive(Clone, Default)]
pub(crate) struct WriteGate {
    inflight: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

/// Holds the gate busy until dropped, so a cancelled or panicking call still releases it.
pub(crate) struct WriteGuard {
    gate: WriteGate,
}

impl WriteGate {
    /// Mark one write in flight until the returned guard drops.
    pub(crate) fn enter(&self) -> WriteGuard {
        self.inflight.fetch_add(1, Ordering::SeqCst);
        WriteGuard { gate: self.clone() }
    }

    /// Resolve once no write is in flight.
    pub(crate) async fn wait_idle(&self) {
        loop {
            // Register before checking, so a release between the check and the await is not lost.
            let released = self.idle.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            if self.inflight.load(Ordering::SeqCst) == 0 {
                return;
            }
            released.await;
        }
    }
}

impl Drop for WriteGuard {
    fn drop(&mut self) {
        self.gate.inflight.fetch_sub(1, Ordering::SeqCst);
        self.gate.idle.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test(start_paused = true)]
    async fn wait_idle_resolves_at_once_when_nothing_is_in_flight() {
        let gate = WriteGate::default();
        timeout(Duration::from_secs(1), gate.wait_idle())
            .await
            .expect("idle");
    }

    #[tokio::test(start_paused = true)]
    async fn wait_idle_blocks_while_a_guard_is_alive() {
        let gate = WriteGate::default();
        let guard = gate.enter();
        let blocked = timeout(Duration::from_secs(60), gate.wait_idle()).await;
        assert!(blocked.is_err(), "resolved with a write in flight");
        drop(guard);
        timeout(Duration::from_secs(1), gate.wait_idle())
            .await
            .expect("idle after drop");
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiter_wakes_when_the_last_guard_drops() {
        let gate = WriteGate::default();
        let first = gate.enter();
        let second = gate.clone().enter();
        let waiter = tokio::spawn({
            let gate = gate.clone();
            async move { gate.wait_idle().await }
        });
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop(first);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!waiter.is_finished(), "one write was still in flight");
        // Dropped without any explicit finish, as a cancelled call would be.
        drop(second);
        timeout(Duration::from_secs(1), waiter)
            .await
            .expect("woke")
            .expect("joined");
    }
}
