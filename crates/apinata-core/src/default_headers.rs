//! The headers core adds to a request when the request has no header of that name, and the
//! per-request `settings.omitHeaders` that switches each off.

use reqwest::header::{ACCEPT, ACCEPT_ENCODING, CONNECTION, HeaderMap, HeaderValue, USER_AGENT};

const USER_AGENT_VALUE: &str = concat!("apinata/", env!("CARGO_PKG_VERSION"));
const ACCEPT_VALUE: &str = "application/json, text/plain, */*";
// What the client puts on the wire for gzip, deflate and brotli; zstd replies are decoded but not
// advertised, as the reference client does.
const ACCEPT_ENCODING_VALUE: &str = "gzip,deflate,br";
// HTTP/1.1 keep-alive is implicit, so hyper sends nothing; naming it makes the wire match the row.
// HTTP/2 forbids the header and the stack drops it there.
const CONNECTION_VALUE: &str = "keep-alive";

/// A header core adds to a send unless the request sets its own or omits it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DefaultHeader {
    /// The header name as shown, such as `User-Agent`.
    pub name: String,
    /// The value that is sent.
    pub value: String,
}

/// The default headers in the order they are listed, with the values a send carries.
pub fn default_headers() -> Vec<DefaultHeader> {
    [
        ("User-Agent", USER_AGENT_VALUE),
        ("Accept", ACCEPT_VALUE),
        ("Accept-Encoding", ACCEPT_ENCODING_VALUE),
        ("Connection", CONNECTION_VALUE),
    ]
    .into_iter()
    .map(|(name, value)| DefaultHeader {
        name: name.to_owned(),
        value: value.to_owned(),
    })
    .collect()
}

/// Whether `omitted` names `name`, compared without case.
pub(crate) fn omits(omitted: &[String], name: &str) -> bool {
    omitted.iter().any(|o| o.eq_ignore_ascii_case(name))
}

/// The defaults a request still gets: all of them except the names in `omitted`, compared
/// without case. `Host` is not a default, so omitting it changes nothing. The HTTP stack always
/// sends some `Accept` and cannot be told otherwise, so an omitted one falls back to the neutral
/// `*/*` it would send anyway; naming it here keeps the timeline and signatures true to the wire.
pub(crate) fn defaults_without(omitted: &[String]) -> HeaderMap {
    let skip = |name: &str| omits(omitted, name);
    let mut defaults = HeaderMap::new();
    if !skip("User-Agent") {
        defaults.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    }
    let accept = if skip("Accept") { "*/*" } else { ACCEPT_VALUE };
    defaults.insert(ACCEPT, HeaderValue::from_static(accept));
    // An omitted Accept-Encoding also needs the client's decompression off (see `ClientKey`),
    // because that layer adds the header itself when the request has none.
    if !skip("Accept-Encoding") {
        defaults.insert(
            ACCEPT_ENCODING,
            HeaderValue::from_static(ACCEPT_ENCODING_VALUE),
        );
    }
    if !skip("Connection") {
        defaults.insert(CONNECTION, HeaderValue::from_static(CONNECTION_VALUE));
    }
    defaults
}
