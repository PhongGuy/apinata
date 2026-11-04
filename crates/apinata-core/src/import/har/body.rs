//! A HAR `postData` to a request body (plan 4k Decision 13).

use super::budget::{Budget, Prefixed};
use super::credentials::{Scrub, slug};
use super::json_members::{self, Shape};
use super::model::{Param, PostData, text};
use super::{secret_names, unknown};
use crate::edit::{BodySpec, KeyValue, MultipartField};
use crate::import::{DocumentError, ImportNote, NoteKind};

const NO_PARAMS: &str = "multipart body has no params to rebuild it from";
const NO_FILE: &str = "file field needs a file chosen";
const TEXT: &str = "request.postData.text";
const CUT: &str = "JSON body ends inside a credential value, so it is not kept";

/// Decision 13; notes at `{at}request.postData...`; unknown postData and param members noted
/// (Decision 14). Each form or multipart field is charged one entry (Decision 18). Notes never
/// carry a body or field value. A non-empty form, multipart text or JSON member value whose
/// name satisfies `secret_names::is_field` becomes a `scrub` reference.
pub(super) fn body(
    post_data: Option<&PostData>,
    scrub: &mut Scrub,
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
) -> Result<BodySpec, DocumentError> {
    let Some(post_data) = post_data else {
        return Ok(BodySpec::None);
    };
    let at = scrub.at;
    if !post_data.rest.is_empty() {
        let at = format!("{at}request.postData.");
        unknown(&post_data.rest, &["comment"], &at, budget, notes)?;
    }
    let params = post_data.params.as_deref().unwrap_or_default();
    for (k, p) in params.iter().enumerate() {
        if !p.rest.is_empty() {
            let at = format!("{at}request.postData.params[{k}].");
            unknown(&p.rest, &["comment"], &at, budget, notes)?;
        }
    }
    let mime = text(post_data.mime_type.as_ref());
    let mime = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    // Browsers send multipart as both raw text and params; only the params can be rebuilt.
    if mime.starts_with("multipart/") {
        if params.is_empty() {
            notes.push(budget.note(at, "request.postData", NoteKind::Dropped, NO_PARAMS)?);
            return Ok(BodySpec::None);
        }
        return multipart(params, scrub, budget, notes);
    }
    let body = text(post_data.text.as_ref());
    if body.is_empty() {
        if params.is_empty() {
            return Ok(BodySpec::None);
        }
        let mut fields = Vec::with_capacity(params.len());
        for (k, p) in params.iter().enumerate() {
            budget.entry()?;
            let path = format!("request.postData.params[{k}]");
            let name = text(p.name.as_ref());
            let value = field(scrub, &path, &name, text(p.value.as_ref()), budget, notes)?;
            fields.push(KeyValue::new(name, value));
        }
        return Ok(BodySpec::FormUrlEncoded(fields));
    }
    Ok(match mime.as_str() {
        "application/json" => json(body, scrub, budget, notes)?,
        m if m.ends_with("+json") => json(body, scrub, budget, notes)?,
        "application/xml" | "text/xml" => BodySpec::Xml(body),
        m if m.ends_with("+xml") => BodySpec::Xml(body),
        "application/x-www-form-urlencoded" => {
            let mut fields = Vec::new();
            for (name, value) in url::form_urlencoded::parse(body.as_bytes()) {
                budget.entry()?;
                let value = field(scrub, TEXT, &name, value.into_owned(), budget, notes)?;
                fields.push(KeyValue::new(name, value));
            }
            BodySpec::FormUrlEncoded(fields)
        }
        "" => BodySpec::Text(body),
        m if m.starts_with("text/") => BodySpec::Text(body),
        m => {
            let detail = format!("{m} body imported as text");
            notes.push(budget.note(at, "request.postData", NoteKind::Converted, detail)?);
            BodySpec::Text(body)
        }
    })
}

/// Multipart fields from `params`: a param with a `fileName` or the value `(binary)` is a file
/// field with an empty path, which needs input; the rest are text fields.
fn multipart(
    params: &[Param],
    scrub: &mut Scrub,
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
) -> Result<BodySpec, DocumentError> {
    let at = scrub.at;
    let mut fields = Vec::with_capacity(params.len());
    for (k, p) in params.iter().enumerate() {
        budget.entry()?;
        let name = text(p.name.as_ref());
        let value = text(p.value.as_ref());
        let is_file = p.file_name.as_ref().is_some_and(|f| !f.is_null()) || value == "(binary)";
        let path = format!("request.postData.params[{k}]");
        let mut part = if is_file {
            notes.push(budget.note(at, &path, NoteKind::NeedsInput, NO_FILE)?);
            MultipartField::files(name, vec![String::new()])
        } else {
            let value = field(scrub, &path, &name, value, budget, notes)?;
            MultipartField::text(name, value)
        };
        part.content_type = text(p.content_type.as_ref());
        fields.push(part);
    }
    Ok(BodySpec::Multipart(fields))
}

/// `value`, or a reference noted at `path` when `name` is a credential field and `value` is
/// not empty.
fn field(
    scrub: &mut Scrub,
    path: &str,
    name: &str,
    value: String,
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
) -> Result<String, DocumentError> {
    if value.is_empty() || !secret_names::is_field(name) {
        return Ok(value);
    }
    scrub.reference(
        path,
        &slug(secret_names::leaf(name)),
        name,
        &value,
        budget,
        notes,
    )
}

/// `text` with each credential member's value replaced by a reference: a non-empty string
/// inside its quotes, a number or a non-empty object or array as a bare reference. Other bytes
/// stay as captured and the notes follow member order. A body the text ends inside a credential
/// value is dropped with a note. A key is compared raw, so one spelled with escapes is missed.
fn json(
    text: String,
    scrub: &mut Scrub,
    budget: &mut Budget,
    notes: &mut Vec<ImportNote>,
) -> Result<BodySpec, DocumentError> {
    let members = json_members::members(&text);
    // A cut-off credential cannot be rewritten whole, so the check runs before any reference
    // is made and leaves no orphan variable behind.
    let cut = members.iter().any(|(key, shape)| {
        matches!(shape, Shape::Unterminated) && secret_names::is_field(&text[key.clone()])
    });
    if cut {
        notes.push(budget.note(scrub.at, TEXT, NoteKind::Dropped, CUT)?);
        return Ok(BodySpec::None);
    }
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (key, shape) in members {
        // Members inside an already replaced composite went into its secret whole.
        if key.start < copied {
            continue;
        }
        let name = &text[key];
        let value = match shape {
            Shape::Str(r) | Shape::Number(r) if !r.is_empty() => r,
            Shape::Composite(r) if !text[r.start + 1..r.end - 1].trim().is_empty() => r,
            _ => continue,
        };
        if !secret_names::is_field(name) {
            continue;
        }
        let reference = scrub.reference(
            TEXT,
            &slug(secret_names::leaf(name)),
            name,
            &text[value.clone()],
            budget,
            notes,
        )?;
        out.push_str(&text[copied..value.start]);
        out.push_str(&reference);
        copied = value.end;
    }
    out.push_str(&text[copied..]);
    Ok(BodySpec::Json(out))
}
