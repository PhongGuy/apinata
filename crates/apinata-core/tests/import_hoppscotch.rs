//! Hoppscotch collection exports: versions, the tree, templates, variables, bodies and auth.

use apinata_core::import::{
    CollectionFormat, CollectionOptions, DocumentError, FormatError, ImportBundle, ImportNote,
    ImportedCollection, ImportedFolder, ImportedItem, ImportedRequestItem, NoteKind,
    parse_collections,
};
use apinata_core::{
    ApiKeyPlacement, AuthSpec, BodySpec, FileEntry, KeyValue, MultipartField, Param, ParamKind,
    RequestKind, VarValue, Variable,
};
use serde_json::{Value, json};

fn parse(input: &Value) -> Vec<ImportBundle> {
    let bytes = serde_json::to_vec(input).expect("serialise");
    parse_collections(
        Some(CollectionFormat::Hoppscotch),
        &bytes,
        &CollectionOptions::default(),
    )
    .expect("parses")
}

/// The only collection `input` yields; it carries no environments.
fn one(input: Value) -> ImportedCollection {
    let mut bundles = parse(&input);
    assert_eq!(bundles.len(), 1);
    assert!(bundles[0].environments.is_empty());
    bundles.remove(0).collection
}

fn error(input: &[u8]) -> DocumentError {
    match parse_collections(
        Some(CollectionFormat::Hoppscotch),
        input,
        &CollectionOptions::default(),
    ) {
        Err(FormatError::Hoppscotch(e)) => e,
        other => panic!("{other:?}"),
    }
}

/// A version 17 GET request named `name`.
fn req(name: &str) -> Value {
    json!({"v": "17", "name": name, "method": "GET", "endpoint": "https://x.test/a", "params": [], "headers": [],
        "auth": {"authType": "inherit", "authActive": true}, "body": {"contentType": null, "body": null},
        "preRequestScript": "", "testScript": "", "requestVariables": [], "responses": {}, "description": null})
}

/// A version 12 collection or folder.
fn coll(name: &str, folders: Vec<Value>, requests: Vec<Value>) -> Value {
    json!({"v": 12, "name": name, "folders": folders, "requests": requests,
        "auth": {"authType": "inherit", "authActive": true}, "headers": [], "variables": [],
        "description": null, "preRequestScript": "", "testScript": ""})
}

/// `value` with every member of `extra` set on it.
fn with(mut value: Value, extra: Value) -> Value {
    for (key, member) in extra.as_object().expect("object") {
        value[key.as_str()] = member.clone();
    }
    value
}

fn without(mut value: Value, key: &str) -> Value {
    value.as_object_mut().expect("object").remove(key);
    value
}

fn folder<'a>(items: &'a [ImportedItem], name: &str) -> &'a ImportedFolder {
    items
        .iter()
        .find_map(|i| match i {
            ImportedItem::Folder(f) if f.name == name => Some(f),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no folder {name}"))
}

/// The request named `name`, searched depth first.
fn request<'a>(items: &'a [ImportedItem], name: &str) -> &'a ImportedRequestItem {
    fn find<'a>(items: &'a [ImportedItem], name: &str) -> Option<&'a ImportedRequestItem> {
        items.iter().find_map(|i| match i {
            ImportedItem::Request(r) if r.name == name => Some(r),
            ImportedItem::Folder(f) => find(&f.items, name),
            _ => None,
        })
    }
    find(items, name).unwrap_or_else(|| panic!("no request {name}"))
}

/// Every note: the collection's, the unwritten ones, then each item's, depth first.
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
fn a_collection_object_is_one_bundle_with_its_name_kept() {
    let c = one(coll("...", vec![], vec![req("r")]));
    assert_eq!((c.name.as_str(), c.items.len()), ("...", 1));
    assert_eq!(one(coll("CON", vec![], vec![])).name, "CON");
    assert_eq!(
        one(without(coll("x", vec![], vec![]), "name")).name,
        "Untitled"
    );
}

#[test]
fn an_array_gives_one_bundle_per_collection_in_order() {
    let bundles = parse(&json!([
        coll("a", vec![], vec![]),
        coll("b", vec![], vec![req("r")])
    ]));
    let names: Vec<&str> = bundles.iter().map(|b| b.collection.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    assert!(bundles.iter().all(|b| b.environments.is_empty()));
}

#[test]
fn input_that_is_not_a_collection_export_is_invalid() {
    for input in [
        &b"[]"[..],
        b"[1]",
        b"\"x\"",
        b"{}",
        br#"{"name":"c","folders":[]}"#,
        br#"[{"name":"c","folders":[],"requests":[]},{"name":"d","requests":[]}]"#,
    ] {
        let e = error(input);
        assert!(
            matches!(e, DocumentError::Invalid { .. }),
            "{}: {e:?}",
            String::from_utf8_lossy(input)
        );
    }
}

#[test]
fn folders_come_before_requests_and_seq_is_the_position() {
    let inner = coll("f1", vec![], vec![req("deep")]);
    let c = one(coll(
        "c",
        vec![inner, coll("f2", vec![], vec![])],
        vec![req("r1"), req("r2")],
    ));
    let order: Vec<(String, u64)> = c
        .items
        .iter()
        .map(|i| match i {
            ImportedItem::Folder(f) => (f.name.clone(), f.seq),
            ImportedItem::Request(r) => (r.name.clone(), r.seq),
            _ => unreachable!("only folders and requests exist"),
        })
        .collect();
    let want = [("f1", 1), ("f2", 2), ("r1", 3), ("r2", 4)].map(|(n, s)| (n.to_owned(), s));
    assert_eq!(order, want);
    assert_eq!(request(&folder(&c.items, "f1").items, "deep").seq, 1);
}

#[test]
fn collection_versions_read_missing_known_and_newer() {
    let c = one(without(coll("c", vec![], vec![]), "v"));
    assert!(notes(&c).is_empty(), "{:?}", notes(&c));
    for v in 1..=12 {
        let c = one(with(coll("c", vec![], vec![]), json!({"v": v})));
        assert!(notes(&c).is_empty(), "v {v}: {:?}", notes(&c));
    }
    let odd = with(coll("f", vec![], vec![]), json!({"v": "x"}));
    let newer = one(with(coll("c", vec![odd], vec![]), json!({"v": 13})));
    assert!(has(&newer, NoteKind::Converted, "collection / v"));
    assert!(has(&newer, NoteKind::Converted, "f / v"));
    assert_eq!(newer.items.len(), 1);
}

#[test]
fn request_versions_read_newer_and_graphql_and_skip_unknown() {
    let gql = json!({"v": 10, "name": "q", "url": "https://x.test/graphql", "query": "{ a }", "headers": [], "variables": "{}"});
    let legacy_gql = json!({"name": "lq", "url": "https://x.test/graphql", "query": "{ a }"});
    let requests = vec![
        with(req("new"), json!({"v": "18"})),
        with(req("odd"), json!({"v": "x1"})),
        gql,
        legacy_gql,
        req("kept"),
    ];
    let c = one(coll("c", vec![], requests));
    let names: Vec<&str> = c
        .items
        .iter()
        .map(|i| match i {
            ImportedItem::Request(r) => r.name.as_str(),
            _ => "folder",
        })
        .collect();
    assert_eq!(names, ["new", "q", "lq", "kept"]);
    assert!(has(&c, NoteKind::Converted, "new / v"));
    for name in ["q", "lq"] {
        assert_eq!(request(&c.items, name).request.kind, RequestKind::Graphql);
    }
    let found = c
        .unwritten
        .iter()
        .any(|n| n.source == "odd" && n.kind == NoteKind::Dropped);
    assert!(found, "odd: {:?}", c.unwritten);
    assert_eq!(
        (request(&c.items, "new").seq, request(&c.items, "kept").seq),
        (1, 4),
        "unwritten requests take no seq"
    );
}

#[test]
fn a_request_keeps_its_method_url_params_headers_and_docs() {
    let r = with(
        req("r"),
        json!({"method": "POST", "endpoint": "https://x.test/a?z=0", "description": "Gets things",
        "params": [{"key": "p", "value": "1", "active": true, "description": "page"}, {"key": "q", "value": "2", "active": false, "description": ""}],
        "headers": [{"key": "X-A", "value": "1", "active": true, "description": ""}, {"key": "X-B", "value": "2", "active": false}]}),
    );
    let plain = with(
        req("plain"),
        json!({"params": [{"key": "p", "value": "1", "active": true}]}),
    );
    let c = one(coll("c", vec![], vec![r, plain]));
    let r = &request(&c.items, "r").request;
    assert_eq!(
        (r.method.as_str(), r.url.as_str(), r.docs.as_str()),
        ("POST", "https://x.test/a?z=0&p=1", "Gets things")
    );
    let mut p = Param::new("p", "1", ParamKind::Query);
    p.description = Some("page".into());
    let mut q = Param::new("q", "2", ParamKind::Query);
    q.enabled = false;
    assert_eq!(r.params, [p, q]);
    let mut b = KeyValue::new("X-B", "2");
    b.enabled = false;
    assert_eq!(r.headers, [KeyValue::new("X-A", "1"), b]);
    assert_eq!(
        request(&c.items, "plain").request.url,
        "https://x.test/a?p=1"
    );
    assert!(notes(&c).is_empty(), "{:?}", notes(&c));
}

#[test]
fn params_go_before_a_fragment() {
    let r = with(
        req("r"),
        json!({"endpoint": "https://x.test/a#top", "params": [{"key": "p", "value": "1", "active": true}]}),
    );
    let c = one(coll("c", vec![], vec![r]));
    assert_eq!(
        request(&c.items, "r").request.url,
        "https://x.test/a?p=1#top"
    );
}

#[test]
fn a_null_collection_version_reads_as_the_newest_with_a_note() {
    let c = one(with(coll("c", vec![], vec![]), json!({"v": null})));
    assert!(has(&c, NoteKind::Converted, "collection / v"));
}

#[test]
fn folders_and_entries_that_cannot_be_read_are_dropped_with_a_note() {
    let bad = with(coll("bad", vec![], vec![]), json!({"headers": "x"}));
    let inner = coll("f", vec![], vec![json!(2)]);
    let c = one(coll("c", vec![json!(1), bad, inner], vec![]));
    assert_eq!(c.items.len(), 1);
    assert_eq!(folder(&c.items, "f").seq, 1);
    for source in ["collection / folders[0]", "bad", "f / requests[0]"] {
        let found = c
            .unwritten
            .iter()
            .any(|n| n.source == source && n.kind == NoteKind::Dropped);
        assert!(found, "{source}: {:?}", c.unwritten);
    }
}

#[test]
fn collection_and_folder_headers_and_docs_are_kept() {
    let f = with(
        coll("f", vec![], vec![]),
        json!({"headers": [{"key": "X-F", "value": "1", "active": true, "description": ""}], "description": "folder docs"}),
    );
    let c = one(with(
        coll("c", vec![f], vec![]),
        json!({"headers": [{"key": "X-C", "value": "2", "active": false}], "description": "root docs"}),
    ));
    let mut xc = KeyValue::new("X-C", "2");
    xc.enabled = false;
    assert_eq!(
        (c.headers.clone(), c.docs.as_str()),
        (vec![xc], "root docs")
    );
    let f = folder(&c.items, "f");
    assert_eq!(
        (f.headers.clone(), f.docs.as_str()),
        (vec![KeyValue::new("X-F", "1")], "folder docs")
    );
}

#[test]
fn unknown_members_are_dropped_with_a_note_and_exempt_ones_are_not() {
    let f = with(
        coll("f", vec![], vec![]),
        json!({"color": "red", "id": "1", "_ref_id": "2"}),
    );
    let r = with(req("r"), json!({"extra": true, "id": "3", "_ref_id": "4"}));
    let c = one(with(
        coll("c", vec![f], vec![r]),
        json!({"zz": 1, "_ref_id": "5"}),
    ));
    let all = notes(&c);
    let dropped: Vec<&str> = all
        .iter()
        .filter(|n| n.kind == NoteKind::Dropped)
        .map(|n| n.source.as_str())
        .collect();
    assert_eq!(dropped, ["collection / zz", "f / color", "r / extra"]);
}
#[test]
fn angle_templates_become_braces_in_values_but_not_names() {
    let r = with(
        req("r"),
        json!({"endpoint": "<<base>>/users/<<id>>",
        "params": [{"key": "k", "value": "<<v>>", "active": true}],
        "headers": [{"key": "<<name>>", "value": "Bearer <<token>>", "active": true}]}),
    );
    let c = one(coll("c", vec![], vec![r]));
    let r = &request(&c.items, "r").request;
    assert_eq!(r.url, "{{base}}/users/{{id}}?k={{v}}");
    assert_eq!(r.params, [Param::new("k", "{{v}}", ParamKind::Query)]);
    assert_eq!(r.headers, [KeyValue::new("<<name>>", "Bearer {{token}}")]);
    assert!(notes(&c).is_empty(), "{:?}", notes(&c));
}

#[test]
fn dynamic_variables_keep_their_name_and_only_unknown_ones_need_input() {
    let r = with(
        req("r"),
        json!({"endpoint": "https://x.test/<<$guid>>/<<$nope>>/<<>>/<<open",
        "headers": [{"key": "X-Id", "value": "<<$guid>>-<<$timestamp>>", "active": true}]}),
    );
    let c = one(coll("c", vec![], vec![r]));
    let r = &request(&c.items, "r").request;
    assert_eq!(r.url, "https://x.test/{{$guid}}/{{$nope}}/<<>>/<<open");
    assert_eq!(r.headers[0].value, "{{$guid}}-{{$timestamp}}");
    let needs: Vec<_> = notes(&c)
        .into_iter()
        .filter(|n| n.kind == NoteKind::NeedsInput)
        .collect();
    assert!(
        needs.iter().all(|n| !n.detail.contains("guid")),
        "{needs:?}"
    );
    assert!(
        needs
            .iter()
            .any(|n| n.source == "r / endpoint" && n.detail.contains("$nope")),
        "{needs:?}"
    );
    assert!(
        needs.iter().all(|n| n.source != "r / headers[0].value"),
        "{needs:?}"
    );
}

#[test]
fn request_variables_become_request_vars() {
    let r = with(
        req("r"),
        json!({"requestVariables": [
        {"key": "id", "value": "<<x>>", "active": true}, {"key": "off", "value": "1", "active": false}]}),
    );
    let c = one(coll("c", vec![], vec![r]));
    let mut off = Variable::new("off", VarValue::Text("1".into()));
    off.enabled = false;
    assert_eq!(
        request(&c.items, "r").request.vars,
        [Variable::new("id", VarValue::Text("{{x}}".into())), off]
    );
}

#[test]
fn collection_and_folder_variables_take_the_initial_value_and_secrets_need_input() {
    let vars = json!([
        {"key": "a", "initialValue": "<<x>>", "currentValue": "cur", "secret": false},
        {"key": "b", "initialValue": "", "currentValue": "3", "secret": false},
        {"key": "s", "initialValue": "", "currentValue": "", "secret": true}]);
    let f = with(
        coll("f", vec![], vec![]),
        json!({"variables": vars.clone()}),
    );
    let c = one(with(coll("c", vec![f], vec![]), json!({"variables": vars})));
    let text = |n: &str, v: &str| Variable::new(n, VarValue::Text(v.into()));
    let want = [text("a", "{{x}}"), text("b", "3"), text("s", "")];
    assert_eq!(c.vars, want);
    assert_eq!(folder(&c.items, "f").vars, want);
    assert!(has(&c, NoteKind::NeedsInput, "collection / variables[2]"));
    assert!(has(&c, NoteKind::NeedsInput, "f / variables[2]"));
}

#[test]
fn scripts_and_saved_responses_are_dropped_with_notes() {
    let r = with(
        req("r"),
        json!({"preRequestScript": "pw.env.set('a', '1')", "testScript": "pw.test('ok', () => {})",
        "responses": {"ok": {"name": "ok", "status": "OK", "code": 200, "headers": [], "body": "{}"}}}),
    );
    let c = one(with(
        coll("c", vec![], vec![r, req("quiet")]),
        json!({"preRequestScript": "hopp.env.set('a', '1')"}),
    ));
    for source in [
        "r / preRequestScript",
        "r / testScript",
        "r / responses",
        "collection / preRequestScript",
    ] {
        assert!(
            has(&c, NoteKind::Dropped, source),
            "{source}: {:?}",
            notes(&c)
        );
    }
    assert_eq!(notes(&c).len(), 4, "{:?}", notes(&c));
}

#[test]
fn exempt_collection_keys_produce_no_notes() {
    let input = json!({"v": 12, "id": "i", "_ref_id": "r", "name": "c", "folders": [], "auth": {"authType": "none", "authActive": true},
        "headers": [], "variables": [], "description": null, "preRequestScript": "", "testScript": "",
        "requests": [{"v": "17", "id": "x", "_ref_id": "y", "name": "r", "method": "POST", "endpoint": "https://x.test", "params": [], "headers": [],
            "auth": {"authType": "inherit", "authActive": true}, "preRequestScript": "", "testScript": "", "requestVariables": [], "responses": {}, "description": null,
            "body": {"contentType": "multipart/form-data", "body": [], "isBulkEditing": false, "showIndividualContentType": false}}]});
    let c = one(input);
    let all = notes(&c);
    assert!(all.is_empty(), "{all:?}");
}
fn body(contents: Value) -> (BodySpec, ImportedCollection) {
    let c = one(coll(
        "c",
        vec![],
        vec![with(req("r"), json!({"method": "POST", "body": contents}))],
    ));
    (request(&c.items, "r").request.body.clone(), c)
}

fn auth(spec: Value) -> (AuthSpec, ImportedCollection) {
    let c = one(coll(
        "c",
        vec![],
        vec![with(req("r"), json!({"auth": spec}))],
    ));
    (request(&c.items, "r").request.auth.clone(), c)
}

#[test]
fn string_bodies_follow_their_content_type() {
    let (sent, got) = (r#"{"a":"<<x>>"}"#, r#"{"a":"{{x}}"}"#);
    for (kind, want) in [
        ("application/json", BodySpec::Json(got.into())),
        ("application/ld+json", BodySpec::Json(got.into())),
        ("application/hal+json", BodySpec::Json(got.into())),
        ("application/vnd.api+json", BodySpec::Json(got.into())),
        ("application/xml", BodySpec::Xml(got.into())),
        ("text/xml", BodySpec::Xml(got.into())),
        ("text/html", BodySpec::Text(got.into())),
        ("text/plain", BodySpec::Text(got.into())),
    ] {
        let (spec, c) = body(json!({"contentType": kind, "body": sent}));
        assert_eq!(spec, want, "{kind}");
        assert!(notes(&c).is_empty(), "{kind}: {:?}", notes(&c));
    }
    assert_eq!(
        body(json!({"contentType": null, "body": null})).0,
        BodySpec::None
    );
    let (spec, c) = body(json!({"contentType": "application/x-custom", "body": "raw"}));
    assert_eq!(spec, BodySpec::Text("raw".into()));
    assert!(has(&c, NoteKind::Converted, "r / body.contentType"));
}

#[test]
fn a_body_without_a_content_type_is_not_sent_and_is_noted_when_not_empty() {
    for empty in [json!(null), json!(""), json!([])] {
        let (spec, c) = body(json!({"contentType": null, "body": empty}));
        assert_eq!(spec, BodySpec::None, "{empty}");
        assert!(notes(&c).is_empty(), "{empty}: {:?}", notes(&c));
    }
    let field = json!({"key": "k", "value": "v", "active": true, "isFile": false});
    for content in [json!("<a/>"), json!([field]), json!({"a": 1})] {
        let (spec, c) = body(json!({"contentType": null, "body": content}));
        assert_eq!(spec, BodySpec::None, "{content}");
        assert!(has(&c, NoteKind::Dropped, "r / body.body"), "{content}");
    }
}

#[test]
fn urlencoded_bodies_read_the_key_value_lines() {
    let text = "a: 1\n# b: 2\n\n\"c:d\": \"x y\"\ne:\nf\ng: <<v>>";
    let (spec, c) = body(
        json!({"contentType": "application/x-www-form-urlencoded", "body": text, "isBulkEditing": false}),
    );
    let mut b = KeyValue::new("b", "2");
    b.enabled = false;
    let want = vec![
        KeyValue::new("a", "1"),
        b,
        KeyValue::new("c:d", "x y"),
        KeyValue::new("e", ""),
        KeyValue::new("f", ""),
        KeyValue::new("g", "{{v}}"),
    ];
    assert_eq!(spec, BodySpec::FormUrlEncoded(want));
    assert!(notes(&c).is_empty(), "{:?}", notes(&c));
}

#[test]
fn multipart_files_lost_by_the_export_need_input() {
    let (spec, c) = body(
        json!({"contentType": "multipart/form-data", "showIndividualContentType": false, "body": [
        {"key": "name", "value": "<<who>>", "active": true, "isFile": false},
        {"key": "doc", "value": [{}], "active": true, "isFile": true, "contentType": "application/pdf"},
        {"key": "off", "value": "x", "active": false, "isFile": false}]}),
    );
    let mut doc = MultipartField::files("doc", vec![String::new()]);
    doc.content_type = "application/pdf".into();
    let mut off = MultipartField::text("off", "x");
    off.enabled = false;
    assert_eq!(
        spec,
        BodySpec::Multipart(vec![MultipartField::text("name", "{{who}}"), doc, off])
    );
    assert!(has(&c, NoteKind::NeedsInput, "r / body.body[1]"));
}

#[test]
fn binary_bodies_become_an_empty_file_that_needs_input() {
    for kind in ["application/octet-stream", "binary"] {
        let (spec, c) = body(json!({"contentType": kind, "body": {}}));
        let mut file = FileEntry::new("");
        file.selected = true;
        assert_eq!(spec, BodySpec::File(vec![file]), "{kind}");
        assert!(has(&c, NoteKind::NeedsInput, "r / body"), "{kind}");
    }
}

#[test]
fn legacy_requests_join_url_and_path_prefer_raw_params_and_drop_form_params() {
    let v0 = |extra: Value| {
        with(
            json!({"name": "old", "method": "POST", "url": "https://x.test", "path": "/p", "auth": "None",
        "headers": [{"key": "H", "value": "1", "active": true}], "params": [{"key": "k", "value": "v", "active": true}, {"key": "off", "value": "x", "active": false}]}),
            extra,
        )
    };
    let c = one(json!({"v": 1, "name": "c", "folders": [], "requests": [
        v0(json!({"name": "raw", "contentType": "application/json", "rawParams": "{\"a\":1}", "body": "ignored", "rawInput": true})),
        v0(json!({"name": "plain", "contentType": "text/plain", "body": "hi"})),
        v0(json!({"name": "empty", "contentType": "", "body": ""})),
        v0(json!({"name": "form", "contentType": "application/x-www-form-urlencoded", "body": "", "bodyParams": [{"key": "a", "value": "1"}]}))]}));
    let raw = &request(&c.items, "raw").request;
    assert_eq!(
        (raw.url.as_str(), raw.headers.clone(), raw.params.len()),
        ("https://x.test/p?k=v", vec![KeyValue::new("H", "1")], 2)
    );
    assert_eq!(raw.body, BodySpec::Json("{\"a\":1}".into()));
    assert_eq!(
        request(&c.items, "plain").request.body,
        BodySpec::Text("hi".into())
    );
    assert_eq!(request(&c.items, "empty").request.body, BodySpec::None);
    assert!(has(&c, NoteKind::Dropped, "form / bodyParams"));
}

#[test]
fn auth_maps_basic_bearer_api_key_inherit_and_none() {
    let key = |placement| AuthSpec::ApiKey {
        key: "X-Key".into(),
        value: "{{k}}".into(),
        placement,
    };
    let api = |add_to: &str| json!({"authType": "api-key", "authActive": true, "key": "X-Key", "value": "<<k>>", "addTo": add_to});
    let cases = [
        (
            json!({"authType": "none", "authActive": true}),
            AuthSpec::None,
        ),
        (
            json!({"authType": "inherit", "authActive": true}),
            AuthSpec::Inherit,
        ),
        (
            json!({"authType": "basic", "authActive": true, "username": "<<u>>", "password": "p"}),
            AuthSpec::Basic {
                username: "{{u}}".into(),
                password: "p".into(),
            },
        ),
        (
            json!({"authType": "bearer", "authActive": true, "token": "<<t>>"}),
            AuthSpec::Bearer {
                token: "{{t}}".into(),
            },
        ),
        (api("HEADERS"), key(ApiKeyPlacement::Header)),
        (api("Headers"), key(ApiKeyPlacement::Header)),
        (api("QUERY_PARAMS"), key(ApiKeyPlacement::Query)),
        (api("Query params"), key(ApiKeyPlacement::Query)),
        (
            json!({"authType": "bearer", "authActive": false, "token": "t", "zz": 1}),
            AuthSpec::None,
        ),
        (
            json!({"authType": "oauth-2", "authActive": false, "grantTypeInfo": {"grantType": "AUTHORIZATION_CODE"}}),
            AuthSpec::None,
        ),
    ];
    for (input, want) in cases {
        let (got, c) = auth(input.clone());
        assert_eq!(got, want, "{input}");
        assert!(notes(&c).is_empty(), "{input}: {:?}", notes(&c));
    }
}

#[test]
fn unsupported_auth_is_none_with_one_note_for_the_whole_object() {
    for kind in [
        "oauth-2",
        "aws-signature",
        "digest",
        "hawk",
        "akamai-eg",
        "jwt",
        "something-new",
    ] {
        let (got, c) = auth(
            json!({"authType": kind, "authActive": true, "grantTypeInfo": {"token": "t"}, "zz": 1}),
        );
        assert_eq!(got, AuthSpec::None, "{kind}");
        assert!(
            has(&c, NoteKind::Converted, "r / auth"),
            "{kind}: {:?}",
            notes(&c)
        );
        assert_eq!(notes(&c).len(), 1, "{kind}: {:?}", notes(&c));
    }
}

#[test]
fn a_root_inherits_nothing_and_folders_keep_their_auth() {
    let f = with(
        coll("f", vec![], vec![]),
        json!({"auth": {"authType": "bearer", "authActive": true, "token": "t"}}),
    );
    let c = one(coll("c", vec![f, coll("g", vec![], vec![])], vec![]));
    assert_eq!(c.auth, AuthSpec::None);
    assert_eq!(
        folder(&c.items, "f").auth,
        AuthSpec::Bearer { token: "t".into() }
    );
    assert_eq!(folder(&c.items, "g").auth, AuthSpec::Inherit);
    let v1 = one(
        json!({"v": 1, "name": "c", "folders": [{"v": 1, "name": "f", "folders": [], "requests": []}], "requests": []}),
    );
    assert_eq!(
        (v1.auth.clone(), folder(&v1.items, "f").auth.clone()),
        (AuthSpec::None, AuthSpec::Inherit)
    );
}

#[test]
fn legacy_auth_strings_map() {
    let v0 = |name: &str, extra: Value| {
        with(
            json!({"name": name, "method": "GET", "url": "https://x.test", "path": "", "headers": [], "params": []}),
            extra,
        )
    };
    let c = one(json!({"v": 1, "name": "c", "folders": [], "requests": [
        v0("none", json!({"auth": "None"})),
        v0("basic", json!({"auth": "Basic Auth", "httpUser": "<<u>>", "httpPassword": "p"})),
        v0("bearer", json!({"auth": "Bearer Token", "bearerToken": "t"})),
        v0("odd", json!({"auth": "Digest"})),
        v0("missing", json!({}))]}));
    let auth_of = |n: &str| request(&c.items, n).request.auth.clone();
    assert_eq!(auth_of("none"), AuthSpec::None);
    assert_eq!(
        auth_of("basic"),
        AuthSpec::Basic {
            username: "{{u}}".into(),
            password: "p".into()
        }
    );
    assert_eq!(auth_of("bearer"), AuthSpec::Bearer { token: "t".into() });
    assert_eq!(auth_of("odd"), AuthSpec::None);
    assert!(has(&c, NoteKind::Converted, "odd / auth"));
    assert_eq!(auth_of("missing"), AuthSpec::None);
}

#[test]
fn the_versions_missing_from_the_fixtures_read_without_notes() {
    // Collection v9 and request versions 4, 9, 10, 14 and 16 appear in no vendored fixture (research L5).
    let at = |v: &str, extra: Value| with(req(&format!("v{v}")), with(json!({"v": v}), extra));
    let requests = vec![
        at(
            "4",
            json!({"auth": {"authType": "api-key", "authActive": true, "key": "k", "value": "v", "addTo": "QUERY_PARAMS"}}),
        ),
        at(
            "9",
            json!({"body": {"contentType": "multipart/form-data", "showIndividualContentType": true,
            "body": [{"key": "a", "value": "1", "active": true, "isFile": false, "contentType": "text/plain"}]}}),
        ),
        at(
            "10",
            json!({"body": {"contentType": "application/x-www-form-urlencoded", "isBulkEditing": true, "body": "a: 1"}}),
        ),
        at("14", json!({})),
        at("16", json!({"_ref_id": "req_1"})),
    ];
    let c = one(with(coll("c", vec![], requests), json!({"v": 9})));
    let all = notes(&c);
    assert!(all.is_empty(), "{all:?}");
    let api = AuthSpec::ApiKey {
        key: "k".into(),
        value: "v".into(),
        placement: ApiKeyPlacement::Query,
    };
    assert_eq!(request(&c.items, "v4").request.auth, api);
    let mut a = MultipartField::text("a", "1");
    a.content_type = "text/plain".into();
    assert_eq!(
        request(&c.items, "v9").request.body,
        BodySpec::Multipart(vec![a])
    );
    assert_eq!(
        request(&c.items, "v10").request.body,
        BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "1")])
    );
    assert_eq!(request(&c.items, "v16").request.url, "https://x.test/a");
}

#[test]
fn a_request_with_a_null_version_is_not_imported() {
    let c = one(coll(
        "c",
        vec![],
        vec![with(req("nul"), json!({"v": null})), req("kept")],
    ));
    assert!(
        c.unwritten
            .iter()
            .any(|n| n.source == "nul" && n.kind == NoteKind::Dropped),
        "{:?}",
        c.unwritten
    );
    assert_eq!(request(&c.items, "kept").seq, 1);
    assert_eq!(c.items.len(), 1);
}

#[test]
fn a_legacy_request_never_loses_its_endpoint() {
    let only = json!({"v": "0", "name": "only", "endpoint": "https://x.test/e"});
    let both = json!({"name": "both", "url": "https://x.test", "path": "/p", "endpoint": "https://x.test/e"});
    let c = one(coll("c", vec![], vec![only, both]));
    assert_eq!(request(&c.items, "only").request.url, "https://x.test/e");
    assert!(
        notes(&c).iter().all(|n| !n.source.starts_with("only")),
        "{:?}",
        notes(&c)
    );
    assert_eq!(request(&c.items, "both").request.url, "https://x.test/p");
    assert!(
        has(&c, NoteKind::Dropped, "both / endpoint"),
        "{:?}",
        notes(&c)
    );
}

#[test]
fn a_secret_variable_note_holds_whether_or_not_a_value_was_exported() {
    let vars = json!([{"key": "s", "initialValue": "shown", "currentValue": "", "secret": true}]);
    let c = one(with(coll("c", vec![], vec![]), json!({"variables": vars})));
    let note = notes(&c)
        .into_iter()
        .find(|n| n.source == "collection / variables[0]")
        .expect("a note");
    assert!(!note.detail.contains("not exported"), "{}", note.detail);
}

#[test]
fn a_differing_current_value_is_noted_when_the_initial_value_wins() {
    let vars = json!([
        {"key": "a", "initialValue": "1", "currentValue": "9", "secret": false},
        {"key": "b", "initialValue": "2", "currentValue": "2", "secret": false},
        {"key": "c", "initialValue": "3", "currentValue": "", "secret": false},
        {"key": "d", "initialValue": "", "currentValue": "4", "secret": false}]);
    let c = one(with(coll("c", vec![], vec![]), json!({"variables": vars})));
    let text = |n: &str, v: &str| Variable::new(n, VarValue::Text(v.into()));
    assert_eq!(
        c.vars,
        [
            text("a", "1"),
            text("b", "2"),
            text("c", "3"),
            text("d", "4")
        ]
    );
    let dropped: Vec<&str> = notes(&c)
        .iter()
        .filter(|n| n.kind == NoteKind::Dropped)
        .map(|n| n.source.as_str())
        .collect();
    assert_eq!(dropped, ["collection / variables[0].currentValue"]);
    assert!(notes(&c).iter().all(|n| !n.detail.contains('9')));
}

#[test]
fn a_collection_variable_without_a_name_is_dropped_with_a_note() {
    let vars = json!([
        {"initialValue": "1", "currentValue": "", "secret": false},
        {"key": "", "initialValue": "2", "currentValue": "", "secret": false},
        {"key": "a", "initialValue": "3", "currentValue": "", "secret": false}]);
    let f = with(
        coll("f", vec![], vec![]),
        json!({"variables": vars.clone()}),
    );
    let c = one(with(coll("c", vec![f], vec![]), json!({"variables": vars})));
    let want = [Variable::new("a", VarValue::Text("3".into()))];
    assert_eq!(c.vars, want);
    assert_eq!(folder(&c.items, "f").vars, want);
    for source in [
        "collection / variables[0]",
        "collection / variables[1]",
        "f / variables[0]",
        "f / variables[1]",
    ] {
        assert!(has(&c, NoteKind::Dropped, source), "{source}");
    }
}

#[test]
fn urlencoded_lines_without_a_key_are_skipped_and_noted_when_they_carry_a_value() {
    let text = "a: 1\n: v\n#\n# : w\n:\nb: 2";
    let (spec, c) = body(
        json!({"contentType": "application/x-www-form-urlencoded", "body": text, "isBulkEditing": false}),
    );
    assert_eq!(
        spec,
        BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "1"), KeyValue::new("b", "2")])
    );
    let dropped: Vec<&str> = notes(&c)
        .iter()
        .filter(|n| n.kind == NoteKind::Dropped)
        .map(|n| n.source.as_str())
        .collect();
    assert_eq!(dropped, ["r / body.body", "r / body.body"]);
}
