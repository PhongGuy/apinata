//! The Python targets: runnable scripts built on `requests` or the standard library's
//! `http.client` that print the status and body.

use url::Url;

use crate::execute::{form_payload, url_credentials};
use crate::multipart::PartValue;
use crate::resolve::{Body, ResolvedRequest};

use super::{
    BOUNDARY, Segment, base_name, digest_credentials, multipart_segments, omitted_auth,
    sent_headers,
};

/// `text` as a single-quoted Python literal. Line breaks, tabs and other control characters
/// are escaped so the literal stays on one line; non-ASCII is kept, since Python source is UTF-8.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// A string literal sent as UTF-8 bytes: `requests` would otherwise encode a `str` body or
/// header value as Latin-1, failing or mangling anything outside it.
fn utf8(text: &str) -> String {
    format!("{}.encode('utf-8')", literal(text))
}

/// A script sending the request with `requests`. Multipart parts all go in `files`, text
/// parts with no filename, so part order is kept and a text-only form is still multipart.
pub(super) fn requests(req: &ResolvedRequest) -> String {
    let mut out = String::new();
    if let Some(note) = omitted_auth(req, true) {
        out.push_str(&format!("# {note}\n"));
    }
    out.push_str("import requests\n");
    let digest = digest_credentials(req);
    if digest.is_some() {
        out.push_str("from requests.auth import HTTPDigestAuth\n");
    }
    out.push_str(&format!("\nurl = {}\n", literal(req.url())));

    let mut args = vec![literal(req.method()), "url".to_string()];
    let headers: Vec<_> = sent_headers(req).collect();
    if !headers.is_empty() {
        out.push_str("headers = {\n");
        for (name, value) in headers {
            let value = if value.is_ascii() {
                literal(value)
            } else {
                utf8(value)
            };
            out.push_str(&format!("    {}: {value},\n", literal(name)));
        }
        out.push_str("}\n");
        args.push("headers=headers".to_string());
    }

    match req.body() {
        Body::None => {}
        Body::Text(text) => {
            out.push_str(&format!("data = {}\n", utf8(text)));
            args.push("data=data".to_string());
        }
        Body::FormUrlEncoded(pairs) => {
            out.push_str(&format!("data = {}\n", utf8(&form_payload(pairs))));
            args.push("data=data".to_string());
        }
        Body::Multipart(parts) => {
            out.push_str("files = [\n");
            for part in parts {
                let mut file = match part.value() {
                    PartValue::Text(text) => format!("None, {}", literal(text)),
                    PartValue::File(path) => format!(
                        "{}, open({}, 'rb')",
                        literal(&base_name(path)),
                        literal(&path.to_string_lossy())
                    ),
                };
                if let Some(ct) = part.content_type() {
                    file.push_str(&format!(", {}", literal(ct)));
                }
                out.push_str(&format!("    ({}, ({file})),\n", literal(part.name())));
            }
            out.push_str("]\n");
            args.push("files=files".to_string());
        }
        Body::File { path, .. } => {
            out.push_str(&format!(
                "data = open({}, 'rb')\n",
                literal(&path.to_string_lossy())
            ));
            args.push("data=data".to_string());
        }
    }
    if !req.follow_redirects() {
        args.push("allow_redirects=False".to_string());
    }
    if let Some((user, password)) = digest {
        args.push(format!(
            "auth=HTTPDigestAuth({}, {})",
            literal(user),
            literal(password)
        ));
    }

    out.push_str(&format!(
        "\nresponse = requests.request({})\nprint(response.status_code)\nprint(response.text)\n",
        args.join(", ")
    ));
    out
}

/// A file's bytes, read when the script runs.
fn read_bytes(path: &std::path::Path) -> String {
    format!("Path({}).read_bytes()", literal(&path.to_string_lossy()))
}

/// A script sending the request with `http.client`. The URL is split here into the connection
/// and the request target, since `http.client` takes them apart and refuses a raw space or
/// non-ASCII. Every body goes out as bytes with a Content-Length; multipart is built by hand.
pub(super) fn http_client(req: &ResolvedRequest) -> String {
    let mut out = String::new();
    if let Some(note) = omitted_auth(req, false) {
        out.push_str(&format!("# {note}\n"));
    }
    if req.follow_redirects() {
        out.push_str("# http.client does not follow redirects; this prints the first response.\n");
    }
    let url = Url::parse(req.url()).ok();
    if url.is_none() {
        out.push_str("# The URL does not parse, so it is sent whole as the request target.\n");
    }
    let has = |header: &str| sent_headers(req).any(|(n, _)| n.eq_ignore_ascii_case(header));
    // URL credentials become the Basic header the executor sends, encoded when the script runs
    // so the user and password stay readable.
    let credentials = url
        .as_ref()
        .and_then(url_credentials)
        .filter(|_| !has("authorization"));
    let reads_file = match req.body() {
        Body::File { .. } => true,
        Body::Multipart(parts) => parts
            .iter()
            .any(|p| matches!(p.value(), PartValue::File(_))),
        _ => false,
    };

    if credentials.is_some() {
        out.push_str("import base64\n");
    }
    out.push_str("import http.client\n");
    if reads_file {
        out.push_str("from pathlib import Path\n");
    }

    let (class, host, port, target) = match &url {
        Some(url) => {
            let class = if url.scheme() == "https" {
                "HTTPSConnection"
            } else {
                "HTTPConnection"
            };
            let target = match url.query() {
                Some(query) => format!("{}?{query}", url.path()),
                None => url.path().to_string(),
            };
            (
                class,
                url.host_str().unwrap_or_default(),
                url.port(),
                target,
            )
        }
        None => ("HTTPConnection", "", None, req.url().to_string()),
    };
    let port = port.map(|p| format!(", {p}")).unwrap_or_default();
    out.push_str(&format!(
        "\nconn = http.client.{class}({}{port})\n",
        literal(host)
    ));

    let mut headers: Vec<(String, String)> = sent_headers(req)
        .map(|(name, value)| {
            let value = if value.is_ascii() {
                literal(value)
            } else {
                utf8(value)
            };
            (literal(name), value)
        })
        .collect();
    if let Some((user, password)) = credentials {
        let pair = format!("{user}:{}", password.unwrap_or_default());
        headers.push((
            literal("Authorization"),
            format!(
                "'Basic ' + base64.b64encode({}).decode('ascii')",
                utf8(&pair)
            ),
        ));
    }
    if matches!(req.body(), Body::Multipart(_)) {
        headers.push((
            literal("Content-Type"),
            literal(&format!("multipart/form-data; boundary={BOUNDARY}")),
        ));
    }
    if headers.is_empty() {
        out.push_str("headers = {}\n");
    } else {
        out.push_str("headers = {\n");
        for (name, value) in headers {
            out.push_str(&format!("    {name}: {value},\n"));
        }
        out.push_str("}\n");
    }

    let body = match req.body() {
        Body::None => None,
        Body::Text(text) => Some(utf8(text)),
        Body::FormUrlEncoded(pairs) => Some(utf8(&form_payload(pairs))),
        Body::File { path, .. } => Some(read_bytes(path)),
        Body::Multipart(parts) => {
            let mut joined = String::from("b''.join([\n");
            for segment in multipart_segments(parts) {
                let item = match segment {
                    Segment::Text(text) => utf8(&text),
                    Segment::File(path) => read_bytes(path),
                };
                joined.push_str(&format!("    {item},\n"));
            }
            joined.push_str("])");
            Some(joined)
        }
    };
    let body_arg = match body {
        Some(body) => {
            out.push_str(&format!("body = {body}\n"));
            "body=body, "
        }
        None => "",
    };

    out.push_str(&format!(
        "\nconn.request({}, {}, {body_arg}headers=headers)\nresponse = conn.getresponse()\n\
         print(response.status)\nprint(response.read().decode('utf-8', 'replace'))\n",
        literal(req.method()),
        literal(&target)
    ));
    out
}
