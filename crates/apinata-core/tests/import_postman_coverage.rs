//! Notes for what Apinata does not carry over: scripts, examples, folder variables, and (spec
//! criterion 3) an unknown key planted at the collection, info, folder, url, body and auth level
//! of every collection fixture, which must come back through the `rest` catch-alls.

use apinata_core::import::{ImportNote, ImportedCollection, ImportedItem, NoteKind, parse_postman};
use serde_json::Value;
use std::fs;
use std::path::Path;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";
const PROBE: &str = "zzApinataProbe";

/// A v2.1 collection named `c`; `extra` adds top-level members (starting with a comma).
fn parse(extra: &str, items: &str) -> ImportedCollection {
    let json = format!(r#"{{"info": {{"name": "c", "schema": "{V21}"}}{extra}, "item": {items}}}"#);
    parse_postman(json.as_bytes()).expect("parses")
}

fn has(notes: &[ImportNote], kind: NoteKind, source: &str) -> bool {
    notes.iter().any(|n| n.kind == kind && n.source == source)
}

/// A level to probe: its name, which item to plant in, where in it, and the note source suffix.
type Level<'a> = (&'a str, &'a dyn Fn(&Value) -> bool, &'a [&'a str], &'a str);

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

/// Add `PROBE: 1` to the object at key path `at` under `node`.
fn insert(node: &mut Value, at: &[&str]) {
    let target = at
        .iter()
        .fold(node, |v, k| v.get_mut(*k).expect("path exists"));
    target
        .as_object_mut()
        .expect("an object")
        .insert(PROBE.into(), Value::from(1));
}

/// Plant the probe at `at` in the first item, depth first, that `pick` accepts.
fn plant(items: &mut [Value], pick: &dyn Fn(&Value) -> bool, at: &[&str]) -> bool {
    for item in items.iter_mut() {
        if pick(item) {
            insert(item, at);
            return true;
        }
        if let Some(children) = item.get_mut("item").and_then(Value::as_array_mut)
            && plant(children, pick, at)
        {
            return true;
        }
    }
    false
}

/// A request the parser writes: a non-blank method.
fn live(item: &Value) -> bool {
    let Some(request) = item.get("request").and_then(Value::as_object) else {
        return false;
    };
    request
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|m| !m.trim().is_empty())
}

fn member_is_object(item: &Value, key: &str) -> bool {
    item.get("request")
        .and_then(|r| r.get(key))
        .is_some_and(Value::is_object)
}

#[test]
fn every_fixture_reports_unknown_keys_at_each_level() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/postman/collections");
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .expect("fixtures")
        .map(|e| e.expect("entry").path())
        .collect();
    paths.sort();
    let is_folder =
        |v: &Value| v.get("request").is_none() && v.get("item").is_some_and(Value::is_array);
    let url = |v: &Value| live(v) && member_is_object(v, "url");
    let body = |v: &Value| live(v) && member_is_object(v, "body");
    let auth = |v: &Value| {
        live(v)
            && member_is_object(v, "auth")
            && matches!(
                v["request"]["auth"]["type"].as_str(),
                Some("basic" | "bearer" | "apikey" | "noauth")
            )
    };
    let levels: [Level; 4] = [
        ("folder", &is_folder, &[], " / zzApinataProbe"),
        (
            "url",
            &url,
            &["request", "url"],
            " / request.url.zzApinataProbe",
        ),
        (
            "body",
            &body,
            &["request", "body"],
            " / request.body.zzApinataProbe",
        ),
        (
            "auth",
            &auth,
            &["request", "auth"],
            " / request.auth.zzApinataProbe",
        ),
    ];
    let mut planted = [0usize; 4];
    let mut checked = 0;
    for path in paths {
        let bytes = fs::read(&path).expect("fixture");
        if parse_postman(&bytes).is_err() {
            continue; // the rejected fixtures have no report
        }
        let mut json: Value = serde_json::from_slice(&bytes).expect("json");
        let doc = if json.get("info").is_none() {
            json.get_mut("collection").expect("envelope")
        } else {
            &mut json
        };
        insert(doc, &[]);
        insert(doc, &["info"]);
        let mut hits = [false; 4];
        if let Some(items) = doc.get_mut("item").and_then(Value::as_array_mut) {
            for (i, (_, pick, at, _)) in levels.iter().enumerate() {
                hits[i] = plant(items, *pick, at);
            }
        }
        let c =
            parse_postman(&serde_json::to_vec(&json).expect("serialise")).expect("still parses");
        let dropped: Vec<&str> = notes(&c)
            .into_iter()
            .filter(|n| n.kind == NoteKind::Dropped)
            .map(|n| n.source.as_str())
            .collect();
        let name = path.display();
        for expected in [
            "collection / zzApinataProbe",
            "collection / info.zzApinataProbe",
        ] {
            assert!(
                dropped.contains(&expected),
                "{name}: no {expected}: {dropped:?}"
            );
        }
        for (i, (level, _, _, suffix)) in levels.iter().enumerate() {
            if hits[i] {
                planted[i] += 1;
                let found = dropped
                    .iter()
                    .any(|s| s.ends_with(suffix) && !s.starts_with("collection / "));
                assert!(
                    found,
                    "{name}: the {level} probe is not reported: {dropped:?}"
                );
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 28, "31 collection fixtures, 3 rejected");
    assert!(
        planted.iter().all(|&n| n > 0),
        "a level was never probed: {planted:?}"
    );
}

#[test]
fn scripts_examples_and_unknown_keys_are_reported_and_metadata_is_not() {
    let c = parse(
        r#", "event": [{"listen": "prerequest", "script": {"exec": ["pm.x()"]}}]"#,
        r#"[
        {"name": "f", "variable": [{"key": "v", "value": "1"}], "item": [
            {"name": "r", "_postman_id": "id", "id": "id",
             "event": [{"listen": "test", "script": {"exec": ["pm.test()"]}}, {"listen": "prerequest", "script": {"exec": [""]}}],
             "response": [{"name": "ok"}],
             "request": {"method": "GET", "url": "u", "proxy": {"host": "p"}, "header": [{"key": "a", "value": "1", "type": "text"}]}}]}]"#,
    );
    assert!(
        has(&c.notes, NoteKind::Converted, "collection / event[0]"),
        "{:?}",
        c.notes
    );
    let ImportedItem::Folder(f) = &c.items[0] else {
        panic!("folder")
    };
    assert!(
        has(&f.notes, NoteKind::Dropped, "f / variable"),
        "{:?}",
        f.notes
    );
    let ImportedItem::Request(r) = &f.items[0] else {
        panic!("request")
    };
    let mut sources: Vec<&str> = r.notes.iter().map(|n| n.source.as_str()).collect();
    sources.sort_unstable();
    sources.dedup();
    assert_eq!(
        sources,
        [
            "f / r / event[0]",
            "f / r / request.proxy",
            "f / r / response"
        ],
        "a blank script, ids and a header's type are not reported"
    );
    assert!(
        r.notes
            .iter()
            .filter(|n| n.source.contains("event["))
            .all(|n| n.kind == NoteKind::Converted),
        "{:?}",
        r.notes
    );
    assert!(
        r.notes
            .iter()
            .filter(|n| !n.source.contains("event["))
            .all(|n| n.kind == NoteKind::Dropped)
    );
}

#[test]
fn an_auth_beside_the_request_and_unknown_auth_members_are_reported() {
    let c = parse(
        r#", "auth": {"type": "bearer", "bearer": [], "basic": []}"#,
        r#"[
        {"name": "r", "auth": {"type": "basic"},
         "event": [{"listen": "test", "script": {"exec": "pm.test()"}}],
         "request": {"method": "GET", "url": "u"}}]"#,
    );
    assert!(
        has(&c.notes, NoteKind::Dropped, "collection / auth.basic"),
        "{:?}",
        c.notes
    );
    assert!(
        !has(&c.notes, NoteKind::Dropped, "collection / auth.bearer"),
        "{:?}",
        c.notes
    );
    let ImportedItem::Request(r) = &c.items[0] else {
        panic!("request")
    };
    let mut sources: Vec<&str> = r.notes.iter().map(|n| n.source.as_str()).collect();
    sources.sort_unstable();
    sources.dedup();
    assert_eq!(sources, ["r / auth", "r / event[0]"], "{:?}", r.notes);
}

/// Events at the three levels become scripts and `Converted` notes at the event's source.
#[test]
fn events_at_every_level_become_scripts_with_converted_notes() {
    let c = parse(
        r#", "event": [
            {"listen": "prerequest", "script": {"exec": ["pm.environment.set('a', 1);"]}},
            {"listen": "test", "script": {"exec": ["x();"]}},
            {"listen": "test", "script": {"exec": "pm.environment.set('b', 2);\nlast();"}}]"#,
        r#"[
        {"name": "f", "event": [{"listen": "prerequest", "script": {"exec": ["pm.variables.set('f', 1);", "go();"]}}],
         "item": [
            {"name": "r",
             "event": [{"listen": "test", "script": {"exec": ["pm.visualizer.set('u');"]}},
                       {"listen": "prerequest", "script": {"exec": [" "]}},
                       {"listen": "other", "script": {"exec": ["ignored();"]}}],
             "request": {"method": "GET", "url": "u"}}]}]"#,
    );
    assert_eq!(c.scripts.pre_request, "bru.setEnvVar('a', 1);");
    assert_eq!(c.scripts.post_response, "bru.setEnvVar('b', 2);\nlast();");
    for (source, text) in [
        ("collection / event[0]", "pre-request script carried"),
        ("collection / event[2]", "post-response script carried"),
    ] {
        let note = c.notes.iter().find(|n| n.source == source).expect(source);
        assert_eq!(note.kind, NoteKind::Converted);
        assert!(note.detail.contains(text), "{}", note.detail);
    }
    assert!(has(&c.notes, NoteKind::Dropped, "collection / event[1]"));
    let ImportedItem::Folder(f) = &c.items[0] else {
        panic!("folder")
    };
    assert_eq!(f.scripts.pre_request, "bru.setVar('f', 1);\ngo();");
    assert_eq!(f.scripts.post_response, "");
    assert!(has(&f.notes, NoteKind::Converted, "f / event[0]"));
    let ImportedItem::Request(r) = &f.items[0] else {
        panic!("request")
    };
    assert_eq!(r.request.scripts.post_response, "pm.visualizer.set('u');");
    assert_eq!(r.request.scripts.pre_request, "");
    let unsupported = r
        .notes
        .iter()
        .find(|n| n.detail.contains("pm.visualizer.set"))
        .expect("an unsupported note");
    assert_eq!(unsupported.kind, NoteKind::Converted);
    assert_eq!(unsupported.source, "f / r / event[0]");
    assert!(unsupported.detail.contains("kept as written"));
    assert!(
        !r.notes
            .iter()
            .chain(&f.notes)
            .chain(&c.notes)
            .any(|n| n.detail.to_lowercase().contains("bruno"))
    );
    assert!(!r.notes.iter().any(|n| n.source == "f / r / event[1]"));
    assert!(has(&r.notes, NoteKind::Dropped, "f / r / event[2]"));
}

/// The sorted note sources of the only request in a collection whose sole item is `request`.
fn request_sources(request: &str) -> Vec<String> {
    let c = parse("", &format!(r#"[{{"name": "r", "request": {request}}}]"#));
    let ImportedItem::Request(r) = &c.items[0] else {
        panic!("request")
    };
    assert!(
        r.notes.iter().all(|n| n.kind == NoteKind::Dropped),
        "{:?}",
        r.notes
    );
    let mut sources: Vec<String> = r.notes.iter().map(|n| n.source.clone()).collect();
    sources.sort_unstable();
    sources
}

#[test]
fn unknown_collection_variable_members_are_reported() {
    let c = parse(
        r#", "variable": [{"key": "a", "value": "1", "type": "string", "disabled": false,
            "id": "x", "description": "the base"}, "stray"]"#,
        "[]",
    );
    let sources: Vec<&str> = c.notes.iter().map(|n| n.source.as_str()).collect();
    assert_eq!(
        sources,
        [
            "collection / variable[0].description",
            "collection / variable[1]"
        ],
        "{:?}",
        c.notes
    );
    assert!(c.notes.iter().all(|n| n.kind == NoteKind::Dropped));
}

#[test]
fn data_under_an_inactive_body_mode_is_reported() {
    let sources = request_sources(
        r#"{"method": "POST", "url": "u", "body": {"mode": "raw", "raw": "x",
            "options": {"raw": {"language": "json"}}, "urlencoded": [],
            "formdata": [{"key": "a", "value": "1"}], "file": {}}}"#,
    );
    assert_eq!(sources, ["r / request.body.formdata"]);
}

#[test]
fn a_body_with_an_unknown_or_missing_mode_is_reported() {
    for mode in [r#""mode": "nope", "#, ""] {
        let sources = request_sources(&format!(
            r#"{{"method": "POST", "url": "u", "body": {{{mode}"raw": "x"}}}}"#
        ));
        assert_eq!(sources, ["r / request.body.mode"], "mode `{mode}`");
    }
    let empty = request_sources(r#"{"method": "POST", "url": "u", "body": {"raw": ""}}"#);
    assert!(empty.is_empty(), "an empty body loses nothing: {empty:?}");
}

#[test]
fn unknown_header_and_query_entry_members_are_reported() {
    let sources = request_sources(
        r#"{"method": "GET",
            "url": {"raw": "u?q=1", "query": [{"key": "q", "value": "1", "disabled": false,
                "description": "d", "type": "text", "equals": true}]},
            "header": [{"key": "H", "name": "H", "value": "1", "type": "text", "src": "s"},
                {"key": "J", "name": "Other", "value": "2"}]}"#,
    );
    assert_eq!(
        sources,
        [
            "r / request.header[0].src",
            "r / request.header[1].name",
            "r / request.url.query[0].equals"
        ]
    );
}

#[test]
fn a_request_behavior_that_is_not_an_object_is_reported() {
    let c = parse(
        "",
        r#"[{"name": "r", "protocolProfileBehavior": [true],
             "request": {"method": "GET", "url": "u"}}]"#,
    );
    let ImportedItem::Request(r) = &c.items[0] else {
        panic!("request")
    };
    assert!(
        has(&r.notes, NoteKind::Dropped, "r / protocolProfileBehavior"),
        "{:?}",
        r.notes
    );
}

#[test]
fn list_members_that_are_not_lists_are_reported() {
    let sources = request_sources(
        r#"{"method": "POST", "header": {"H": "1"},
            "url": {"raw": "u", "query": {"q": "1"}, "variable": "v"},
            "body": {"mode": "urlencoded", "urlencoded": {"a": "1"}}}"#,
    );
    assert_eq!(
        sources,
        [
            "r / request.body.urlencoded",
            "r / request.header",
            "r / request.url.query",
            "r / request.url.variable"
        ]
    );
    let empty =
        request_sources(r#"{"method": "GET", "header": {}, "url": {"raw": "u", "query": null}}"#);
    assert!(empty.is_empty(), "empty members lose nothing: {empty:?}");
}
