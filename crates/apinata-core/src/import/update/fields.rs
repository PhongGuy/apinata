//! The operation key and the canonical hashes of the seven spec-owned fields of a request.

use crate::document::Document;
use crate::error::EditError;
use crate::import::{ImportError, ImportedRequest};
use crate::request::{RequestFile, RequestKind};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use yaml_serde::Value;

/// A request field an OpenAPI update owns and may refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ChangedField {
    /// The URL and the params together.
    Target,
    /// The headers.
    Headers,
    /// The body.
    Body,
    /// The docs.
    Docs,
    /// The tags.
    Tags,
    /// The request variables.
    Vars,
    /// The HTTP method as the file holds it.
    Method,
}

impl ChangedField {
    /// Every field, in declaration order.
    pub const ALL: [ChangedField; 7] = [
        ChangedField::Target,
        ChangedField::Headers,
        ChangedField::Body,
        ChangedField::Docs,
        ChangedField::Tags,
        ChangedField::Vars,
        ChangedField::Method,
    ];

    /// The stable name used in the baseline and in summaries.
    pub fn id(self) -> &'static str {
        match self {
            ChangedField::Target => "target",
            ChangedField::Headers => "headers",
            ChangedField::Body => "body",
            ChangedField::Docs => "docs",
            ChangedField::Tags => "tags",
            ChangedField::Vars => "vars",
            ChangedField::Method => "method",
        }
    }
}

/// One short hash per field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct FieldHashes {
    hashes: BTreeMap<ChangedField, String>,
}

impl FieldHashes {
    /// Hashes from `(field, hash)` pairs.
    pub fn new(pairs: impl IntoIterator<Item = (ChangedField, String)>) -> FieldHashes {
        FieldHashes {
            hashes: pairs.into_iter().collect(),
        }
    }

    /// The hash of `f`, when recorded.
    pub fn get(&self, f: ChangedField) -> Option<&str> {
        self.hashes.get(&f).map(String::as_str)
    }

    /// The recorded hashes in field order.
    pub fn iter(&self) -> impl Iterator<Item = (ChangedField, &str)> {
        self.hashes.iter().map(|(f, h)| (*f, h.as_str()))
    }
}

/// "GET /users/{}": method upper-cased; url minus a leading {{var}} prefix and the query
/// and fragment; `//` collapsed; trailing `/` dropped; a segment that is wholly `:name` or
/// `{name}` becomes `{}`; any other segment (such as `:id.json`) is kept literally.
pub fn operation_key(method: &str, url: &str) -> String {
    let mut rest = url;
    // A leading {{var}} is the base URL, which differs between specs and environments.
    if let Some(after) = rest.strip_prefix("{{")
        && let Some(end) = after.find("}}")
    {
        rest = &after[end + 2..];
    }
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let segments: Vec<String> = rest
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            if is_param_segment(s) {
                "{}".into()
            } else {
                s.to_owned()
            }
        })
        .collect();
    format!("{} /{}", method.to_uppercase(), segments.join("/"))
}

// A segment that is only a path parameter: `:name` or `{name}`.
fn is_param_segment(s: &str) -> bool {
    // A dot or other punctuation means the segment has literal text around the parameter.
    let named = |n: &str| {
        !n.is_empty()
            && n.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    };
    s.strip_prefix(':').is_some_and(named)
        || s.strip_prefix('{')
            .and_then(|r| r.strip_suffix('}'))
            .is_some_and(named)
}

/// The hashes of what `request` would write into a fresh request file.
///
/// The request goes through the same setters as a file edit, so normalisation (tags, docs)
/// applies on both sides of a comparison with [`file_field_hashes`].
pub fn spec_field_hashes(request: &ImportedRequest) -> Result<FieldHashes, ImportError> {
    let root = crate::collection::new_request_root("scratch", 1, request.kind);
    let mut scratch =
        RequestFile::from_document(Document::draft(PathBuf::from("scratch.yml"), root));
    refresh_fields(&mut scratch, request, &ChangedField::ALL)?;
    Ok(file_field_hashes(&scratch))
}

/// The hashes of the seven fields as `file` holds them now, unsaved edits included.
pub fn file_field_hashes(file: &RequestFile) -> FieldHashes {
    FieldHashes::new(
        ChangedField::ALL
            .into_iter()
            .map(|f| (f, hash_value(subtree(file, f)))),
    )
}

/// Write `fields` of `request` into `file`, in memory, and nothing else: never auth, settings
/// or scripts. A GraphQL file keeps its body, since an HTTP spec has no query to give it.
pub(crate) fn refresh_fields(
    file: &mut RequestFile,
    request: &ImportedRequest,
    fields: &[ChangedField],
) -> Result<(), EditError> {
    for field in fields {
        match field {
            ChangedField::Target => {
                file.set_url_and_params(request.url.clone(), &request.params);
            }
            ChangedField::Headers => file.set_headers(request.headers.clone())?,
            ChangedField::Body => {
                if file.kind() == RequestKind::Http {
                    file.set_body(request.body.clone());
                }
            }
            ChangedField::Docs => file.set_docs(request.docs.as_str()),
            ChangedField::Tags => file.set_tags(request.tags.clone()),
            ChangedField::Vars => file.set_vars(request.vars.clone()),
            // A GraphQL file is always POST, which an HTTP spec has no say in.
            ChangedField::Method => {
                if file.kind() == RequestKind::Http {
                    file.set_method(request.method.clone());
                }
            }
        }
    }
    Ok(())
}

// The YAML subtree a field owns; None when every key of it is absent.
fn subtree(file: &RequestFile, field: ChangedField) -> Option<Value> {
    let doc = file.document();
    let k = file.root_key();
    let get = |keys: &[&str]| doc.get(keys).cloned();
    match field {
        ChangedField::Target => {
            let (url, params) = (get(&[k, "url"]), get(&[k, "params"]));
            (url.is_some() || params.is_some()).then(|| {
                Value::Sequence(vec![
                    url.unwrap_or(Value::Null),
                    params.unwrap_or(Value::Null),
                ])
            })
        }
        ChangedField::Headers => get(&[k, "headers"]),
        ChangedField::Body => get(&[k, "body"]),
        ChangedField::Docs => get(&["docs"]),
        ChangedField::Tags => get(&["info", "tags"]),
        ChangedField::Vars => get(&["runtime", "variables"]),
        ChangedField::Method => Some(Value::String(file.method().to_owned())),
    }
}

// First 8 bytes of SHA-256 over the subtree's YAML text, as 16 lowercase hex characters. An
// absent subtree hashes `~`. SHA-256 rather than DefaultHasher, whose output may change.
fn hash_value(value: Option<Value>) -> String {
    let text = match value {
        Some(v) => yaml_serde::to_string(&v).unwrap_or_default(),
        None => "~".to_owned(),
    };
    Sha256::digest(text.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
