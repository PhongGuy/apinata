//! Environment variables, color and `extends`, written in Bruno's `stringifyEnvironment` form.

use crate::collection::Environment;
use crate::document::{DiskState, Fields, Origin, write_entry};
use crate::edit::{Redacted, VarValue, Variable, disabled, text};
use crate::error::SaveError;
use crate::js;
use std::fmt;
use yaml_serde::Value;

/// Bruno's writer order for an environment file's top level.
pub(crate) const TOP: &[&str] = &["name", "extends", "color", "variables", "externalSecrets"];
/// Bruno's writer order for one variable entry.
const ENTRY: &[&str] = &["secret", "name", "value", "type", "disabled", "description"];

/// A variable of an environment file.
#[derive(Clone)]
#[non_exhaustive]
pub struct EnvVariable {
    /// The name.
    pub name: String,
    /// The value. A secret's value is never written, only its type.
    pub value: VarValue,
    /// Whether the variable is set.
    pub enabled: bool,
    /// Whether the value lives outside the file.
    pub secret: bool,
    origin: Option<Origin>,
}

impl EnvVariable {
    /// An enabled, plain variable.
    pub fn new(name: impl Into<String>, value: VarValue) -> EnvVariable {
        EnvVariable {
            name: name.into(),
            value,
            enabled: true,
            secret: false,
            origin: None,
        }
    }

    /// An enabled secret with no value.
    pub fn secret(name: impl Into<String>) -> EnvVariable {
        let mut var = EnvVariable::new(name, VarValue::Text(String::new()));
        var.secret = true;
        var
    }

    /// Read one entry, remembering the YAML it came from.
    fn from_yaml(entry: &Value) -> EnvVariable {
        let secret = entry.get("secret") == Some(&Value::Bool(true));
        let mut var = if secret {
            // A secret keeps only its declared type; Bruno reads no value for it.
            let value = match entry.get("type").and_then(Value::as_str) {
                Some(kind) if kind != "string" => VarValue::Typed {
                    kind: kind.to_owned(),
                    data: String::new(),
                },
                _ => VarValue::Text(String::new()),
            };
            let mut var = EnvVariable::new(text(entry, "name"), value);
            var.secret = true;
            var.enabled = !js::is_disabled(entry);
            var
        } else {
            let plain = Variable::from_yaml(entry);
            let mut var = EnvVariable::new(plain.name, plain.value);
            var.enabled = plain.enabled;
            var
        };
        var.origin = Some(Origin::new(entry.clone(), var.fields()));
        var
    }

    /// The entry's YAML: as read when unchanged, else Bruno's form with unknown keys kept.
    fn to_yaml(&self) -> Value {
        write_entry(self.origin.as_ref(), self.fields(), ENTRY)
    }

    fn fields(&self) -> Fields {
        let secret_kind = match (&self.value, self.secret) {
            (VarValue::Typed { kind, .. }, true) if kind != "string" => {
                Some(Value::from(kind.as_str()))
            }
            _ => None,
        };
        vec![
            ("secret", self.secret.then_some(Value::Bool(true))),
            ("name", Some(Value::from(self.name.as_str()))),
            ("value", (!self.secret).then(|| value_yaml(&self.value))),
            ("type", secret_kind),
            ("disabled", disabled(self.enabled)),
        ]
    }

    fn value_len(&self) -> usize {
        match &self.value {
            VarValue::Text(text) => text.len(),
            VarValue::Typed { data, .. } => data.len(),
        }
    }
}

/// Bruno writes a `string` typed value as plain text and any other type as `{type, data}`.
fn value_yaml(value: &VarValue) -> Value {
    match value {
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

/// Compares the public fields only; where an entry was read from does not matter.
impl PartialEq for EnvVariable {
    fn eq(&self, other: &Self) -> bool {
        (&self.name, &self.value, self.enabled, self.secret)
            == (&other.name, &other.value, other.enabled, other.secret)
    }
}

impl Eq for EnvVariable {}

/// Shows the value's size only, since variables often carry credentials.
impl fmt::Debug for EnvVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvVariable")
            .field("name", &self.name)
            .field("value", &Redacted(self.value_len()))
            .field("enabled", &self.enabled)
            .field("secret", &self.secret)
            .finish()
    }
}

impl Environment {
    /// The variables in file order, disabled ones included.
    pub fn variables(&self) -> Vec<EnvVariable> {
        self.document()
            .get(&["variables"])
            .and_then(Value::as_sequence)
            .map(|items| items.iter().map(EnvVariable::from_yaml).collect())
            .unwrap_or_default()
    }

    /// Replace the variables; none removes the key. A secret's value is never written.
    pub fn set_variables(&mut self, variables: Vec<EnvVariable>) {
        let items = variables.iter().map(EnvVariable::to_yaml).collect();
        self.document_mut().set_list(&["variables"], items, &[TOP]);
    }

    /// The color, when it is a non-empty string.
    pub fn color(&self) -> Option<String> {
        self.document()
            .get(&["color"])
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_owned)
    }

    /// Set the color; `None` or an empty string removes it.
    pub fn set_color(&mut self, color: Option<String>) {
        if color == self.color() {
            return;
        }
        match color.filter(|c| !c.is_empty()) {
            Some(color) => self
                .document_mut()
                .set(&["color"], Value::String(color), &[TOP]),
            None => {
                self.document_mut().remove(&["color"]);
            }
        }
    }

    /// The name of the environment this one extends, trimmed, when non-empty.
    pub fn extends(&self) -> Option<String> {
        self.document()
            .get(&["extends"])
            .and_then(Value::as_str)
            .map(|p| js::js_trim(p).to_owned())
            .filter(|p| !p.is_empty())
    }

    /// Set the parent environment. The name is trimmed; `None` or a blank name removes the key.
    pub fn set_extends(&mut self, parent: Option<String>) {
        let parent = parent
            .map(|p| js::js_trim(&p).to_owned())
            .filter(|p| !p.is_empty());
        if parent == self.extends() {
            return;
        }
        match parent {
            Some(parent) => self
                .document_mut()
                .set(&["extends"], Value::String(parent), &[TOP]),
            None => {
                self.document_mut().remove(&["extends"]);
            }
        }
    }

    /// Whether an edit has changed the file since it was loaded or last saved.
    pub fn is_dirty(&self) -> bool {
        self.document().is_dirty()
    }

    /// What the disk holds for this environment; only a dirty one outlives its file.
    pub fn disk_state(&self) -> DiskState {
        self.document().disk_state()
    }

    /// Write the file when it changed; refuses to overwrite a file that changed on disk.
    pub fn save(&mut self) -> Result<(), SaveError> {
        self.document_mut().save()
    }
}
