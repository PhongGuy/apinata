//! Re-sending a request whose send failed in transit or was answered with 502, 503 or 504.

use crate::cancel::CancelHandle;
use crate::execute::{ConnectCause, ExecError, Executor, SendTag};
use crate::network::PacWarning;
use crate::resolve::ResolvedRequest;
use crate::response::Response;
use std::time::Duration;

/// The longest pause before a retry, whatever the server's `Retry-After` asks.
const MAX_WAIT: Duration = Duration::from_secs(60);

/// How many times, and how patiently, a send is repeated.
#[derive(Clone, Copy, Default)]
pub(crate) struct Retry {
    pub retries: u32,
    pub delay: Duration,
    pub unsafe_methods: bool,
}

/// Told `(attempt about to start, total attempts, pause)` just before each pause between attempts.
pub(crate) type RetryHook = std::sync::Arc<dyn Fn(u32, u32, Duration) + Send + Sync>;

/// What a run of sends ended with.
pub(crate) struct Sent {
    /// The last attempt's outcome.
    pub(crate) sent: Result<Response, ExecError>,
    pub(crate) pac: Vec<PacWarning>,
    /// How many sends were made.
    pub(crate) attempts: u32,
    /// Set when a cancel ended the pause before the next attempt, so `sent` is the attempt before.
    pub(crate) cancelled: bool,
}

/// Sends `req`, repeating it while the send is retryable and retries remain. Returns the last
/// attempt's outcome and PAC warnings with the number of sends made; a cancel during the pause
/// keeps that attempt and marks it. An answer that was read as an event-stream is never repeated,
/// except a refused subscription handshake, which is judged as a GET.
pub(crate) async fn send_with_retries(
    exec: &Executor,
    req: &ResolvedRequest,
    cancel: &CancelHandle,
    tag: &SendTag,
    retry: Retry,
    on_retry: Option<&RetryHook>,
) -> Sent {
    let subscription = crate::subscription::is_subscription(req);
    let allowed =
        retry.retries > 0 && (retry.unsafe_methods || subscription || idempotent(req.method()));
    let mut attempts = 1;
    loop {
        // The log numbers the sends only when a repeat was possible.
        let numbered = SendTag {
            attempt: allowed.then_some((attempts, retry.retries + 1)),
            ..tag.clone()
        };
        let (sent, pac) = exec.send_as_reporting(req, cancel, &numbered).await;
        let done = |sent, pac, cancelled| Sent {
            sent,
            pac,
            attempts,
            cancelled,
        };
        if !allowed || attempts > retry.retries {
            return done(sent, pac, false);
        }
        let Some(asked) = retry_wanted(&sent, subscription) else {
            return done(sent, pac, false);
        };
        let wait = retry.delay.max(asked).min(MAX_WAIT);
        // Announced before the pause starts, with the earlier attempt already fully read.
        if let Some(hook) = on_retry {
            hook(attempts + 1, retry.retries + 1, wait);
        }
        if !wait.is_zero() && cancel.run(tokio::time::sleep(wait)).await.is_none() {
            return done(sent, pac, true);
        }
        if cancel.is_cancelled() {
            return done(sent, pac, true);
        }
        attempts += 1;
    }
}

/// Methods a repeat cannot make worse.
fn idempotent(method: &str) -> bool {
    ["GET", "HEAD", "OPTIONS", "PUT", "DELETE", "TRACE"]
        .iter()
        .any(|m| method.eq_ignore_ascii_case(m))
}

/// `Some(pause the server asked for)` when the outcome is worth another send, else `None`. A
/// refused `subscription` handshake carries a `StreamEnd` yet nothing was subscribed, so it counts.
fn retry_wanted(sent: &Result<Response, ExecError>, subscription: bool) -> Option<Duration> {
    match sent {
        Ok(response)
            if matches!(response.status(), 502..=504)
                && (subscription || response.stream_end().is_none()) =>
        {
            Some(retry_after(response).unwrap_or_default())
        }
        Ok(_) => None,
        Err(ExecError::Timeout { run_wide: None, .. } | ExecError::Transport { .. }) => {
            Some(Duration::ZERO)
        }
        Err(ExecError::Connect { cause, .. }) if !is_tls(*cause) => Some(Duration::ZERO),
        Err(_) => None,
    }
}

fn is_tls(cause: ConnectCause) -> bool {
    matches!(
        cause,
        ConnectCause::Tls
            | ConnectCause::TlsUnknownIssuer
            | ConnectCause::TlsWrongProtocol
            | ConnectCause::TlsExpired
    )
}

/// The `Retry-After` header as a pause: whole seconds, or an HTTP date (a past one is zero).
fn retry_after(response: &Response) -> Option<Duration> {
    let (_, value) = response
        .headers()
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))?;
    pause_from(value)
}

/// A `Retry-After` value as a pause.
fn pause_from(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let ahead = date.signed_duration_since(chrono::Utc::now());
    Some(ahead.to_std().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::{idempotent, pause_from};
    use std::time::Duration;

    #[test]
    fn only_repeatable_methods_are_idempotent() {
        assert!(idempotent("get") && idempotent("PUT") && idempotent("DELETE"));
        assert!(!idempotent("POST") && !idempotent("PATCH"));
    }

    #[test]
    fn retry_after_reads_seconds_and_dates() {
        assert_eq!(pause_from(" 7 "), Some(Duration::from_secs(7)));
        let past = "Wed, 21 Oct 2015 07:28:00 GMT";
        assert_eq!(pause_from(past), Some(Duration::ZERO));
        assert_eq!(pause_from("soon"), None);
    }
}
