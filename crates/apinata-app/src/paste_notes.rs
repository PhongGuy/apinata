//! The paste banner's sentences: one plain sentence per import note, with the names it
//! quotes shown as code rather than between literal backticks.

use crate::command::HeldSecret;
use apinata_core::BodySpec;
use apinata_core::import::{ImportNote, ImportedRequest, NoteKind};

/// One piece of a banner sentence: plain words, or a name shown as code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// The text, without quoting marks.
    pub text: String,
    /// Whether the text is a name shown as code.
    pub code: bool,
}

/// `note`'s detail as one sentence: capitalised, ending in a full stop, with each
/// backtick-quoted name as a code part.
pub fn sentence(note: &ImportNote) -> Vec<Part> {
    let mut parts: Vec<Part> = note
        .detail
        .split('`')
        .enumerate()
        .filter(|(_, text)| !text.is_empty())
        .map(|(i, text)| Part {
            text: text.to_owned(),
            code: i % 2 == 1,
        })
        .collect();
    if let Some(first) = parts.first_mut().filter(|p| !p.code) {
        let mut chars = first.text.chars();
        if let Some(c) = chars.next() {
            first.text = c.to_uppercase().chain(chars).collect();
        }
    }
    let ends = parts
        .last()
        .is_some_and(|p| !p.code && p.text.ends_with(['.', '!', '?']));
    if !ends && !parts.is_empty() {
        parts.push(Part {
            text: ".".to_owned(),
            code: false,
        });
    }
    parts
}

/// `parts` as plain text, as the banner's accessible label reads it.
pub fn plain(parts: &[Part]) -> String {
    parts.iter().map(|p| p.text.as_str()).collect()
}

/// Rewrites the note of each held secret in `notes` to say where its value goes when the
/// request is saved, naming the credential as `request` (the parsed paste) holds it.
pub fn apply_held(notes: &mut [ImportNote], held: &[HeldSecret], request: &ImportedRequest) {
    for secret in held {
        let Some(note) = notes
            .iter_mut()
            .find(|n| n.source == secret.source && n.kind == NoteKind::NeedsInput)
        else {
            continue;
        };
        let what = credential(&secret.source, request);
        let name = &secret.name;
        match &secret.environment {
            Ok(env) => {
                note.kind = NoteKind::Converted;
                note.detail = format!(
                    "{what} is kept as the secret `{name}` in the {env} environment when you save"
                );
            }
            Err(why) => {
                note.detail = format!(
                    "{what} is not written to the file and needs the secret `{name}`, since {why}"
                );
            }
        }
    }
}

// The credential a secret note's `source` names, in plain words: `curl auth.password` is
// "The password", `curl headers[0]` names the header.
fn credential(source: &str, request: &ImportedRequest) -> String {
    let member = source.strip_prefix("curl ").unwrap_or(source);
    let index = |prefix: &str| -> Option<usize> {
        member.strip_prefix(prefix)?.strip_suffix(']')?.parse().ok()
    };
    if let Some(i) = index("headers[")
        && let Some(h) = request.headers.get(i)
    {
        return format!("The `{}` header", h.name);
    }
    if let Some(i) = index("params[")
        && let Some(p) = request.params.get(i)
    {
        return format!("The `{}` query parameter", p.name);
    }
    if let Some(i) = index("body[") {
        let name = match &request.body {
            BodySpec::FormUrlEncoded(fields) => fields.get(i).map(|f| f.name.clone()),
            BodySpec::Multipart(fields) => fields.get(i).map(|f| f.name.clone()),
            _ => None,
        };
        if let Some(name) = name {
            return format!("The `{name}` form field");
        }
    }
    let words = match member {
        "auth.password" => "The password",
        "auth.token" => "The token",
        "auth.api_key" => "The API key",
        "auth.secret_access_key" | "flag --aws-sigv4" => "The AWS secret access key",
        "auth.session_token" => "The AWS session token",
        "url" => "A secret in the URL",
        _ => "A secret value",
    };
    words.to_owned()
}
