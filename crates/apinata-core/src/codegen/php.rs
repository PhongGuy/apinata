//! The PHP target: a script sending the request with the curl extension and printing the
//! status and body. Digest auth is written natively with `CURLAUTH_DIGEST`.

use std::collections::HashSet;

use crate::execute::form_payload;
use crate::multipart::{Part, PartValue};
use crate::resolve::{Body, ResolvedRequest};

use super::{
    BOUNDARY, Segment, base_name, digest_credentials, has_typed_text, multipart_segments,
    omitted_auth, sent_headers,
};

/// `text` as a PHP double-quoted literal, which expands `$` and backslash escapes, so both
/// are escaped. ASCII controls use `\xHH`; any other character, non-ASCII included, is kept.
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
            '\u{b}' => out.push_str("\\v"),
            '\u{1b}' => out.push_str("\\e"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A path read at run time.
fn read_file(path: &std::path::Path) -> String {
    format!("file_get_contents({})", literal(&path.to_string_lossy()))
}

/// Whether the parts must be written by hand: an array cannot repeat a key, and the extension
/// cannot give a text part its own content type.
fn hand_built(parts: &[Part]) -> bool {
    let mut names = HashSet::new();
    has_typed_text(parts) || !parts.iter().all(|p| names.insert(p.name()))
}

/// The `CURLOPT_POSTFIELDS` value, or `None` without a body.
fn post_fields(req: &ResolvedRequest) -> Option<String> {
    match req.body() {
        Body::None => None,
        Body::Text(text) => Some(literal(text)),
        Body::FormUrlEncoded(pairs) => Some(literal(&form_payload(pairs))),
        Body::File { path, .. } => Some(read_file(path)),
        Body::Multipart(parts) if hand_built(parts) => {
            let items: Vec<String> = multipart_segments(parts)
                .into_iter()
                .map(|segment| match segment {
                    Segment::Text(text) => literal(&text),
                    Segment::File(path) => read_file(path),
                })
                .collect();
            Some(items.join("\n        . "))
        }
        Body::Multipart(parts) => {
            let mut out = String::from("[\n");
            for part in parts {
                let value = match part.value() {
                    PartValue::Text(value) => literal(value),
                    PartValue::File(path) => format!(
                        "new CURLFile({}, {}, {})",
                        literal(&path.to_string_lossy()),
                        literal(part.content_type().unwrap_or("")),
                        literal(&base_name(path))
                    ),
                };
                out.push_str(&format!("        {} => {value},\n", literal(part.name())));
            }
            out.push_str("    ]");
            Some(out)
        }
    }
}

/// A PHP script calling `curl_exec`. A header with an empty value is written `Name;`, since
/// libcurl reads `Name:` as "remove this header".
pub(super) fn curl(req: &ResolvedRequest) -> String {
    let mut out = String::from("<?php\n");
    if let Some(note) = omitted_auth(req, true) {
        out.push_str(&format!("// {note}\n"));
    }
    out.push_str("\n$curl = curl_init();\ncurl_setopt_array($curl, [\n");
    out.push_str(&format!("    CURLOPT_URL => {},\n", literal(req.url())));
    // `CUSTOMREQUEST` HEAD would wait for a body that never comes; `NOBODY` sends a real HEAD.
    if req.method() == "HEAD" && matches!(req.body(), Body::None) {
        out.push_str("    CURLOPT_NOBODY => true,\n");
    } else {
        out.push_str(&format!(
            "    CURLOPT_CUSTOMREQUEST => {},\n",
            literal(req.method())
        ));
    }
    out.push_str("    CURLOPT_RETURNTRANSFER => true,\n");

    let mut headers: Vec<String> = sent_headers(req)
        .map(|(name, value)| {
            if value.is_empty() {
                literal(&format!("{name};"))
            } else {
                literal(&format!("{name}: {value}"))
            }
        })
        .collect();
    if matches!(req.body(), Body::Multipart(parts) if hand_built(parts)) {
        headers.push(literal(&format!(
            "Content-Type: multipart/form-data; boundary={BOUNDARY}"
        )));
    }
    if !headers.is_empty() {
        out.push_str("    CURLOPT_HTTPHEADER => [\n");
        for header in headers {
            out.push_str(&format!("        {header},\n"));
        }
        out.push_str("    ],\n");
    }
    if let Some(fields) = post_fields(req) {
        out.push_str(&format!("    CURLOPT_POSTFIELDS => {fields},\n"));
    }
    if let Some((username, password)) = digest_credentials(req) {
        out.push_str(&format!(
            "    CURLOPT_HTTPAUTH => CURLAUTH_DIGEST,\n    CURLOPT_USERPWD => {},\n",
            literal(&format!("{username}:{password}"))
        ));
    }
    out.push_str(
        "]);\n\n$response = curl_exec($curl);\n\
         echo curl_getinfo($curl, CURLINFO_RESPONSE_CODE), \"\\n\";\n\
         echo $response, \"\\n\";\n",
    );
    out
}
