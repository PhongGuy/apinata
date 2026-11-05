//! The parts of a Hoppscotch export the importer reads. Every member any collection or request
//! version defines is declared, so no version raises a note for another's members; each
//! struct's `rest` keeps the members it does not name.

use crate::js;
use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, Unexpected};
use serde_json::{Map, Value};

/// A text member: a string as is, a number or boolean as JavaScript prints it.
pub(super) struct Text(pub(super) String);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Text, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Null => Ok(Text(String::new())),
            Value::Array(_) => Err(de::Error::invalid_type(Unexpected::Seq, &"text")),
            Value::Object(_) => Err(de::Error::invalid_type(Unexpected::Map, &"text")),
            scalar => Ok(Text(js::to_text(&scalar))),
        }
    }
}

/// The text of an optional member; missing or `null` is empty.
pub(super) fn text(member: Option<Text>) -> String {
    member.map(|t| t.0).unwrap_or_default()
}

/// A collection or one of its folders, which share one shape at every level.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Collection {
    pub(super) name: Option<Text>,
    pub(super) folders: Option<Vec<Value>>,
    pub(super) requests: Option<Vec<Value>>,
    pub(super) auth: Option<Auth>,
    pub(super) headers: Option<Vec<Pair>>,
    pub(super) variables: Option<Vec<Variable>>,
    pub(super) description: Option<Text>,
    pub(super) pre_request_script: Option<Text>,
    pub(super) test_script: Option<Text>,
    /// Read from the raw value, where `null` differs from missing.
    #[serde(rename = "v")]
    _v: Option<IgnoredAny>,
    #[serde(rename = "id")]
    _id: Option<IgnoredAny>,
    #[serde(rename = "_ref_id")]
    _ref_id: Option<IgnoredAny>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// A REST request of any version; the legacy (v0) members sit beside the later ones.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Request {
    /// Typed so a wrong-typed name fails the request; the tree reads it from the raw value.
    #[expect(dead_code, reason = "the name is read before the request deserialises")]
    pub(super) name: Option<Text>,
    pub(super) method: Option<Text>,
    pub(super) endpoint: Option<Text>,
    pub(super) params: Option<Vec<Pair>>,
    pub(super) headers: Option<Vec<Pair>>,
    pub(super) pre_request_script: Option<Text>,
    pub(super) test_script: Option<Text>,
    pub(super) auth: Option<Auth>,
    pub(super) body: Option<BodyMember>,
    pub(super) request_variables: Option<Vec<Pair>>,
    pub(super) responses: Option<Value>,
    pub(super) description: Option<Text>,
    pub(super) url: Option<Text>,
    pub(super) path: Option<Text>,
    pub(super) content_type: Option<Text>,
    pub(super) raw_params: Option<Text>,
    pub(super) body_params: Option<Value>,
    pub(super) http_user: Option<Text>,
    pub(super) http_password: Option<Text>,
    pub(super) bearer_token: Option<Text>,
    #[serde(rename = "v")]
    _v: Option<IgnoredAny>,
    #[serde(rename = "id")]
    _id: Option<IgnoredAny>,
    #[serde(rename = "_ref_id")]
    _ref_id: Option<IgnoredAny>,
    #[serde(rename = "rawInput")]
    _raw_input: Option<IgnoredAny>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// A GraphQL request of any version, always sent as a POST to `url`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GqlRequest {
    #[expect(dead_code, reason = "the name is read before the request deserialises")]
    pub(super) name: Option<Text>,
    pub(super) url: Option<Text>,
    pub(super) headers: Option<Vec<Pair>>,
    pub(super) query: Option<Text>,
    pub(super) variables: Option<Text>,
    pub(super) auth: Option<Auth>,
    pub(super) description: Option<Text>,
    pub(super) pre_request_script: Option<Text>,
    pub(super) test_script: Option<Text>,
    pub(super) responses: Option<Value>,
    #[serde(rename = "v")]
    _v: Option<IgnoredAny>,
    #[serde(rename = "id")]
    _id: Option<IgnoredAny>,
    #[serde(rename = "_ref_id")]
    _ref_id: Option<IgnoredAny>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// A param, header or request variable.
#[derive(Deserialize)]
pub(super) struct Pair {
    pub(super) key: Option<Text>,
    pub(super) value: Option<Text>,
    pub(super) active: Option<bool>,
    pub(super) description: Option<Text>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// A collection or folder variable.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Variable {
    pub(super) key: Option<Text>,
    pub(super) initial_value: Option<Text>,
    pub(super) current_value: Option<Text>,
    pub(super) secret: Option<bool>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// An environment of any version; `variables` stays raw so one bad entry is skipped alone.
#[derive(Deserialize)]
pub(super) struct Environment {
    pub(super) name: Option<Text>,
    pub(super) variables: Vec<Value>,
    /// Read from the raw value, where `null` differs from missing.
    #[serde(rename = "v")]
    _v: Option<IgnoredAny>,
    #[serde(rename = "id")]
    _id: Option<IgnoredAny>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// An environment variable: `value` before version 2, `initialValue` and `currentValue` after.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EnvEntry {
    pub(super) key: Option<Text>,
    pub(super) value: Option<Text>,
    pub(super) initial_value: Option<Text>,
    pub(super) current_value: Option<Text>,
    pub(super) secret: Option<bool>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// An auth member: a legacy string such as `Basic Auth`, or an object whose type-specific
/// members are read only when it is on and of a mapped type, so nothing else is reported.
pub(super) enum Auth {
    Legacy(String),
    Off,
    Unsupported(String),
    Mapped(Mapped, Map<String, Value>),
}

/// The auth types Apinata models, with their members as text.
pub(super) enum Mapped {
    None,
    Inherit,
    Basic {
        username: String,
        password: String,
    },
    Bearer {
        token: String,
    },
    ApiKey {
        key: String,
        value: String,
        add_to: Option<String>,
    },
}

impl<'de> Deserialize<'de> for Auth {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Auth, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Head {
            auth_type: Option<Text>,
            auth_active: Option<bool>,
            #[serde(flatten)]
            rest: Map<String, Value>,
        }
        #[derive(Deserialize)]
        struct Basic {
            username: Option<Text>,
            password: Option<Text>,
            #[serde(flatten)]
            rest: Map<String, Value>,
        }
        #[derive(Deserialize)]
        struct Bearer {
            token: Option<Text>,
            #[serde(flatten)]
            rest: Map<String, Value>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ApiKey {
            key: Option<Text>,
            value: Option<Text>,
            add_to: Option<Text>,
            #[serde(flatten)]
            rest: Map<String, Value>,
        }
        fn read<'de, T: Deserialize<'de>, E: de::Error>(rest: Map<String, Value>) -> Result<T, E> {
            T::deserialize(Value::Object(rest)).map_err(E::custom)
        }

        let head = match Value::deserialize(deserializer)? {
            Value::String(legacy) => return Ok(Auth::Legacy(legacy)),
            other => Head::deserialize(other).map_err(de::Error::custom)?,
        };
        if head.auth_active == Some(false) {
            return Ok(Auth::Off);
        }
        let (mapped, rest) = match text(head.auth_type).as_str() {
            "none" => (Mapped::None, head.rest),
            "inherit" => (Mapped::Inherit, head.rest),
            "basic" => {
                let b: Basic = read(head.rest)?;
                let (username, password) = (text(b.username), text(b.password));
                (Mapped::Basic { username, password }, b.rest)
            }
            "bearer" => {
                let b: Bearer = read(head.rest)?;
                (
                    Mapped::Bearer {
                        token: text(b.token),
                    },
                    b.rest,
                )
            }
            "api-key" => {
                let k: ApiKey = read(head.rest)?;
                let (key, value) = (text(k.key), text(k.value));
                let add_to = k.add_to.map(|t| t.0);
                (Mapped::ApiKey { key, value, add_to }, k.rest)
            }
            other => return Ok(Auth::Unsupported(other.to_owned())),
        };
        Ok(Auth::Mapped(mapped, rest))
    }
}

/// A request's body member: a legacy string before version 1, an object after.
pub(super) enum BodyMember {
    Legacy(String),
    Object(Body),
}

/// A version 1 or later body; multipart entries are typed, other contents kept raw.
pub(super) struct Body {
    pub(super) content_type: Option<String>,
    pub(super) content: Content,
    pub(super) rest: Map<String, Value>,
}

pub(super) enum Content {
    Form(Vec<FormField>),
    Raw(Value),
}

/// A multipart entry; a file's contents never survive export.
pub(super) struct FormField {
    pub(super) key: String,
    /// The text, or `None` for a file.
    pub(super) value: Option<String>,
    pub(super) active: Option<bool>,
    pub(super) content_type: Option<String>,
    pub(super) rest: Map<String, Value>,
}

impl<'de> Deserialize<'de> for BodyMember {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<BodyMember, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            content_type: Option<Text>,
            body: Option<Value>,
            #[serde(rename = "showIndividualContentType")]
            _individual: Option<IgnoredAny>,
            #[serde(rename = "isBulkEditing")]
            _bulk: Option<IgnoredAny>,
            #[serde(flatten)]
            rest: Map<String, Value>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Entry {
            key: Option<Text>,
            value: Option<Value>,
            active: Option<bool>,
            is_file: Option<bool>,
            content_type: Option<Text>,
            #[serde(flatten)]
            rest: Map<String, Value>,
        }

        let raw = match Value::deserialize(deserializer)? {
            object @ Value::Object(_) => Raw::deserialize(object).map_err(de::Error::custom)?,
            other => {
                return Text::deserialize(other)
                    .map(|t| BodyMember::Legacy(t.0))
                    .map_err(de::Error::custom);
            }
        };
        let content_type = raw.content_type.map(|t| t.0);
        let body = raw.body.unwrap_or(Value::Null);
        let content = if content_type.as_deref() == Some("multipart/form-data") {
            let entries = Option::<Vec<Entry>>::deserialize(body).map_err(de::Error::custom)?;
            let mut fields = Vec::new();
            for e in entries.unwrap_or_default() {
                let value = match e.is_file {
                    Some(true) => None,
                    _ => Some(text(
                        Option::<Text>::deserialize(e.value.unwrap_or(Value::Null))
                            .map_err(de::Error::custom)?,
                    )),
                };
                fields.push(FormField {
                    key: text(e.key),
                    value,
                    active: e.active,
                    content_type: e.content_type.map(|t| t.0),
                    rest: e.rest,
                });
            }
            Content::Form(fields)
        } else {
            Content::Raw(body)
        };
        Ok(BodyMember::Object(Body {
            content_type,
            content,
            rest: raw.rest,
        }))
    }
}
