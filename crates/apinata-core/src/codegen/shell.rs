//! The shell targets: curl, which wraps `to_curl`, and HTTPie. Both write one argument per
//! line and single-quote every word.

use crate::curl_export::{SEPARATOR, quote, to_curl};
use crate::execute::form_payload;
use crate::multipart::PartValue;
use crate::resolve::{Body, ResolvedRequest};

use super::{digest_credentials, omitted_auth, sent_headers};

/// `to_curl` with curl's own digest flags, and a leading comment for SigV4.
pub(super) fn curl(req: &ResolvedRequest) -> String {
    let mut out = String::new();
    if let Some(note) = omitted_auth(req, true) {
        out.push_str(&format!("# {note}\n"));
    }
    out.push_str(&to_curl(req));
    if let Some((user, password)) = digest_credentials(req) {
        out.push_str(SEPARATOR);
        out.push_str("--digest");
        out.push_str(SEPARATOR);
        out.push_str(&format!("--user {}", quote(&format!("{user}:{password}"))));
    }
    out
}

/// One side of an HTTPie request item with every separator character backslash-escaped, so
/// only the separator we write splits it and no value reads a file or becomes JSON. Known
/// limits: a `\` right before one of `:;=@` is not kept exact, and a path holding `;type=` splits.
fn item_word(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, ':' | ';' | '=' | '@') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// An `http` command. `--ignore-stdin` keeps a script's stdin from becoming the body; headers
/// keep their resolved order, and an empty value is written `Name;`, HTTPie's empty header.
/// A text part's content type is dropped, since HTTPie has no `;type=` for a field; only file
/// parts carry one.
pub(super) fn httpie(req: &ResolvedRequest) -> String {
    let mut first = String::from("http --ignore-stdin");
    if let Some((user, password)) = digest_credentials(req) {
        first.push_str(" --auth-type digest --auth ");
        first.push_str(&quote(&format!("{user}:{password}")));
    }
    // An escaped item after an option is not re-read as an item, so the flag goes first.
    if matches!(req.body(), Body::Multipart(_)) {
        first.push_str(" --multipart");
    }
    first.push_str(&format!(" {} {}", quote(req.method()), quote(req.url())));

    let mut args = vec![first];
    for (name, value) in sent_headers(req) {
        let item = if value.is_empty() {
            format!("{};", item_word(name))
        } else {
            format!("{}:{}", item_word(name), item_word(value))
        };
        args.push(quote(&item));
    }

    match req.body() {
        Body::None => {}
        Body::Text(text) => args.push(format!("--raw {}", quote(text))),
        Body::FormUrlEncoded(pairs) => {
            args.push(format!("--raw {}", quote(&form_payload(pairs))));
        }
        Body::Multipart(parts) => {
            for part in parts {
                let item = match part.value() {
                    PartValue::Text(text) => {
                        format!("{}={}", item_word(part.name()), item_word(text))
                    }
                    PartValue::File(path) => {
                        let mut item = format!(
                            "{}@{}",
                            item_word(part.name()),
                            item_word(&path.to_string_lossy())
                        );
                        if let Some(ct) = part.content_type() {
                            item.push_str(";type=");
                            item.push_str(ct);
                        }
                        item
                    }
                };
                args.push(quote(&item));
            }
        }
        Body::File { path, .. } => args.push(format!("@{}", quote(&path.to_string_lossy()))),
    }

    let command = args.join(SEPARATOR);
    match omitted_auth(req, true) {
        Some(note) => format!("# {note}\n{command}"),
        None => command,
    }
}
