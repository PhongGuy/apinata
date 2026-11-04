//! The Rust target: a `main` sending the request with `reqwest::blocking` and printing the
//! status and body. Every builder link sits on its own line with its arguments inline, one
//! fixed layout whatever the length; it parses with `rustfmt`, which may rewrap long lines.

use crate::execute::form_payload;
use crate::multipart::{Part, PartValue};
use crate::resolve::{Body, ResolvedRequest};

use super::{disposition_escape, omitted_auth, part_type, sent_headers};

/// `text` as a Rust string literal. Controls are escaped so it stays on one line, as are the
/// bidirectional controls rustc refuses inside a literal; other non-ASCII is kept.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 0x20
                || c == '\u{7f}'
                || ('\u{202a}'..='\u{202e}').contains(&c)
                || ('\u{2066}'..='\u{2069}').contains(&c) =>
            {
                out.push_str(&format!("\\u{{{:x}}}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The method as a `reqwest::Method`: a constant for a standard one, else parsed from bytes.
fn method(name: &str) -> String {
    const STANDARD: [&str; 9] = [
        "GET", "POST", "PUT", "DELETE", "HEAD", "OPTIONS", "CONNECT", "PATCH", "TRACE",
    ];
    if STANDARD.contains(&name) {
        return format!("reqwest::Method::{name}");
    }
    let mut bytes = String::from("b\"");
    for b in name.bytes() {
        match b {
            b'\\' => bytes.push_str("\\\\"),
            b'"' => bytes.push_str("\\\""),
            0x20..=0x7e => bytes.push(b as char),
            b => bytes.push_str(&format!("\\x{b:02x}")),
        }
    }
    format!("reqwest::Method::from_bytes({bytes}\")?")
}

/// A builder link `.name(args)` on its own line at `indent`.
fn call(indent: &str, name: &str, args: &[String]) -> String {
    format!("{indent}.{name}({})\n", args.join(", "))
}

/// The `let form = Form::new()...;` statement. Names go out as written, not percent-encoded
/// by reqwest, except the `"`, CR and LF a quoted parameter cannot hold.
fn form(parts: &[Part]) -> String {
    let indent = "        ";
    let mut out = String::from("    let form = Form::new()\n");
    out.push_str(&format!("{indent}.percent_encode_noop()\n"));
    for part in parts {
        let name = literal(&disposition_escape(part.name()));
        let value = match (part.value(), part.content_type()) {
            (PartValue::Text(text), None) => {
                out.push_str(&call(indent, "text", &[name, literal(text)]));
                continue;
            }
            (PartValue::Text(text), Some(ct)) => {
                format!("Part::text({}).mime_str({})?", literal(text), literal(ct))
            }
            // A file part is typed explicitly, so it never takes reqwest's own guess.
            (PartValue::File(path), _) => format!(
                "Part::file({})?.mime_str({})?",
                literal(&path.to_string_lossy()),
                literal(part_type(part).unwrap_or_default())
            ),
        };
        out.push_str(&call(indent, "part", &[name, value]));
    }
    out.pop();
    out.push_str(";\n");
    out
}

/// A Rust program using `reqwest::blocking`. A header value that is not ASCII goes through
/// `HeaderValue::from_bytes`, since a `&str` value must be visible ASCII.
pub(super) fn reqwest(req: &ResolvedRequest) -> String {
    let mut out = String::new();
    if let Some(note) = omitted_auth(req, false) {
        out.push_str(&format!("// {note}\n"));
    }
    out.push_str(
        "// Cargo.toml: reqwest = { version = \"0.13\", features = [\"blocking\", \"multipart\"] }\n\n",
    );
    // `Part` is needed only by a file part or a typed text part; plain text goes through `text`.
    if let Body::Multipart(parts) = req.body() {
        let needs_part = parts
            .iter()
            .any(|p| matches!(p.value(), PartValue::File(_)) || p.content_type().is_some());
        out.push_str(if needs_part {
            "use reqwest::blocking::multipart::{Form, Part};\n\n"
        } else {
            "use reqwest::blocking::multipart::Form;\n\n"
        });
    }
    out.push_str("fn main() -> Result<(), Box<dyn std::error::Error>> {\n");
    if let Body::Multipart(parts) = req.body() {
        out.push_str(&form(parts));
    }

    let indent = "        ";
    out.push_str("    let response = reqwest::blocking::Client::new()\n");
    out.push_str(&call(
        indent,
        "request",
        &[method(req.method()), literal(req.url())],
    ));
    for (name, value) in sent_headers(req) {
        let value = if value.is_ascii() {
            literal(value)
        } else {
            format!(
                "reqwest::header::HeaderValue::from_bytes({}.as_bytes())?",
                literal(value)
            )
        };
        out.push_str(&call(indent, "header", &[literal(name), value]));
    }
    let body = match req.body() {
        Body::None => None,
        Body::Text(text) => Some(("body", literal(text))),
        Body::FormUrlEncoded(pairs) => Some(("body", literal(&form_payload(pairs)))),
        Body::File { path, .. } => Some((
            "body",
            format!("std::fs::File::open({})?", literal(&path.to_string_lossy())),
        )),
        Body::Multipart(_) => Some(("multipart", "form".to_owned())),
    };
    if let Some((name, arg)) = body {
        out.push_str(&call(indent, name, &[arg]));
    }
    out.push_str(&format!("{indent}.send()?;\n"));
    out.push_str(
        "    println!(\"{}\", response.status());\n    println!(\"{}\", response.text()?);\n    Ok(())\n}\n",
    );
    out
}
