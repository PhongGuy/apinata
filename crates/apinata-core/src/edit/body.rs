//! A request's `http.body` or `graphql.body` as the app edits it.

use super::{Redacted, description, description_yaml, disabled, text};
use crate::KeyValue;
use crate::document::{Fields, Origin, write_entry};
use crate::js;
use std::fmt;
use std::path::Path;
use yaml_serde::{Mapping, Value};

/// Bruno's key order for a multipart entry.
const MULTIPART_ORDER: &[&str] = &[
    "name",
    "type",
    "value",
    "contentType",
    "description",
    "disabled",
];

/// Bruno's key order for a file body entry.
const FILE_ORDER: &[&str] = &["filePath", "contentType", "selected", "description"];

/// Key order inside `http.body`.
pub(crate) const BODY_ORDER: &[&str] = &["type", "data"];

/// Key order inside a GraphQL request's `graphql.body`.
const GRAPHQL_BODY_ORDER: &[&str] = &["query", "variables"];

/// A request body.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BodySpec {
    /// No body.
    None,
    /// JSON text.
    Json(String),
    /// XML text.
    Xml(String),
    /// Plain text.
    Text(String),
    /// A SPARQL query, sent as raw text.
    Sparql(String),
    /// URL-encoded form fields.
    FormUrlEncoded(Vec<KeyValue>),
    /// Multipart form fields.
    Multipart(Vec<MultipartField>),
    /// Files, one of which is selected.
    File(Vec<FileEntry>),
    /// A GraphQL request's query and variables, the only body such a request has.
    Graphql {
        /// The query text.
        query: String,
        /// The variables as written: JSON, possibly with comments and `{{var}}` references.
        variables: String,
    },
    /// A body mode this editor does not model; it is shown but never written.
    Unsupported {
        /// The mode as written in the file, or `unknown`.
        kind: String,
    },
}

/// Shows sizes only, since bodies often carry credentials.
impl fmt::Debug for BodySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BodySpec::None => f.write_str("None"),
            BodySpec::Json(t) => f.debug_tuple("Json").field(&Redacted(t.len())).finish(),
            BodySpec::Xml(t) => f.debug_tuple("Xml").field(&Redacted(t.len())).finish(),
            BodySpec::Text(t) => f.debug_tuple("Text").field(&Redacted(t.len())).finish(),
            BodySpec::Sparql(t) => f.debug_tuple("Sparql").field(&Redacted(t.len())).finish(),
            BodySpec::FormUrlEncoded(v) => f.debug_tuple("FormUrlEncoded").field(v).finish(),
            BodySpec::Multipart(v) => f.debug_tuple("Multipart").field(v).finish(),
            BodySpec::File(v) => f.debug_tuple("File").field(v).finish(),
            BodySpec::Graphql { query, variables } => f
                .debug_struct("Graphql")
                .field("query", &Redacted(query.len()))
                .field("variables", &Redacted(variables.len()))
                .finish(),
            BodySpec::Unsupported { kind } => {
                f.debug_struct("Unsupported").field("kind", kind).finish()
            }
        }
    }
}

/// The value of a multipart field.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MultipartValue {
    /// Text.
    Text(String),
    /// Paths of files to upload.
    Files(Vec<String>),
}

impl fmt::Debug for MultipartValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MultipartValue::Text(t) => f.debug_tuple("Text").field(&Redacted(t.len())).finish(),
            MultipartValue::Files(p) => f.debug_tuple("Files").field(p).finish(),
        }
    }
}

/// One multipart form field.
#[derive(Clone)]
#[non_exhaustive]
pub struct MultipartField {
    /// The field name.
    pub name: String,
    /// The text or files.
    pub value: MultipartValue,
    /// The part's content type; blank for none.
    pub content_type: String,
    /// Whether the field is sent.
    pub enabled: bool,
    /// A note on the field; blank text is not written.
    pub description: Option<String>,
    origin: Option<Origin>,
}

impl MultipartField {
    /// Equal (`==`) and read from the same YAML, so an unknown key the app does not model
    /// also counts; `==` itself ignores where an entry was read from.
    pub fn same_source(&self, other: &MultipartField) -> bool {
        self == other && Origin::same(self.origin.as_ref(), other.origin.as_ref())
    }

    /// An enabled text field with no content type.
    pub fn text(name: impl Into<String>, value: impl Into<String>) -> MultipartField {
        MultipartField::new(name.into(), MultipartValue::Text(value.into()))
    }

    /// An enabled file field with no content type.
    pub fn files(name: impl Into<String>, paths: Vec<String>) -> MultipartField {
        MultipartField::new(name.into(), MultipartValue::Files(paths))
    }

    fn new(name: String, value: MultipartValue) -> MultipartField {
        MultipartField {
            name,
            value,
            content_type: String::new(),
            enabled: true,
            description: None,
            origin: None,
        }
    }

    pub(crate) fn from_yaml(entry: &Value) -> MultipartField {
        let value = if text(entry, "type") == "file" {
            MultipartValue::Files(match entry.get("value") {
                Some(Value::Sequence(items)) => {
                    items.iter().filter_map(js::yaml_to_string).collect()
                }
                Some(scalar) => js::yaml_to_string(scalar).into_iter().collect(),
                None => Vec::new(),
            })
        } else {
            MultipartValue::Text(text(entry, "value"))
        };
        let mut field = MultipartField::new(text(entry, "name"), value);
        field.content_type = text(entry, "contentType");
        field.enabled = !js::is_disabled(entry);
        field.description = description(entry);
        field.origin = Some(Origin::new(entry.clone(), field.fields()));
        field
    }

    pub(crate) fn to_yaml(&self) -> Value {
        write_entry(self.origin.as_ref(), self.fields(), MULTIPART_ORDER)
    }

    fn fields(&self) -> Fields {
        let (kind, value) = match &self.value {
            MultipartValue::Text(t) => ("text", Value::from(t.as_str())),
            MultipartValue::Files(paths) => (
                "file",
                Value::Sequence(paths.iter().map(|p| Value::from(p.as_str())).collect()),
            ),
        };
        vec![
            ("name", Some(Value::from(self.name.as_str()))),
            ("type", Some(Value::from(kind))),
            ("value", Some(value)),
            (
                "contentType",
                (!js::js_trim(&self.content_type).is_empty())
                    .then(|| Value::from(self.content_type.as_str())),
            ),
            ("description", description_yaml(self.description.as_deref())),
            ("disabled", disabled(self.enabled)),
        ]
    }
}

/// Compares the public fields only; where an entry was read from does not matter.
impl PartialEq for MultipartField {
    fn eq(&self, other: &Self) -> bool {
        (
            &self.name,
            &self.value,
            &self.content_type,
            self.enabled,
            &self.description,
        ) == (
            &other.name,
            &other.value,
            &other.content_type,
            other.enabled,
            &other.description,
        )
    }
}

impl Eq for MultipartField {}

impl fmt::Debug for MultipartField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultipartField")
            .field("name", &self.name)
            .field("value", &self.value)
            .field("content_type", &self.content_type)
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// One file of a file body.
#[derive(Clone)]
#[non_exhaustive]
pub struct FileEntry {
    /// The file's path.
    pub path: String,
    /// The content type; blank for none.
    pub content_type: String,
    /// Whether this is the file that is sent.
    pub selected: bool,
    origin: Option<Origin>,
}

impl FileEntry {
    /// Equal (`==`) and read from the same YAML, so an unknown key the app does not model
    /// also counts; `==` itself ignores where an entry was read from.
    pub fn same_source(&self, other: &FileEntry) -> bool {
        self == other && Origin::same(self.origin.as_ref(), other.origin.as_ref())
    }

    /// An unselected entry with no content type.
    pub fn new(path: impl Into<String>) -> FileEntry {
        FileEntry {
            path: path.into(),
            content_type: String::new(),
            selected: false,
            origin: None,
        }
    }

    pub(crate) fn from_yaml(entry: &Value) -> FileEntry {
        let mut file = FileEntry::new(text(entry, "filePath"));
        file.content_type = text(entry, "contentType");
        file.selected = entry
            .get("selected")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        file.origin = Some(Origin::new(entry.clone(), file.fields()));
        file
    }

    pub(crate) fn to_yaml(&self) -> Value {
        write_entry(self.origin.as_ref(), self.fields(), FILE_ORDER)
    }

    fn fields(&self) -> Fields {
        vec![
            ("filePath", Some(Value::from(self.path.as_str()))),
            ("contentType", Some(Value::from(self.content_type.as_str()))),
            ("selected", Some(Value::Bool(self.selected))),
        ]
    }
}

/// Compares the public fields only; where an entry was read from does not matter.
impl PartialEq for FileEntry {
    fn eq(&self, other: &Self) -> bool {
        (&self.path, &self.content_type, self.selected)
            == (&other.path, &other.content_type, other.selected)
    }
}

impl Eq for FileEntry {}

impl fmt::Debug for FileEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileEntry")
            .field("path", &self.path)
            .field("content_type", &self.content_type)
            .field("selected", &self.selected)
            .finish()
    }
}

impl BodySpec {
    /// Read `http.body`; absent is [`BodySpec::None`].
    pub(crate) fn from_yaml(body: Option<&Value>) -> BodySpec {
        let Some(body) = body.filter(|b| !b.is_null()) else {
            return BodySpec::None;
        };
        let list = || -> Vec<&Value> {
            body.get("data")
                .and_then(Value::as_sequence)
                .map(|items| items.iter().collect())
                .unwrap_or_default()
        };
        match text(body, "type").as_str() {
            "json" => BodySpec::Json(text(body, "data")),
            "xml" => BodySpec::Xml(text(body, "data")),
            "text" => BodySpec::Text(text(body, "data")),
            "sparql" => BodySpec::Sparql(text(body, "data")),
            "form-urlencoded" => {
                BodySpec::FormUrlEncoded(list().into_iter().map(KeyValue::from_yaml).collect())
            }
            "multipart-form" => {
                BodySpec::Multipart(list().into_iter().map(MultipartField::from_yaml).collect())
            }
            "file" => BodySpec::File(list().into_iter().map(FileEntry::from_yaml).collect()),
            other => BodySpec::Unsupported {
                kind: if other.is_empty() { "unknown" } else { other }.to_owned(),
            },
        }
    }

    /// Read `graphql.body`: its `query` and `variables`, each blank when absent.
    pub(crate) fn from_graphql_yaml(body: Option<&Value>) -> BodySpec {
        let field = |key| body.map(|b| text(b, key)).unwrap_or_default();
        BodySpec::Graphql {
            query: field("query"),
            variables: field("variables"),
        }
    }

    /// The `graphql.body` mapping for a `Graphql` body, each field only when non-empty;
    /// `None` when both are empty, or for any other mode.
    pub(crate) fn to_graphql_yaml(&self) -> Option<Value> {
        let BodySpec::Graphql { query, variables } = self else {
            return None;
        };
        let mut map = Mapping::new();
        for (key, value) in GRAPHQL_BODY_ORDER.iter().zip([query, variables]) {
            if !value.is_empty() {
                map.insert(Value::from(*key), Value::from(value.as_str()));
            }
        }
        (!map.is_empty()).then_some(Value::Mapping(map))
    }

    /// The `type` and `data` to write; `None` for modes that are not written.
    pub(crate) fn to_yaml(&self) -> Option<(&'static str, Option<Value>)> {
        fn seq(items: impl Iterator<Item = Value>) -> Option<Value> {
            let items: Vec<Value> = items.collect();
            (!items.is_empty()).then_some(Value::Sequence(items))
        }
        Some(match self {
            BodySpec::None | BodySpec::Graphql { .. } | BodySpec::Unsupported { .. } => {
                return None;
            }
            BodySpec::Json(t) => ("json", Some(Value::from(t.as_str()))),
            BodySpec::Xml(t) => ("xml", Some(Value::from(t.as_str()))),
            BodySpec::Text(t) => ("text", Some(Value::from(t.as_str()))),
            BodySpec::Sparql(t) => ("sparql", Some(Value::from(t.as_str()))),
            BodySpec::FormUrlEncoded(v) => {
                ("form-urlencoded", seq(v.iter().map(KeyValue::to_yaml)))
            }
            BodySpec::Multipart(v) => {
                ("multipart-form", seq(v.iter().map(MultipartField::to_yaml)))
            }
            BodySpec::File(v) => ("file", seq(v.iter().map(FileEntry::to_yaml))),
        })
    }

    /// A fresh `{type, data?}` mapping.
    pub(crate) fn fresh_mapping(kind: &str, data: Option<Value>) -> Value {
        let mut map = Mapping::new();
        map.insert(Value::from("type"), Value::from(kind));
        if let Some(data) = data {
            map.insert(Value::from("data"), data);
        }
        Value::Mapping(map)
    }
}

/// A picked path as a request file stores it: relative with native separators inside the
/// collection at `root` (but not `root` itself), else `picked` as given.
pub fn collection_relative(root: &Path, picked: &Path) -> String {
    // Lexical first; a root opened through a symlink only matches the picker's resolved
    // path once both are canonical, and a path that cannot be canonicalised stays as given.
    let inside = |root: &Path, picked: &Path| match picked.strip_prefix(root) {
        Ok(rel) if !rel.as_os_str().is_empty() => Some(rel.to_string_lossy().into_owned()),
        _ => None,
    };
    inside(root, picked)
        .or_else(|| {
            let (root, picked) = (root.canonicalize().ok()?, picked.canonicalize().ok()?);
            inside(&root, &picked)
        })
        .unwrap_or_else(|| picked.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::{FileEntry, MultipartField, collection_relative};
    use std::path::Path;
    use yaml_serde::Value;

    fn yaml(text: &str) -> Value {
        yaml_serde::from_str(text).unwrap()
    }

    // Entries read from YAML that differs only in a key the app does not model are equal,
    // yet only `same_source` tells them apart; identical reads agree on both.
    #[test]
    fn same_source_also_compares_the_yaml_a_body_entry_was_read_from() {
        let old = "name: a\ntype: text\nvalue: \"1\"\nx-note: old\n";
        let new = old.replace("x-note: old", "x-note: new");
        let (a, b) = (
            MultipartField::from_yaml(&yaml(old)),
            MultipartField::from_yaml(&yaml(&new)),
        );
        assert!(a == b && !a.same_source(&b));
        assert!(a.same_source(&MultipartField::from_yaml(&yaml(old))));
        assert!(!a.same_source(&MultipartField::text("a", "1")));
        let old = "filePath: /a\ncontentType: \"\"\nselected: true\nx-note: old\n";
        let new = old.replace("x-note: old", "x-note: new");
        let (a, b) = (
            FileEntry::from_yaml(&yaml(old)),
            FileEntry::from_yaml(&yaml(&new)),
        );
        assert!(a == b && !a.same_source(&b));
        assert!(a.same_source(&FileEntry::from_yaml(&yaml(old))));
    }

    #[test]
    fn a_path_inside_the_root_is_relative_and_one_outside_is_kept() {
        let root = Path::new("/c/col");
        assert_eq!(
            collection_relative(root, Path::new("/c/col/d/f.txt")),
            "d/f.txt"
        );
        assert_eq!(
            collection_relative(root, Path::new("/c/col2/x")),
            "/c/col2/x"
        );
        assert_eq!(collection_relative(root, root), "/c/col");
    }
}
