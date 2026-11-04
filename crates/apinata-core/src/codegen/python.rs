//! The Python target: a runnable script built on `requests` that prints the status and body.

use crate::execute::form_payload;
use crate::multipart::PartValue;
use crate::resolve::{Body, ResolvedRequest};

use super::{base_name, digest_credentials, omitted_auth, sent_headers};

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
