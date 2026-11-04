//! An unknown key planted at each level of every importable fixture comes back as a note;
//! exempt keys never do.

use apinata_core::import::{
    CollectionFormat, CollectionOptions, ImportBundle, ImportedItem, NoteKind, parse_collections,
};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

const PROBE: &str = "zzApinataProbe";

fn parse(input: &[u8]) -> Vec<ImportBundle> {
    parse_collections(
        Some(CollectionFormat::Insomnia),
        input,
        &CollectionOptions::default(),
    )
    .expect("parses")
}

fn insert(node: &mut Value) {
    if let Some(map) = node.as_object_mut() {
        map.insert(PROBE.into(), Value::from(1));
    }
}

/// Sources of every `Dropped` note: collection, unwritten, items, environments.
fn dropped(bundles: &[ImportBundle]) -> Vec<String> {
    fn walk(items: &[ImportedItem], out: &mut Vec<String>) {
        for item in items {
            let (notes, children) = match item {
                ImportedItem::Folder(f) => (&f.notes, f.items.as_slice()),
                ImportedItem::Request(r) => (&r.notes, &[][..]),
                _ => unreachable!("only folders and requests exist"),
            };
            out.extend(
                notes
                    .iter()
                    .filter(|n| n.kind == NoteKind::Dropped)
                    .map(|n| n.source.clone()),
            );
            walk(children, out);
        }
    }
    let mut out = Vec::new();
    for b in bundles {
        let c = &b.collection;
        out.extend(
            c.notes
                .iter()
                .chain(&c.unwritten)
                .filter(|n| n.kind == NoteKind::Dropped)
                .map(|n| n.source.clone()),
        );
        walk(&c.items, &mut out);
        for env in &b.environments {
            out.extend(
                env.notes
                    .iter()
                    .filter(|n| n.kind == NoteKind::Dropped)
                    .map(|n| n.source.clone()),
            );
        }
    }
    out
}

/// How many notes match `want`: an exact source for collection and environment levels, else a
/// suffix of an item note.
fn count(sources: &[String], want: &str) -> usize {
    let exact = want == PROBE || want.starts_with("collection / ");
    let hit = |s: &&String| {
        if exact {
            s.as_str() == want
        } else {
            s.ends_with(want) && !s.starts_with("collection / ")
        }
    };
    sources.iter().filter(hit).count()
}

fn mapped_auth(auth: &Value) -> bool {
    let kind = auth.get("type").and_then(Value::as_str);
    matches!(kind, Some("basic" | "bearer" | "apikey" | "none"))
        && auth.get("addTo").and_then(Value::as_str) != Some("cookie")
        && auth.get("disabled") != Some(&Value::Bool(true))
}

fn written_request(r: &Value) -> bool {
    let mime = r
        .get("body")
        .and_then(|b| b.get("mimeType"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    r.get("method").is_some()
        && r.get("reflectionApi").is_none()
        && !mime.starts_with("application/graphql")
}

/// Plant the probe in the request's own levels; returns the expected source suffixes.
fn plant_request(r: &mut Value, levels: &[&'static str], want: &mut Vec<&'static str>) {
    insert(r);
    want.push(" / zzApinataProbe");
    for &level in levels {
        let (target, suffix) = match level {
            "meta" => (r.get_mut("meta"), " / meta.zzApinataProbe"),
            "body" => (
                r.get_mut("body").filter(|b| b.get("mimeType").is_some()),
                " / body.zzApinataProbe",
            ),
            "authentication" => (
                r.get_mut("authentication").filter(|a| mapped_auth(a)),
                " / authentication.zzApinataProbe",
            ),
            "settings" => (r.get_mut("settings"), " / settings.zzApinataProbe"),
            _ => (
                r.get_mut("headers").and_then(|h| h.get_mut(0)),
                " / headers[0].zzApinataProbe",
            ),
        };
        if let Some(target) = target.filter(|t| t.is_object()) {
            insert(target);
            want.push(suffix);
        }
    }
}

fn plant_v4(doc: &mut Value) -> Vec<&'static str> {
    let mut want = Vec::new();
    let mut seen = Vec::new();
    for r in doc["resources"].as_array_mut().expect("resources") {
        let kind = r["_type"].as_str().unwrap_or_default().to_owned();
        if seen.contains(&kind) {
            continue;
        }
        match kind.as_str() {
            "workspace" => {
                insert(r);
                want.push("collection / zzApinataProbe");
            }
            "request_group" => {
                insert(r);
                want.push(" / zzApinataProbe");
            }
            "environment" => {
                insert(r);
                want.push(PROBE);
            }
            "request" if written_request(r) => {
                plant_request(r, &["body", "authentication", "headers"], &mut want)
            }
            _ => continue,
        }
        seen.push(kind);
    }
    want
}

fn plant_v5(doc: &mut Value) -> Vec<&'static str> {
    fn walk(items: &mut [Value], want: &mut Vec<&'static str>, done: &mut (bool, bool)) {
        for item in items {
            if item.get("children").is_some()
                || (item.get("method").is_none() && item.get("url").is_none())
            {
                if !done.1 {
                    insert(item);
                    want.push(" / zzApinataProbe");
                    done.1 = true;
                }
                if let Some(children) = item.get_mut("children").and_then(Value::as_array_mut) {
                    walk(children, want, done);
                }
            } else if !done.0 && written_request(item) {
                plant_request(
                    item,
                    &["meta", "body", "authentication", "settings", "headers"],
                    want,
                );
                done.0 = true;
            }
        }
    }
    insert(doc);
    let mut want = vec!["collection / zzApinataProbe"];
    if let Some(items) = doc.get_mut("collection").and_then(Value::as_array_mut) {
        walk(items, &mut want, &mut (false, false));
    }
    if let Some(envs) = doc.get_mut("environments").filter(|e| e.is_object()) {
        insert(envs);
        want.push(PROBE);
        if let Some(sub) = envs.get_mut("subEnvironments").and_then(|s| s.get_mut(0)) {
            insert(sub);
            want.push(PROBE);
        }
    }
    want
}

#[test]
fn every_fixture_reports_a_probe_at_each_level() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/insomnia");
    let mut probed = 0;
    for sub in ["bruno", "kong"] {
        let mut paths: Vec<_> = fs::read_dir(root.join(sub))
            .expect("dir")
            .map(|e| e.expect("entry").path())
            .collect();
        paths.sort();
        for path in paths {
            let name = path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            if name.starts_with("LICENSE")
                || matches!(
                    name.as_str(),
                    "insomnia-malformed.json" | "global-environment.yaml"
                )
            {
                continue;
            }
            let mut doc: Value = yaml_serde::from_slice(&fs::read(&path).expect("read"))
                .expect("json-compatible yaml");
            let want = if doc.get("resources").is_some() {
                plant_v4(&mut doc)
            } else {
                plant_v5(&mut doc)
            };
            let sources = dropped(&parse(&serde_json::to_vec(&doc).expect("json")));
            for suffix in want
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
            {
                let planted = want.iter().filter(|w| **w == suffix).count();
                assert!(
                    count(&sources, suffix) >= planted,
                    "{name}: {suffix} x{planted}: {sources:#?}"
                );
            }
            probed += 1;
        }
    }
    assert_eq!(probed, 15);
}

#[test]
fn exempt_keys_are_not_reported() {
    let v4 = json!({"_type": "export", "__export_format": 4, "__export_date": "d", "__export_source": "s", "resources": [
        {"_id": "wrk_1", "_type": "workspace", "parentId": null, "name": "c", "created": 1, "modified": 2, "description": "", "scope": "collection", "type": "Workspace"},
        {"_id": "fld_1", "_type": "request_group", "parentId": "wrk_1", "name": "F", "metaSortKey": 1, "environment": {}, "environmentPropertyOrder": null, "created": 1, "modified": 2, "description": ""},
        {"_id": "req_1", "_type": "request", "parentId": "fld_1", "name": "R", "method": "GET", "url": "https://x.test", "isPrivate": false, "metaSortKey": -1, "body": {}, "authentication": {}, "parameters": [], "headers": [],
         "settingEncodeUrl": true, "settingStoreCookies": true, "settingSendCookies": true, "settingDisableRenderRequestBody": false, "settingRebuildPath": true, "settingFollowRedirects": "global", "created": 1, "modified": 2},
        {"_id": "env_1", "_type": "environment", "parentId": "wrk_1", "name": "Base", "data": {"a": "1"}, "dataPropertyOrder": {"&": ["a"]}, "color": null, "isPrivate": false, "metaSortKey": 1, "environmentType": "kv", "kvPairData": [{"name": "a", "value": "1"}], "created": 1, "modified": 2}]});
    assert_eq!(
        dropped(&parse(v4.to_string().as_bytes())),
        Vec::<String>::new()
    );
    let v5 = "type: collection.insomnia.rest/5.0\nschema_version: \"5.1\"\nname: c\nmeta: {id: wrk_1, created: 1, modified: 2, isPrivate: false}\ncollection:\n  - {name: R, meta: {id: req_1, created: 1, modified: 2, isPrivate: false, sortKey: -1}, method: GET, url: https://x.test}\nenvironments:\n  name: Base\n  meta: {id: env_1, created: 1, modified: 2, isPrivate: false}\n  color: null\n  data: {a: '1'}\n";
    assert_eq!(dropped(&parse(v5.as_bytes())), Vec::<String>::new());
}
