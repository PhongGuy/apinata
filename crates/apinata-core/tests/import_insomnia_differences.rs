//! Where the Insomnia importer departs from the reference converter at `1301ea17` (research R2
//! section 3). One test each:
//! - folder requests are not duplicated (`folder_requests_are_not_duplicated`);
//! - other workspaces become their own collections and environments (`other_workspaces_are_kept_apart`);
//! - sort keys are honoured (`sort_keys_are_honoured`);
//! - empty folders and empty-URL requests are kept (`empty_folders_and_urls_are_kept`);
//! - arrays replace rather than merge by index (`arrays_replace`);
//! - only a leading `_.` is stripped, and spaces only around the path (`only_a_leading_prefix_is_stripped`);
//! - filters and tags stay literal with a note (`filters_and_tags_are_noted`);
//! - `{}` and disabled auth inherit, apikey maps (`auth_inherits_and_api_keys_map`);
//! - enabled query params are appended to the URL (`query_params_join_the_url`);
//! - a v5 file without `collection` and a spec document import (`more_documents_import`);
//! - bodies of other MIME types are kept as text (`other_mime_bodies_are_text`);
//! - descriptions, folder headers, folder auth and folder variables are kept (`folder_and_docs_are_kept`).
//!
//! Pinned instead in `tests/import_insomnia.rs`:
//! - `null` environment values become empty with a note (`each_sub_environment_is_the_base_deep_merged_under_it`);
//! - vault keys become valueless secrets (`vault_keys_become_secrets_without_values`);
//! - docs are kept and scripts noted (`descriptions_become_docs_and_scripts_are_dropped`), settings
//!   mapped or noted (`encode_and_redirect_settings_map_and_others_are_noted_off_default`);
//! - multipart file parts and octet-stream file bodies are kept
//!   (`file_parts_other_mime_types_and_graphql_are_noted`, `bodies_follow_their_mime_type`);
//! - a custom bearer prefix becomes an `Authorization` header (`a_custom_bearer_prefix_becomes_a_header`);
//! - export formats other than 4 and `environment.insomnia.rest` documents are refused
//!   (`only_v4_and_v5_collections_are_read`).

use apinata_core::import::{
    CollectionFormat, CollectionOptions, ImportBundle, ImportedItem, NoteKind, parse_collections,
};
use apinata_core::{ApiKeyPlacement, AuthSpec, BodySpec, KeyValue};
use serde_json::{Value, json};

fn parse(input: &str) -> Vec<ImportBundle> {
    parse_collections(
        Some(CollectionFormat::Insomnia),
        input.as_bytes(),
        &CollectionOptions::default(),
    )
    .expect("parses")
}

fn v4(resources: Value) -> String {
    json!({"_type": "export", "__export_format": 4, "resources": resources}).to_string()
}

fn wrk(id: &str) -> Value {
    json!({"_id": id, "_type": "workspace", "parentId": null, "name": id})
}

fn req(id: &str, parent: &str, extra: Value) -> Value {
    let mut r = json!({"_id": id, "_type": "request", "parentId": parent, "name": id, "method": "GET", "url": "https://x.test/"});
    r.as_object_mut()
        .expect("object")
        .extend(extra.as_object().expect("object").clone());
    r
}

fn names(items: &[ImportedItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| match i {
            ImportedItem::Folder(f) => format!("{}/", f.name),
            ImportedItem::Request(r) => r.name.clone(),
            _ => unreachable!("only folders and requests exist"),
        })
        .collect()
}

fn first_request(b: &[ImportBundle]) -> apinata_core::import::ImportedRequestItem {
    match &b[0].collection.items[0] {
        ImportedItem::Request(r) => r.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn folder_requests_are_not_duplicated() {
    let b = parse(&v4(
        json!([wrk("w"), {"_id": "f", "_type": "request_group", "parentId": "w", "name": "F"}, req("r", "f", json!({}))]),
    ));
    let ImportedItem::Folder(f) = &b[0].collection.items[0] else {
        panic!("folder")
    };
    assert_eq!(names(&f.items), ["r"]);
}

#[test]
fn other_workspaces_are_kept_apart() {
    let env = |id: &str, parent: &str, key: &str| {
        let mut data = serde_json::Map::new();
        data.insert(key.to_owned(), Value::from("1"));
        json!({"_id": id, "_type": "environment", "parentId": parent, "name": id, "data": data})
    };
    let b = parse(&v4(json!([
        wrk("w1"),
        wrk("w2"),
        env("e1", "w1", "a"),
        env("e2", "w2", "b"),
        env("e2s", "e2", "c")
    ])));
    assert_eq!(b.len(), 2);
    assert_eq!(b[0].environments.len(), 1);
    assert_eq!(
        b[1].environments
            .iter()
            .map(|e| e.variables.len())
            .collect::<Vec<_>>(),
        [1, 2]
    );
}

#[test]
fn sort_keys_are_honoured() {
    let b = parse(
        "type: collection.insomnia.rest/5.0\nname: c\ncollection:\n  - {name: B, meta: {id: b, sortKey: 5}, method: GET, url: x}\n  - {name: A, meta: {id: a, sortKey: 1}, method: GET, url: x}\n",
    );
    assert_eq!(names(&b[0].collection.items), ["A", "B"]);
}

#[test]
fn empty_folders_and_urls_are_kept() {
    let b = parse(
        "type: collection.insomnia.rest/5.0\nname: c\ncollection:\n  - {name: E, meta: {id: e, sortKey: 1}}\n  - {name: R, meta: {id: r, sortKey: 2}, method: GET}\n",
    );
    assert_eq!(names(&b[0].collection.items), ["E/", "R"]);
}

#[test]
fn arrays_replace() {
    let b = parse(
        "type: collection.insomnia.rest/5.0\nname: c\nenvironments:\n  name: Base\n  data: {arr: [1, 2]}\n  subEnvironments:\n    - {name: S, data: {arr: [9]}}\n",
    );
    let names: Vec<&str> = b[0].environments[1]
        .variables
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    assert_eq!(names, ["arr[0]"]);
}

#[test]
fn only_a_leading_prefix_is_stripped() {
    let r = first_request(&parse(&v4(json!([
        wrk("w"),
        req("r", "w", json!({"url": "{{ my_.x }}/{{ _.a }}"}))
    ]))));
    assert_eq!(r.request.url, "{{my_.x}}/{{a}}");
}

#[test]
fn filters_and_tags_are_noted() {
    let r = first_request(&parse(&v4(json!([
        wrk("w"),
        req("r", "w", json!({"url": "{{ x | upper }}/{% uuid 'v4' %}"}))
    ]))));
    assert_eq!(r.request.url, "{{ x | upper }}/{% uuid 'v4' %}");
    assert!(
        r.notes
            .iter()
            .any(|n| n.kind == NoteKind::NeedsInput && n.source == "r / url")
    );
}

#[test]
fn auth_inherits_and_api_keys_map() {
    let auth = |a: Value| {
        first_request(&parse(&v4(json!([
            wrk("w"),
            req("r", "w", json!({"authentication": a}))
        ]))))
        .request
        .auth
    };
    assert_eq!(auth(json!({})), AuthSpec::Inherit);
    assert_eq!(
        auth(json!({"type": "bearer", "token": "t", "disabled": true})),
        AuthSpec::Inherit
    );
    let key = AuthSpec::ApiKey {
        key: "k".into(),
        value: "v".into(),
        placement: ApiKeyPlacement::Query,
    };
    assert_eq!(
        auth(json!({"type": "apikey", "key": "k", "value": "v", "addTo": "queryParams"})),
        key
    );
}

#[test]
fn query_params_join_the_url() {
    let r = first_request(&parse(&v4(json!([
        wrk("w"),
        req(
            "r",
            "w",
            json!({"parameters": [{"name": "a", "value": "1"}]})
        )
    ]))));
    assert_eq!(r.request.url, "https://x.test/?a=1");
}

#[test]
fn more_documents_import() {
    assert!(
        parse("type: collection.insomnia.rest/5.0\nname: c\n")[0]
            .collection
            .items
            .is_empty()
    );
    assert_eq!(
        parse("type: spec.insomnia.rest/5.0\nname: s\n")[0]
            .collection
            .name,
        "s"
    );
}

#[test]
fn other_mime_bodies_are_text() {
    let body = json!({"body": {"mimeType": "application/edn", "text": "{:a 1}"}});
    let r = first_request(&parse(&v4(json!([wrk("w"), req("r", "w", body)]))));
    assert_eq!(r.request.body, BodySpec::Text("{:a 1}".into()));
}

#[test]
fn folder_and_docs_are_kept() {
    let b = parse(
        "type: collection.insomnia.rest/5.0\nname: c\ncollection:\n  - name: F\n    meta: {id: f}\n    headers: [{name: H, value: '1'}]\n    authentication: {type: bearer, token: t}\n    environment: {v: '2'}\n    children:\n      - {name: R, meta: {id: r, description: Docs}, method: GET, url: x}\n",
    );
    let ImportedItem::Folder(f) = &b[0].collection.items[0] else {
        panic!("folder")
    };
    assert_eq!(
        (f.headers.clone(), f.auth.clone(), f.vars.len()),
        (
            vec![KeyValue::new("H", "1")],
            AuthSpec::Bearer { token: "t".into() },
            1
        )
    );
    let ImportedItem::Request(r) = &f.items[0] else {
        panic!("request")
    };
    assert_eq!(r.request.docs, "Docs");
}
