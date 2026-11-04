//! Builders shared by the session log's unit tests.

use super::entry::{logged_body, logged_error, logged_headers, logged_method, logged_url};
use super::redact::Redactor;
use super::{ConsoleLine, LoggedHop, NetworkEntry, SendSource};
use crate::{ConsoleLevel, ScriptPhase};
use std::time::{Duration, SystemTime};

/// An entry carrying `value` in its URLs, a header, an error and a cookie, with `body` both ways.
pub(crate) fn entry_with(r: &Redactor, value: &str, body: &[u8], body_cap: usize) -> NetworkEntry {
    let headers = vec![
        ("X-Key".to_owned(), value.to_owned()),
        ("Authorization".to_owned(), format!("Bearer {value}")),
    ];
    let url = format!("https://api.test/items?key={value}");
    let hop = hop_with(
        r,
        &url,
        &headers,
        &[("Set-Cookie".to_owned(), format!("s={value}"))],
    );
    let (method, method_cut) = logged_method(r, "GET");
    let (url, url_cut) = logged_url(r, &url);
    let (error, error_cut) = logged_error(r, &format!("failed near {value}"));
    let (request_headers, request_folded) = logged_headers(r, &headers);
    NetworkEntry {
        seq: 0,
        at: SystemTime::UNIX_EPOCH,
        source: SendSource::Request,
        method,
        url,
        status: Some(200),
        error: Some(error),
        duration: Duration::from_millis(7),
        request_headers,
        request_body: logged_body(r, body, body_cap),
        response_body: logged_body(r, body, body_cap),
        hops: vec![hop],
        hops_omitted: 0,
        attempt: None,
        method_cut,
        url_cut,
        error_cut,
        request_folded,
    }
}

/// A 200 GET hop to `url` with the given header lists, built through the field builders.
pub(crate) fn hop_with(
    r: &Redactor,
    url: &str,
    request: &[(String, String)],
    response: &[(String, String)],
) -> LoggedHop {
    let (method, method_cut) = logged_method(r, "GET");
    let (url, url_cut) = logged_url(r, url);
    let (request_headers, request_folded) = logged_headers(r, request);
    let (response_headers, response_folded) = logged_headers(r, response);
    LoggedHop {
        method,
        url,
        status: 200,
        request_headers,
        response_headers,
        elapsed: Duration::from_millis(5),
        pac: None,
        override_from: None,
        timing: None,
        version: None,
        remote_addr: None,
        via_proxy: false,
        method_cut,
        url_cut,
        pac_cut: None,
        override_cut: None,
        request_folded,
        response_folded,
    }
}

pub(crate) fn line_with(r: &Redactor, text: &str) -> ConsoleLine {
    ConsoleLine::capture(
        SystemTime::UNIX_EPOCH,
        "Get items",
        Some(ScriptPhase::PreRequest),
        ConsoleLevel::Log,
        text,
        r,
    )
}

/// Every text field an entry publishes, hops included.
pub(crate) fn entry_texts(e: &NetworkEntry) -> Vec<String> {
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
        out.extend(
            hop.request_headers
                .iter()
                .chain(&hop.response_headers)
                .flat_map(|(n, v)| [n.clone(), v.clone()]),
        );
    }
    out
}
