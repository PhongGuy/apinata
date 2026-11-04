//! Typed values for editing request files.

pub(crate) mod auth;
pub(crate) mod auth_json;
pub(crate) mod body;
pub(crate) mod edgegrid;
pub(crate) mod oauth1;
pub(crate) mod oauth2;
pub(crate) mod settings;

pub use auth::{ApiKeyPlacement, AuthSpec};
#[doc(hidden)]
pub use auth_json::paths as auth_json_paths;
pub use auth_json::{AuthJsonError, Position as AuthJsonPosition, parse_auth_json};
pub use body::{BodySpec, FileEntry, MultipartField, MultipartValue, collection_relative};
pub use edgegrid::EdgeGridSpec;
pub use oauth1::{
    OAuth1KeyKind, OAuth1Placement, OAuth1PrivateKey, OAuth1SignatureMethod, OAuth1Spec,
};
pub use oauth2::{
    ClientAuth, OAuth2Flow, OAuth2Param, OAuth2ParamPlacement, OAuth2Spec, OAuth2Stage,
    TokenPlacement, TokenSource,
};
pub(crate) use settings::omitted_names;
pub use settings::{RequestSettings, Timeout};

use crate::document::{Fields, Origin, write_entry};
use crate::error::EditError;
use crate::js;
use std::fmt;
use yaml_serde::Value;

/// Bruno's key order for a header entry.
const KEY_VALUE_ORDER: &[&str] = &["name", "value", "description", "disabled"];

/// A header entry.
#[derive(Clone)]
#[non_exhaustive]
pub struct KeyValue {
    /// The name.
    pub name: String,
    /// The value, possibly with `{{variables}}`.
    pub value: String,
    /// Whether the entry is sent.
    pub enabled: bool,
    /// A note on the entry; blank text is not written.
    pub description: Option<String>,
    origin: Option<Origin>,
}

impl KeyValue {
    /// Equal (`==`) and read from the same YAML, so an unknown key the app does not model
    /// also counts; `==` itself ignores where an entry was read from.
    pub fn same_source(&self, other: &KeyValue) -> bool {
        self == other && Origin::same(self.origin.as_ref(), other.origin.as_ref())
    }

    /// An enabled entry.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> KeyValue {
        KeyValue {
            name: name.into(),
            value: value.into(),
            enabled: true,
            description: None,
            origin: None,
        }
    }

    /// Read one list entry as Bruno does, remembering the YAML it came from.
    pub(crate) fn from_yaml(entry: &Value) -> KeyValue {
        let mut kv = KeyValue::new(text(entry, "name"), text(entry, "value"));
        kv.enabled = !js::is_disabled(entry);
        kv.description = description(entry);
        kv.origin = Some(Origin::new(entry.clone(), kv.fields()));
        kv
    }

    /// The entry's YAML: as read when unchanged, else Bruno's form with unknown keys kept.
    pub(crate) fn to_yaml(&self) -> Value {
        write_entry(self.origin.as_ref(), self.fields(), KEY_VALUE_ORDER)
    }

    fn fields(&self) -> Fields {
        vec![
            ("name", Some(Value::from(self.name.as_str()))),
            ("value", Some(Value::from(self.value.as_str()))),
            ("description", description_yaml(self.description.as_deref())),
            ("disabled", disabled(self.enabled)),
        ]
    }
}

/// Compares the public fields only; where an entry was read from does not matter.
impl PartialEq for KeyValue {
    fn eq(&self, other: &Self) -> bool {
        (&self.name, &self.value, self.enabled, &self.description)
            == (&other.name, &other.value, other.enabled, &other.description)
    }
}

impl Eq for KeyValue {}

/// Shows the value's size only, since header values often carry credentials.
impl fmt::Debug for KeyValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyValue")
            .field("name", &self.name)
            .field("value", &Redacted(self.value.len()))
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// Header entries as YAML; a name with a line break is [`EditError::InvalidHeaderName`],
/// since no header can carry it.
pub(crate) fn headers_to_yaml(headers: &[KeyValue]) -> Result<Vec<Value>, EditError> {
    if let Some(row) = headers.iter().position(|h| h.name.contains(['\r', '\n'])) {
        return Err(EditError::InvalidHeaderName { row });
    }
    Ok(headers.iter().map(KeyValue::to_yaml).collect())
}

/// A value shown in `Debug` by its size only.
pub(crate) struct Redacted(pub(crate) usize);

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} bytes>", self.0)
    }
}

/// `key` of a list entry as Bruno's `ensureString` reads it; empty when absent or not a scalar.
pub(crate) fn text(entry: &Value, key: &str) -> String {
    entry
        .get(key)
        .and_then(js::yaml_to_string)
        .unwrap_or_default()
}

/// An entry's `description` as Bruno reads it: a string, or a mapping's `content` string,
/// kept only when not blank.
pub(crate) fn description(entry: &Value) -> Option<String> {
    let found = match entry.get("description")? {
        Value::Mapping(map) => map.get("content")?.as_str()?,
        other => other.as_str()?,
    };
    (!js::js_trim(found).is_empty()).then(|| found.to_owned())
}

/// Bruno writes a description only when it is not blank.
pub(crate) fn description_yaml(description: Option<&str>) -> Option<Value> {
    description
        .filter(|d| !js::js_trim(d).is_empty())
        .map(Value::from)
}

/// Bruno writes `disabled: true` only for a disabled entry.
pub(crate) fn disabled(enabled: bool) -> Option<Value> {
    (!enabled).then_some(Value::Bool(true))
}

/// Bruno's key order for a variable entry.
const VARIABLE_ORDER: &[&str] = &["name", "value", "description", "disabled"];

/// The value of a request variable.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VarValue {
    /// Plain text.
    Text(String),
    /// A value with a declared type, such as `number`, and its data as text.
    Typed {
        /// The declared type.
        kind: String,
        /// The data.
        data: String,
    },
}

impl VarValue {
    fn from_yaml(value: Option<&Value>) -> VarValue {
        match value {
            Some(map @ Value::Mapping(_)) if map.get("type").is_some() => VarValue::Typed {
                kind: text(map, "type"),
                data: text(map, "data"),
            },
            Some(scalar) => VarValue::Text(js::yaml_to_string(scalar).unwrap_or_default()),
            None => VarValue::Text(String::new()),
        }
    }

    /// Bruno writes a `string` typed value as plain text.
    fn to_yaml(&self) -> Value {
        match self {
            VarValue::Typed { kind, data } if kind != "string" => {
                let mut map = yaml_serde::Mapping::new();
                map.insert(Value::from("type"), Value::from(kind.as_str()));
                map.insert(Value::from("data"), Value::from(data.as_str()));
                Value::Mapping(map)
            }
            VarValue::Typed { data, .. } => Value::from(data.as_str()),
            VarValue::Text(text) => Value::from(text.as_str()),
        }
    }

    fn len(&self) -> usize {
        match self {
            VarValue::Text(text) => text.len(),
            VarValue::Typed { data, .. } => data.len(),
        }
    }
}

/// A request variable.
#[derive(Clone)]
#[non_exhaustive]
pub struct Variable {
    /// The name.
    pub name: String,
    /// The value.
    pub value: VarValue,
    /// Whether the variable is set.
    pub enabled: bool,
    origin: Option<Origin>,
}

impl Variable {
    /// Equal (`==`) and read from the same YAML, so an unknown key the app does not model
    /// also counts; `==` itself ignores where an entry was read from.
    pub fn same_source(&self, other: &Variable) -> bool {
        self == other && Origin::same(self.origin.as_ref(), other.origin.as_ref())
    }

    /// An enabled variable.
    pub fn new(name: impl Into<String>, value: VarValue) -> Variable {
        Variable {
            name: name.into(),
            value,
            enabled: true,
            origin: None,
        }
    }

    /// Read one list entry, remembering the YAML it came from.
    pub(crate) fn from_yaml(entry: &Value) -> Variable {
        let mut var = Variable::new(text(entry, "name"), VarValue::from_yaml(entry.get("value")));
        var.enabled = !js::is_disabled(entry);
        var.origin = Some(Origin::new(entry.clone(), var.fields()));
        var
    }

    /// The entry's YAML: as read when unchanged, else Bruno's form with unknown keys kept.
    pub(crate) fn to_yaml(&self) -> Value {
        write_entry(self.origin.as_ref(), self.fields(), VARIABLE_ORDER)
    }

    fn fields(&self) -> Fields {
        vec![
            ("name", Some(Value::from(self.name.as_str()))),
            ("value", Some(self.value.to_yaml())),
            ("disabled", disabled(self.enabled)),
        ]
    }
}

/// Compares the public fields only; where an entry was read from does not matter.
impl PartialEq for Variable {
    fn eq(&self, other: &Self) -> bool {
        (&self.name, &self.value, self.enabled) == (&other.name, &other.value, other.enabled)
    }
}

impl Eq for Variable {}

/// Shows the value's size only, since variables often carry credentials.
impl fmt::Debug for Variable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Variable")
            .field("name", &self.name)
            .field("value", &Redacted(self.value.len()))
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// Bruno's key order for a param entry.
const PARAM_ORDER: &[&str] = &["name", "value", "type", "description", "disabled"];

/// Where a request param goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParamKind {
    /// A query string pair.
    Query,
    /// A `:name` path segment.
    Path,
}

impl ParamKind {
    fn as_str(self) -> &'static str {
        match self {
            ParamKind::Query => "query",
            ParamKind::Path => "path",
        }
    }
}

/// A request param.
#[derive(Clone)]
#[non_exhaustive]
pub struct Param {
    /// The name.
    pub name: String,
    /// The value, possibly with `{{variables}}`.
    pub value: String,
    /// Query or path.
    pub kind: ParamKind,
    /// Whether the param is sent.
    pub enabled: bool,
    /// A note on the param; blank text is not written.
    pub description: Option<String>,
    origin: Option<Origin>,
}

impl Param {
    /// Equal (`==`) and read from the same YAML, so an unknown key the app does not model
    /// also counts; `==` itself ignores where an entry was read from.
    pub fn same_source(&self, other: &Param) -> bool {
        self == other && Origin::same(self.origin.as_ref(), other.origin.as_ref())
    }

    /// An enabled param.
    pub fn new(name: impl Into<String>, value: impl Into<String>, kind: ParamKind) -> Param {
        Param {
            name: name.into(),
            value: value.into(),
            kind,
            enabled: true,
            description: None,
            origin: None,
        }
    }

    /// Read one list entry, remembering the YAML it came from. Any `type` but `path` reads
    /// as a query param.
    pub(crate) fn from_yaml(entry: &Value) -> Param {
        let kind = match text(entry, "type").as_str() {
            "path" => ParamKind::Path,
            _ => ParamKind::Query,
        };
        let mut param = Param::new(text(entry, "name"), text(entry, "value"), kind);
        param.enabled = !js::is_disabled(entry);
        param.description = description(entry);
        param.origin = Some(Origin::new(entry.clone(), param.fields()));
        param
    }

    /// The entry's YAML: as read when unchanged, else Bruno's form with unknown keys kept.
    pub(crate) fn to_yaml(&self) -> Value {
        write_entry(self.origin.as_ref(), self.fields(), PARAM_ORDER)
    }

    fn fields(&self) -> Fields {
        vec![
            ("name", Some(Value::from(self.name.as_str()))),
            ("value", Some(Value::from(self.value.as_str()))),
            ("type", Some(Value::from(self.kind.as_str()))),
            ("description", description_yaml(self.description.as_deref())),
            ("disabled", disabled(self.enabled)),
        ]
    }
}

/// Compares the public fields only; where a param was read from does not matter.
impl PartialEq for Param {
    fn eq(&self, other: &Self) -> bool {
        (
            &self.name,
            &self.value,
            self.kind,
            self.enabled,
            &self.description,
        ) == (
            &other.name,
            &other.value,
            other.kind,
            other.enabled,
            &other.description,
        )
    }
}

impl Eq for Param {}

/// Shows the value's size only, since query values often carry credentials.
impl fmt::Debug for Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Param")
            .field("name", &self.name)
            .field("value", &Redacted(self.value.len()))
            .field("kind", &self.kind)
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(text: &str) -> Value {
        yaml_serde::from_str(text).unwrap()
    }

    // Entries read from YAML that differs only in a key the app does not model are equal,
    // yet only `same_source` tells them apart; identical reads agree on both.
    #[test]
    fn same_source_also_compares_the_yaml_an_entry_was_read_from() {
        let old = "name: a\nvalue: \"1\"\nx-note: old\n";
        let new = "name: a\nvalue: \"1\"\nx-note: new\n";
        let (a, b) = (
            KeyValue::from_yaml(&yaml(old)),
            KeyValue::from_yaml(&yaml(new)),
        );
        assert!(a == b && !a.same_source(&b));
        assert!(a.same_source(&KeyValue::from_yaml(&yaml(old))));
        let (a, b) = (Param::from_yaml(&yaml(old)), Param::from_yaml(&yaml(new)));
        assert!(a == b && !a.same_source(&b));
        assert!(a.same_source(&Param::from_yaml(&yaml(old))));
        let (a, b) = (
            Variable::from_yaml(&yaml(old)),
            Variable::from_yaml(&yaml(new)),
        );
        assert!(a == b && !a.same_source(&b));
        assert!(a.same_source(&Variable::from_yaml(&yaml(old))));
        assert!(!a.same_source(&Variable::new("a", VarValue::Text("1".into()))));
    }
}
