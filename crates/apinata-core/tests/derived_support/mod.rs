//! Echo servers, auth blocks and wire checks shared by the derived-credential tests.
#![allow(dead_code)] // each test file uses a different subset

use apinata_core::Response;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use wiremock::{Request, Respond, ResponseTemplate};

const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The user-supplied secrets inside the blocks below; a derived value never holds one.
pub const USER_SECRETS: [&str; 6] = [
    "cs-1",
    "ts-1",
    "secret-key",
    "wsse-pass-1",
    "eg-secret-1",
    "digest-pass-1",
];

pub const SIGV4: &str = "  auth:\n    type: awsv4\n    accessKeyId: AKIDTEST\n    secretAccessKey: secret-key\n    sessionToken: \"\"\n    service: execute-api\n    region: eu-west-1\n    profileName: \"\"\n";
pub const WSSE: &str = "  auth:\n    type: wsse\n    username: bob\n    password: wsse-pass-1\n";
pub const DIGEST: &str =
    "  auth:\n    type: digest\n    username: bob\n    password: digest-pass-1\n";
pub const EDGEGRID: &str = "  auth:\n    type: akamai-edgegrid\n    accessToken: akab-access-1\n    clientToken: akab-client-1\n    clientSecret: eg-secret-1\n    headersToSign: \"content-type,user-agent,accept\"\n";

pub fn oauth1(placement: &str) -> String {
    format!(
        "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: cs-1\n    accessToken: tk\n    accessTokenSecret: ts-1\n    signatureMethod: HMAC-SHA1\n    placement: {placement}\n"
    )
}

pub fn enc(s: &str) -> String {
    utf8_percent_encode(s, UNRESERVED).to_string()
}

/// Every hop's url, request headers and request body, one per line.
pub fn sent_text(res: &Response) -> String {
    let mut lines = Vec::new();
    for hop in res.timeline() {
        lines.push(hop.url.clone());
        lines.push(String::from_utf8_lossy(&hop.request_body).into_owned());
        lines.extend(hop.request_headers.iter().map(|(n, v)| format!("{n}: {v}")));
    }
    lines.join("\n")
}

/// Whether `value` went out as sent, raw, percent-encoded or form-encoded.
pub fn on_wire(res: &Response, value: &str) -> bool {
    let text = sent_text(res);
    let form: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
    [value.to_owned(), enc(value), form]
        .iter()
        .any(|form| text.contains(form.as_str()))
}

fn seen(req: &Request, name: &str) -> String {
    req.headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

/// Answers 200 with `<authorization> | <x-wsse> | <url>` as the body.
pub struct Echo;

impl Respond for Echo {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_string(format!(
            "{} | {} | {}",
            seen(req, "authorization"),
            seen(req, "x-wsse"),
            req.url
        ))
    }
}

/// `Echo`, but a digest challenge (401) while the request carries no credential header.
pub struct Challenge;

impl Respond for Challenge {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        if seen(req, "authorization").is_empty() && seen(req, "x-wsse").is_empty() {
            return ResponseTemplate::new(401).insert_header(
                "www-authenticate",
                r#"Digest realm="r", nonce="n0", qop="auth", opaque="op""#,
            );
        }
        Echo.respond(req)
    }
}
