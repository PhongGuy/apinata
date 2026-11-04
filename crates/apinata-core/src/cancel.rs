//! Cancelling a send from outside it.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio_util::sync::{CancellationToken, DropGuard};

/// A handle that cancels the sends it is passed to; clones share one cancellation.
#[derive(Debug, Clone, Default)]
pub struct CancelHandle {
    token: CancellationToken,
    stop: Arc<StopState>,
}

/// Stop is separate from cancel: it ends only a body read, and only while one is armed.
#[derive(Debug, Default)]
struct StopState {
    armed: AtomicBool,
    /// Replaced on every arm, so a stop that fired for one send never pre-stops the next.
    token: Mutex<CancellationToken>,
}

impl StopState {
    fn token(&self) -> CancellationToken {
        self.token.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl CancelHandle {
    /// A handle that is not yet cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels every send using this handle or a clone of it, now and later.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Ends the response body being read now, keeping what arrived. A no-op unless a body read
    /// is in progress, so a stop before the headers never carries over.
    pub fn stop(&self) {
        if self.stop.armed.load(Ordering::SeqCst) {
            self.stop.token().cancel();
        }
    }

    /// Starts honouring `stop` with a fresh signal; the read loop calls it when the final
    /// headers arrive.
    pub(crate) fn arm(&self) {
        *self.stop.token.lock().unwrap_or_else(|e| e.into_inner()) = CancellationToken::new();
        self.stop.armed.store(true, Ordering::SeqCst);
    }

    /// Stops honouring `stop`; the read loop calls it when the body read ends.
    pub(crate) fn disarm(&self) {
        self.stop.armed.store(false, Ordering::SeqCst);
    }

    /// Completes once an armed `stop` happened; pending otherwise.
    pub(crate) async fn stopped(&self) {
        self.stop.token().cancelled_owned().await;
    }

    /// Whether `cancel` has been called on this handle or a clone of it.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Runs `fut` until it finishes or the handle is cancelled; `None` once cancelled,
    /// including when it already was, in which case `fut` is never polled.
    pub(crate) async fn run<F: Future>(&self, fut: F) -> Option<F::Output> {
        self.token.run_until_cancelled(fut).await
    }

    /// A handle cancelled with this one and also when the returned guard drops, so work started
    /// under it ends with the future that holds the guard.
    pub(crate) fn child_until_dropped(&self) -> (CancelHandle, DropGuard) {
        let token = self.token.child_token();
        let guard = token.clone().drop_guard();
        let stop = Arc::clone(&self.stop);
        (CancelHandle { token, stop }, guard)
    }
}
#[cfg(test)]
mod stop_tests {
    use super::*;
    use std::time::Duration;

    async fn done(h: &CancelHandle) -> bool {
        tokio::time::timeout(Duration::from_millis(50), h.stopped())
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn stop_before_arming_is_a_no_op_that_does_not_stick() {
        let h = CancelHandle::new();
        h.stop();
        h.arm();
        assert!(!done(&h).await);
    }

    #[tokio::test]
    async fn an_armed_stop_completes_for_clones_and_children() {
        let h = CancelHandle::new();
        let (child, _guard) = h.child_until_dropped();
        h.arm();
        h.clone().stop();
        assert!(done(&h).await);
        assert!(done(&child).await);
        assert!(!h.is_cancelled());
    }

    #[tokio::test]
    async fn stop_after_disarm_is_a_no_op() {
        let h = CancelHandle::new();
        h.arm();
        h.disarm();
        h.stop();
        assert!(!done(&h).await);
    }

    #[tokio::test]
    async fn a_second_arm_is_not_pre_stopped_by_the_first_stop() {
        let h = CancelHandle::new();
        h.arm();
        h.stop();
        h.disarm();
        h.arm();
        assert!(!done(&h).await);
    }
}
