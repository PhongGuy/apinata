//! Proxy auto-config: running a PAC script to pick the proxy for each hop.
//!
//! `isInNet(host, pattern, mask)` follows the `netmask` package that `pac-resolver` uses, which
//! accepts more than browsers do. A mask may be a dotted address or a bit count (`"8"`), and
//! the mask may be left out with the pattern written as CIDR: `isInNet(h, "10.0.0.0/8")` is
//! true for `10.1.2.3`. Chrome and Firefox expect all three arguments and answer false for
//! the two-argument CIDR form, so a script that must behave the same in a browser passes the
//! mask. With an explicit mask, a pattern that still contains a slash never matches.

mod cache;
mod directive;
mod eval;
mod fetch;
mod source;

pub use cache::clear_pac_cache;
pub use source::{PacSource, PacSourceError};

use std::fmt;
use std::time::Duration;

use url::Url;

use crate::network::ResolvedProxy;
pub(crate) use cache::{PAC_CACHE, PacCache};
use eval::EvalError;
pub(crate) use fetch::FetchOptions;
use fetch::fetch;

/// How long a hop waits for its evaluation: both 1 s steps plus a margin.
pub(crate) const EVAL_WAIT: Duration = Duration::from_millis(2_250);

/// Why a hop went direct instead of where PAC would have sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PacWarning {
    /// The PAC file could not be fetched; the message names the redacted source.
    Fetch {
        /// The redacted source and the cause.
        message: String,
    },
    /// The script did not compile, threw, or ran out of memory.
    Script {
        /// The script's error message.
        message: String,
    },
    /// The script ran past its time limit.
    Timeout,
    /// The answer's first directive is not a usable route; cut to 200 characters.
    Answer {
        /// The script's answer, at most 200 characters.
        answer: String,
    },
}

// The cause alone, so callers can prefix it with what fell back to direct.
impl fmt::Display for PacWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PacWarning::Fetch { message } => write!(f, "could not be fetched: {message}"),
            PacWarning::Script { message } => write!(f, "script failed: {message}"),
            PacWarning::Timeout => f.write_str("ran past its time limit"),
            PacWarning::Answer { answer } => {
                write!(f, "answered {answer:?}, which is not a usable proxy")
            }
        }
    }
}

/// One send's PAC state: its warnings, at most one per variant, and a fetch failure that later
/// hops of the same send reuse instead of fetching again.
#[derive(Debug, Default)]
pub(crate) struct PacSend {
    warnings: Vec<PacWarning>,
    fetch_failed: Option<String>,
}

impl PacSend {
    /// The warnings this send collected, in the order they first occurred.
    pub(crate) fn into_warnings(self) -> Vec<PacWarning> {
        self.warnings
    }

    fn record(&mut self, warning: &PacWarning) {
        let kind = std::mem::discriminant(warning);
        if !self
            .warnings
            .iter()
            .any(|w| std::mem::discriminant(w) == kind)
        {
            self.warnings.push(warning.clone());
        }
    }
}

/// The proxy PAC sends `url` through (`None` is direct) and the line `Hop.pac` records:
/// the route (`DIRECT`, `scheme://host:port`) or the fallback's cause.
pub(crate) async fn resolve_hop(
    cache: &PacCache,
    source: &PacSource,
    fetch_opts: &FetchOptions,
    url: &Url,
    send: &mut PacSend,
) -> (Option<ResolvedProxy>, String) {
    match answer_for(cache, source, fetch_opts, url, send).await {
        Ok(Some(proxy)) => {
            let note = proxy.to_string();
            (Some(proxy), note)
        }
        Ok(None) => (None, "DIRECT".to_owned()),
        Err(warning) => {
            send.record(&warning);
            (None, warning.to_string())
        }
    }
}

/// The hop's route from the cached script, or the warning that sends it direct.
async fn answer_for(
    cache: &PacCache,
    source: &PacSource,
    fetch_opts: &FetchOptions,
    url: &Url,
    send: &mut PacSend,
) -> Result<Option<ResolvedProxy>, PacWarning> {
    if let Some(message) = &send.fetch_failed {
        return Err(PacWarning::Fetch {
            message: message.clone(),
        });
    }
    let script = match cache
        .get(source.as_str(), || fetch(source, fetch_opts))
        .await
    {
        Ok(script) => script,
        Err(message) => {
            send.fetch_failed = Some(message.clone());
            return Err(PacWarning::Fetch { message });
        }
    };
    let (target, host) = (
        url.as_str().to_owned(),
        url.host_str().unwrap_or_default().to_owned(),
    );
    // The JavaScript is bounded by the interrupt handler; the wait also bounds a helper blocked
    // in DNS. Only a script that failed to compile is evicted, so the next send fetches afresh.
    let task = tokio::task::spawn_blocking(move || eval::evaluate(&script, &target, &host));
    let answer = match tokio::time::timeout(EVAL_WAIT, task).await {
        Err(_) | Ok(Ok(Err(EvalError::Timeout))) => return Err(PacWarning::Timeout),
        Ok(Err(join)) => {
            return Err(PacWarning::Script {
                message: format!("the evaluation stopped: {join}"),
            });
        }
        Ok(Ok(Err(EvalError::Compile(message)))) => {
            cache.forget(source.as_str());
            return Err(PacWarning::Script { message });
        }
        Ok(Ok(Err(EvalError::Script(message)))) => return Err(PacWarning::Script { message }),
        Ok(Ok(Ok(answer))) => answer,
    };
    directive::first_directive(&answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Instant;

    fn cache() -> PacCache {
        PacCache::new(Arc::new(Instant::now))
    }

    /// A PAC file and its source; the directory keeps it alive.
    fn pac(body: &str) -> (tempfile::TempDir, PathBuf, PacSource) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("proxy.pac");
        std::fs::write(&file, body).expect("write");
        let source = PacSource::parse(file.to_str().expect("utf-8"), None).expect("source");
        (dir, file, source)
    }

    fn find(body: &str) -> String {
        format!("function FindProxyForURL(url, host) {{ {body} }}")
    }

    fn url(text: &str) -> Url {
        Url::parse(text).expect("url")
    }

    fn opts() -> FetchOptions {
        FetchOptions::new(false, None, true)
    }

    #[tokio::test]
    async fn a_proxy_answer_routes_the_hop_and_records_the_route() {
        let (_dir, _, source) = pac(&find(
            "return url === 'http://a.test:8080/p?q=1' && host === 'a.test' ? 'PROXY 127.0.0.1:3128' : 'DIRECT';",
        ));
        let cache = cache();
        let mut send = PacSend::default();
        let (proxy, note) = resolve_hop(
            &cache,
            &source,
            &opts(),
            &url("http://a.test:8080/p?q=1"),
            &mut send,
        )
        .await;
        assert_eq!(
            proxy.map(|p| p.to_string()).as_deref(),
            Some("http://127.0.0.1:3128")
        );
        assert_eq!(note, "http://127.0.0.1:3128");
        let (proxy, note) =
            resolve_hop(&cache, &source, &opts(), &url("http://b.test/"), &mut send).await;
        assert_eq!((proxy, note.as_str()), (None, "DIRECT"));
        assert!(send.into_warnings().is_empty());
    }

    #[tokio::test]
    async fn a_fetch_failure_is_not_retried_within_a_send() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("late.pac");
        let source = PacSource::parse(file.to_str().expect("utf-8"), None).expect("source");
        let cache = cache();
        let mut send = PacSend::default();
        let (proxy, note) =
            resolve_hop(&cache, &source, &opts(), &url("http://a.test/"), &mut send).await;
        assert!(proxy.is_none());
        assert!(
            note.starts_with("could not be fetched: ") && note.contains("late.pac"),
            "{note}"
        );
        std::fs::write(&file, find("return 'PROXY 127.0.0.1:3128';")).expect("write");
        let (proxy, _) = resolve_hop(
            &cache,
            &source,
            &opts(),
            &url("http://a.test/next"),
            &mut send,
        )
        .await;
        assert!(proxy.is_none(), "the same send does not fetch again");
        let warnings = send.into_warnings();
        assert!(
            matches!(warnings.as_slice(), [PacWarning::Fetch { .. }]),
            "{warnings:?}"
        );
        let mut next = PacSend::default();
        let (proxy, _) =
            resolve_hop(&cache, &source, &opts(), &url("http://a.test/"), &mut next).await;
        assert!(proxy.is_some(), "the next send fetches again");
        assert!(next.into_warnings().is_empty());
    }

    #[tokio::test]
    async fn one_warning_per_cause_within_a_send() {
        let (_dir, _, source) = pac(&find(
            "if (host === 'a.test') throw new Error('boom'); return 'FTP x:1';",
        ));
        let cache = cache();
        let mut send = PacSend::default();
        for target in [
            "http://a.test/1",
            "http://a.test/2",
            "http://b.test/1",
            "http://b.test/2",
        ] {
            let (proxy, _) = resolve_hop(&cache, &source, &opts(), &url(target), &mut send).await;
            assert!(proxy.is_none(), "{target}");
        }
        let warnings = send.into_warnings();
        assert!(
            matches!(
                warnings.as_slice(),
                [PacWarning::Script { message }, PacWarning::Answer { answer }]
                    if message == "boom" && answer == "FTP x:1"
            ),
            "{warnings:?}"
        );
    }

    #[tokio::test]
    async fn a_script_that_fails_to_compile_is_dropped_from_the_cache() {
        let (_dir, file, source) = pac("var nothing = 1;");
        let cache = cache();
        let mut send = PacSend::default();
        let (_, note) =
            resolve_hop(&cache, &source, &opts(), &url("http://a.test/"), &mut send).await;
        assert!(note.starts_with("script failed: "), "{note}");
        std::fs::write(&file, find("return 'PROXY 127.0.0.1:3128';")).expect("write");
        let mut next = PacSend::default();
        let (proxy, _) =
            resolve_hop(&cache, &source, &opts(), &url("http://a.test/"), &mut next).await;
        assert!(proxy.is_some(), "a fresh fetch replaced the broken script");
    }

    #[tokio::test]
    async fn a_runaway_script_times_out_and_goes_direct() {
        let (_dir, _, source) = pac(&find("while (true) {}"));
        let mut send = PacSend::default();
        let (proxy, note) = resolve_hop(
            &cache(),
            &source,
            &opts(),
            &url("http://a.test/"),
            &mut send,
        )
        .await;
        assert_eq!((proxy, note.as_str()), (None, "ran past its time limit"));
        assert_eq!(send.into_warnings(), [PacWarning::Timeout]);
    }
}
