//! The raw HTTP target: the HTTP/1.1 message as it goes on the wire, for bug reports and
//! `.http` files. The head ends its lines in CRLF and the body is written as is; notes are
//! `#` lines before the request line.

use url::Url;

use crate::execute::{form_payload, url_credentials};
use crate::resolve::{Body, ResolvedRequest};

use super::{BOUNDARY, Segment, multipart_segments, omitted_auth, sent_headers};

/// Whether `text` is an RFC 9110 token, the grammar of methods and header names.
fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// The body as written, or `None` when it reads a file, which a pure generator cannot inline;
/// then each file stands as a `< path` line, as in `.http` files.
fn body_text(req: &ResolvedRequest) -> (Option<String>, Option<String>) {
    match req.body() {
        Body::None => (None, None),
        Body::Text(text) => (Some(text.clone()), None),
        Body::FormUrlEncoded(pairs) => (Some(form_payload(pairs)), None),
        Body::File { path, .. } => (None, Some(format!("< {}", path.to_string_lossy()))),
        Body::Multipart(parts) => {
            let mut text = String::new();
            let mut reads_file = false;
            for segment in multipart_segments(parts) {
                match segment {
                    Segment::Text(t) => text.push_str(&t),
                    Segment::File(path) => {
                        reads_file = true;
                        text.push_str(&format!("< {}", path.to_string_lossy()));
                    }
                }
            }
            if reads_file {
                (None, Some(text))
            } else {
                (Some(text), None)
            }
        }
    }
}

/// The request as an HTTP/1.1 message. `Host` comes first unless the request sets one, URL
/// credentials become the Basic header the client sends, and `Content-Length` counts the body
/// in UTF-8 bytes. A header that could not go on the wire as one line is left out and named.
pub(super) fn message(req: &ResolvedRequest) -> String {
    let mut notes = Vec::new();
    if let Some(note) = omitted_auth(req, false) {
        notes.push(note.to_string());
    }
    let url = Url::parse(req.url()).ok();
    let target = match &url {
        Some(url) => match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_string(),
        },
        None => req.url().to_string(),
    };
    let method = if is_token(req.method()) {
        req.method().to_string()
    } else {
        notes.push("The method is not a valid HTTP token.".to_string());
        req.method().escape_debug().to_string()
    };

    let (sent, forged): (Vec<_>, Vec<_>) = sent_headers(req)
        .partition(|(name, value)| is_token(name) && !value.contains(['\r', '\n', '\0']));
    if !forged.is_empty() {
        let names: Vec<String> = forged
            .iter()
            .map(|(name, _)| name.escape_debug().to_string())
            .collect();
        notes.push(format!(
            "Left out, since a line break in a header would end it early: {}.",
            names.join(", ")
        ));
    }
    let has = |header: &str| sent.iter().any(|(n, _)| n.eq_ignore_ascii_case(header));

    let mut head = vec![format!("{method} {target} HTTP/1.1")];
    if let Some(url) = &url
        && !has("host")
        && let Some(host) = url.host_str()
    {
        match url.port() {
            Some(port) => head.push(format!("Host: {host}:{port}")),
            None => head.push(format!("Host: {host}")),
        }
    }
    if let Some((user, password)) = url.as_ref().and_then(url_credentials)
        && !has("authorization")
    {
        use base64::Engine;
        let pair = format!("{user}:{}", password.unwrap_or_default());
        let encoded = base64::engine::general_purpose::STANDARD.encode(pair);
        head.push(format!("Authorization: Basic {encoded}"));
    }
    for (name, value) in &sent {
        head.push(format!("{name}: {value}"));
    }
    if matches!(req.body(), Body::Multipart(_)) {
        head.push(format!(
            "Content-Type: multipart/form-data; boundary={BOUNDARY}"
        ));
    }
    let (inline, with_files) = body_text(req);
    if let Some(body) = &inline
        && !has("content-length")
    {
        head.push(format!("Content-Length: {}", body.len()));
    }
    if with_files.is_some() {
        notes.push(
            "Each `< path` line stands for that file's bytes, so Content-Length is left out."
                .to_string(),
        );
    }

    let mut out = String::new();
    for note in notes {
        out.push_str(&format!("# {note}\n"));
    }
    out.push_str(&head.join("\r\n"));
    out.push_str("\r\n\r\n");
    out.push_str(&inline.or(with_files).unwrap_or_default());
    out
}
