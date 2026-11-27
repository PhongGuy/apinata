//! The Kotlin target: a `main` sending the request with OkHttp 4 or newer and printing the
//! status and body. Bodies carry no media type of their own, so the request's Content-Type
//! header is the one sent.

use crate::execute::form_payload;
use crate::multipart::PartValue;
use crate::resolve::{Body, ResolvedRequest};

use super::{base_name, omitted_auth, part_type, sent_headers, strict_url};

/// Methods OkHttp refuses to send with a body, and methods it refuses to send without one.
const NO_BODY: [&str; 2] = ["GET", "HEAD"];
const NEEDS_BODY: [&str; 5] = ["POST", "PUT", "PATCH", "PROPPATCH", "REPORT"];

/// `text` as a Kotlin string literal. Every `$` is escaped, since `$name` interpolates; Kotlin
/// has no `\x`, so other controls and the line separators use the fixed-width `\uXXXX`.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            c if (c as u32) < 0x20 || c == '\u{7f}' || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A file as a request body, typed `media` when given.
fn file_body(path: &std::path::Path, media: Option<&str>) -> String {
    let media = match media {
        Some(ct) => format!("{}.toMediaType()", literal(ct)),
        None => "null".to_string(),
    };
    format!(
        "File({}).asRequestBody({media})",
        literal(&path.to_string_lossy())
    )
}

/// A `main` building one `Request` and sending it with a client that follows redirects unless
/// the request does not. Multipart goes through `MultipartBody`, which writes each part's type.
pub(super) fn ok_http(req: &ResolvedRequest) -> String {
    let mut notes = Vec::new();
    if let Some(note) = omitted_auth(req, false) {
        notes.push(note.to_string());
    }
    let method = req.method();
    let mut imports = vec!["okhttp3.OkHttpClient", "okhttp3.Request"];
    let mut setup = String::new();

    let body = match req.body() {
        _ if NO_BODY.contains(&method) && !matches!(req.body(), Body::None) => {
            notes.push(format!(
                "OkHttp sends no body with {method}; the body is left out."
            ));
            None
        }
        Body::None if NEEDS_BODY.contains(&method) => {
            imports.push("okhttp3.RequestBody.Companion.toRequestBody");
            Some("ByteArray(0).toRequestBody(null)".to_string())
        }
        Body::None => None,
        Body::Text(text) => {
            imports.push("okhttp3.RequestBody.Companion.toRequestBody");
            Some(format!("{}.toRequestBody(null)", literal(text)))
        }
        Body::FormUrlEncoded(pairs) => {
            imports.push("okhttp3.RequestBody.Companion.toRequestBody");
            Some(format!(
                "{}.toRequestBody(null)",
                literal(&form_payload(pairs))
            ))
        }
        Body::File { path, .. } => {
            imports.push("java.io.File");
            imports.push("okhttp3.RequestBody.Companion.asRequestBody");
            Some(file_body(path, None))
        }
        Body::Multipart(parts) => {
            imports.push("okhttp3.MultipartBody");
            setup.push_str(
                "    val body = MultipartBody.Builder()\n        .setType(MultipartBody.FORM)\n",
            );
            for part in parts {
                let (file_name, value) = match part.value() {
                    PartValue::Text(text) => {
                        imports.push("okhttp3.RequestBody.Companion.toRequestBody");
                        let media = match part.content_type() {
                            Some(ct) => {
                                imports.push("okhttp3.MediaType.Companion.toMediaType");
                                format!("{}.toMediaType()", literal(ct))
                            }
                            None => "null".to_string(),
                        };
                        (
                            "null".to_string(),
                            format!("{}.toRequestBody({media})", literal(text)),
                        )
                    }
                    PartValue::File(path) => {
                        imports.push("java.io.File");
                        imports.push("okhttp3.MediaType.Companion.toMediaType");
                        imports.push("okhttp3.RequestBody.Companion.asRequestBody");
                        (literal(&base_name(path)), file_body(path, part_type(part)))
                    }
                };
                setup.push_str(&format!(
                    "        .addPart(MultipartBody.Part.createFormData({}, {file_name}, {value}))\n",
                    literal(part.name())
                ));
            }
            setup.push_str("        .build()\n");
            Some("body".to_string())
        }
    };

    let headers: Vec<_> = sent_headers(req).collect();
    if !headers.is_empty() {
        imports.push("okhttp3.Headers");
        setup.push_str("    val headers = Headers.Builder()\n");
        for (name, value) in &headers {
            // `add` refuses any value outside printable ASCII; the unsafe form sends it as UTF-8.
            let add = if value.is_ascii() {
                "add"
            } else {
                "addUnsafeNonAscii"
            };
            setup.push_str(&format!(
                "        .{add}({}, {})\n",
                literal(name),
                literal(value)
            ));
        }
        setup.push_str("        .build()\n");
    }

    let mut out = String::new();
    for note in notes {
        out.push_str(&format!("// {note}\n"));
    }
    imports.sort_unstable();
    imports.dedup();
    for import in imports {
        out.push_str(&format!("import {import}\n"));
    }
    out.push_str("\nfun main() {\n");
    if req.follow_redirects() {
        out.push_str("    val client = OkHttpClient()\n");
    } else {
        out.push_str(
            "    val client = OkHttpClient.Builder()\n        .followRedirects(false)\n        .build()\n",
        );
    }
    out.push_str(&setup);
    out.push_str(&format!(
        "    val request = Request.Builder()\n        .url({})\n",
        literal(&strict_url(req.url()))
    ));
    if !headers.is_empty() {
        out.push_str("        .headers(headers)\n");
    }
    out.push_str(&format!(
        "        .method({}, {})\n        .build()\n",
        literal(method),
        body.as_deref().unwrap_or("null")
    ));
    out.push_str(
        "    client.newCall(request).execute().use { response ->\n\
         \x20       println(response.code)\n\
         \x20       println(response.body?.string())\n    }\n}\n",
    );
    out
}
