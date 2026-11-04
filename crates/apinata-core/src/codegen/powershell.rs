//! The PowerShell target: a PowerShell 7 script sending the request with `Invoke-RestMethod`,
//! which prints the response body. Multipart bodies use the .NET classes the C# target uses.

use crate::execute::form_payload;
use crate::multipart::PartValue;
use crate::resolve::{Body, ResolvedRequest};

use super::csharp::CONTENT_HEADERS;
use super::{base_name, disposition_escape, omitted_auth, part_type, sent_headers};

/// The methods `-Method` names; any other goes through `-CustomMethod`.
const METHODS: [&str; 8] = [
    "Get", "Post", "Put", "Delete", "Patch", "Head", "Options", "Trace",
];

/// `text` as a PowerShell string literal. Single quotes keep everything literal, so they are
/// used unless a control or line separator needs an escape; inside them every typographic
/// single quote is a quote too and is doubled. Double quotes escape `$`, the backtick and
/// every quote character with a backtick.
fn literal(text: &str) -> String {
    let needs_escape = |c: char| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}');
    let mut out = String::with_capacity(text.len() + 2);
    if !text.chars().any(needs_escape) {
        out.push('\'');
        for c in text.chars() {
            if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
                out.push(c);
            }
            out.push(c);
        }
        out.push('\'');
        return out;
    }
    out.push('"');
    for c in text.chars() {
        match c {
            '`' | '$' | '"' | '\u{201c}' | '\u{201d}' | '\u{201e}' => {
                out.push('`');
                out.push(c);
            }
            '\0' => out.push_str("`0"),
            '\u{7}' => out.push_str("`a"),
            '\u{8}' => out.push_str("`b"),
            '\u{c}' => out.push_str("`f"),
            '\n' => out.push_str("`n"),
            '\r' => out.push_str("`r"),
            '\t' => out.push_str("`t"),
            '\u{b}' => out.push_str("`v"),
            c if needs_escape(c) => out.push_str(&format!("`u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The statements that set `$body`, or `None` without a body.
fn body(req: &ResolvedRequest) -> Option<String> {
    let bytes = |text: &str| {
        format!(
            "$body = [System.Text.Encoding]::UTF8.GetBytes({})\n",
            literal(text)
        )
    };
    match req.body() {
        Body::None => None,
        Body::Text(text) => Some(bytes(text)),
        Body::FormUrlEncoded(pairs) => Some(bytes(&form_payload(pairs))),
        Body::File { path, .. } => Some(format!(
            "$body = [System.IO.File]::ReadAllBytes({})\n",
            literal(&path.to_string_lossy())
        )),
        Body::Multipart(parts) => {
            let mut out =
                String::from("$body = [System.Net.Http.MultipartFormDataContent]::new()\n");
            let mut explained = false;
            for (i, part) in parts.iter().enumerate() {
                let var = format!("$part{}", i + 1);
                // As in the C# target, an untyped text part drops StringContent's default type.
                let (make, file_name) = match part.value() {
                    PartValue::Text(value) => (
                        format!("[System.Net.Http.StringContent]::new({})", literal(value)),
                        None,
                    ),
                    PartValue::File(path) => (
                        format!(
                            "[System.Net.Http.StreamContent]::new([System.IO.File]::OpenRead({}))",
                            literal(&path.to_string_lossy())
                        ),
                        Some(base_name(path)),
                    ),
                };
                let ctype = part_type(part).map_or("$null".to_string(), |t| {
                    format!(
                        "[System.Net.Http.Headers.MediaTypeHeaderValue]::Parse({})",
                        literal(t)
                    )
                });
                out.push_str(&format!(
                    "{var} = {make}\n{var}.Headers.ContentType = {ctype}\n"
                ));
                let name = disposition_escape(part.name());
                let file_name = file_name.map(|f| disposition_escape(&f));
                // `Add` would RFC 2047-encode a non-ASCII name; part headers are written as
                // Latin-1, so a disposition holding the UTF-8 bytes as chars sends them as-is.
                if !name.is_ascii() || file_name.as_ref().is_some_and(|f| !f.is_ascii()) {
                    if !explained {
                        out.push_str("# Part headers are written as Latin-1; this carries the name's UTF-8 bytes.\n");
                        explained = true;
                    }
                    let mut value = format!("form-data; name=\"{name}\"");
                    if let Some(f) = file_name {
                        value.push_str(&format!("; filename=\"{f}\""));
                    }
                    out.push_str(&format!(
                        "[void]{var}.Headers.TryAddWithoutValidation('Content-Disposition', [System.Text.Encoding]::Latin1.GetString([System.Text.Encoding]::UTF8.GetBytes({})))\n$body.Add({var})\n",
                        literal(&value)
                    ));
                } else if let Some(f) = file_name {
                    out.push_str(&format!(
                        "$body.Add({var}, {}, {})\n",
                        literal(&name),
                        literal(&f)
                    ));
                } else {
                    out.push_str(&format!("$body.Add({var}, {})\n", literal(&name)));
                }
            }
            Some(out)
        }
    }
}

/// A PowerShell 7 script calling `Invoke-RestMethod`. With a body, Content-Type moves to
/// `-ContentType`, or for multipart other content headers go on `$body`, which the cmdlet
/// would otherwise drop. A non-ASCII header cannot be sent and is named in a leading note.
pub(super) fn rest_method(req: &ResolvedRequest) -> String {
    let mut notes = Vec::new();
    if let Some(note) = omitted_auth(req, false) {
        notes.push(note.to_string());
    }
    // The cmdlet's own handler refuses non-ASCII header bytes and offers no encoding option.
    let (headers, refused): (Vec<_>, Vec<_>) =
        sent_headers(req).partition(|(name, value)| name.is_ascii() && value.is_ascii());
    if !refused.is_empty() {
        // Names are escaped as in a literal, without its quotes, so a line break stays in the note.
        let names: Vec<String> = refused
            .iter()
            .map(|(name, _)| {
                let quoted = literal(name);
                quoted[1..quoted.len() - 1].to_string()
            })
            .collect();
        notes.push(format!(
            "Invoke-RestMethod sends header names and values as ASCII only; left out: {}.",
            names.join(", ")
        ));
    }

    let body = body(req);
    let multipart = matches!(req.body(), Body::Multipart(_));
    let moves = |name: &str| {
        let name = name.to_ascii_lowercase();
        body.is_some()
            && if multipart {
                CONTENT_HEADERS.contains(&name.as_str())
            } else {
                name == "content-type"
            }
    };
    let (moved, headers): (Vec<_>, Vec<_>) = headers.into_iter().partition(|(name, _)| moves(name));

    let mut out = String::new();
    for note in notes {
        out.push_str(&format!("# {note}\n"));
    }
    out.push_str(&format!(
        "#Requires -Version 7\n\n$url = {}\n",
        literal(req.url())
    ));
    if headers.is_empty() {
        out.push_str("$headers = [ordered]@{}\n");
    } else {
        out.push_str("$headers = [ordered]@{\n");
        for (name, value) in headers {
            out.push_str(&format!("    {} = {}\n", literal(name), literal(value)));
        }
        out.push_str("}\n");
    }

    let method = match METHODS
        .iter()
        .find(|m| m.eq_ignore_ascii_case(req.method()))
    {
        Some(m) => format!("-Method {m}"),
        None => format!("-CustomMethod {}", literal(req.method())),
    };
    let mut call = format!("Invoke-RestMethod -Uri $url {method} -Headers $headers");
    if let Some(set) = body {
        out.push_str(&set);
        for (name, value) in moved {
            if multipart {
                out.push_str(&format!(
                    "[void]$body.Headers.TryAddWithoutValidation({}, {})\n",
                    literal(name),
                    literal(value)
                ));
            } else {
                call.push_str(&format!(" -ContentType {}", literal(value)));
            }
        }
        call.push_str(" -Body $body");
    }
    out.push_str(&format!("{call} -SkipHeaderValidation\n"));
    out
}
