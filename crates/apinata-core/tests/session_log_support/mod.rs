//! Helpers for the session log integration tests.
#![allow(dead_code)] // each test binary uses a different subset

use apinata_core::{ConsoleLine, ExecOptions, Executor, NetworkEntry, SessionLog};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use std::sync::Arc;

const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// An executor recording into a fresh log, and the log.
pub fn logged(opts: ExecOptions) -> (Executor, SessionLog) {
    let log = SessionLog::new();
    (
        Executor::new(opts.session_log(log.clone())).expect("client"),
        log,
    )
}

pub fn entries(log: &SessionLog) -> Vec<Arc<NetworkEntry>> {
    log.network_since(0).entries
}

pub fn lines(log: &SessionLog) -> Vec<Arc<ConsoleLine>> {
    log.console_since(0).entries
}

/// The one network entry `log` holds.
pub fn only(log: &SessionLog) -> Arc<NetworkEntry> {
    let mut all = entries(log);
    assert_eq!(all.len(), 1, "{all:?}");
    all.remove(0)
}

/// The value of header `name` in `pairs`, in any case.
pub fn header<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Every text field of `e`, hops included.
pub fn texts(e: &NetworkEntry) -> Vec<String> {
    let mut out = vec![
        e.method.clone(),
        e.url.clone(),
        e.request_body.text.clone(),
        e.response_body.text.clone(),
    ];
    out.extend(e.error.clone());
    out.extend(
        e.request_headers
            .iter()
            .flat_map(|(n, v)| [n.clone(), v.clone()]),
    );
    for hop in &e.hops {
        out.extend([hop.method.clone(), hop.url.clone()]);
        out.extend(hop.pac.clone());
        out.extend(
            hop.request_headers
                .iter()
                .chain(&hop.response_headers)
                .flat_map(|(n, v)| [n.clone(), v.clone()]),
        );
    }
    out
}

/// Fails if any published entry or console line holds `secret`, raw or percent-encoded, in
/// any ASCII case.
pub fn assert_absent(log: &SessionLog, secret: &str) {
    let forms = [
        secret.to_ascii_lowercase(),
        utf8_percent_encode(secret, UNRESERVED)
            .to_string()
            .to_ascii_lowercase(),
    ];
    let mut all: Vec<String> = entries(log).iter().flat_map(|e| texts(e)).collect();
    all.extend(
        lines(log)
            .iter()
            .flat_map(|l| [l.request.clone(), l.text.clone()]),
    );
    for text in &all {
        let folded = text.to_ascii_lowercase();
        for form in &forms {
            assert!(!folded.contains(form.as_str()), "{form} in {text}");
        }
    }
}
