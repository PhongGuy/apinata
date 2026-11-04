//! The per-leg deadline that replaces reqwest's total timeout.

use std::future::Future;
use std::time::Duration;

/// An absolute point in time after which a leg (headers plus body) has run out of time.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Deadline(Option<tokio::time::Instant>);

impl Deadline {
    /// A deadline `t` from now; `None` never expires.
    pub(crate) fn after(t: Option<Duration>) -> Deadline {
        Deadline(t.map(|t| tokio::time::Instant::now() + t))
    }

    /// Completes when the deadline passes; pending forever when there is none.
    pub(crate) async fn expired(&self) {
        match self.0 {
            Some(at) => tokio::time::sleep_until(at).await,
            None => std::future::pending().await,
        }
    }

    /// Runs `fut` until it finishes (`Some`) or the deadline passes (`None`).
    pub(crate) async fn run<F: Future>(&self, fut: F) -> Option<F::Output> {
        match self.0 {
            Some(at) => tokio::time::timeout_at(at, fut).await.ok(),
            None => Some(fut.await),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn no_limit_never_expires() {
        let d = Deadline::after(None);
        assert_eq!(d.run(async { 7 }).await, Some(7));
    }

    #[tokio::test]
    async fn an_elapsed_deadline_yields_none() {
        let d = Deadline::after(Some(Duration::from_millis(50)));
        assert_eq!(d.run(std::future::pending::<()>()).await, None);
    }

    #[tokio::test]
    async fn the_deadline_is_absolute_across_runs() {
        let d = Deadline::after(Some(Duration::from_millis(200)));
        assert!(
            d.run(tokio::time::sleep(Duration::from_millis(60)))
                .await
                .is_some()
        );
        // At least 60 ms of the 200 are spent, so a second 300 ms wait overruns.
        assert!(
            d.run(tokio::time::sleep(Duration::from_millis(300)))
                .await
                .is_none()
        );
    }
}
