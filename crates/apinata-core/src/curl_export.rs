//! Renders a resolved request as a POSIX shell `curl` command, one argument per line.

use std::path::Path;

use crate::codegen::{base_name, disposition_escape, sent_headers};
use crate::execute::form_payload;
use crate::multipart::{Part, PartValue};
use crate::resolve::{Body, ResolvedRequest};

pub(crate) const SEPARATOR: &str = " \\\n  ";

/// The request as a POSIX shell `curl` command, every value single-quoted. Secret values are
/// included as resolved; check `ResolvedRequest::sensitive_values` before showing it.
///
/// Transport settings are not written, so curl's defaults apply. Known limits: a multipart part
/// name holding `;` or `"` but no `=` is written as is (one holding `=` has `"`, CR and LF
/// percent-encoded), a header whose value is only whitespace is not sent by curl, and a non-UTF-8
/// path is rendered lossily.
pub fn to_curl(req: &ResolvedRequest) -> String {
    // `--head` refuses a body, so a HEAD with one names its method instead.
    let head = req.method() == "HEAD" && matches!(req.body(), Body::None);
    let mut args = vec![if head {
        "curl --head".to_owned()
    } else {
        format!("curl --request {}", quote(req.method()))
    }];
    args.push(format!("--url {}", quote(req.url())));
    if req.url().contains(['[', ']', '{', '}']) {
        args.push("--globoff".to_owned());
    }

    // curl writes its own multipart Content-Type with the boundary it picks, so ours goes.
    for (name, value) in sent_headers(req) {
        let header = if value.is_empty() {
            format!("{name};")
        } else {
            format!("{name}: {value}")
        };
        args.push(format!("--header {}", quote(&header)));
    }

    match req.body() {
        Body::None => {}
        Body::Text(text) => args.push(format!("--data-raw {}", quote(text))),
        Body::FormUrlEncoded(pairs) => {
            args.push(format!("--data-raw {}", quote(&form_payload(pairs))));
        }
        Body::Multipart(parts) => {
            args.extend(
                parts
                    .iter()
                    .map(|p| format!("--form {}", quote(&form_arg(p)))),
            );
        }
        Body::File { path, .. } => {
            args.push(format!(
                "--data-binary {}",
                quote(&format!("@{}", lossy(path)))
            ));
        }
    }
    args.join(SEPARATOR)
}

/// One `--form` argument: `name=value` or `name=@path`, with `;type=` when a type is given.
/// curl splits the name at the first `=`, so such a name goes in a `headers=` Content-Disposition
/// of its own, which replaces curl's, behind a placeholder name.
fn form_arg(part: &Part) -> String {
    let own_name = part.name().contains('=');
    let name = if own_name { "x" } else { part.name() };
    let mut arg = match part.value() {
        PartValue::Text(text) => format!("{name}={}", form_word(text)),
        PartValue::File(path) => format!("{name}=@{}", form_word(&lossy(path))),
    };
    if let Some(ct) = part.content_type() {
        arg.push_str(";type=");
        arg.push_str(ct);
    }
    if own_name {
        let mut disposition = format!("form-data; name=\"{}\"", disposition_escape(part.name()));
        if let PartValue::File(path) = part.value() {
            disposition.push_str(&format!(
                "; filename=\"{}\"",
                disposition_escape(&base_name(path))
            ));
        }
        arg.push_str(&format!(
            ";headers=\"Content-Disposition: {}\"",
            disposition.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    arg
}

/// A `--form` value double-quoted when curl would otherwise read it as a file, an attribute
/// separator or an escape, or trim its edge whitespace; inside the quotes only `\` and `"` are
/// escaped.
fn form_word(value: &str) -> String {
    let special = value.starts_with(['@', '<', '"'])
        || value.contains([';', ',', '\\'])
        || value.starts_with(char::is_whitespace)
        || value.ends_with(char::is_whitespace);
    if !special {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn lossy(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A single-quoted shell word; an embedded `'` closes the quote, is escaped and reopens it.
pub(crate) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
