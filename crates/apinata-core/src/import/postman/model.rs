//! The parts of a Postman collection the importer reads. Members that take several shapes stay
//! JSON values; each struct's `rest` keeps every member it does not name.

use serde::Deserialize;
use serde_json::{Map, Value};

/// Export metadata that carries no request data, so it is never reported as unmapped.
pub(super) const METADATA: [&str; 9] = [
    "id",
    "_postman_id",
    "uid",
    "_exporter_id",
    "_collection_link",
    "schema",
    "_postman_variable_scope",
    "_postman_exported_at",
    "_postman_exported_using",
];

/// A collection, after any `{collection: ...}` envelope is unwrapped.
#[derive(Deserialize)]
pub(super) struct Collection {
    pub(super) info: Info,
    pub(super) item: Vec<Value>,
    pub(super) auth: Option<Value>,
    pub(super) variable: Option<Value>,
    #[serde(rename = "protocolProfileBehavior")]
    pub(super) protocol_profile_behavior: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

#[derive(Deserialize)]
pub(super) struct Info {
    pub(super) name: Option<Value>,
    pub(super) description: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

/// A folder or a request; it is a folder when `request` is falsy.
#[derive(Deserialize, Default)]
pub(super) struct Item {
    pub(super) name: Option<Value>,
    pub(super) request: Option<Value>,
    pub(super) item: Option<Value>,
    pub(super) auth: Option<Value>,
    pub(super) description: Option<Value>,
    #[serde(rename = "protocolProfileBehavior")]
    pub(super) protocol_profile_behavior: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

#[derive(Deserialize, Default)]
pub(super) struct Request {
    pub(super) method: Option<Value>,
    pub(super) url: Option<Value>,
    pub(super) header: Option<Value>,
    pub(super) body: Option<Value>,
    pub(super) description: Option<Value>,
    pub(super) auth: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

#[derive(Deserialize, Default)]
pub(super) struct Url {
    pub(super) raw: Option<Value>,
    pub(super) protocol: Option<Value>,
    pub(super) host: Option<Value>,
    pub(super) path: Option<Value>,
    pub(super) port: Option<Value>,
    pub(super) query: Option<Value>,
    pub(super) variable: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

#[derive(Deserialize, Default)]
pub(super) struct Body {
    pub(super) mode: Option<Value>,
    pub(super) raw: Option<Value>,
    pub(super) options: Option<Value>,
    pub(super) urlencoded: Option<Value>,
    pub(super) formdata: Option<Value>,
    pub(super) file: Option<Value>,
    pub(super) graphql: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}
