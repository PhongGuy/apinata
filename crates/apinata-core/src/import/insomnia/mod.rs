//! Importing Insomnia exports, v4 and v5.

mod auth;
mod env;
mod model;
mod request;
mod template;
mod tree;

use super::budget::{ENTRIES, NOTE_BYTES};
use super::load::{Loaded, Syntax, merge_key_notes};
use super::{
    CollectionOptions, DocumentError, ImportBundle, ImportNote, ImportedCollection, NoteKind,
};
use crate::edit::AuthSpec;
use model::{Document, leftovers, name_or, rest_paths, text};
use serde_json::Value;

/// Parse an Insomnia export into its collections and their environments.
pub(crate) fn parse_insomnia(
    loaded: Loaded,
    _options: &CollectionOptions,
) -> Result<Vec<ImportBundle>, DocumentError> {
    parse_with(loaded, NOTE_BYTES, ENTRIES)
}

/// `parse_insomnia` with at most `note_bytes` of copied text and `entries` entries; past either
/// the import is `TooLarge`.
fn parse_with(
    loaded: Loaded,
    note_bytes: u64,
    entries: u64,
) -> Result<Vec<ImportBundle>, DocumentError> {
    let mut budget = Budget::new(note_bytes, entries);
    let mut merge_notes = Vec::new();
    if loaded.syntax == Syntax::Yaml {
        for note in merge_key_notes(&loaded.value) {
            budget.charge(note.source.len() + note.detail.len())?;
            merge_notes.push(note);
        }
    }
    let mut bundles = match Version::of(&loaded.value)? {
        Version::V4 => v4(loaded.value, &mut budget)?,
        Version::V5 => vec![v5(loaded.value, &mut budget)?],
    };
    if let Some(first) = bundles.first_mut() {
        first.collection.unwritten.splice(0..0, merge_notes);
    }
    Ok(bundles)
}

enum Version {
    V4,
    V5,
}

impl Version {
    /// v4 is an `export` envelope of format 4; v5 is a collection or spec document.
    fn of(root: &Value) -> Result<Version, DocumentError> {
        if root.get("_type").and_then(Value::as_str) == Some("export") {
            let format = root.get("__export_format");
            let found = match format {
                None | Some(Value::Null) => "insomnia export format (missing)".to_owned(),
                Some(_) if format.and_then(Value::as_f64) == Some(4.0) => return Ok(Version::V4),
                Some(_) => format!("insomnia export format {}", text(format)),
            };
            return Err(DocumentError::Unsupported { found });
        }
        let Some(kind) = root.get("type").and_then(Value::as_str) else {
            return Err(DocumentError::Invalid {
                message: "not an Insomnia export".into(),
            });
        };
        let readable = ["collection.insomnia.rest/5.", "spec.insomnia.rest/5."];
        if readable.iter().any(|p| kind.starts_with(p)) {
            Ok(Version::V5)
        } else {
            Err(DocumentError::Unsupported {
                found: kind.to_owned(),
            })
        }
    }
}

fn v4(mut root: Value, budget: &mut Budget) -> Result<Vec<ImportBundle>, DocumentError> {
    let resources = match root.get_mut("resources").map(Value::take) {
        Some(Value::Array(list)) => list,
        _ => {
            return Err(DocumentError::Invalid {
                message: "`resources` is not a list".into(),
            });
        }
    };
    let mut unread = Vec::new();
    if let Some(root) = root.as_object() {
        rest_paths(root, "collection / ", &["resources"], &mut unread);
    }
    let (workspaces, stray) = tree::v4(resources, budget)?;
    let mut bundles = Vec::with_capacity(workspaces.len());
    for workspace in workspaces {
        let mut unwritten = workspace.notes;
        let items = tree::items(workspace.nodes, &mut Vec::new(), &mut unwritten, budget)?;
        let mut environments = Vec::new();
        for (base, subs) in workspace.environments {
            environments.extend(env::environments(base, subs, budget)?);
        }
        let mut b = bundle(workspace.name, workspace.docs, items, unwritten);
        b.environments = environments;
        bundles.push(b);
    }
    if let Some(first) = bundles.first_mut() {
        budget.unread(&[], unread, &mut first.collection.unwritten)?;
        first.collection.unwritten.extend(stray);
    }
    Ok(bundles)
}

fn v5(root: Value, budget: &mut Budget) -> Result<ImportBundle, DocumentError> {
    let doc: Document = serde_json::from_value(root).map_err(|_| DocumentError::Invalid {
        message: "not an Insomnia export".into(),
    })?;
    let mut unwritten = Vec::new();
    let dropped = [
        (doc.spec.is_some(), "spec", "the API spec is not imported"),
        (
            doc.test_suites.is_some(),
            "testSuites",
            "test suites are not imported",
        ),
        (
            holds_cookies(doc.cookie_jar.as_ref()),
            "cookieJar",
            "cookies are not imported",
        ),
    ];
    for (present, key, detail) in dropped {
        if present {
            let source = format!("collection / {key}");
            unwritten.push(budget.note(&[], &source, NoteKind::Dropped, detail)?);
        }
    }
    let mut unread = Vec::new();
    rest_paths(&doc.rest, "collection / ", &[], &mut unread);
    leftovers(
        doc.meta.as_ref(),
        "collection / meta",
        &["id", "sortKey", "description"],
        &mut unread,
    );
    budget.unread(&[], unread, &mut unwritten)?;
    let list = match doc.collection {
        None => Vec::new(),
        Some(Value::Array(list)) => list,
        Some(_) => {
            return Err(DocumentError::Invalid {
                message: "`collection` is not a list".into(),
            });
        }
    };
    let nodes = tree::v5(list, &mut unwritten, budget)?;
    let items = tree::items(nodes, &mut Vec::new(), &mut unwritten, budget)?;
    let environments = env::v5(doc.environments, &mut unwritten, budget)?;
    let name = name_or(doc.name.as_ref(), "Insomnia collection");
    let docs = text(doc.meta.as_ref().and_then(|m| m.get("description")));
    let mut b = bundle(name, docs, items, unwritten);
    b.environments = environments;
    Ok(b)
}

fn holds_cookies(jar: Option<&Value>) -> bool {
    jar.and_then(|j| j.get("cookies"))
        .and_then(Value::as_array)
        .is_some_and(|c| !c.is_empty())
}

fn bundle(
    name: String,
    docs: String,
    items: Vec<super::ImportedItem>,
    unwritten: Vec<ImportNote>,
) -> ImportBundle {
    ImportBundle {
        collection: ImportedCollection {
            scripts: Default::default(),
            name,
            docs,
            auth: AuthSpec::None,
            headers: Vec::new(),
            vars: Vec::new(),
            items,
            notes: Vec::new(),
            unwritten,
            spec_source: false,
        },
        environments: Vec::new(),
    }
}

pub(super) use crate::import::budget::Budget;

/// Notes whose source is an ancestor trail plus a name.
pub(super) trait Crumbs {
    /// One `Dropped` note per unread member, sourced `trail / path`.
    fn unread(
        &mut self,
        trail: &[String],
        unread: Vec<model::Unread>,
        notes: &mut Vec<ImportNote>,
    ) -> Result<(), DocumentError>;

    /// A note sourced `trail / name`, charged before its source is built.
    fn note(
        &mut self,
        trail: &[String],
        name: &str,
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> Result<ImportNote, DocumentError>;
}

impl Crumbs for Budget {
    fn unread(
        &mut self,
        trail: &[String],
        unread: Vec<model::Unread>,
        notes: &mut Vec<ImportNote>,
    ) -> Result<(), DocumentError> {
        for (path, detail) in unread {
            notes.push(Crumbs::note(self, trail, &path, NoteKind::Dropped, detail)?);
        }
        Ok(())
    }

    fn note(
        &mut self,
        trail: &[String],
        name: &str,
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> Result<ImportNote, DocumentError> {
        let mut pieces: Vec<&str> = Vec::with_capacity(trail.len() * 2 + 1);
        for t in trail {
            pieces.push(t);
            pieces.push(" / ");
        }
        pieces.push(name);
        self.note_from(&pieces, kind, detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::ImportedItem;
    use crate::import::load::load;
    use serde_json::json;

    fn parse(root: &Value, note_bytes: u64) -> Result<Vec<ImportBundle>, DocumentError> {
        parse_with(load(root.to_string().as_bytes())?, note_bytes, ENTRIES)
    }

    fn parse_entries(root: &Value, entries: u64) -> Result<Vec<ImportBundle>, DocumentError> {
        parse_with(load(root.to_string().as_bytes())?, NOTE_BYTES, entries)
    }

    fn too_many(limit: u64) -> DocumentError {
        DocumentError::TooLarge {
            limit,
            unit: "imported entries",
        }
    }

    #[test]
    fn every_sub_repeating_a_base_of_many_short_values_is_counted() {
        // Short values cost little text but a whole variable each, repeated by every sub.
        let mut resources = vec![
            json!({"_id": "wrk_1", "_type": "workspace"}),
            json!({"_id": "env_b", "_type": "environment", "parentId": "wrk_1",
                "data": {"a": (0..100).collect::<Vec<u32>>()}}),
        ];
        for i in 0..12 {
            resources.push(json!({"_id": format!("env_{i}"), "_type": "environment",
                "parentId": "env_b", "data": {}}));
        }
        let root = export(Value::Array(resources));
        assert_eq!(
            parse_entries(&root, 1000).map(|_| ()).unwrap_err(),
            too_many(1000)
        );
        let bundles = parse_entries(&root, 2000).expect("parses");
        assert_eq!(bundles[0].environments.len(), 13);
    }

    /// A workspace whose base environment holds `data`, under two empty subs.
    fn base_under_two_subs(data: Value) -> Value {
        export(json!([
            {"_id": "wrk_1", "_type": "workspace"},
            {"_id": "env_b", "_type": "environment", "parentId": "wrk_1", "data": data},
            {"_id": "env_1", "_type": "environment", "parentId": "env_b", "data": {}},
            {"_id": "env_2", "_type": "environment", "parentId": "env_b", "data": {}},
        ]))
    }

    /// The import of `root` fails one entry below `exact` and succeeds at it.
    fn counts_exactly(root: &Value, exact: u64) {
        let under = exact - 1;
        assert_eq!(
            parse_entries(root, under).map(|_| ()).unwrap_err(),
            too_many(under)
        );
        assert!(parse_entries(root, exact).is_ok());
    }

    #[test]
    fn every_key_of_an_object_base_is_counted_per_environment() {
        let data: serde_json::Map<String, Value> =
            (0..100).map(|i| (format!("k{i}"), json!(1))).collect();
        counts_exactly(&base_under_two_subs(Value::Object(data)), 300);
    }

    #[test]
    fn every_vault_key_is_counted_per_environment() {
        let vault: serde_json::Map<String, Value> =
            (0..100).map(|i| (format!("k{i}"), json!("x"))).collect();
        counts_exactly(
            &base_under_two_subs(json!({"__insomnia_vault": vault})),
            300,
        );
    }

    #[test]
    fn every_note_is_counted() {
        let mut resources = vec![json!({"_id": "wrk_1", "_type": "workspace"})];
        for i in 0..100 {
            resources.push(json!({"_id": format!("g{i}"), "_type": "grpc_request",
                "parentId": "wrk_1"}));
        }
        counts_exactly(&export(Value::Array(resources)), 100);
    }

    #[test]
    fn requests_folders_and_rows_are_counted() {
        let rows: Vec<Value> = (0..50)
            .map(|i| json!({"name": format!("h{i}"), "value": "v"}))
            .collect();
        let mut resources = vec![
            json!({"_id": "wrk_1", "_type": "workspace"}),
            json!({"_id": "fld", "_type": "request_group", "parentId": "wrk_1"}),
        ];
        for i in 0..10 {
            resources.push(
                json!({"_id": format!("r{i}"), "_type": "request", "parentId": "fld",
                "url": "u", "headers": rows, "parameters": rows}),
            );
        }
        let root = export(Value::Array(resources));
        // 1 folder, 10 requests, 1000 rows.
        assert_eq!(
            parse_entries(&root, 1010).map(|_| ()).unwrap_err(),
            too_many(1010)
        );
        assert!(parse_entries(&root, 1011).is_ok());
    }

    fn requests(items: &[ImportedItem]) -> usize {
        let count = |item: &ImportedItem| match item {
            ImportedItem::Folder(f) => requests(&f.items),
            ImportedItem::Request(_) => 1,
        };
        items.iter().map(count).sum()
    }

    fn export(resources: Value) -> Value {
        json!({"_type": "export", "__export_format": 4, "resources": resources})
    }

    #[test]
    fn duplicate_ids_and_shared_parents_attach_each_resource_once() {
        let root = export(json!([
            {"_id": "wrk_1", "_type": "workspace", "name": "c"},
            {"_id": "wrk_1", "_type": "workspace", "name": "twin"},
            {"_id": "wrk_1", "_type": "request_group", "parentId": "wrk_1", "name": "as ws"},
            {"_id": "fld", "_type": "request_group", "parentId": "wrk_1", "name": "a"},
            {"_id": "fld", "_type": "request_group", "parentId": "wrk_1", "name": "b"},
            {"_id": "r1", "_type": "request", "parentId": "fld", "url": "u"},
            {"_id": "r2", "_type": "request", "parentId": "wrk_1", "url": "u"},
        ]));
        let bundles = parse(&root, NOTE_BYTES).expect("parses");
        let total: usize = bundles.iter().map(|b| requests(&b.collection.items)).sum();
        assert_eq!(total, 2);
    }

    #[test]
    fn note_text_past_the_budget_is_too_large() {
        // Long folder names repeated in the breadcrumb of every unwritten item below them.
        let mut resources = vec![json!({"_id": "wrk_1", "_type": "workspace"})];
        let mut parent = "wrk_1".to_owned();
        for depth in 0..100 {
            let id = format!("fld_{depth}");
            resources.push(
                json!({"_id": id, "_type": "request_group", "parentId": parent,
                "name": "n".repeat(1000)}),
            );
            parent = id;
        }
        for i in 0..100 {
            resources.push(json!({"_id": format!("g{i}"), "_type": "grpc_request",
                "parentId": parent}));
        }
        let root = export(Value::Array(resources));
        let small = parse(&root, 1_000_000).map(|_| ()).unwrap_err();
        let expected = DocumentError::TooLarge {
            limit: 1_000_000,
            unit: "imported bytes",
        };
        assert_eq!(small, expected);
        let bundles = parse(&root, NOTE_BYTES).expect("parses");
        assert_eq!(bundles[0].collection.unwritten.len(), 100);
    }

    #[test]
    fn every_sub_repeating_a_large_base_is_charged() {
        // Each sub environment repeats the whole base, so N subs of a 1 MB base yield N MB.
        let data: serde_json::Map<String, Value> = (0..1000)
            .map(|i| (format!("k{i}"), Value::String("v".repeat(1000))))
            .collect();
        let mut resources = vec![
            json!({"_id": "wrk_1", "_type": "workspace"}),
            json!({"_id": "env_b", "_type": "environment", "parentId": "wrk_1", "data": data}),
        ];
        for i in 0..20 {
            resources.push(json!({"_id": format!("env_{i}"), "_type": "environment",
                "parentId": "env_b", "data": {"k0": "own"}}));
        }
        let root = export(Value::Array(resources));
        let expected = DocumentError::TooLarge {
            limit: 5_000_000,
            unit: "imported bytes",
        };
        assert_eq!(parse(&root, 5_000_000).map(|_| ()).unwrap_err(), expected);
        let bundles = parse(&root, NOTE_BYTES).expect("parses");
        let envs = &bundles[0].environments;
        assert_eq!(envs.len(), 21);
        assert!(envs.iter().all(|e| e.variables.len() == 1000));
    }

    #[test]
    fn flattened_names_of_deep_long_keys_are_charged() {
        // Every level's name repeats all the keys above it: 100 levels of 10 kB keys give
        // about 50 MB of names from a 1 MB input.
        let key = "k".repeat(10_000);
        let mut data = json!("leaf");
        for _ in 0..100 {
            data = json!({ key.clone(): data });
        }
        let root = export(json!([
            {"_id": "wrk_1", "_type": "workspace"},
            {"_id": "env_b", "_type": "environment", "parentId": "wrk_1", "data": data},
        ]));
        let expected = DocumentError::TooLarge {
            limit: 10_000_000,
            unit: "imported bytes",
        };
        assert_eq!(parse(&root, 10_000_000).map(|_| ()).unwrap_err(), expected);
        let bundles = parse(&root, NOTE_BYTES).expect("parses");
        assert_eq!(bundles[0].environments[0].variables.len(), 1);
    }

    /// Every `Dropped` note source of the first bundle, collection, items and environments.
    fn dropped(bundle: &ImportBundle) -> Vec<String> {
        fn walk(items: &[ImportedItem], out: &mut Vec<String>) {
            for item in items {
                let notes = match item {
                    ImportedItem::Folder(f) => {
                        walk(&f.items, out);
                        &f.notes
                    }
                    ImportedItem::Request(r) => &r.notes,
                };
                out.extend(
                    notes
                        .iter()
                        .filter(|n| n.kind == NoteKind::Dropped)
                        .map(|n| n.source.clone()),
                );
            }
        }
        let mut out: Vec<String> = bundle
            .collection
            .unwritten
            .iter()
            .map(|n| n.source.clone())
            .collect();
        walk(&bundle.collection.items, &mut out);
        for env in &bundle.environments {
            out.extend(env.notes.iter().map(|n| n.source.clone()));
        }
        out.sort();
        out
    }

    #[test]
    fn members_lost_by_shape_are_noted() {
        let v5 = json!({"type": "collection.insomnia.rest/5.0", "collection": [
            {"name": "R", "method": "GET", "url": "u", "children": [{"name": "lost", "method": "GET"}],
                "body": "raw", "environment": {"v": "1"}, "settings": {"timeout": 5, "cookies": {"jar": 1}}},
            {"name": "F", "children": {"name": "not a list"}},
            {"name": "G", "children": [{"name": "S", "method": "GET", "settings": true,
                "meta": {"tags": ["a"]}, "headers": [{"name": "h", "value": "v", "id": "p1", "multiline": true}]}]},
        ], "environments": {"name": "Base", "data": {"": "nameless", "ok": "1"}, "zz": 1,
            "subEnvironments": [{"name": "Sub", "meta": {"extra": 1}}]}});
        let bundles = parse(&v5, NOTE_BYTES).expect("parses");
        let expected = [
            "F / children",
            "G / S / headers[0].multiline",
            "G / S / meta.tags",
            "G / S / settings",
            "R / body",
            "R / children",
            "R / environment",
            "R / settings.cookies.jar",
            "R / settings.timeout",
            // The sub inherits the base's nameless key, so each environment notes it.
            "data.",
            "data.",
            "meta.extra",
            "zz",
        ];
        assert_eq!(dropped(&bundles[0]), expected);
        let base = &bundles[0].environments[0];
        assert!(base.variables.iter().all(|v| !v.name.is_empty()));
    }

    #[test]
    fn unknown_v4_envelope_and_resource_keys_are_noted() {
        let mut root = export(json!([
            {"_id": "wrk_1", "_type": "workspace", "certificates": []},
            {"_id": "fld", "_type": "request_group", "parentId": "wrk_1", "name": "F", "x": 1},
            {"_id": "r1", "_type": "request", "parentId": "fld", "name": "R", "url": "u",
                "body": {"mimeType": "multipart/form-data", "params": [
                    {"name": "f", "type": "file", "fileName": "", "src": 1}]}},
            {"_id": "env", "_type": "environment", "parentId": "wrk_1", "data": {}, "y": 1},
        ]));
        root["extra"] = json!(true);
        let bundles = parse(&root, NOTE_BYTES).expect("parses");
        let expected = [
            "F / R / body.params[0].src",
            "F / x",
            "collection / certificates",
            "collection / extra",
            "y",
        ];
        assert_eq!(dropped(&bundles[0]), expected);
    }

    #[test]
    fn body_members_the_mime_type_does_not_use_are_noted() {
        let request = |name: &str, body: Value| {
            json!({"_id": name, "_type": "request", "parentId": "wrk_1", "name": name, "url": "u",
                "body": body})
        };
        let root = export(json!([
            {"_id": "wrk_1", "_type": "workspace", "scope": "collection"},
            {"_id": "fld", "_type": "request_group", "parentId": "wrk_1", "name": "F", "scope": "x"},
            request("file", json!({"mimeType": "application/octet-stream", "fileName": "/a"})),
            request("json", json!({"mimeType": "application/json", "text": "{}", "fileName": "/a",
                "params": [{"name": "p"}]})),
            request("form", json!({"mimeType": "application/x-www-form-urlencoded", "text": "t",
                "params": []})),
            request("empty", json!({"mimeType": "application/json", "text": "", "params": [],
                "fileName": ""})),
        ]));
        let bundles = parse(&root, NOTE_BYTES).expect("parses");
        let expected = [
            "F / scope",
            "form / body.text",
            "json / body.fileName",
            "json / body.params",
        ];
        assert_eq!(dropped(&bundles[0]), expected);
    }

    #[test]
    fn folder_variables_stay_on_their_folder() {
        let root = export(json!([
            {"_id": "wrk_1", "_type": "workspace"},
            {"_id": "fld_a", "_type": "request_group", "parentId": "wrk_1",
                "environment": {"v": "1"}},
            {"_id": "fld_b", "_type": "request_group", "parentId": "fld_a"},
        ]));
        let bundles = parse(&root, NOTE_BYTES).expect("parses");
        let ImportedItem::Folder(a) = &bundles[0].collection.items[0] else {
            panic!("a folder");
        };
        let ImportedItem::Folder(b) = &a.items[0] else {
            panic!("a folder");
        };
        assert_eq!(a.vars.len(), 1);
        assert!(b.vars.is_empty());
    }
}
