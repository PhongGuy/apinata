//! The JavaScript targets: Node ES modules sending the request with `fetch` or `axios` and
//! printing the status and body.

use crate::execute::form_payload;
use crate::multipart::PartValue;
use crate::resolve::{Body, ResolvedRequest};

use super::{
    BOUNDARY, Segment, base_name, has_typed_text, multipart_segments, omitted_auth, sent_headers,
};

/// `text` as a single-quoted JavaScript literal. Control characters and the line separators
/// U+2028 and U+2029 are escaped, so the literal stays on one line in any editor.
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
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// A header value literal. Node writes header strings as Latin-1, so a non-ASCII value is
/// re-encoded to make its UTF-8 bytes go out unchanged.
fn header_value(value: &str) -> String {
    if value.is_ascii() {
        literal(value)
    } else {
        format!("Buffer.from({}).toString('latin1')", literal(value))
    }
}

/// The module up to the send call: the omitted-auth line, imports, and the `url`, `headers`
/// and `body_name` constants. Returns it with whether the request has a body.
fn prelude(req: &ResolvedRequest, import: Option<&str>, body_name: &str) -> (String, bool) {
    let mut out = String::new();
    if let Some(note) = omitted_auth(req, false) {
        out.push_str(&format!("// {note}\n"));
    }
    let reads_file = match req.body() {
        Body::File { .. } => true,
        Body::Multipart(parts) => parts
            .iter()
            .any(|p| matches!(p.value(), PartValue::File(_))),
        _ => false,
    };
    if reads_file {
        out.push_str("import fs from 'node:fs';\n");
    }
    if let Some(import) = import {
        out.push_str(import);
        out.push('\n');
    }
    if reads_file || import.is_some() {
        out.push('\n');
    }

    // FormData cannot type a text part without making it a file part, so a typed text part
    // makes the module write the whole body, and with it the boundary header.
    let hand_built = matches!(req.body(), Body::Multipart(parts) if has_typed_text(parts));
    out.push_str(&format!("const url = {};\n", literal(req.url())));
    let mut headers: Vec<(String, String)> = sent_headers(req)
        .map(|(name, value)| (literal(name), header_value(value)))
        .collect();
    if hand_built {
        headers.push((
            literal("Content-Type"),
            literal(&format!("multipart/form-data; boundary={BOUNDARY}")),
        ));
    }
    if headers.is_empty() {
        out.push_str("const headers = {};\n");
    } else {
        out.push_str("const headers = {\n");
        for (name, value) in headers {
            out.push_str(&format!("  {name}: {value},\n"));
        }
        out.push_str("};\n");
    }

    let has_body = match req.body() {
        Body::None => false,
        Body::Text(text) => {
            out.push_str(&format!("const {body_name} = {};\n", literal(text)));
            true
        }
        Body::FormUrlEncoded(pairs) => {
            out.push_str(&format!(
                "const {body_name} = {};\n",
                literal(&form_payload(pairs))
            ));
            true
        }
        Body::Multipart(parts) if hand_built => {
            out.push_str(&format!("const {body_name} = Buffer.concat([\n"));
            for segment in multipart_segments(parts) {
                let item = match segment {
                    Segment::Text(text) => format!("Buffer.from({})", literal(&text)),
                    Segment::File(path) => {
                        format!("fs.readFileSync({})", literal(&path.to_string_lossy()))
                    }
                };
                out.push_str(&format!("  {item},\n"));
            }
            out.push_str("]);\n");
            true
        }
        Body::Multipart(parts) => {
            out.push_str("const form = new FormData();\n");
            for part in parts {
                let value = match part.value() {
                    PartValue::Text(text) => literal(text),
                    PartValue::File(path) => {
                        let read = format!("fs.readFileSync({})", literal(&path.to_string_lossy()));
                        let blob = match part.content_type() {
                            Some(ct) => format!("new Blob([{read}], {{ type: {} }})", literal(ct)),
                            None => format!("new Blob([{read}])"),
                        };
                        format!("{blob}, {}", literal(&base_name(path)))
                    }
                };
                out.push_str(&format!(
                    "form.append({}, {value});\n",
                    literal(part.name())
                ));
            }
            out.push_str(&format!("const {body_name} = form;\n"));
            true
        }
        Body::File { path, .. } => {
            out.push_str(&format!(
                "const {body_name} = fs.readFileSync({});\n",
                literal(&path.to_string_lossy())
            ));
            true
        }
    };
    (out, has_body)
}

/// A module sending the request with the built-in `fetch`.
pub(super) fn fetch(req: &ResolvedRequest) -> String {
    let (mut out, has_body) = prelude(req, None, "body");
    let body = if has_body { ", body" } else { "" };
    out.push_str(&format!(
        "\nconst response = await fetch(url, {{ method: {}, headers{body} }});\n\
         console.log(response.status);\nconsole.log(await response.text());\n",
        literal(req.method())
    ));
    out
}

/// A module sending the request with `axios`. Both transforms are the identity, so the body
/// goes out as written and the response prints as text, and any status resolves.
pub(super) fn axios(req: &ResolvedRequest) -> String {
    let (mut out, has_body) = prelude(req, Some("import axios from 'axios';"), "data");
    out.push_str("\nconst response = await axios.request({\n");
    out.push_str(&format!(
        "  method: {},\n  url,\n  headers,\n",
        literal(req.method())
    ));
    if has_body {
        out.push_str("  data,\n");
    }
    out.push_str(
        "  transformRequest: (d) => d,\n  transformResponse: (r) => r,\n  validateStatus: () => true,\n});\n\
         console.log(response.status);\nconsole.log(response.data);\n",
    );
    out
}
