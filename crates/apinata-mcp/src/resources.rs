//! MCP resources: the collection's requests, folders and environments as read-only JSON.
//!
//! Every resource is a view built from the loaded [`Collection`], never a file read, so a path
//! outside the root, a symlink target, `.git` and `.env` are unreachable by construction. A URI is
//! `apinata://<collection>/<item path>` for a request, the same with a trailing `/` for a folder
//! (the bare root is the collection's own config) and `apinata://<collection>/@environment/<name>`
//! for an environment.

use apinata_core::{Collection, ItemPath, ListedItem, ListedKind};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use rmcp::model::Resource;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

use apinata_core::{EnvOverlay, RuntimeVars, SecretStore, TokenCache};

use crate::options::McpOptions;
use crate::reads;
use crate::send;

const SCHEME: &str = "apinata://";
pub(crate) const ENVIRONMENT: &str = "@environment";
const JSON: &str = "application/json";

/// Everything but the unreserved characters is escaped, so a segment never holds `/`.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// What a resource URI names.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Folder(String),
    Request(String),
    Environment(String),
}

/// Why a read failed: the URI names nothing the collection holds, or the item exists but its
/// document could not be built.
pub(crate) enum ReadError {
    NotFound,
    Failed(String),
}

pub(crate) fn encode(segment: &str) -> String {
    utf8_percent_encode(segment, SEGMENT).to_string()
}

pub(crate) fn base(collection: &Collection) -> String {
    format!("{SCHEME}{}/", encode(collection.name()))
}

/// The target `uri` names within the collection called `name`; `None` for any other collection,
/// scheme or a path that is not a valid item path (`..`, `\`, `:`, empty segments).
pub(crate) fn parse(uri: &str, name: &str) -> Option<Target> {
    let rest = uri.strip_prefix(SCHEME)?;
    let (host, path) = rest.split_once('/')?;
    if percent_decode_str(host).decode_utf8().ok()? != name {
        return None;
    }
    if let Some(env) = path
        .strip_prefix(ENVIRONMENT)
        .and_then(|r| r.strip_prefix('/'))
    {
        let env = percent_decode_str(env).decode_utf8().ok()?;
        return (!env.is_empty() && !env.contains('/')).then(|| Target::Environment(env.into()));
    }
    let folder = path.is_empty() || path.ends_with('/');
    // Each segment is decoded on its own, so an escaped slash cannot pass for a separator and
    // every item has exactly one URI.
    let mut segments = Vec::new();
    for raw in path.trim_end_matches('/').split('/') {
        let segment = percent_decode_str(raw).decode_utf8().ok()?;
        if segment.contains('/') {
            return None;
        }
        segments.push(segment.into_owned());
    }
    let item = ItemPath::new(&segments.join("/"))?;
    let path = item.as_str().to_owned();
    Some(if folder {
        Target::Folder(path)
    } else {
        Target::Request(path)
    })
}

/// Every HTTP and GraphQL request, every folder, every environment and the root.
pub(crate) fn list(collection: &Collection) -> Vec<Resource> {
    let prefix = base(collection);
    let mut out = vec![
        Resource::new(prefix.clone(), collection.name())
            .with_description("The collection's own headers, auth, variables and docs")
            .with_mime_type(JSON),
    ];
    if let Ok(items) = collection.list_items(None) {
        walk(&prefix, items.items(), &mut out);
    }
    for env in collection.environments() {
        let uri = format!("{prefix}{ENVIRONMENT}/{}", encode(env.name()));
        out.push(
            Resource::new(uri, format!("environment {}", env.name()))
                .with_description("An environment's variables; secret values are never shown")
                .with_mime_type(JSON),
        );
    }
    out
}

/// One string per listed resource covering everything `resources/list` shows, so a rename or a
/// changed description differs from the previous listing.
pub(crate) fn signature(collection: &Collection) -> Vec<String> {
    list(collection)
        .iter()
        .map(|r| serde_json::to_string(r).unwrap_or_default())
        .collect()
}

fn walk(prefix: &str, items: &[ListedItem], out: &mut Vec<Resource>) {
    for item in items {
        let escaped: Vec<String> = item.path().split('/').map(encode).collect();
        let uri = format!("{prefix}{}", escaped.join("/"));
        match item.kind() {
            ListedKind::Request => out.push(
                Resource::new(uri, item.name())
                    .with_title(item.path())
                    .with_description("A request file")
                    .with_mime_type(JSON),
            ),
            ListedKind::Folder => {
                out.push(
                    Resource::new(format!("{uri}/"), item.name())
                        .with_title(format!("{}/", item.path()))
                        .with_description("A folder's shared headers, auth, variables and docs")
                        .with_mime_type(JSON),
                );
                if let Some(children) = item.children() {
                    walk(prefix, children, out);
                }
            }
            // A request type Apinata does not model, or a file that failed to load, has no document
            // to show.
            _ => {}
        }
    }
}

/// Whether the collection holds the item `target` names.
pub(crate) fn exists(collection: &Collection, target: &Target) -> bool {
    match target {
        Target::Request(path) => {
            ItemPath::new(path).is_some_and(|p| collection.request(&p).is_some())
        }
        Target::Folder(path) => {
            ItemPath::new(path).is_some_and(|p| p.is_root() || collection.folder(&p).is_some())
        }
        Target::Environment(name) => collection.environment(name).is_some(),
    }
}

/// The JSON document the matching read tool returns for `target`.
pub(crate) fn read(collection: &Collection, target: &Target) -> Result<Value, ReadError> {
    match target {
        Target::Request(path) => {
            let known = ItemPath::new(path).is_some_and(|p| collection.request(&p).is_some());
            if !known {
                return Err(ReadError::NotFound);
            }
            document(reads::request(collection, path))
        }
        Target::Folder(path) => {
            let known =
                ItemPath::new(path).is_some_and(|p| p.is_root() || collection.folder(&p).is_some());
            if !known {
                return Err(ReadError::NotFound);
            }
            document(reads::folder(collection, path))
        }
        Target::Environment(name) => {
            if collection.environment(name).is_none() {
                return Err(ReadError::NotFound);
            }
            document(reads::environment(collection, name))
        }
    }
}

fn document<T: Serialize>(doc: Result<T, String>) -> Result<Value, ReadError> {
    let doc = doc.map_err(ReadError::Failed)?;
    serde_json::to_value(doc).map_err(|e| ReadError::Failed(e.to_string()))
}

/// Every secret value the collection holds: those of the default environment and of each named
/// one, so the session redactor knows them before a resource or prompt is built. A context that
/// cannot be built (a keychain failure, say) adds nothing; its values were never read.
pub(crate) fn secret_values(
    collection: &Collection,
    options: &McpOptions,
    store: &Arc<dyn SecretStore>,
) -> Vec<String> {
    let named = collection
        .environments()
        .iter()
        .map(|env| Some(env.name().to_owned()));
    let mut values = Vec::new();
    for env in std::iter::once(None).chain(named) {
        let context = send::context(
            collection,
            env.as_deref(),
            &Default::default(),
            options,
            Arc::clone(store),
            RuntimeVars::new(),
            TokenCache::in_memory(),
            EnvOverlay::new(),
        );
        if let Ok(context) = context {
            values.extend(context.secret_values());
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_names_each_kind() {
        assert_eq!(
            parse("apinata://shop/", "shop"),
            Some(Target::Folder(String::new()))
        );
        assert_eq!(
            parse("apinata://shop/a%20b/", "shop"),
            Some(Target::Folder("a b".into()))
        );
        assert_eq!(
            parse("apinata://shop/a/b.yml", "shop"),
            Some(Target::Request("a/b.yml".into()))
        );
        assert_eq!(
            parse("apinata://shop/@environment/dev", "shop"),
            Some(Target::Environment("dev".into()))
        );
    }

    #[test]
    fn parse_refuses_what_is_not_a_clean_item_path() {
        for uri in [
            "apinata://shop/../x",
            "apinata://shop/%2e%2e/x",
            "apinata://shop/a/%2E%2E/x.yml",
            "apinata://shop/a%2f..%2fx.yml",
            "apinata://shop/a//b.yml",
            "apinata://shop/a:b.yml",
            "apinata://other/x.yml",
            "apinata://shop",
            "file:///etc/passwd",
            "apinata://shop/%ff.yml",
            "apinata://shop/a%2fb.yml",
            "apinata://shop/a%2Fb/",
            "apinata://shop/@environment/a%2fb",
        ] {
            assert_eq!(parse(uri, "shop"), None, "{uri}");
        }
    }
}
