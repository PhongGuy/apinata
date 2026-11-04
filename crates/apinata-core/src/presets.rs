//! Collection presets: the request type and URL a new request starts with.

use crate::document::Document;
use crate::js;
use crate::request::RequestKind;
use yaml_serde::Value;

/// Bruno's writer key orders under `extensions.bruno` (`stringifyCollection.ts`).
const BRUNO: &[&str] = &["ignore", "presets", "scripts"];
const PRESETS: &[&str] = &["request", "defaultEnvironment"];
const REQUEST: &[&str] = &["type", "url"];
const PATH: [&str; 4] = ["extensions", "bruno", "presets", "request"];

/// `extensions.bruno.presets` of `opencollection.yml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CollectionPresets {
    /// The request type new requests start as; empty when unset.
    pub request_type: String,
    /// The URL new requests start with; empty when unset.
    pub request_url: String,
    default_environment: String,
}

impl CollectionPresets {
    /// Presets with a request type and URL.
    pub fn new(request_type: impl Into<String>, request_url: impl Into<String>) -> Self {
        CollectionPresets {
            request_type: request_type.into(),
            request_url: request_url.into(),
            default_environment: String::new(),
        }
    }

    /// The default environment's name; empty when unset. Editing presets leaves it alone.
    pub fn default_environment(&self) -> &str {
        &self.default_environment
    }

    /// The kind a new request starts as: GraphQL for `graphql`, otherwise HTTP, which is also
    /// what the types this app cannot create (`grpc`, `ws`) and unknown ones mean.
    pub fn kind(&self) -> RequestKind {
        match self.request_type.as_str() {
            "graphql" => RequestKind::Graphql,
            _ => RequestKind::Http,
        }
    }

    pub(crate) fn read(doc: &Document) -> CollectionPresets {
        let text = |keys: &[&str]| {
            doc.get(keys)
                .and_then(js::yaml_to_string)
                .unwrap_or_default()
        };
        CollectionPresets {
            request_type: text(&["extensions", "bruno", "presets", "request", "type"]),
            request_url: text(&["extensions", "bruno", "presets", "request", "url"]),
            default_environment: text(&["extensions", "bruno", "presets", "defaultEnvironment"]),
        }
    }

    /// Write the type and URL as Bruno's writer does: each only when non-empty, `request` only
    /// while it holds a key, and `presets` and `bruno` only while they hold anything. Keys this
    /// reader does not know, `defaultEnvironment` included, stay. `extensions` stays, as the
    /// reference always writes it.
    pub(crate) fn write(&self, doc: &mut Document, top: &[&str]) {
        let order: &[&[&str]] = &[top, BRUNO, PRESETS, REQUEST];
        for (key, value) in [("type", &self.request_type), ("url", &self.request_url)] {
            let keys = [PATH[0], PATH[1], PATH[2], PATH[3], key];
            if value.is_empty() {
                doc.remove(&keys);
            } else {
                doc.set(&keys, Value::String(value.clone()), order);
            }
        }
        doc.remove_if_empty(&PATH);
        doc.remove_if_empty(&PATH[..3]);
        doc.remove_if_empty(&PATH[..2]);
    }
}
