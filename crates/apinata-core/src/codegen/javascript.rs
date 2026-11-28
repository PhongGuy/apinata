//! The JavaScript targets: Node ES modules sending the request with `fetch`, `axios` or the
//! built-in `http` module and printing the status and body.

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
    // Node's `manual` hands back the 3xx response itself, unlike a browser's opaque one.
    let redirect = if req.follow_redirects() {
        ""
    } else {
        ", redirect: 'manual'"
    };
    out.push_str(&format!(
        "\nconst response = await fetch(url, {{ method: {}, headers{body}{redirect} }});\n\
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
    if !req.follow_redirects() {
        out.push_str("  maxRedirects: 0,\n");
    }
    out.push_str(
        "  transformRequest: (d) => d,\n  transformResponse: (r) => r,\n  validateStatus: () => true,\n});\n\
         console.log(response.status);\nconsole.log(response.data);\n",
    );
    out
}

/// A module sending the request with Node's `http` or `https` module, picked by the scheme.
/// The body goes to `end` whole, so Node sends a Content-Length; multipart is built by hand.
/// Node passes URL credentials on as Basic auth itself.
pub(super) fn node_http(req: &ResolvedRequest) -> String {
    let mut out = String::new();
    if let Some(note) = omitted_auth(req, false) {
        out.push_str(&format!("// {note}\n"));
    }
    if req.follow_redirects() {
        out.push_str(
            "// Node's http module does not follow redirects; this prints the first response.\n",
        );
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
    let secure = req
        .url()
        .get(..6)
        .is_some_and(|s| s.eq_ignore_ascii_case("https:"));
    let module = if secure { "https" } else { "http" };
    out.push_str(&format!("import {module} from 'node:{module}';\n\n"));

    out.push_str(&format!("const url = {};\n", literal(req.url())));
    let mut headers: Vec<(String, String)> = sent_headers(req)
        .map(|(name, value)| (literal(name), header_value(value)))
        .collect();
    if matches!(req.body(), Body::Multipart(_)) {
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
    let read =
        |path: &std::path::Path| format!("fs.readFileSync({})", literal(&path.to_string_lossy()));
    let body = match req.body() {
        Body::None => None,
        Body::Text(text) => Some(literal(text)),
        Body::FormUrlEncoded(pairs) => Some(literal(&form_payload(pairs))),
        Body::File { path, .. } => Some(read(path)),
        Body::Multipart(parts) => {
            let mut joined = String::from("Buffer.concat([\n");
            for segment in multipart_segments(parts) {
                let item = match segment {
                    Segment::Text(text) => format!("Buffer.from({})", literal(&text)),
                    Segment::File(path) => read(path),
                };
                joined.push_str(&format!("  {item},\n"));
            }
            joined.push_str("])");
            Some(joined)
        }
    };
    let end = match body {
        Some(body) => {
            out.push_str(&format!("const body = {body};\n"));
            "body"
        }
        None => "",
    };

    out.push_str(&format!(
        "\nconst req = {module}.request(url, {{ method: {}, headers }}, (res) => {{\n\
         \x20 console.log(res.statusCode);\n\
         \x20 const chunks = [];\n\
         \x20 res.on('data', (chunk) => chunks.push(chunk));\n\
         \x20 res.on('end', () => console.log(Buffer.concat(chunks).toString()));\n\
         }});\nreq.end({end});\n",
        literal(req.method())
    ));
    out
}
