//! Insomnia v4 and v5 exports through the public parse.

use apinata_core::import::{
    CollectionFormat, CollectionOptions, DocumentError, FormatError, ImportBundle, ImportNote,
    ImportedCollection, ImportedEnvironment, ImportedFolder, ImportedItem, ImportedRequestItem,
    NoteKind, parse_collections,
};
use apinata_core::{ApiKeyPlacement, AuthSpec, RequestKind};
use apinata_core::{
    BodySpec, FileEntry, KeyValue, MultipartField, Param, ParamKind, RequestSettings, Timeout,
    VarValue, Variable,
};
use serde_json::{Value, json};

fn parse(input: &str) -> Result<Vec<ImportBundle>, FormatError> {
    parse_collections(
        Some(CollectionFormat::Insomnia),
        input.as_bytes(),
        &CollectionOptions::default(),
    )
}

fn bundles(input: &str) -> Vec<ImportBundle> {
    parse(input).expect("parses")
}

fn one(input: &str) -> ImportedCollection {
    let mut all = bundles(input);
    assert_eq!(all.len(), 1, "one collection");
    all.remove(0).collection
}

fn refused(input: &str) -> DocumentError {
    match parse(input) {
        Err(FormatError::Insomnia(e)) => e,
        other => panic!("{other:?}"),
    }
}

/// A v4 JSON export of `resources`.
fn v4(resources: Value) -> String {
    json!({"_type": "export", "__export_format": 4, "resources": resources}).to_string()
}

/// A v5 collection named `c`, with `body` (YAML starting at column 0) appended.
fn v5(body: &str) -> String {
    format!(
        "type: collection.insomnia.rest/5.0\nschema_version: \"5.1\"\nname: c\nmeta:\n  id: wrk_1\n{body}"
    )
}

fn wrk(id: &str, name: &str) -> Value {
    json!({"_id": id, "_type": "workspace", "parentId": null, "name": name})
}

fn group(id: &str, parent: &str, name: &str, sort: f64) -> Value {
    json!({"_id": id, "_type": "request_group", "parentId": parent, "name": name, "metaSortKey": sort})
}

fn req(id: &str, parent: &str, name: &str, sort: f64) -> Value {
    json!({"_id": id, "_type": "request", "parentId": parent, "name": name, "method": "GET",
        "url": "https://x.test/", "metaSortKey": sort})
}

/// `"<seq> <name>"` per item, with a trailing `/` on folders.
fn listing(items: &[ImportedItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            ImportedItem::Folder(f) => format!("{} {}/", f.seq, f.name),
            ImportedItem::Request(r) => format!("{} {}", r.seq, r.name),
            _ => unreachable!("only folders and requests exist"),
        })
        .collect()
}

fn folder<'a>(items: &'a [ImportedItem], name: &str) -> &'a ImportedFolder {
    let found = items.iter().find_map(|i| match i {
        ImportedItem::Folder(f) if f.name == name => Some(f),
        _ => None,
    });
    found.expect(name)
}

fn request<'a>(items: &'a [ImportedItem], name: &str) -> &'a ImportedRequestItem {
    let found = items.iter().find_map(|i| match i {
        ImportedItem::Request(r) if r.name == name => Some(r),
        _ => None,
    });
    found.expect(name)
}

/// Every note of `c`: its own, the unwritten ones, then each item's, depth first.
fn notes(c: &ImportedCollection) -> Vec<&ImportNote> {
    fn walk<'a>(items: &'a [ImportedItem], out: &mut Vec<&'a ImportNote>) {
        for item in items {
            match item {
                ImportedItem::Folder(f) => {
                    out.extend(&f.notes);
                    walk(&f.items, out);
                }
                ImportedItem::Request(r) => out.extend(&r.notes),
                _ => unreachable!("only folders and requests exist"),
            }
        }
    }
    let mut out: Vec<&ImportNote> = c.notes.iter().chain(&c.unwritten).collect();
    walk(&c.items, &mut out);
    out
}

fn has(c: &ImportedCollection, kind: NoteKind, source: &str) -> bool {
    notes(c)
        .iter()
        .any(|n| n.kind == kind && n.source == source)
}

#[test]
fn v4_resources_become_one_tree_sorted_by_sort_key() {
    let c = one(&v4(json!([
        req("req_a", "wrk_1", "A", 2.0),
        wrk("wrk_1", "Shop"),
        group("fld_f", "wrk_1", "F", -5.0),
        req("req_c", "fld_f", "C", 0.0),
        req("req_tie_a", "wrk_1", "tie a", 3.0),
        req("req_b", "wrk_1", "B", 1.0),
        req("req_tie_b", "wrk_1", "tie b", 3.0),
    ])));
    assert_eq!(c.name, "Shop");
    assert_eq!(
        listing(&c.items),
        ["1 F/", "2 B", "3 A", "4 tie b", "5 tie a"]
    );
    assert_eq!(listing(&folder(&c.items, "F").items), ["1 C"]);
    let a = &request(&c.items, "A").request;
    assert_eq!(
        (a.method.as_str(), a.url.as_str()),
        ("GET", "https://x.test/")
    );
}

#[test]
fn each_v4_workspace_is_its_own_bundle_in_resource_order() {
    let all = bundles(&v4(json!([
        wrk("wrk_2", "Two"),
        req("req_1", "wrk_1", "in one", 0.0),
        wrk("wrk_1", "One"),
        req("req_2", "wrk_2", "in two", 0.0),
    ])));
    let names: Vec<&str> = all.iter().map(|b| b.collection.name.as_str()).collect();
    assert_eq!(names, ["Two", "One"]);
    assert_eq!(listing(&all[0].collection.items), ["1 in two"]);
    assert_eq!(listing(&all[1].collection.items), ["1 in one"]);
}

#[test]
fn v5_items_sort_by_meta_sort_key_and_empty_ones_are_kept() {
    let c = one(&v5("collection:
  - name: B
    meta: {id: req_b, sortKey: 5}
    method: GET
    url: https://x.test/b
  - name: Empty
    meta: {id: fld_e, sortKey: 3}
  - name: A
    meta: {id: req_a, sortKey: 1}
    method: POST
  - name: F
    meta: {id: fld_f, sortKey: 9}
    children:
      - {name: R, meta: {id: req_r}, method: GET, url: https://x.test/r}
"));
    assert_eq!(listing(&c.items), ["1 A", "2 Empty/", "3 B", "4 F/"]);
    assert!(folder(&c.items, "Empty").items.is_empty());
    assert_eq!(listing(&folder(&c.items, "F").items), ["1 R"]);
    let a = &request(&c.items, "A").request;
    assert_eq!((a.method.as_str(), a.url.as_str()), ("POST", ""));
}

#[test]
fn only_v4_and_v5_collections_are_read() {
    let old = json!({"_type": "export", "__export_format": 3, "resources": []}).to_string();
    assert_eq!(
        refused(&old),
        DocumentError::Unsupported {
            found: "insomnia export format 3".into()
        }
    );
    for kind in [
        "environment.insomnia.rest/5.0",
        "mock.insomnia.rest/5.0",
        "mcpClient.insomnia/5.0",
    ] {
        let e = refused(&format!("type: {kind}\nname: x\n"));
        assert_eq!(e, DocumentError::Unsupported { found: kind.into() });
    }
    assert!(matches!(
        refused(&v4(json!([req("r", "wrk_1", "r", 0.0)]))),
        DocumentError::Invalid { .. }
    ));
    let unnumbered = json!({"_type": "export", "resources": []}).to_string();
    assert_eq!(
        refused(&unnumbered),
        DocumentError::Unsupported {
            found: "insomnia export format (missing)".into()
        }
    );
    let flat = r#"{"_type":"export","__export_format":4,"resources":{}}"#;
    assert!(matches!(refused(flat), DocumentError::Invalid { .. }));
}

#[test]
fn a_spec_document_imports_its_collection_and_drops_the_spec() {
    let c = one("type: spec.insomnia.rest/5.0
name: api
meta: {id: wrk_1}
collection:
  - {name: r, meta: {id: req_1}, method: GET, url: https://x.test}
spec:
  contents: {openapi: 3.0.0}
testSuites:
  - name: s
");
    assert_eq!(listing(&c.items), ["1 r"]);
    assert!(has(&c, NoteKind::Dropped, "collection / spec"));
    assert!(has(&c, NoteKind::Dropped, "collection / testSuites"));
}

#[test]
fn a_v5_collection_without_items_imports_empty() {
    let c = one(&v5(
        "cookieJar:\n  name: Default Jar\n  meta: {id: jar_1}\n  cookies:\n    - {key: a, value: b}\n",
    ));
    assert_eq!(c.name, "c");
    assert!(c.items.is_empty());
    assert!(has(&c, NoteKind::Dropped, "collection / cookieJar"));
    let quiet = one(&v5(
        "cookieJar:\n  name: Default Jar\n  meta: {id: jar_1}\n",
    ));
    assert!(notes(&quiet).is_empty(), "{:#?}", notes(&quiet));
}

#[test]
fn items_without_an_http_mapping_are_unwritten_with_a_note() {
    let c = one(&v4(json!([
        wrk("wrk_1", "c"),
        {"_id": "greq_1", "_type": "grpc_request", "parentId": "wrk_1", "name": "Stream"},
        {"_id": "ws-req_1", "_type": "websocket_request", "parentId": "wrk_1", "name": "Socket"},
        req("req_o", "fld_gone", "Orphan", 0.0),
        req("req_k", "wrk_1", "Kept", 0.0),
    ])));
    assert_eq!(listing(&c.items), ["1 Kept"]);
    for source in ["Stream", "Socket", "resources[3]"] {
        assert!(
            c.unwritten.iter().any(|n| n.source == source),
            "{source}: {:#?}",
            c.unwritten
        );
    }
    let v = one(&v5("collection:
  - {name: g, meta: {id: greq_1}, reflectionApi: {enabled: false}, url: x}
  - {name: w, meta: {id: ws-req_1}, url: 'wss://x.test'}
"));
    assert!(v.items.is_empty());
    assert_eq!(v.unwritten.len(), 2, "{:#?}", v.unwritten);
}

#[test]
fn the_workspace_gives_name_and_docs_and_no_auth() {
    let ws = json!([{"_id": "wrk_1", "_type": "workspace", "parentId": null, "name": "Shop", "description": "All of it"}]);
    let c = one(&v4(ws));
    assert_eq!((c.name.as_str(), c.docs.as_str()), ("Shop", "All of it"));
    assert_eq!(c.auth, AuthSpec::None);
    let v = one(
        "type: collection.insomnia.rest/5.0\nname: Shop\nmeta: {id: wrk_1, description: Five}\n",
    );
    assert_eq!(v.docs, "Five");
}

#[test]
fn yaml_merge_keys_are_noted() {
    let c = one(&v5("collection:
  - &r {name: a, meta: {id: req_a}, method: GET, url: https://x.test}
  - <<: *r
    name: b
"));
    let merged = c
        .unwritten
        .iter()
        .any(|n| n.kind == NoteKind::Converted && n.source == "collection[1].<<");
    assert!(merged, "{:#?}", c.unwritten);
}

/// A v4 workspace `wrk_1` holding request `R`, with `extra` merged over `req`'s members.
fn v4_one(extra: Value) -> ImportedCollection {
    let mut r = req("req_r", "wrk_1", "R", 0.0);
    r.as_object_mut()
        .expect("object")
        .extend(extra.as_object().expect("object").clone());
    one(&v4(json!([wrk("wrk_1", "c"), r])))
}

fn v4_request(extra: Value) -> ImportedRequestItem {
    request(&v4_one(extra).items, "R").clone()
}

fn off(name: &str, value: &str) -> KeyValue {
    let mut kv = KeyValue::new(name, value);
    kv.enabled = false;
    kv
}

#[test]
fn enabled_query_params_join_the_url_and_all_are_listed() {
    let r = v4_request(json!({"url": "https://x.test/a", "parameters": [
        {"name": "q", "value": "1"},
        {"name": "off", "value": "2", "disabled": true},
        {"name": "p", "value": 3, "description": "page"}]}));
    assert_eq!(r.request.url, "https://x.test/a?q=1&p=3");
    let mut disabled = Param::new("off", "2", ParamKind::Query);
    disabled.enabled = false;
    let mut page = Param::new("p", "3", ParamKind::Query);
    page.description = Some("page".into());
    assert_eq!(
        r.request.params,
        [Param::new("q", "1", ParamKind::Query), disabled, page]
    );
    let joined = v4_request(
        json!({"url": "https://x.test/a?x=0", "parameters": [{"name": "q", "value": "1"}]}),
    );
    assert_eq!(joined.request.url, "https://x.test/a?x=0&q=1");
}

#[test]
fn the_query_joins_before_the_fragment_after_any_question_mark() {
    let cases = [
        ("https://x.test/a&", "https://x.test/a&?q=1"),
        ("https://x.test/a?", "https://x.test/a?q=1"),
        ("https://x.test/a?x=0&", "https://x.test/a?x=0&q=1"),
        ("https://x.test/a#top", "https://x.test/a?q=1#top"),
        ("https://x.test/a?x=0#top", "https://x.test/a?x=0&q=1#top"),
        ("https://x.test/a#top?x=0", "https://x.test/a?q=1#top?x=0"),
        (
            "https://x.test/{{ a['#'] }}",
            "https://x.test/{{ a['#'] }}?q=1",
        ),
    ];
    for (url, want) in cases {
        let r = v4_request(json!({"url": url, "parameters": [{"name": "q", "value": "1"}]}));
        assert_eq!(r.request.url, want, "{url}");
    }
}

#[test]
fn path_params_and_headers_keep_their_flags() {
    let r = v4_request(json!({"url": "https://x.test/u/:id",
        "pathParameters": [{"name": "id", "value": "7"}],
        "headers": [{"name": "A", "value": "1", "description": ""},
                    {"name": "B", "value": "2", "disabled": true, "description": "why"}]}));
    assert_eq!(r.request.url, "https://x.test/u/:id");
    assert_eq!(r.request.params, [Param::new("id", "7", ParamKind::Path)]);
    let mut b = off("B", "2");
    b.description = Some("why".into());
    assert_eq!(r.request.headers, [KeyValue::new("A", "1"), b]);
}

#[test]
fn bodies_follow_their_mime_type() {
    let mut file = FileEntry::new("/home/u/data.bin");
    file.selected = true;
    let cases: Vec<(Value, BodySpec)> = vec![
        (json!({}), BodySpec::None),
        (json!({"mimeType": "", "text": ""}), BodySpec::None),
        (
            json!({"mimeType": "application/json; charset=utf-8", "text": "{\"a\":1}"}),
            BodySpec::Json("{\"a\":1}".into()),
        ),
        (
            json!({"mimeType": "application/x-www-form-urlencoded", "params": [{"name": "a", "value": "1"}, {"name": "b", "value": "2", "disabled": true}]}),
            BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "1"), off("b", "2")]),
        ),
        (
            json!({"mimeType": "text/plain", "text": "hi"}),
            BodySpec::Text("hi".into()),
        ),
        (
            json!({"mimeType": "", "text": "raw"}),
            BodySpec::Text("raw".into()),
        ),
        (
            json!({"mimeType": "application/xml", "text": "<a/>"}),
            BodySpec::Xml("<a/>".into()),
        ),
        (
            json!({"mimeType": "text/xml", "text": "<b/>"}),
            BodySpec::Xml("<b/>".into()),
        ),
        (
            json!({"mimeType": "application/octet-stream", "fileName": "/home/u/data.bin"}),
            BodySpec::File(vec![file]),
        ),
        (
            json!({"mimeType": "application/yaml", "text": "a: 1"}),
            BodySpec::Text("a: 1".into()),
        ),
    ];
    for (body, want) in cases {
        let r = v4_request(json!({"method": "POST", "body": body.clone()}));
        assert_eq!(r.request.body, want, "{body}");
    }
}

#[test]
fn file_parts_other_mime_types_and_graphql_are_noted() {
    let r = v4_request(
        json!({"method": "POST", "body": {"mimeType": "multipart/form-data", "params": [
        {"name": "t", "value": "x"}, {"name": "f", "type": "file", "fileName": "/tmp/a.png"}]}}),
    );
    let parts = vec![
        MultipartField::text("t", "x"),
        MultipartField::files("f", vec!["/tmp/a.png".into()]),
    ];
    assert_eq!(r.request.body, BodySpec::Multipart(parts));
    let converted = |r: &ImportedRequestItem, source: &str| {
        r.notes
            .iter()
            .any(|n| n.kind == NoteKind::Converted && n.source == source)
    };
    assert!(
        converted(&r, "R / body.params[1].fileName"),
        "{:#?}",
        r.notes
    );
    let yaml = v4_request(
        json!({"method": "POST", "body": {"mimeType": "application/yaml", "text": "a: 1"}}),
    );
    assert!(converted(&yaml, "R / body.mimeType"), "{:#?}", yaml.notes);
    let gql = v4_request(
        json!({"method": "POST", "body": {"mimeType": "application/graphql", "text": "{\"query\":\"{ a }\"}"}}),
    );
    assert_eq!(gql.request.kind, RequestKind::Graphql);
    assert_eq!(
        gql.request.body,
        BodySpec::Graphql {
            query: "{ a }".into(),
            variables: String::new()
        }
    );
}

#[test]
fn encode_and_redirect_settings_map_and_others_are_noted_off_default() {
    let r = v4_request(
        json!({"settingEncodeUrl": false, "settingFollowRedirects": "off",
        "settingSendCookies": true, "settingStoreCookies": false, "settingRebuildPath": true,
        "settingDisableRenderRequestBody": false}),
    );
    let mut want = RequestSettings::default();
    want.timeout = Timeout::Inherit;
    want.encode_url = false;
    want.follow_redirects = false;
    want.forward_authorization_header = false;
    assert_eq!(r.request.settings, Some(want));
    let dropped = |r: &ImportedRequestItem| -> Vec<String> {
        r.notes
            .iter()
            .filter(|n| n.kind == NoteKind::Dropped)
            .map(|n| n.source.clone())
            .collect()
    };
    assert_eq!(dropped(&r), ["R / settingStoreCookies"]);
    assert_eq!(v4_request(json!({})).request.settings, None);
    let v = one(&v5("collection:
  - name: R
    meta: {id: req_r}
    method: GET
    url: https://x.test
    settings: {encodeUrl: true, followRedirects: global, renderRequestBody: false, cookies: {send: true, store: true}, rebuildPath: true}
"));
    let r = request(&v.items, "R");
    let mut inherited = RequestSettings::default();
    inherited.timeout = Timeout::Inherit;
    inherited.forward_authorization_header = false;
    assert_eq!(r.request.settings, Some(inherited));
    assert_eq!(dropped(r), ["R / settings.renderRequestBody"]);
}

#[test]
fn descriptions_become_docs_and_scripts_are_dropped() {
    let r = v4_request(
        json!({"description": "Gets it", "preRequestScript": "insomnia.x()", "afterResponseScript": ""}),
    );
    assert_eq!(r.request.docs, "Gets it");
    let sources: Vec<&str> = r.notes.iter().map(|n| n.source.as_str()).collect();
    assert_eq!(sources, ["R / preRequestScript"]);
    let v = one(&v5("collection:
  - name: F
    meta: {id: fld_f, description: Folder docs}
    headers:
      - {name: X-F, value: '1'}
    scripts: {preRequest: 'a()'}
    children:
      - name: R
        meta: {id: req_r, description: Req docs}
        method: GET
        url: https://x.test
        scripts: {afterResponse: 'b()'}
"));
    let f = folder(&v.items, "F");
    assert_eq!(f.docs, "Folder docs");
    assert_eq!(f.headers, [KeyValue::new("X-F", "1")]);
    assert!(
        f.notes
            .iter()
            .any(|n| n.kind == NoteKind::Dropped && n.source == "F / scripts.preRequest")
    );
    let r = request(&f.items, "R");
    assert_eq!(r.request.docs, "Req docs");
    assert!(
        r.notes
            .iter()
            .any(|n| n.kind == NoteKind::Dropped && n.source == "F / R / scripts.afterResponse")
    );
}

fn auth_of(auth: Value) -> ImportedRequestItem {
    v4_request(json!({"authentication": auth}))
}

#[test]
fn request_auth_maps_inherit_none_basic_bearer_and_api_keys() {
    let basic = AuthSpec::Basic {
        username: "u".into(),
        password: "p".into(),
    };
    let bearer = AuthSpec::Bearer { token: "t".into() };
    let key = |placement| AuthSpec::ApiKey {
        key: "K".into(),
        value: "k".into(),
        placement,
    };
    let cases: Vec<(Value, AuthSpec)> = vec![
        (json!({}), AuthSpec::Inherit),
        (json!({"username": "u", "token": "t"}), AuthSpec::Inherit),
        (json!({"type": "", "token": "t"}), AuthSpec::Inherit),
        (
            json!({"type": "basic", "username": "u", "password": "p", "disabled": true}),
            AuthSpec::Inherit,
        ),
        (json!({"type": "none"}), AuthSpec::None),
        (
            json!({"type": "basic", "username": "u", "password": "p", "useISO88591": false}),
            basic,
        ),
        (json!({"type": "bearer", "token": "t"}), bearer.clone()),
        (
            json!({"type": "bearer", "token": "t", "prefix": "Bearer"}),
            bearer,
        ),
        (
            json!({"type": "apikey", "key": "K", "value": "k"}),
            key(ApiKeyPlacement::Header),
        ),
        (
            json!({"type": "apikey", "key": "K", "value": "k", "addTo": "header"}),
            key(ApiKeyPlacement::Header),
        ),
        (
            json!({"type": "apikey", "key": "K", "value": "k", "addTo": "queryParams"}),
            key(ApiKeyPlacement::Query),
        ),
    ];
    for (auth, want) in cases {
        let r = auth_of(auth.clone());
        assert_eq!(r.request.auth, want, "{auth}");
        assert!(r.notes.is_empty(), "{auth}: {:#?}", r.notes);
    }
    assert_eq!(v4_request(json!({})).request.auth, AuthSpec::Inherit);
    let iso =
        auth_of(json!({"type": "basic", "username": "u", "password": "p", "useISO88591": true}));
    assert!(
        iso.notes
            .iter()
            .any(|n| n.source == "R / authentication.useISO88591"),
        "{:#?}",
        iso.notes
    );
}

#[test]
fn a_custom_bearer_prefix_becomes_a_header() {
    let r = auth_of(json!({"type": "bearer", "token": "t", "prefix": "Token"}));
    assert_eq!(r.request.auth, AuthSpec::None);
    assert_eq!(
        r.request.headers,
        [KeyValue::new("Authorization", "Token t")]
    );
    let converted = r
        .notes
        .iter()
        .any(|n| n.kind == NoteKind::Converted && n.source == "R / authentication.prefix");
    assert!(converted, "{:#?}", r.notes);
    let bare = auth_of(json!({"type": "bearer", "token": "t", "prefix": "NO_PREFIX"}));
    assert_eq!(bare.request.headers, [KeyValue::new("Authorization", "t")]);
}

#[test]
fn other_auth_types_are_none_with_a_note() {
    for auth in [
        json!({"type": "apikey", "key": "c", "value": "v", "addTo": "cookie"}),
        json!({"type": "digest", "username": "u", "password": "p"}),
        json!({"type": "oauth2", "grantType": "client_credentials"}),
        json!({"type": "oauth1"}),
        json!({"type": "ntlm"}),
        json!({"type": "hawk"}),
        json!({"type": "iam"}),
        json!({"type": "asap"}),
        json!({"type": "netrc"}),
        json!({"type": "singleToken", "token": "t"}),
    ] {
        let r = auth_of(auth.clone());
        assert_eq!(r.request.auth, AuthSpec::None, "{auth}");
        let noted = r
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::Converted && n.source == "R / authentication");
        assert!(noted, "{auth}: {:#?}", r.notes);
    }
}

#[test]
fn folder_auth_maps_and_requests_inherit_it() {
    let c = one(&v4(json!([
        wrk("wrk_1", "c"),
        {"_id": "fld_f", "_type": "request_group", "parentId": "wrk_1", "name": "F",
         "authentication": {"type": "bearer", "token": "t"}},
        {"_id": "fld_g", "_type": "request_group", "parentId": "wrk_1", "name": "G"},
        req("req_r", "fld_f", "R", 0.0),
    ])));
    let f = folder(&c.items, "F");
    assert_eq!(f.auth, AuthSpec::Bearer { token: "t".into() });
    assert_eq!(folder(&c.items, "G").auth, AuthSpec::Inherit);
    assert_eq!(request(&f.items, "R").request.auth, AuthSpec::Inherit);
    let v = one(&v5(
        "collection:\n  - {name: N, meta: {id: fld_n}, authentication: {type: none}, children: []}\n",
    ));
    assert_eq!(folder(&v.items, "N").auth, AuthSpec::None);
}

#[test]
fn a_malformed_authentication_inherits_with_a_note() {
    assert!(auth_of(Value::Null).notes.is_empty());
    for auth in [json!("bearer"), json!([1]), json!(3)] {
        let r = auth_of(auth.clone());
        assert_eq!(r.request.auth, AuthSpec::Inherit, "{auth}");
        let noted = r
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::Dropped && n.source == "R / authentication");
        assert!(noted, "{auth}: {:#?}", r.notes);
    }
}

#[test]
fn unread_keys_of_a_mapped_auth_are_noted() {
    let r = auth_of(json!({"type": "bearer", "token": "t", "extra": 1}));
    assert_eq!(r.request.auth, AuthSpec::Bearer { token: "t".into() });
    let noted = r
        .notes
        .iter()
        .any(|n| n.kind == NoteKind::Dropped && n.source == "R / authentication.extra");
    assert!(noted, "{:#?}", r.notes);
}

#[test]
fn an_explicit_authorization_header_wins_over_a_custom_bearer_prefix() {
    let r = v4_request(json!({
        "headers": [{"name": "authorization", "value": "Mine"}],
        "authentication": {"type": "bearer", "token": "t", "prefix": "Token"},
    }));
    assert_eq!(r.request.auth, AuthSpec::None);
    assert_eq!(r.request.headers, [KeyValue::new("authorization", "Mine")]);
    let noted = r
        .notes
        .iter()
        .any(|n| n.source == "R / authentication.prefix");
    assert!(noted, "{:#?}", r.notes);
}

#[test]
fn templates_are_rewritten_in_values_and_tags_are_noted() {
    let r = v4_request(
        json!({"method": "POST", "url": "{{ _.base }}/u/{% uuid 'v4' %}",
        "parameters": [{"name": "n", "value": "{{ _.page }}"}],
        "headers": [{"name": "{{ _.h }}", "value": "{{ x | upper }}"}],
        "body": {"mimeType": "application/json", "text": "{\"a\": \"{{ _.a.b }}\"}"},
        "authentication": {"type": "basic", "username": "{{ _.user }}", "password": "{% prompt 'p' %}"}}),
    );
    let q = &r.request;
    assert_eq!(q.url, "{{base}}/u/{% uuid 'v4' %}?n={{page}}");
    assert_eq!(q.params, [Param::new("n", "{{page}}", ParamKind::Query)]);
    assert_eq!(q.headers, [KeyValue::new("{{ _.h }}", "{{ x | upper }}")]);
    assert_eq!(q.body, BodySpec::Json("{\"a\": \"{{a.b}}\"}".into()));
    assert_eq!(
        q.auth,
        AuthSpec::Basic {
            username: "{{user}}".into(),
            password: "{% prompt 'p' %}".into()
        }
    );
    let needs: Vec<&ImportNote> = r
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::NeedsInput)
        .collect();
    let sources: Vec<&str> = needs.iter().map(|n| n.source.as_str()).collect();
    assert_eq!(
        sources,
        [
            "R / url",
            "R / headers[0].value",
            "R / authentication.password"
        ]
    );
    assert!(
        needs[0].detail.contains("`{% uuid 'v4' %}`"),
        "{}",
        needs[0].detail
    );
}

fn env(id: &str, parent: &str, name: &str, sort: f64, data: Value) -> Value {
    json!({"_id": id, "_type": "environment", "parentId": parent, "name": name, "metaSortKey": sort, "data": data})
}

fn text(value: &VarValue) -> String {
    match value {
        VarValue::Text(t) => t.clone(),
        _ => panic!("a typed value"),
    }
}

/// `name=value` per variable, or `name secret` for a secret.
fn listed(env: &ImportedEnvironment) -> Vec<String> {
    let line = |v: &apinata_core::EnvVariable| {
        if v.secret {
            format!("{} secret", v.name)
        } else {
            format!("{}={}", v.name, text(&v.value))
        }
    };
    env.variables.iter().map(line).collect()
}

fn folder_vars(vars: &[Variable]) -> Vec<String> {
    vars.iter()
        .map(|v| format!("{}={}", v.name, text(&v.value)))
        .collect()
}

#[test]
fn each_sub_environment_is_the_base_deep_merged_under_it() {
    let all = bundles(&v4(json!([
        wrk("wrk_1", "c"),
        env(
            "env_base",
            "wrk_1",
            "Base Environment",
            0.0,
            json!({"url": "{{ scheme }}://{{ _.host }}",
            "user": {"name": "admin", "roles": ["a", "u"]}, "n": 30000, "on": true, "none": null, "empty": {},
            "t": "{% uuid 'v4' %}"})
        ),
        env(
            "env_dev",
            "env_base",
            "Dev",
            2.0,
            json!({"user": {"name": "dev", "roles": ["x"]}, "extra": 1.5})
        ),
        env("env_stage", "env_base", "Stage", 1.0, json!({"on": false})),
    ])));
    let envs = &all[0].environments;
    let names: Vec<&str> = envs.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Base Environment", "Stage", "Dev"]);
    let common = [
        "url={{scheme}}://{{host}}",
        "user.name=admin",
        "user.roles[0]=a",
        "user.roles[1]=u",
        "n=30000",
        "on=true",
        "none=",
        "t={% uuid 'v4' %}",
    ];
    assert_eq!(listed(&envs[0]), common);
    assert_eq!(
        listed(&envs[1]),
        common.map(|v| if v == "on=true" { "on=false" } else { v })
    );
    assert_eq!(
        listed(&envs[2]),
        [
            "url={{scheme}}://{{host}}",
            "user.name=dev",
            "user.roles[0]=x",
            "n=30000",
            "on=true",
            "none=",
            "t={% uuid 'v4' %}",
            "extra=1.5"
        ]
    );
    assert!(
        envs[0]
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::NeedsInput && n.source == "data.t"),
        "{:#?}",
        envs[0].notes
    );
    assert!(
        envs[0]
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::Converted && n.source == "data.none"),
        "{:#?}",
        envs[0].notes
    );
    assert!(
        envs.iter()
            .flat_map(|e| &e.variables)
            .all(|v| text(&v.value) != "null")
    );
    assert!(
        bundles(&v4(json!([wrk("wrk_1", "c")])))[0]
            .environments
            .is_empty()
    );
}

#[test]
fn environments_stay_with_their_workspace() {
    let all = bundles(&v4(json!([
        wrk("wrk_1", "One"),
        wrk("wrk_2", "Two"),
        env("env_1", "wrk_1", "B1", 0.0, json!({"a": "1"})),
        env("env_1s", "env_1", "S1", 0.0, json!({"b": "2"})),
        env("env_2", "wrk_2", "B2", 0.0, json!({"c": "3"})),
    ])));
    let names = |i: usize| {
        all[i]
            .environments
            .iter()
            .map(|e| e.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(0), ["B1", "S1"]);
    assert_eq!(names(1), ["B2"]);
    assert_eq!(listed(&all[0].environments[1]), ["a=1", "b=2"]);
}

#[test]
fn v5_environments_read_the_base_and_its_subs() {
    let all = bundles(&v5("environments:
  name: Base
  meta: {id: env_b}
  data:
    s: base
    o: {x: 1001, y: kept}
  subEnvironments:
    - name: A
      meta: {id: env_a, isPrivate: true}
      data: {o: {x: 1111}}
"));
    let envs = &all[0].environments;
    assert_eq!(
        envs.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["Base", "A"]
    );
    assert_eq!(listed(&envs[1]), ["s=base", "o.x=1111", "o.y=kept"]);
    let unnamed = bundles(&v5("environments:\n  data: {a: b}\n"));
    assert_eq!(unnamed[0].environments[0].name, "Base Environment");
}

#[test]
fn vault_keys_become_secrets_without_values() {
    let all = bundles(&v4(json!([
        wrk("wrk_1", "c"),
        env(
            "env_b",
            "wrk_1",
            "Base",
            0.0,
            json!({"a": "1", "__insomnia_vault": {"token": "ZW5jcnlwdGVk"}})
        ),
    ])));
    assert_eq!(
        listed(&all[0].environments[0]),
        ["a=1", "vault.token secret"]
    );
    assert!(!format!("{:?}", all[0].environments).contains("ZW5jcnlwdGVk"));
}

#[test]
fn folder_environments_become_folder_variables() {
    let c = one(&v5("collection:
  - name: F
    meta: {id: fld_f}
    environment:
      cascadeVar: fromFolder
      o: {k: 1}
      t: '{{ _.cascadeVar }}-x'
    children: []
"));
    assert_eq!(
        folder_vars(&folder(&c.items, "F").vars),
        ["cascadeVar=fromFolder", "o.k=1", "t={{cascadeVar}}-x"]
    );
}

/// Run `f` on a thread with a 2 MiB stack, the size tokio workers and test threads get, so a
/// deep tree cannot pass only because the main thread's stack is larger.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("no overflow")
}

/// The folder depth down the first item of each level, and the items at the bottom.
fn first_chain(items: &[ImportedItem]) -> (usize, &[ImportedItem]) {
    let (mut depth, mut items) = (0, items);
    while let Some(ImportedItem::Folder(f)) = items.first() {
        depth += 1;
        items = &f.items;
    }
    (depth, items)
}

#[test]
fn a_deep_folder_chain_is_cut_at_128_without_overflowing() {
    let mut resources = vec![wrk("wrk_1", "c")];
    let mut parent = "wrk_1".to_owned();
    for i in 0..10_000 {
        let id = format!("fld_{i}");
        resources.push(group(&id, &parent, "g", 0.0));
        parent = id;
    }
    resources.push(req("req_deep", &parent, "deep", 0.0));
    let input = v4(Value::Array(resources));
    let c = on_small_stack(move || one(&input));
    assert_eq!(first_chain(&c.items).0, 128);
    assert!(
        c.unwritten
            .iter()
            .any(|n| n.kind == NoteKind::Dropped && n.detail.contains("128")),
        "{:#?}",
        c.unwritten
    );
    // The cut folder's note covers everything below it; nothing there is noted again.
    assert_eq!(c.unwritten.len(), 1, "{:#?}", c.unwritten.first());
    on_small_stack(move || drop(c));
}

#[test]
fn a_v4_chain_at_the_cap_keeps_every_folder_and_its_request() {
    let mut resources = vec![wrk("wrk_1", "c")];
    let mut parent = "wrk_1".to_owned();
    for i in 0..128 {
        let id = format!("fld_{i}");
        let mut g = group(&id, &parent, "g", 0.0);
        g["headers"] = json!([{"name": "x", "value": "{{ _.v }}"}]);
        g["authentication"] = json!({"type": "bearer", "token": "t"});
        resources.push(g);
        parent = id;
    }
    resources.push(req("req_deep", &parent, "deep", 0.0));
    let input = v4(Value::Array(resources));
    let c = on_small_stack(move || one(&input));
    let (depth, bottom) = first_chain(&c.items);
    assert_eq!((depth, listing(bottom)), (128, vec!["1 deep".to_owned()]));
    assert!(c.unwritten.is_empty(), "{:#?}", c.unwritten);
    on_small_stack(move || drop(c));
}

#[test]
fn parent_cycles_are_unwritten_and_end() {
    let c = one(&v4(json!([
        wrk("wrk_1", "c"),
        group("fld_a", "fld_b", "A", 0.0),
        group("fld_b", "fld_a", "B", 0.0),
        group("fld_s", "fld_s", "S", 0.0),
        req("req_k", "wrk_1", "K", 0.0),
    ])));
    assert_eq!(listing(&c.items), ["1 K"]);
    assert_eq!(
        c.unwritten
            .iter()
            .filter(|n| n.source.starts_with("resources["))
            .count(),
        3
    );
}

/// A v5 collection of `depth` nested folders, each with a header and auth, around one request.
fn v5_chain(depth: usize) -> String {
    let mut yaml = String::from("type: collection.insomnia.rest/5.0\nname: c\ncollection:\n");
    for d in 0..depth {
        let pad = "    ".repeat(d);
        yaml.push_str(&format!(
            "{pad}  - name: f{d}\n{pad}    headers: [{{name: x, value: v}}]\n{pad}    children:\n"
        ));
    }
    yaml.push_str(&format!(
        "{}  - {{name: r, method: GET, url: https://x.test}}\n",
        "    ".repeat(depth)
    ));
    yaml
}

#[test]
fn v5_folders_nest_to_the_loader_depth() {
    let c = one(&v5_chain(60));
    let (depth, bottom) = first_chain(&c.items);
    assert_eq!((depth, listing(bottom)), (60, vec!["1 r".to_owned()]));
}

#[test]
fn v5_folders_at_the_loader_depth_fit_a_small_stack() {
    // The deepest chain the loader accepts; one level more is refused as too deep.
    let deepest = (1..200)
        .take_while(|&d| parse(&v5_chain(d)).is_ok())
        .last()
        .expect("some depth parses");
    assert!(deepest >= 60, "{deepest}");
    let input = v5_chain(deepest);
    let c = on_small_stack(move || one(&input));
    assert_eq!(first_chain(&c.items).0, deepest);
    on_small_stack(move || drop(c));
}
