//! Importing OpenAPI 3 and Swagger 2.0 descriptions.

mod body;
mod bounds;
mod coverage;
mod group;
mod operations;
mod params;
mod refs;
mod sample;
mod security;
mod servers;
mod version;
mod xml;

use super::load::{Loaded, Syntax, merge_key_notes};
use super::{
    CollectionOptions, DocumentError, ImportBundle, ImportNote, ImportedCollection, NoteKind,
};
use bounds::{Bounds, Budget};
use refs::{RefKind, Resolved, Resolver};
use serde_json::Value;
use std::borrow::Cow;
use version::Version;

/// One document being imported: its root, declared version and `$ref` resolver.
pub(crate) struct Doc<'a> {
    pub(crate) root: &'a Value,
    pub(crate) version: Version,
    refs: Resolver<'a>,
    /// Every entry the import produces is charged here.
    pub(crate) budget: Budget,
    /// Resolver notes already charged, waiting for `take_ref_notes`.
    ref_notes: Vec<ImportNote>,
    /// The limits the budget enforces, plus the sample bounds `sample` reads.
    pub(crate) bounds: Bounds,
    /// Schema steps taken by every sample of this document so far.
    pub(crate) visits: u64,
    /// Auth placeholders used so far, added to the environments as secrets at the end.
    pub(crate) placeholders: security::Placeholders,
    /// Shared `$ref` targets the coverage walk has read, with their unknown keys.
    pub(crate) walked: coverage::Walked,
}

impl<'a> Doc<'a> {
    pub(crate) fn new(root: &'a Value, version: Version, bounds: Bounds) -> Doc<'a> {
        Doc {
            root,
            version,
            refs: Resolver::new(root, version),
            budget: Budget::new(bounds),
            ref_notes: Vec::new(),
            bounds,
            visits: 0,
            placeholders: security::Placeholders::default(),
            walked: coverage::Walked::default(),
        }
    }

    /// Resolve `value` as `Resolver::resolve` does, charging the bytes its sibling merge copied
    /// and its notes at once, so a fan-out of merges stops within one merge of the budget.
    pub(crate) fn resolve<'v>(
        &mut self,
        value: &'v Value,
        kind: RefKind,
        source: &str,
    ) -> Result<Cow<'v, Value>, DocumentError>
    where
        'a: 'v,
    {
        self.resolve_followed(value, kind, source)
            .map(|resolved| resolved.value)
    }

    /// `resolve` with the hops kept, for walks that `enter` and `leave` the resolved value.
    pub(crate) fn resolve_followed<'v>(
        &mut self,
        value: &'v Value,
        kind: RefKind,
        source: &str,
    ) -> Result<Resolved<'v>, DocumentError>
    where
        'a: 'v,
    {
        let resolved = self.refs.resolve(value, kind, source);
        self.budget.charge_bytes(self.refs.take_copied())?;
        for note in self.refs.take_notes() {
            self.budget.charge_note(&note)?;
            self.ref_notes.push(note);
        }
        Ok(resolved)
    }

    /// A Reference Object's target as `Resolver::target` gives it, with its notes charged; a
    /// walk that only reads keys uses it so a sibling merge is not copied again.
    pub(crate) fn resolve_target<'v>(
        &mut self,
        value: &'v Value,
        source: &str,
    ) -> Result<Resolved<'v>, DocumentError>
    where
        'a: 'v,
    {
        let resolved = self.refs.target(value, source);
        for note in self.refs.take_notes() {
            self.budget.charge_note(&note)?;
            self.ref_notes.push(note);
        }
        Ok(resolved)
    }

    /// Mark the hops of a resolved value as being walked, so a ref back to them is a cycle.
    pub(crate) fn enter(&mut self, followed: &[String]) {
        self.refs.enter(followed);
    }

    /// Undo the matching `enter`.
    pub(crate) fn leave(&mut self, followed: &[String]) {
        self.refs.leave(followed);
    }

    /// The resolver's notes since the last call, already charged, oldest first.
    pub(crate) fn take_ref_notes(&mut self) -> Vec<ImportNote> {
        std::mem::take(&mut self.ref_notes)
    }
}

/// Parse an OpenAPI or Swagger description into its collections and their environments.
pub(crate) fn parse_openapi(
    loaded: Loaded,
    options: &CollectionOptions,
) -> Result<Vec<ImportBundle>, DocumentError> {
    parse_openapi_with(loaded, options, Bounds::DEFAULT)
}

/// `parse_openapi` within `bounds`; past the entry budget the whole import is `TooLarge`.
pub(crate) fn parse_openapi_with(
    loaded: Loaded,
    options: &CollectionOptions,
    bounds: Bounds,
) -> Result<Vec<ImportBundle>, DocumentError> {
    // The walk borrows the loaded value, so it stays alive until the bundle is built.
    let root = &loaded.value;
    let version = Version::of(root)?;
    let mut doc = Doc::new(root, version, bounds);
    let mut unwritten = Vec::new();
    if loaded.syntax == Syntax::Yaml {
        for note in merge_key_notes(root) {
            doc.budget.charge_note(&note)?;
            unwritten.push(note);
        }
    }
    if root.get("webhooks").is_some() {
        let note = ImportNote::new("webhooks", NoteKind::Dropped, "webhooks are not imported");
        doc.budget.charge_note(&note)?;
        unwritten.push(note);
    }
    // The collection's auth comes first, so its placeholders lead the first-use order.
    let mut notes = Vec::new();
    let auth = security::collection_auth(&mut doc, &mut notes)?;
    let operations = operations::collect(&mut doc, &mut unwritten)?;
    let mut pairs = Vec::with_capacity(operations.len());
    for op in operations {
        let request = operations::request_for(&mut doc, &op)?;
        pairs.push((op, request));
    }
    let items = group::build(&mut doc, pairs, options.group_by, &mut unwritten)?;
    coverage::document_notes(&mut doc, &mut unwritten)?;
    let mut environments = servers::environments(&mut doc, &mut notes)?;
    security::add_secrets(&mut doc, &mut environments)?;
    let info = root.get("info");
    let text = |key: &str| info.and_then(|i| i.get(key)).and_then(Value::as_str);
    let title = text("title").unwrap_or_default().trim();
    let docs = text("description").unwrap_or_default();
    doc.budget.charge_text(title)?;
    doc.budget.charge_text(docs)?;
    let collection = ImportedCollection {
        scripts: Default::default(),
        name: if title.is_empty() {
            "Untitled Collection".to_owned()
        } else {
            title.to_owned()
        },
        docs: docs.to_owned(),
        auth,
        headers: Vec::new(),
        vars: Vec::new(),
        items,
        notes,
        unwritten,
        spec_source: true,
    };
    Ok(vec![ImportBundle {
        collection,
        environments,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::load::load;
    use serde_json::{Map, Value, json};

    fn run(doc: &Value, entries: u64) -> Result<Vec<ImportBundle>, DocumentError> {
        let loaded = load(doc.to_string().as_bytes()).expect("loads");
        parse_openapi_with(
            loaded,
            &CollectionOptions::default(),
            Bounds {
                entries,
                ..Bounds::DEFAULT
            },
        )
    }

    fn run_bytes(doc: &Value, bytes: u64) -> Result<Vec<ImportBundle>, DocumentError> {
        let loaded = load(doc.to_string().as_bytes()).expect("loads");
        let bounds = Bounds {
            bytes,
            ..Bounds::DEFAULT
        };
        parse_openapi_with(loaded, &CollectionOptions::default(), bounds)
    }

    /// `paths` refs, each with `extra` beside its `$ref`, to one path item.
    fn shared_item(version: &str, paths: usize, extra: &Value, item: &Value) -> Value {
        let mut map = Map::new();
        for i in 0..paths {
            let mut r = extra.as_object().cloned().unwrap_or_default();
            r.insert("$ref".into(), json!("#/components/pathItems/item"));
            map.insert(format!("/p{i}"), Value::Object(r));
        }
        json!({"openapi": version, "servers": [{"url": "https://x.test"}], "paths": map,
            "components": {"pathItems": {"item": item}}})
    }

    #[test]
    fn copied_description_text_stops_at_the_byte_budget() {
        // A 64 KiB description is copied into each request's docs (and its name).
        let item = json!({"get": {"summary": "s", "description": "d".repeat(64 << 10)}});
        let doc = shared_item("3.0.0", 64, &json!({}), &item);
        assert_eq!(
            run_bytes(&doc, 8 << 20).expect("4 MiB of docs fit")[0]
                .collection
                .items
                .len(),
            64
        );
        assert_eq!(
            run_bytes(&doc, 1 << 20).unwrap_err(),
            DocumentError::TooLarge {
                limit: 1 << 20,
                unit: "imported bytes"
            }
        );
    }

    #[test]
    fn a_sibling_merge_charges_the_copied_path_item() {
        // Each 3.1 ref with a summary sibling copies the path item, here 64 KiB of extension
        // the requests never use.
        let item = json!({"get": {}, "x-big": "x".repeat(64 << 10)});
        let doc = shared_item("3.1.0", 64, &json!({"summary": "s"}), &item);
        assert!(run_bytes(&doc, 8 << 20).is_ok());
        assert_eq!(
            run_bytes(&doc, 1 << 20).unwrap_err(),
            DocumentError::TooLarge {
                limit: 1 << 20,
                unit: "imported bytes"
            }
        );
        let plain = shared_item("3.1.0", 64, &json!({}), &item);
        assert!(run_bytes(&plain, 1 << 20).is_ok(), "no sibling, no copy");
    }

    #[test]
    fn sibling_merged_parameters_stop_within_one_merge_of_the_byte_budget() {
        // Each 3.1 parameter ref with a summary sibling copies a 64 KiB description; the walk
        // must fail at the first merge past the budget, not after building all of them.
        let refs: Vec<Value> = (0..256)
            .map(|_| json!({"$ref": "#/components/parameters/p", "summary": "s"}))
            .collect();
        let target = json!({"name": "q", "in": "query", "description": "d".repeat(64 << 10)});
        let root = json!({"openapi": "3.1.0", "paths": {"/a": {"get": {"parameters": refs}}},
            "components": {"parameters": {"p": target}}});
        let limit = 1 << 20;
        let bounds = Bounds {
            bytes: limit,
            ..Bounds::DEFAULT
        };
        let mut doc = Doc::new(&root, Version::of(&root).expect("3.1"), bounds);
        let ops = operations::collect(&mut doc, &mut Vec::new()).expect("one path item");
        let err = operations::request_for(&mut doc, &ops[0]).unwrap_err();
        assert_eq!(
            err,
            DocumentError::TooLarge {
                limit,
                unit: "imported bytes"
            }
        );
        let one_merge = bounds::approx_size(&target) + 64;
        assert!(
            doc.budget.bytes() <= limit + one_merge,
            "charged {} bytes before failing",
            doc.budget.bytes()
        );
    }

    fn fan_out(paths: usize) -> Value {
        let mut map = Map::new();
        for i in 0..paths {
            map.insert(
                format!("/p{i}"),
                json!({"$ref": "#/components/pathItems/item"}),
            );
        }
        json!({"openapi": "3.1.0", "servers": [{"url": "https://x.test"}], "paths": map,
            "components": {"pathItems": {"item": {"get": {}, "put": {}}}}})
    }

    #[test]
    fn path_item_fan_out_stops_at_the_entry_budget() {
        assert_eq!(
            run(&fan_out(40), 100).expect("80 requests fit")[0]
                .collection
                .items
                .len(),
            80
        );
        assert_eq!(
            run(&fan_out(60), 100).unwrap_err(),
            DocumentError::TooLarge {
                limit: 100,
                unit: "imported entries"
            }
        );
    }

    #[test]
    fn notes_count_against_the_budget() {
        // Each external path-item ref is one note in `unwritten` and no request.
        let mut paths = Map::new();
        for i in 0..16 {
            paths.insert(
                format!("/p{i}"),
                json!({"$ref": format!("other.yaml#/p{i}")}),
            );
        }
        let doc =
            json!({"openapi": "3.0.0", "servers": [{"url": "https://x.test"}], "paths": paths});
        assert_eq!(
            run(&doc, 30).expect("sixteen notes fit")[0]
                .collection
                .unwritten
                .len(),
            16
        );
        assert!(matches!(
            run(&doc, 10),
            Err(DocumentError::TooLarge { limit: 10, .. })
        ));
    }

    #[test]
    fn server_variables_count_against_the_entry_budget() {
        // One environment with `baseUrl` and fifty variables, and the same as one request's
        // variables: 52 entries and 52 more with the request.
        let vars: Map<String, Value> = (0..50)
            .map(|i| (format!("v{i}"), json!({"default": "x"})))
            .collect();
        let server = json!([{"url": "https://x.test", "variables": vars}]);
        let doc = json!({"openapi": "3.0.0", "servers": server,
            "paths": {"/a": {"servers": server, "get": {}}}});
        assert!(run(&doc, 104).is_ok());
        assert_eq!(
            run(&doc, 103).unwrap_err(),
            DocumentError::TooLarge {
                limit: 103,
                unit: "imported entries"
            }
        );
    }

    #[test]
    fn a_large_default_used_many_times_in_a_server_url_stays_cheap() {
        // Spelled out, each URL would be 1 GB; only its scheme is read, so 4 MiB suffices.
        let url = format!("{{s}}://x.test/{}", "{a}".repeat(100_000));
        let server = json!([{"url": url, "variables": {
            "s": {"default": "https"}, "a": {"default": "x".repeat(10_000)}}}]);
        let doc = json!({"openapi": "3.0.0", "servers": server,
            "paths": {"/a": {"servers": server, "get": {}}}});
        let bundle = run_bytes(&doc, 4 << 20)
            .expect("imports within 4 MiB")
            .remove(0);
        assert!(
            bundle.collection.notes.is_empty(),
            "the URL reads as absolute"
        );
        assert_eq!(bundle.environments[0].variables.len(), 3);
    }

    #[test]
    fn parameter_fan_out_stops_at_the_entry_budget() {
        let params: Vec<Value> = (0..30)
            .map(|_| json!({"$ref": "#/components/parameters/p"}))
            .collect();
        let mut paths = Map::new();
        for i in 0..10 {
            paths.insert(
                format!("/p{i}"),
                json!({"get": {"parameters": params.clone()}}),
            );
        }
        let doc = json!({"openapi": "3.0.0", "servers": [{"url": "https://x.test"}], "paths": paths,
            "components": {"parameters": {"p": {"name": "q", "in": "query", "example": "1"}}}});
        // 300 occurrences are charged although each request ends with one param.
        assert!(run(&doc, 1_000).is_ok());
        assert!(matches!(
            run(&doc, 100),
            Err(DocumentError::TooLarge { limit: 100, .. })
        ));
    }

    #[test]
    fn a_shared_parameter_example_is_charged_per_request() {
        // One 64 KiB header example, referenced by 64 operations, is copied 64 times.
        let mut paths = Map::new();
        for i in 0..64 {
            paths.insert(
                format!("/p{i}"),
                json!({"get": {"parameters": [{"$ref": "#/components/parameters/p"}]}}),
            );
        }
        let doc = json!({"openapi": "3.0.0", "servers": [{"url": "https://x.test"}], "paths": paths,
            "components": {"parameters": {"p": {"name": "h", "in": "header", "example": "x".repeat(64 << 10)}}}});
        assert!(run_bytes(&doc, 8 << 20).is_ok());
        assert_eq!(
            run_bytes(&doc, 1 << 20).unwrap_err(),
            DocumentError::TooLarge {
                limit: 1 << 20,
                unit: "imported bytes"
            }
        );
    }

    #[test]
    fn path_folders_count_against_the_entry_budget() {
        let mut paths = Map::new();
        for i in 0..10 {
            paths.insert(format!("/a{i}/b/c"), json!({"get": {}}));
        }
        let doc =
            json!({"openapi": "3.0.0", "servers": [{"url": "https://x.test"}], "paths": paths});
        let loaded = || load(doc.to_string().as_bytes()).expect("loads");
        let by_path = CollectionOptions {
            group_by: crate::import::GroupBy::Path,
            ..CollectionOptions::default()
        };
        // Ten requests and thirty folders; the slack covers the environment's variable.
        let bounds = |entries| Bounds {
            entries,
            ..Bounds::DEFAULT
        };
        assert!(parse_openapi_with(loaded(), &by_path, bounds(45)).is_ok());
        let err = parse_openapi_with(loaded(), &by_path, bounds(39)).unwrap_err();
        assert_eq!(
            err,
            DocumentError::TooLarge {
                limit: 39,
                unit: "imported entries"
            }
        );
        assert!(parse_openapi_with(loaded(), &CollectionOptions::default(), bounds(39)).is_ok());
    }

    /// `ops` operations whose JSON bodies share one schema of 60 properties with 1 KiB examples.
    fn sampled_bodies(ops: usize) -> Value {
        let props: Map<String, Value> = (0..60)
            .map(|i| {
                let prop = json!({"type": "string", "example": "x".repeat(1 << 10)});
                (format!("p{i:02}"), prop)
            })
            .collect();
        let body = json!({"content": {"application/json": {"schema": {"type": "object", "properties": props}}}});
        let mut paths = Map::new();
        for i in 0..ops {
            let op = json!({"post": {"requestBody": {"$ref": "#/components/requestBodies/b"}}});
            paths.insert(format!("/p{i}"), op);
        }
        json!({"openapi": "3.0.0", "servers": [{"url": "https://x.test"}], "paths": paths,
            "components": {"requestBodies": {"b": body}}})
    }

    #[test]
    fn body_sample_fan_out_stops_at_the_byte_budget() {
        // Each body is a fresh 61 KiB sample, inside the 64 KiB bound, plus its pretty text;
        // the walk must fail within one body of the limit, not after building every body.
        assert!(run_bytes(&sampled_bodies(32), 8 << 20).is_ok());
        let root = sampled_bodies(4096);
        let limit = 1 << 20;
        let bounds = Bounds {
            bytes: limit,
            ..Bounds::DEFAULT
        };
        let mut doc = Doc::new(&root, Version::V30, bounds);
        let ops = operations::collect(&mut doc, &mut Vec::new()).expect("4096 operations");
        let mut built = Vec::new();
        let err = ops
            .iter()
            .find_map(|op| match operations::request_for(&mut doc, op) {
                Ok(request) => {
                    built.push(request);
                    None
                }
                Err(e) => Some(e),
            })
            .expect("the byte budget runs out");
        assert_eq!(
            err,
            DocumentError::TooLarge {
                limit,
                unit: "imported bytes"
            }
        );
        assert!(built.len() < 32, "{} bodies built", built.len());
        assert!(
            doc.budget.bytes() <= limit + (128 << 10),
            "charged {} bytes before failing",
            doc.budget.bytes()
        );
    }
}
