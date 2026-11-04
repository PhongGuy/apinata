//! The response an executed request produced, as assertions read it.

use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::cookies::SetCookie;
use crate::http_version::HttpVersion;
use crate::resolve::redact_url;
use crate::script::ScriptPhase;
use crate::sse::SseEvent;
use crate::stream::StreamEnd;

/// An HTTP response: status, headers, body and timing, built with the `with_*` methods.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Response {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    text: String,
    response_time: Duration,
    url: String,
    timeline: Vec<Hop>,
    size: u64,
    truncated: bool,
    stream_end: Option<StreamEnd>,
    body_file: Option<PathBuf>,
    derived: Vec<String>,
    /// The cookies the request's scripts wrote, each with its phase, in write order.
    scripted: Vec<(ScriptPhase, SetCookie)>,
}

/// Where a [`Response::set_cookies`] or [`crate::RequestResult::set_cookies`] row came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CookieOrigin<'a> {
    /// A `Set-Cookie` header of this hop.
    Hop(&'a Hop),
    /// A script of the request, through `bru.cookies`, run in this phase.
    Script(ScriptPhase),
}

impl<'a> CookieOrigin<'a> {
    /// The hop that set the cookie; `None` for a script's.
    pub fn hop(&self) -> Option<&'a Hop> {
        match self {
            CookieOrigin::Hop(hop) => Some(hop),
            CookieOrigin::Script(_) => None,
        }
    }

    /// The phase of the script that set the cookie; `None` for a hop's.
    pub fn phase(&self) -> Option<ScriptPhase> {
        match self {
            CookieOrigin::Hop(_) => None,
            CookieOrigin::Script(phase) => Some(*phase),
        }
    }
}

/// One request and response of an exchange, redirects included, as it went over the wire.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Hop {
    /// The method sent.
    pub method: String,
    /// The URL sent to.
    pub url: String,
    /// The request's headers plus the client defaults it lacked, lowercase names. Headers the
    /// HTTP stack adds itself, such as `host` and a body's computed `content-length`, are absent.
    pub request_headers: Vec<(String, String)>,
    /// The status received.
    pub status: u16,
    /// The headers received, in arrival order with lowercase names.
    pub response_headers: Vec<(String, String)>,
    /// The time from sending the hop to the end of reading its body.
    pub elapsed: Duration,
    /// The first [`HOP_BODY_LIMIT`] bytes of the body this hop sent, empty when it sent none.
    pub request_body: Vec<u8>,
    /// The full length of the body this hop sent.
    pub request_body_size: u64,
    /// Under PAC, the route the script chose for this hop (`DIRECT` or `scheme://host:port`) or
    /// why it went direct; `None` outside PAC mode.
    pub pac: Option<String>,
    /// Where the hop's time went; `None` for a hop that was not measured (an NTLM leg).
    pub timing: Option<crate::HopTiming>,
    /// Every `Set-Cookie` this hop's response carried, in arrival order, with the jar's verdict.
    pub cookies: Vec<SetCookie>,
    pub(crate) version: Option<HttpVersion>,
    pub(crate) remote_addr: Option<SocketAddr>,
    pub(crate) via_proxy: bool,
    pub(crate) override_from: Option<String>,
}

impl Hop {
    /// The hop's URL as host, port and path, the way the app and `--verbose` label a hop: no
    /// scheme, userinfo, query or fragment. Unparseable text is returned as it is.
    pub fn label(&self) -> String {
        match url::Url::parse(&self.url) {
            Ok(u) if u.host_str().is_some() => {
                let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
                format!("{}{port}{}", u.host_str().unwrap_or_default(), u.path())
            }
            _ => self.url.clone(),
        }
    }

    /// The HTTP version this hop was answered over; `None` only for one outside [`HttpVersion`].
    pub fn version(&self) -> Option<HttpVersion> {
        self.version
    }

    /// The address of the peer this hop's connection reached; `None` when the stack did not
    /// report one. Through a proxy it is the proxy's, see [`Hop::via_proxy`].
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.remote_addr
    }

    /// Whether the hop went through a proxy, so [`Hop::remote_addr`] is the proxy's address.
    pub fn via_proxy(&self) -> bool {
        self.via_proxy
    }

    /// The host override this hop used, as `name -> address`; `None` when DNS answered.
    pub fn override_from(&self) -> Option<&str> {
        self.override_from.as_deref()
    }

    /// The peer as shown to a person: `127.0.0.1:8080`, or `proxy 127.0.0.1:3128` through a
    /// proxy; `None` when unknown, so nothing is invented.
    pub fn peer_label(&self) -> Option<String> {
        let addr = self.remote_addr?;
        Some(if self.via_proxy {
            format!("proxy {addr}")
        } else {
            addr.to_string()
        })
    }
}

/// How many bytes of each hop's request body a [`Hop`] keeps.
pub const HOP_BODY_LIMIT: usize = 64 * 1024;

// Like Response's Debug: the redacted URL, header names and body size only, since values and
// bodies carry secrets.
impl fmt::Debug for Hop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = |headers: &[(String, String)]| -> Vec<String> {
            headers.iter().map(|(name, _)| name.clone()).collect()
        };
        f.debug_struct("Hop")
            .field("method", &self.method)
            .field("url", &redact_url(&self.url))
            .field("request_headers", &names(&self.request_headers))
            .field("status", &self.status)
            .field("response_headers", &names(&self.response_headers))
            .field("elapsed", &self.elapsed)
            .field("request_body_size", &self.request_body_size)
            .field("pac", &self.pac)
            .field("version", &self.version)
            .field("remote_addr", &self.remote_addr)
            .field("via_proxy", &self.via_proxy)
            .field("override_from", &self.override_from)
            .field("cookies", &self.cookies)
            .finish()
    }
}

impl Response {
    /// A response with `status` and everything else empty.
    pub fn new(status: u16) -> Response {
        Response {
            status,
            status_text: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
            text: String::new(),
            response_time: Duration::ZERO,
            url: String::new(),
            timeline: Vec::new(),
            size: 0,
            truncated: false,
            stream_end: None,
            body_file: None,
            derived: Vec::new(),
            scripted: Vec::new(),
        }
    }

    /// Values the executor computed for this send (signatures, nonces, answers), for redaction.
    pub fn derived_credentials(&self) -> &[String] {
        &self.derived
    }

    /// Records the derived values, de-duplicated and without empty ones.
    pub fn with_derived_credentials(mut self, values: Vec<String>) -> Response {
        for v in values {
            if !v.is_empty() && !self.derived.contains(&v) {
                self.derived.push(v);
            }
        }
        self
    }

    /// How a streamed body ended; `None` when the body was not read as a stream.
    pub fn stream_end(&self) -> Option<&StreamEnd> {
        self.stream_end.as_ref()
    }

    /// Records how the streamed body ended.
    pub fn with_stream_end(mut self, e: StreamEnd) -> Response {
        self.stream_end = Some(e);
        self
    }

    /// The file the whole body was streamed to, when the send asked for one.
    pub fn body_file(&self) -> Option<&Path> {
        self.body_file.as_deref()
    }

    /// Records the file the whole body was written to.
    pub fn with_body_file(mut self, path: impl Into<PathBuf>) -> Response {
        self.body_file = Some(path.into());
        self
    }

    /// The body parsed as Server-Sent Events.
    pub fn events(&self) -> Vec<SseEvent> {
        crate::sse::parse(&self.body)
    }

    /// Sets the reason phrase, such as `OK`.
    pub fn with_status_text(mut self, text: impl Into<String>) -> Response {
        self.status_text = text.into();
        self
    }

    /// Appends a header; the name is ASCII-lowercased and repeats are kept in order.
    pub fn with_header(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Response {
        self.headers
            .push((name.as_ref().to_ascii_lowercase(), value.into()));
        self
    }

    /// Sets the raw body bytes and their decoded text as a whole body: the size becomes the
    /// byte count and any earlier truncation mark is cleared.
    pub fn with_body(mut self, bytes: impl Into<Vec<u8>>, text: impl Into<String>) -> Response {
        self.body = bytes.into();
        self.text = text.into();
        self.size = self.body.len() as u64;
        self.truncated = false;
        self
    }

    /// Marks the body as cut off at the size limit, `size` being the decoded bytes read.
    /// Call it after the body is set, since setting a body clears the mark.
    pub fn with_truncated(mut self, size: u64) -> Response {
        self.truncated = true;
        self.size = size;
        self
    }

    /// Sets the byte count shown, for a body kept elsewhere or dropped; leaves `truncated` alone.
    /// Call it after the body is set, since setting a body resets the size.
    pub fn with_size(mut self, size: u64) -> Response {
        self.size = size;
        self
    }

    /// Sets the body from text; the bytes are its UTF-8 encoding.
    pub fn with_text_body(self, text: impl Into<String>) -> Response {
        let text = text.into();
        let bytes = text.as_bytes().to_vec();
        self.with_body(bytes, text)
    }

    /// Sets the time the whole exchange took, redirects included.
    pub fn with_response_time(mut self, time: Duration) -> Response {
        self.response_time = time;
        self
    }

    /// Sets the final URL, after any redirects.
    pub fn with_url(mut self, url: impl Into<String>) -> Response {
        self.url = url.into();
        self
    }

    /// Sets the hops the exchange went through, the last one being this response.
    pub(crate) fn with_timeline(mut self, timeline: Vec<Hop>) -> Response {
        self.timeline = timeline;
        self
    }

    /// The hops in the order they were sent; empty for a response built by hand.
    pub fn timeline(&self) -> &[Hop] {
        &self.timeline
    }

    /// Sets the cookies the request's scripts wrote, each with its phase, in write order.
    pub(crate) fn with_script_cookies(
        mut self,
        scripted: Vec<(ScriptPhase, SetCookie)>,
    ) -> Response {
        self.scripted = scripted;
        self
    }

    /// Every cookie set for this response, each with where it came from, in the order it was
    /// set: the pre-request scripts', each hop's in timeline order, then the later scripts'.
    pub fn set_cookies(&self) -> impl Iterator<Item = (CookieOrigin<'_>, &SetCookie)> {
        // The pre-request rows were set before the first hop went out; every other phase's after.
        let scripts = |before: bool| {
            (self.scripted.iter())
                .filter(move |(phase, _)| (*phase == ScriptPhase::PreRequest) == before)
                .map(|(phase, c)| (CookieOrigin::Script(*phase), c))
        };
        let hops = (self.timeline.iter())
            .flat_map(|hop| hop.cookies.iter().map(move |c| (CookieOrigin::Hop(hop), c)));
        scripts(true).chain(hops).chain(scripts(false))
    }

    /// The peer address of the answering hop (the last); see [`Hop::remote_addr`].
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.timeline.last()?.remote_addr()
    }

    /// Whether the answering hop went through a proxy; see [`Hop::via_proxy`].
    pub fn via_proxy(&self) -> bool {
        self.timeline.last().is_some_and(Hop::via_proxy)
    }

    /// The answering hop's peer as shown to a person; see [`Hop::peer_label`].
    pub fn peer_label(&self) -> Option<String> {
        self.timeline.last()?.peer_label()
    }

    /// The status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The reason phrase.
    pub fn status_text(&self) -> &str {
        &self.status_text
    }

    /// The headers in arrival order, with lowercase names.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// The raw body bytes.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The decoded body text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The decoded body length; for a truncated body, the decoded bytes read before stopping.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Whether reading stopped at the size limit, so the body holds only its start.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// The time the whole exchange took.
    pub fn response_time(&self) -> Duration {
        self.response_time
    }

    /// The final URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The body as bruno-cli's `parseDataFromResponse` sees it: parsed JSON whatever the content
    /// type, else the text without a leading byte order mark.
    pub(crate) fn body_value(&self) -> Value {
        let text = self.text.strip_prefix('\u{feff}').unwrap_or(&self.text);
        serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()))
    }

    /// The headers as Node's `IncomingMessage.headers` merges repeats.
    pub(crate) fn headers_value(&self) -> Map<String, Value> {
        let mut merged = Map::new();
        for (name, value) in &self.headers {
            let name = name.as_str();
            match merged.get_mut(name) {
                None if name == "set-cookie" => {
                    merged.insert(
                        name.to_owned(),
                        Value::Array(vec![Value::String(value.clone())]),
                    );
                }
                None => {
                    merged.insert(name.to_owned(), Value::String(value.clone()));
                }
                Some(Value::Array(cookies)) => cookies.push(Value::String(value.clone())),
                Some(_) if SINGLE_VALUED.contains(&name) => {}
                Some(Value::String(joined)) => {
                    joined.push_str(if name == "cookie" { "; " } else { ", " });
                    joined.push_str(value);
                }
                Some(_) => {}
            }
        }
        merged
    }
}

// The URL query, headers such as Set-Cookie and the body can carry secrets, so Debug shows
// the redacted URL, header names and body sizes only, as ResolvedRequest's Debug does.
impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("status_text", &self.status_text)
            .field("headers", &names)
            .field("body_bytes", &self.body.len())
            .field("response_time", &self.response_time)
            .field("url", &redact_url(&self.url))
            .field("timeline", &self.timeline)
            .field("size", &self.size)
            .field("truncated", &self.truncated)
            .field("stream_end", &self.stream_end)
            .field("body_file", &self.body_file)
            .field("derived_credentials", &self.derived.len())
            .field("script_cookies", &self.scripted)
            .finish()
    }
}

/// Headers whose repeats Node drops, keeping the first value.
const SINGLE_VALUED: [&str; 18] = [
    "age",
    "authorization",
    "content-length",
    "content-type",
    "etag",
    "expires",
    "from",
    "host",
    "if-modified-since",
    "if-unmodified-since",
    "last-modified",
    "location",
    "max-forwards",
    "proxy-authorization",
    "referer",
    "retry-after",
    "server",
    "user-agent",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_builder_stores_what_it_is_given() {
        let res = Response::new(201)
            .with_status_text("Created")
            .with_header("X-Trace", "1")
            .with_body(vec![0xff, 0x41], "\u{fffd}A")
            .with_response_time(Duration::from_millis(84))
            .with_url("https://x.test/final");
        assert_eq!(res.status(), 201);
        assert_eq!(res.status_text(), "Created");
        assert_eq!(res.headers(), [("x-trace".to_owned(), "1".to_owned())]);
        assert_eq!(res.body(), [0xff, 0x41]);
        assert_eq!(res.text(), "\u{fffd}A");
        assert_eq!(res.response_time(), Duration::from_millis(84));
        assert_eq!(res.url(), "https://x.test/final");
        assert_eq!(
            Response::new(204).with_text_body("é").body(),
            "é".as_bytes()
        );
        let reset = Response::new(200).with_truncated(9).with_text_body("ab");
        assert_eq!((reset.truncated(), reset.size()), (false, 2));
    }

    #[test]
    fn debug_hides_the_query_values_and_header_values() {
        let res = Response::new(200)
            .with_header("set-cookie", "sid=SECRET")
            .with_text_body("token=SECRET")
            .with_url("https://x.test/a?api_key=SECRET");
        let shown = format!("{res:?}");
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(shown.contains("set-cookie"), "{shown}");
        assert!(shown.contains("https://x.test/a?api_key="), "{shown}");
    }

    #[test]
    fn the_body_is_json_when_it_parses_whatever_the_content_type() {
        let body = |text: &str| {
            Response::new(200)
                .with_header("content-type", "text/plain")
                .with_text_body(text)
                .body_value()
        };
        assert_eq!(body("{\"a\":[1,2]}"), json!({"a": [1, 2]}));
        assert_eq!(body("\u{feff}{\"a\":1}"), json!({"a": 1}));
        assert_eq!(body(" 42 "), json!(42));
        assert_eq!(body("\"s\""), json!("s"));
        assert_eq!(body("null"), json!(null));
        assert_eq!(body("not json"), json!("not json"));
        assert_eq!(body("\u{feff}plain"), json!("plain"));
        assert_eq!(body(""), json!(""));
        assert_eq!(body("{\"k\":1,\"k\":2}"), json!({"k": 2}));
        // JSON.parse accepts these, and nesting past serde_json's 128 levels; serde_json does not,
        // so they stay text (accepted divergence).
        assert_eq!(body("{\"n\":1e400}"), json!("{\"n\":1e400}"));
        assert_eq!(body("\"\\ud800\""), json!("\"\\ud800\""));
    }

    #[test]
    fn repeated_headers_merge_as_node_does() {
        // Node 24 with axios 1.18.0, probed against a raw socket server on 2026-10-09.
        let res = [
            ("Content-Type", "text/plain"),
            ("content-type", "application/json"),
            ("X-Multi", "a"),
            ("x-multi", "b"),
            ("Set-Cookie", "a=1"),
            ("Set-Cookie", "b=2"),
            ("Cookie", "c=1"),
            ("Cookie", "d=2"),
            ("Location", "/x"),
            ("Location", "/y"),
            ("X-Empty", ""),
            ("Date", "1"),
            ("Date", "2"),
            ("Server", "1"),
            ("Server", "2"),
        ]
        .into_iter()
        .fold(Response::new(200), |r, (k, v)| r.with_header(k, v));
        assert_eq!(
            Value::Object(res.headers_value()),
            json!({"content-type": "text/plain", "x-multi": "a, b", "set-cookie": ["a=1", "b=2"], "cookie": "c=1; d=2",
                   "location": "/x", "x-empty": "", "date": "1, 2", "server": "1"})
        );
        let one = Response::new(200).with_header("Set-Cookie", "only=1");
        assert_eq!(
            Value::Object(one.headers_value()),
            json!({"set-cookie": ["only=1"]})
        );
    }
}
