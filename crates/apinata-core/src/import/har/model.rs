//! The parts of a HAR entry the importer reads. Scalars stay loose `Value`s, read as text the
//! way JavaScript prints them; each struct's `rest` keeps the members it does not name.

use crate::js;
use serde::Deserialize;
use serde_json::{Map, Value};

/// The text of an optional scalar; missing or `null` is empty.
pub(super) fn text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(v) => js::to_text(v),
    }
}

/// One `log.entries` element; its request is checked for shape before it deserializes.
#[derive(Deserialize)]
pub(super) struct Entry {
    pub(super) comment: Option<Value>,
    pub(super) request: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Request {
    pub(super) method: Option<Value>,
    pub(super) url: Option<Value>,
    pub(super) comment: Option<Value>,
    pub(super) headers: Option<Vec<Header>>,
    pub(super) cookies: Option<Vec<Cookie>>,
    pub(super) post_data: Option<PostData>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

#[derive(Deserialize)]
pub(super) struct Header {
    pub(super) name: Option<Value>,
    pub(super) value: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

#[derive(Deserialize)]
pub(super) struct Cookie {
    pub(super) name: Option<Value>,
    pub(super) value: Option<Value>,
    #[serde(flatten)]
    #[expect(dead_code, reason = "Decision 14 notes no unknown cookie members")]
    pub(super) rest: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PostData {
    pub(super) mime_type: Option<Value>,
    pub(super) text: Option<Value>,
    pub(super) params: Option<Vec<Param>>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Param {
    pub(super) name: Option<Value>,
    pub(super) value: Option<Value>,
    pub(super) file_name: Option<Value>,
    pub(super) content_type: Option<Value>,
    #[serde(flatten)]
    pub(super) rest: Map<String, Value>,
}
