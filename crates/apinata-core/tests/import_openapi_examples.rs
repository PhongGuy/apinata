//! Response examples in an OpenAPI document become saved examples on the imported request.

use crate::openapi_support;

use crate::import_update_support::FakeBin;
use apinata_core::import::{
    ChangedField, ImportBundle, NoteKind, UpdateOptions, add_request, apply_update, plan_update,
    write_collection,
};
use apinata_core::{
    Collection, Example, ExampleBodyKind, ExampleResponse, ItemPath, MemoryStore, Trash,
};
use openapi_support::{import, request};
use serde_json::{Value, json};
use tempfile::TempDir;

fn doc(responses: Value) -> Value {
    json!({"openapi": "3.0.3", "servers": [{"url": "https://x.test"}],
        "components": {
            "examples": {"Rex": {"summary": "A dog", "value": {"name": "rex"}}},
            "headers": {"Rate": {"schema": {"type": "integer"}, "example": 10}},
            "responses": {"Missing": {"description": "Not found", "content": {"application/json": {"example": {"error": "gone"}}}}},
            "schemas": {"Pet": {"type": "object", "properties": {"id": {"type": "integer", "example": 4}}}}
        },
        "paths": {"/pets": {"get": {"summary": "R", "responses": responses}}}})
}

fn examples_of(responses: Value) -> (Vec<Example>, Vec<(NoteKind, String, String)>) {
    let c = import(&doc(responses)).collection;
    let r = request(&c, "R");
    let notes = r
        .notes
        .iter()
        .map(|n| (n.kind, n.source.clone(), n.detail.clone()))
        .collect();
    (r.request.examples.clone(), notes)
}

/// (name, status, status text, headers, body kind, body data) of each example.
type Summary = (
    String,
    Option<u16>,
    Option<String>,
    Vec<(String, String)>,
    Option<(ExampleBodyKind, String)>,
);

fn summary(examples: &[Example]) -> Vec<Summary> {
    examples
        .iter()
        .map(|e| {
            let r = e.response.as_ref().expect("a response half");
            (
                e.name.clone(),
                r.status,
                r.status_text.clone(),
                r.headers.clone(),
                r.body.as_ref().map(|b| (b.kind, b.data.clone())),
            )
        })
        .collect()
}

fn ct(value: &str) -> (String, String) {
    ("Content-Type".to_owned(), value.to_owned())
}

#[test]
fn a_media_type_example_becomes_a_status_named_example() {
    let (examples, _) = examples_of(json!({"200": {"description": "The pets",
        "content": {"application/json": {"example": [{"id": 1}]}}}}));
    assert_eq!(
        summary(&examples),
        [(
            "200 Response".to_owned(),
            Some(200),
            Some("OK".to_owned()),
            vec![ct("application/json")],
            Some((
                ExampleBodyKind::Json,
                "[\n  {\n    \"id\": 1\n  }\n]".to_owned()
            )),
        )]
    );
    assert_eq!(examples[0].description.as_deref(), Some("The pets"));
}

#[test]
fn named_examples_keep_their_key_or_summary_and_description() {
    let (examples, _) = examples_of(json!({"201": {"description": "made",
    "content": {"text/plain": {"examples": {
        "short": {"value": "hi", "description": "A short one"},
        "long": {"summary": "Long\nreply", "value": 42}
    }}}}}));
    assert_eq!(
        summary(&examples),
        [
            (
                "short".to_owned(),
                Some(201),
                Some("Created".to_owned()),
                vec![ct("text/plain")],
                Some((ExampleBodyKind::Text, "hi".to_owned())),
            ),
            (
                "Long reply".to_owned(),
                Some(201),
                Some("Created".to_owned()),
                vec![ct("text/plain")],
                Some((ExampleBodyKind::Text, "42".to_owned())),
            ),
        ]
    );
    assert_eq!(examples[0].description.as_deref(), Some("A short one"));
    assert_eq!(examples[1].description, None);
}

#[test]
fn referenced_examples_responses_and_headers_are_followed() {
    let (examples, notes) = examples_of(json!({
        "200": {"description": "ok",
            "headers": {"X-Rate": {"$ref": "#/components/headers/Rate"}, "Content-Type": {"example": "x"}},
            "content": {"application/json": {"examples": {"rex": {"$ref": "#/components/examples/Rex"}}}}},
        "404": {"$ref": "#/components/responses/Missing"}
    }));
    assert!(notes.is_empty(), "{notes:?}");
    let rate = ("X-Rate".to_owned(), "10".to_owned());
    assert_eq!(
        summary(&examples),
        [
            (
                "A dog".to_owned(),
                Some(200),
                Some("OK".to_owned()),
                vec![ct("application/json"), rate],
                Some((
                    ExampleBodyKind::Json,
                    "{\n  \"name\": \"rex\"\n}".to_owned()
                )),
            ),
            (
                "404 Response".to_owned(),
                Some(404),
                Some("Not Found".to_owned()),
                vec![ct("application/json")],
                Some((
                    ExampleBodyKind::Json,
                    "{\n  \"error\": \"gone\"\n}".to_owned()
                )),
            ),
        ]
    );
}

#[test]
fn a_dangling_or_cyclic_example_ref_is_skipped_with_a_note() {
    let (examples, notes) = examples_of(json!({"200": {"description": "ok",
    "content": {"application/json": {"examples": {
        "gone": {"$ref": "#/components/examples/Nope"},
        "loop": {"$ref": "#/components/examples/Loop"},
        "far": {"externalValue": "https://x.test/far.json"},
        "kept": {"value": true}
    }}}}}));
    let names: Vec<&str> = examples.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["kept"]);
    let at = |key: &str| {
        format!("paths./pets.get.responses.200.content.application/json.examples.{key}")
    };
    assert!(
        notes.iter().any(|(_, source, _)| *source == at("gone")),
        "{notes:?}"
    );
    assert!(notes.contains(&(
        NoteKind::Dropped,
        at("far"),
        "the external example `https://x.test/far.json` was not fetched".to_owned()
    )));
}

#[test]
fn a_cyclic_response_ref_is_skipped_without_a_panic() {
    let mut d = doc(json!({"200": {"$ref": "#/components/responses/A"}}));
    d["components"]["responses"]["A"] = json!({"$ref": "#/components/responses/B"});
    d["components"]["responses"]["B"] = json!({"$ref": "#/components/responses/A"});
    let c = import(&d).collection;
    let r = request(&c, "R");
    assert!(r.request.examples.is_empty());
    assert!(!r.notes.is_empty(), "the cut ref is noted");
}

#[test]
fn a_schema_only_json_response_is_sampled_and_a_bare_one_adds_nothing() {
    let (examples, _) = examples_of(json!({
        "200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pet"}}}},
        "204": {"description": "No content"},
        "500": {"description": "oops", "content": {"image/png": {"schema": {"type": "string"}}}},
        "default": {"description": "Default response"}
    }));
    assert_eq!(
        summary(&examples),
        [(
            "200 Response".to_owned(),
            Some(200),
            Some("OK".to_owned()),
            vec![ct("application/json")],
            Some((ExampleBodyKind::Json, "{\n  \"id\": 4\n}".to_owned())),
        )]
    );
}

#[test]
fn a_response_with_only_header_examples_is_an_example_without_a_body() {
    let (examples, _) = examples_of(json!({"204": {"description": "Gone",
        "headers": {"ETag": {"schema": {"type": "string"}, "examples": {"a": {"value": "abc"}}}}}}));
    assert_eq!(
        summary(&examples),
        [(
            "204 Response".to_owned(),
            Some(204),
            Some("No Content".to_owned()),
            vec![("ETag".to_owned(), "abc".to_owned())],
            None,
        )]
    );
}

#[test]
fn swagger2_examples_are_keyed_by_mime_type_and_default_is_skipped() {
    let d = json!({"swagger": "2.0", "host": "x.test", "produces": ["application/xml"],
    "paths": {"/pets": {"get": {"summary": "R", "responses": {
        "200": {"description": "ok", "examples": {"application/json": {"id": 1}}},
        "400": {"description": "bad", "schema": {"type": "object", "properties": {"code": {"type": "integer"}}}},
        "default": {"description": "d", "examples": {"text/plain": "x"}}
    }}}}});
    let c = import(&d).collection;
    let examples = &request(&c, "R").request.examples;
    let s = summary(examples);
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].3, [ct("application/json")]);
    assert_eq!(
        s[0].4,
        Some((ExampleBodyKind::Json, "{\n  \"id\": 1\n}".to_owned()))
    );
    assert_eq!((s[1].0.as_str(), s[1].1), ("400 Response", Some(400)));
    assert_eq!(s[1].3, [ct("application/xml")]);
    assert_eq!(s[1].4.as_ref().map(|b| b.0), Some(ExampleBodyKind::Xml));
}

fn bundle(example: &str) -> ImportBundle {
    let responses = json!({"200": {"description": "ok",
        "content": {"application/json": {"examples": {"one": {"value": {"v": example}}}}}}});
    import(&doc(responses))
}

#[test]
fn imported_examples_are_written_as_saving_a_response_writes_them() {
    let dir = TempDir::new().unwrap();
    let b = bundle("a");
    let (c, _) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .unwrap();
    let written = std::fs::read_to_string(c.root().join("R.yml")).unwrap();

    // The same request saved by hand, then given the same response as an example.
    let mut by_hand = Collection::create(&dir.path().join("hand"), "hand").unwrap();
    let mut plain = request(&b.collection, "R").request.clone();
    let example = plain.examples.remove(0);
    let item = add_request(&mut by_hand, &ItemPath::root(), "R", &plain).unwrap();
    let response = example.response.unwrap();
    by_hand.add_example(&item, "one", "", &response).unwrap();
    let saved = std::fs::read_to_string(by_hand.root().join("R.yml")).unwrap();
    assert_eq!(written, saved);
    assert!(written.contains("examples:\n  - name: one\n"), "{written}");
}

#[test]
fn an_update_refreshes_generated_examples_and_keeps_edited_ones() {
    let dir = TempDir::new().unwrap();
    let b = bundle("a");
    let (mut c, _) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .unwrap();
    let r = ItemPath::new("R.yml").unwrap();
    let s = apply_update(&mut c, &bundle("b"), &UpdateOptions::default(), None).unwrap();
    assert_eq!(s.changed[0].fields, [ChangedField::Examples]);
    let body = |c: &Collection| {
        c.request(&r).unwrap().examples()[0]
            .response
            .as_ref()
            .and_then(|r| r.body.as_ref())
            .map(|b| b.data.clone())
    };
    assert_eq!(body(&c).as_deref(), Some("{\n  \"v\": \"b\"\n}"));

    // An example saved by hand makes the field edited; the spec's next change leaves it.
    let response = c.request(&r).unwrap().examples()[0]
        .response
        .clone()
        .unwrap();
    c.add_example(&r, "mine", "", &response).unwrap();
    let s = plan_update(&c, &bundle("c"), &UpdateOptions::default()).unwrap();
    assert!(s.changed.is_empty());
    assert_eq!(s.edited_fields[0].fields, [ChangedField::Examples]);
}

#[test]
fn a_baseline_from_before_examples_were_imported_takes_them_on_update() {
    let dir = TempDir::new().unwrap();
    let b = import(&doc(json!({})));
    let (c, _) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .unwrap();
    // An older import recorded no examples hash.
    let path = c.root().join("opencollection.yml");
    let text = std::fs::read_to_string(&path).unwrap();
    let older: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("examples:"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_ne!(text, older);
    std::fs::write(&path, older).unwrap();
    let mut c = Collection::open(c.root()).unwrap();
    let s = apply_update(&mut c, &bundle("a"), &UpdateOptions::default(), None).unwrap();
    assert_eq!(s.changed[0].fields, [ChangedField::Examples]);
    let r = ItemPath::new("R.yml").unwrap();
    assert_eq!(c.request(&r).unwrap().examples()[0].name, "one");
}

/// Write `b` under `dir` and drop the examples hashes from its baseline, as an import made
/// before examples were spec-owned recorded it.
fn older_import(dir: &std::path::Path, b: &ImportBundle) -> Collection {
    let (c, _) =
        write_collection(dir, &b.collection, &b.environments, &MemoryStore::new()).unwrap();
    let path = c.root().join("opencollection.yml");
    let text = std::fs::read_to_string(&path).unwrap();
    let older: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("examples:"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_ne!(text, older);
    std::fs::write(&path, older).unwrap();
    Collection::open(c.root()).unwrap()
}

#[test]
fn an_older_baseline_keeps_a_hand_saved_example_against_spec_examples() {
    let dir = TempDir::new().unwrap();
    let mut c = older_import(dir.path(), &import(&doc(json!({}))));
    let r = ItemPath::new("R.yml").unwrap();
    c.add_example(&r, "mine", "", &ExampleResponse::new(418))
        .unwrap();
    let file = c.root().join("R.yml");
    let before = std::fs::read(&file).unwrap();
    let s = apply_update(&mut c, &bundle("a"), &UpdateOptions::default(), None).unwrap();
    assert!(s.changed.is_empty());
    assert_eq!(s.edited_fields[0].fields, [ChangedField::Examples]);
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

fn without_r() -> ImportBundle {
    import(
        &json!({"openapi": "3.0.3", "servers": [{"url": "https://x.test"}],
        "paths": {"/other": {"get": {"summary": "O"}}}}),
    )
}

fn prune() -> UpdateOptions {
    let mut o = UpdateOptions::default();
    o.prune = true;
    o
}

#[test]
fn an_orphan_holding_only_its_generated_examples_is_pruned() {
    let dir = TempDir::new().unwrap();
    let b = bundle("a");
    let (mut c, _) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .unwrap();
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &without_r(), &prune(), Some(&bin as &dyn Trash)).unwrap();
    assert_eq!(s.pruned.len(), 1, "{:?}", s.notes);
    assert!(!c.root().join("R.yml").exists());
}

#[test]
fn an_orphan_with_a_hand_saved_example_is_not_pruned() {
    let dir = TempDir::new().unwrap();
    let b = bundle("a");
    let (mut c, _) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .unwrap();
    let r = ItemPath::new("R.yml").unwrap();
    c.add_example(&r, "mine", "", &ExampleResponse::new(418))
        .unwrap();
    let file = c.root().join("R.yml");
    let before = std::fs::read(&file).unwrap();
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &without_r(), &prune(), Some(&bin as &dyn Trash)).unwrap();
    assert!(s.pruned.is_empty());
    assert!(
        s.notes.iter().any(|n| n.detail.starts_with("not pruned")),
        "{:?}",
        s.notes
    );
    assert_eq!(std::fs::read(&file).unwrap(), before);
}
