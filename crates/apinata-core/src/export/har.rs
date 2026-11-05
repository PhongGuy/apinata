//! Writing one sent request and its response as a HAR 1.2 file with a single entry.

use std::time::{Duration, SystemTime};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Map, Value, json};

use crate::response::Response;
use crate::secret_names::{NamedSecret, redact_wide, replace_secrets};
use crate::session_log::NetworkEntry;

/// Why a sent response cannot be written as HAR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HarRefusal {
    /// The response has no request attached, as one kept from an earlier session.
    #[error("the request is not kept with this response; send it again to export")]
    NoRequest,
    /// The body is binary and holds a secret that cannot be replaced inside it.
    #[error(
        "the response body is binary and holds a secret value that cannot be replaced inside it"
    )]
    BinarySecret,
}

/// The exchange `response` ended at `ended`, redacted against `secrets`, as every shell
/// exports a send: the request as sent, each secret and credential written `[redacted]`.
pub fn sent_exchange(
    response: &Response,
    ended: SystemTime,
    secrets: &[NamedSecret],
) -> Result<HarExchange, HarRefusal> {
    if response.timeline().is_empty() {
        return Err(HarRefusal::NoRequest);
    }
    let started = ended.checked_sub(response.response_time()).unwrap_or(ended);
    let exchange = HarExchange::from_response(response, started);
    if exchange.binary_body_holds_secret(secrets) {
        return Err(HarRefusal::BinarySecret);
    }
    Ok(exchange.redacted(secrets))
}

/// Everything one HAR entry holds: the request as sent and the response as received.
///
/// Fill it with [`HarExchange::from_response`] or [`HarExchange::from_network_entry`], call
/// [`HarExchange::redacted`] before writing, then render it with [`har`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HarExchange {
    /// When the send began.
    pub started: SystemTime,
    /// The request method.
    pub method: String,
    /// The request URL.
    pub url: String,
    /// The HTTP version as HAR words it (`HTTP/1.1`); empty when unknown.
    pub http_version: String,
    /// The request headers.
    pub request_headers: Vec<(String, String)>,
    /// The request body bytes.
    pub request_body: Vec<u8>,
    /// The response status.
    pub status: u16,
    /// The response reason phrase; empty when unknown.
    pub status_text: String,
    /// The response headers.
    pub response_headers: Vec<(String, String)>,
    /// The response body bytes as kept, which is what the app shows.
    pub response_body: Vec<u8>,
    /// Whether the response body was cut at the read limit.
    pub response_truncated: bool,
    /// The measured total time of the send.
    pub time: Duration,
    /// The server's IP as HAR's `serverIPAddress` words it: no port, no brackets around IPv6.
    /// `None` when unknown or when a proxy answered the connection, since the peer was then the
    /// proxy and not the server.
    pub server_ip: Option<String>,
    /// The host override the answering hop used, as `name -> address`; written as the
    /// `_hostOverride` member. `None` when DNS answered.
    pub override_from: Option<String>,
    /// This send's number and the most sends the request could make; written as the `_attempt`
    /// and `_attempts` members. `None` when the request was not retrying.
    pub attempt: Option<(u32, u32)>,
    /// The request body's full length when [`HarExchange::limited`] cut it.
    pub request_size: Option<u64>,
    /// The response body's full length when [`HarExchange::limited`] cut or left it out.
    pub response_size: Option<u64>,
    /// Notes written into the entry's `comment`, such as where a body was cut.
    pub notes: Vec<String>,
}

impl HarExchange {
    /// An exchange for `method` and `url` with everything else empty.
    pub fn new(method: impl Into<String>, url: impl Into<String>) -> HarExchange {
        HarExchange {
            started: SystemTime::now(),
            method: method.into(),
            url: url.into(),
            http_version: String::new(),
            request_headers: Vec::new(),
            request_body: Vec::new(),
            status: 0,
            status_text: String::new(),
            response_headers: Vec::new(),
            response_body: Vec::new(),
            response_truncated: false,
            time: Duration::ZERO,
            server_ip: None,
            override_from: None,
            attempt: None,
            request_size: None,
            response_size: None,
            notes: Vec::new(),
        }
    }

    /// The exchange `response` ended, started at `started`. The request side is its last hop,
    /// the one that was answered; a response without a timeline has no request to show.
    pub fn from_response(response: &Response, started: SystemTime) -> HarExchange {
        let mut x = HarExchange::new("", response.url());
        if let Some(hop) = response.timeline().last() {
            x.method = hop.method.clone();
            x.url = hop.url.clone();
            x.request_headers = hop.request_headers.clone();
            x.request_body = hop.request_body.clone();
            x.http_version = hop.version().map(|v| v.to_string()).unwrap_or_default();
            x.server_ip = hop
                .remote_addr()
                .filter(|_| !hop.via_proxy())
                .map(|a| a.ip().to_string());
            x.override_from = hop.override_from().map(str::to_owned);
        }
        x.started = started;
        x.status = response.status();
        x.status_text = response.status_text().to_owned();
        x.response_headers = response.headers().to_vec();
        x.response_body = response.body().to_vec();
        x.response_truncated = response.truncated();
        x.time = response.response_time();
        x
    }

    /// The exchange a session log entry recorded, or `None` for one that failed before a
    /// response. The log keeps bodies as capped, redacted text; a binary response body is
    /// exact only when the log kept its bytes.
    pub fn from_network_entry(entry: &NetworkEntry) -> Option<HarExchange> {
        let hop = entry.hops.last()?;
        let mut x = HarExchange::new(entry.method.clone(), entry.url.clone());
        x.started = entry.at;
        x.http_version = hop.version().map(|v| v.to_string()).unwrap_or_default();
        x.server_ip = hop
            .remote_addr()
            .filter(|_| !hop.via_proxy())
            .map(|a| a.ip().to_string());
        x.override_from = hop.override_from.clone();
        x.attempt = entry.attempt;
        x.request_headers = entry.request_headers.clone();
        x.request_body = entry.request_body.text.clone().into_bytes();
        x.status = entry.status?;
        // The executor words the status by its standard reason phrase, so the log need not keep it.
        x.status_text = http::StatusCode::from_u16(x.status)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or_default()
            .to_owned();
        x.response_headers = hop.response_headers.clone();
        x.response_body = match entry.response_body.raw() {
            Some(raw) => raw.to_vec(),
            None => entry.response_body.text.clone().into_bytes(),
        };
        x.response_truncated = entry.response_body.truncated;
        x.time = entry.duration;
        Some(x)
    }

    /// This exchange with every URL's userinfo dropped, every secret replaced by `[redacted]`
    /// in the URL, header values and bodies (the request as sent, never its template), and every header masked as
    /// [`crate::mask_response_header`] does (a Set-Cookie keeps its name only, a credential
    /// header or Cookie is hidden whole), so a value the server issued is gone although no known
    /// secret matches it. A request body that is not UTF-8 is redacted through a lossy copy,
    /// which the file holds as lossy text anyway. A binary response body is left as it is: check
    /// it first with [`HarExchange::binary_body_holds_secret`] and refuse to write when it does.
    pub fn redacted(mut self, secrets: &[NamedSecret]) -> HarExchange {
        let secrets: Vec<NamedSecret> = secrets.iter().map(NamedSecret::unnamed_copy).collect();
        for body in [&mut self.request_body, &mut self.response_body] {
            if let Some(wide) = redact_wide(body, secrets.iter().map(NamedSecret::value)) {
                *body = wide;
            }
        }
        let text = |s: &str| replace_secrets(s, &secrets);
        self.url = text(&crate::redact_url_userinfo(&self.url));
        for (_, value) in self
            .request_headers
            .iter_mut()
            .chain(&mut self.response_headers)
        {
            *value = text(value);
        }
        for (name, value) in self
            .request_headers
            .iter_mut()
            .chain(&mut self.response_headers)
        {
            *value = crate::mask_response_header(name, value);
        }
        let lossy = String::from_utf8_lossy(&self.request_body).into_owned();
        let clean = text(&lossy);
        if clean != lossy || std::str::from_utf8(&self.request_body).is_ok() {
            self.request_body = clean.into_bytes();
        }
        if let Ok(s) = std::str::from_utf8(&self.response_body) {
            self.response_body = text(s).into_bytes();
        }
        self
    }

    /// Whether the response body is not UTF-8 and a secret appears in it (read through a lossy
    /// copy), so writing it would leak a value that cannot be replaced inside the bytes. A
    /// UTF-16 copy does not count: [`HarExchange::redacted`] replaces it in place.
    pub fn binary_body_holds_secret(&self, secrets: &[NamedSecret]) -> bool {
        let wide = redact_wide(&self.response_body, secrets.iter().map(NamedSecret::value));
        let body = wide.as_deref().unwrap_or(&self.response_body);
        if std::str::from_utf8(body).is_ok() {
            return false;
        }
        crate::bytes_hold_secret(body, secrets)
    }

    /// This exchange with each body cut to `max` bytes (text at a character boundary), noting
    /// each cut. A binary response body gives its type and size only unless `keep_binary`.
    /// Cut after [`HarExchange::redacted`], so no cut can leave part of a secret behind.
    pub fn limited(mut self, max: usize, keep_binary: bool) -> HarExchange {
        let len = self.request_body.len();
        if len > max {
            self.request_body.truncate(cut_at(&self.request_body, max));
            self.request_size = Some(len as u64);
            let kept = self.request_body.len();
            self.notes
                .push(format!("The request body is cut to {kept} bytes of {len}."));
        }
        let len = self.response_body.len();
        if is_binary(&self.response_body) && !keep_binary {
            self.response_body.clear();
            self.response_size = Some(len as u64);
            self.notes.push(format!(
                "The binary response body ({len} bytes) is left out; ask for more body bytes to include it."
            ));
        } else if len > max {
            self.response_body
                .truncate(cut_at(&self.response_body, max));
            self.response_size = Some(len as u64);
            let kept = self.response_body.len();
            self.notes.push(format!(
                "The response body is cut to {kept} bytes of {len}."
            ));
        }
        self
    }
}

/// Where to cut `bytes` at most `max` long: text at a character boundary, binary anywhere.
fn cut_at(bytes: &[u8], max: usize) -> usize {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.floor_char_boundary(max),
        Err(_) => max.min(bytes.len()),
    }
}

// A body that is valid text without NUL bytes is written as text; anything else is base64.
fn is_binary(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_err() || bytes.contains(&0)
}

/// `exchange` as a pretty-printed HAR 1.2 document.
///
/// Only the total time is measured, so it fills `time` and the `wait` phase; `send` and
/// `receive`, which HAR requires, are 0 and the optional phases -1.
pub fn har(exchange: &HarExchange) -> String {
    har_log(std::slice::from_ref(exchange))
}

/// `exchanges` as one pretty-printed HAR 1.2 document with an entry each, in order.
pub fn har_log(exchanges: &[HarExchange]) -> String {
    let entries: Vec<Value> = exchanges.iter().map(entry).collect();
    let log = json!({
        "version": "1.2",
        "creator": {"name": "Apinata", "version": env!("CARGO_PKG_VERSION")},
        "entries": entries,
    });
    let mut out = serde_json::to_string_pretty(&json!({ "log": log })).unwrap_or_default();
    out.push('\n');
    out
}

fn entry(x: &HarExchange) -> Value {
    let ms = (x.time.as_secs_f64() * 1000.0 * 1000.0).round() / 1000.0;
    let mut entry = Map::new();
    entry.insert("startedDateTime".into(), started(x.started).into());
    entry.insert("time".into(), json!(ms));
    entry.insert("request".into(), request(x));
    entry.insert("response".into(), response(x));
    entry.insert("cache".into(), json!({}));
    entry.insert(
        "timings".into(),
        json!({"blocked": -1, "dns": -1, "connect": -1, "send": 0, "wait": ms, "receive": 0, "ssl": -1}),
    );
    if let Some(ip) = &x.server_ip {
        entry.insert("serverIPAddress".into(), ip.clone().into());
    }
    if let Some(label) = &x.override_from {
        entry.insert("_hostOverride".into(), label.clone().into());
    }
    // HAR 1.2 allows custom entry fields that start with an underscore.
    if let Some((attempt, of)) = x.attempt {
        entry.insert("_attempt".into(), attempt.into());
        entry.insert("_attempts".into(), of.into());
    }
    let mut notes = Vec::new();
    if x.response_truncated {
        notes.push(format!(
            "The response body is truncated to {} bytes at the read limit, as the app shows it.",
            x.response_size.unwrap_or(x.response_body.len() as u64)
        ));
    }
    notes.extend(x.notes.iter().cloned());
    if !notes.is_empty() {
        entry.insert("comment".into(), notes.join(" ").into());
    }
    Value::Object(entry)
}

fn started(at: SystemTime) -> String {
    DateTime::<Utc>::from(at).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn pairs(list: &[(String, String)]) -> Value {
    list.iter()
        .map(|(n, v)| json!({"name": n, "value": v}))
        .collect()
}

fn header<'a>(list: &'a [(String, String)], name: &str) -> Option<&'a str> {
    list.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn version(x: &HarExchange) -> &str {
    if x.http_version.is_empty() {
        "HTTP/1.1"
    } else {
        &x.http_version
    }
}

// A Cookie header holds `a=1; b=2`; HAR wants one object per pair.
fn request_cookies(headers: &[(String, String)]) -> Value {
    let jar = headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("cookie"))
        .flat_map(|(_, v)| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect::<Vec<_>>();
    pairs(&jar)
}

// Each Set-Cookie line starts with its `name=value`; attributes are not kept.
fn response_cookies(headers: &[(String, String)]) -> Value {
    let jar = headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("set-cookie"))
        .filter_map(|(_, v)| v.split(';').next()?.trim().split_once('='))
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect::<Vec<_>>();
    pairs(&jar)
}

fn query(url: &str) -> Value {
    match url::Url::parse(url) {
        Ok(u) => u
            .query_pairs()
            .map(|(n, v)| json!({"name": n, "value": v}))
            .collect(),
        Err(_) => json!([]),
    }
}

fn request(x: &HarExchange) -> Value {
    let mut r = Map::new();
    r.insert("method".into(), x.method.clone().into());
    r.insert("url".into(), x.url.clone().into());
    r.insert("httpVersion".into(), version(x).into());
    r.insert("cookies".into(), request_cookies(&x.request_headers));
    r.insert("headers".into(), pairs(&x.request_headers));
    r.insert("queryString".into(), query(&x.url));
    if !x.request_body.is_empty() {
        let mime = header(&x.request_headers, "content-type").unwrap_or_default();
        let text = String::from_utf8_lossy(&x.request_body);
        r.insert("postData".into(), json!({"mimeType": mime, "text": text}));
    }
    r.insert("headersSize".into(), json!(-1));
    let size = x.request_size.unwrap_or(x.request_body.len() as u64);
    r.insert("bodySize".into(), json!(size));
    Value::Object(r)
}

fn content(x: &HarExchange) -> Value {
    let mime = header(&x.response_headers, "content-type").unwrap_or_default();
    let mut c = Map::new();
    c.insert("size".into(), json!(response_size(x)));
    c.insert("mimeType".into(), mime.into());
    if is_binary(&x.response_body) {
        c.insert("text".into(), STANDARD.encode(&x.response_body).into());
        c.insert("encoding".into(), "base64".into());
    } else if !x.response_body.is_empty() {
        let text = String::from_utf8_lossy(&x.response_body);
        c.insert("text".into(), text.into_owned().into());
    }
    Value::Object(c)
}

fn response_size(x: &HarExchange) -> u64 {
    x.response_size.unwrap_or(x.response_body.len() as u64)
}

fn response(x: &HarExchange) -> Value {
    json!({
        "status": x.status,
        "statusText": x.status_text,
        "httpVersion": version(x),
        "cookies": response_cookies(&x.response_headers),
        "headers": pairs(&x.response_headers),
        "content": content(x),
        "redirectURL": header(&x.response_headers, "location").unwrap_or_default(),
        "headersSize": -1,
        "bodySize": response_size(x),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_secret_is_written_as_sent_redacted_not_as_its_placeholder() {
        let secret = NamedSecret::variable("process.env.apikey", "Key-V4lue-123");
        let mut x = HarExchange::new("GET", "https://api.test/?k=Key-V4lue-123");
        x.request_headers = vec![("x-key".into(), "Bearer Key-V4lue-123".into())];
        let text = har(&x.redacted(&[secret]));
        assert!(!text.contains("Key-V4lue-123"), "{text}");
        assert!(!text.contains("{{process.env.apikey}}"), "{text}");
        assert!(text.contains("\"Bearer [redacted]\""), "{text}");
    }
}
