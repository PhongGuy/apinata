//! Every vendored Hoppscotch fixture imported, written and snapshotted, and detected.

use apinata_core::import::{
    CollectionFormat, CollectionOptions, DocumentError, EnvironmentFormat, FormatError,
    ImportBundle, ImportNote, ImportReport, ImportedCollection, ImportedItem, NoteKind,
    add_environment, parse_collections, parse_environments, write_collection,
};
use apinata_core::{Collection, MemoryStore};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const REFUSED_COLLECTIONS: &[(&str, &str)] = &[
    ("malformed-coll-2.json", "invalid"),
    ("malformed-coll.json", "parse"),
    ("notjson-coll.txt", "parse"),
];
const REFUSED_ENVIRONMENTS: &[(&str, &str)] = &[
    ("env-flag-envs.json", "invalid"),
    ("malformed-envs.json", "invalid"),
];

fn fixtures(kind: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hoppscotch")
        .join(kind);
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("fixture dir")
        .map(|e| e.expect("entry").path())
        .collect();
    paths.sort();
    paths
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .expect("stem")
        .to_string_lossy()
        .into_owned()
}

fn collections(input: &[u8]) -> Result<Vec<ImportBundle>, FormatError> {
    parse_collections(
        Some(CollectionFormat::Hoppscotch),
        input,
        &CollectionOptions::default(),
    )
}

fn refusal(e: &FormatError) -> &'static str {
    match e {
        FormatError::Hoppscotch(DocumentError::Parse { .. }) => "parse",
        FormatError::Hoppscotch(DocumentError::Invalid { .. }) => "invalid",
        _ => "other",
    }
}

/// Every file under `root` by its `/`-separated relative path.
fn files(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .expect("under root")
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.insert(rel, fs::read_to_string(&path).expect("utf-8"));
        }
    }
    out
}

fn render(root: &Path, report: &ImportReport) -> String {
    let mut out = String::new();
    for (path, text) in files(root) {
        out.push_str(&format!("== {path}\n{text}"));
        if !text.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str("== report\n");
    for n in &report.notes {
        out.push_str(&format!(
            "{:?} {}: {} ({})\n",
            n.kind,
            n.source,
            n.detail,
            n.file.as_deref().unwrap_or("-")
        ));
    }
    for (env, var) in &report.secrets_to_enter {
        out.push_str(&format!("secret to enter: {env} / {var}\n"));
    }
    out
}

/// A written file with its names left out (Decision 11): `name:` lines, and the query
/// parameter names of a `url:` line, keeping each value after `=`.
fn values_only(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let key = line.trim_start().trim_start_matches("- ");
        if key.starts_with("name:") {
            continue;
        }
        match line.split_once('?').filter(|_| key.starts_with("url:")) {
            Some((base, query)) => {
                out.push_str(base);
                for pair in query.split('&') {
                    out.push(' ');
                    out.push_str(pair.split_once('=').map_or("", |(_, value)| value));
                }
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// Whether `text` still holds a `<<name>>` template.
fn has_angle_template(text: &str) -> bool {
    text.match_indices("<<").any(|(i, _)| {
        let rest = &text[i + 2..];
        rest.find(">>")
            .is_some_and(|end| end > 0 && !rest[..end].contains(['<', '>', '\n']))
    })
}

#[test]
fn every_collection_fixture_imports_and_matches_its_snapshot() {
    let mut imported = 0;
    for path in fixtures("collections") {
        let name = file_name(&path);
        let result = collections(&fs::read(&path).expect("read"));
        if let Some((_, want)) = REFUSED_COLLECTIONS.iter().find(|(f, _)| *f == name) {
            let e = result.expect_err(&name);
            assert_eq!(refusal(&e), *want, "{name}: {e}");
            continue;
        }
        let bundles = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        let (mut text, mut values) = (String::new(), String::new());
        for (i, bundle) in bundles.iter().enumerate() {
            let dir = TempDir::new().expect("tempdir");
            let (written, report) = write_collection(
                dir.path(),
                &bundle.collection,
                &bundle.environments,
                &MemoryStore::new(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            text.push_str(&format!("### collection {i}: {}\n", bundle.collection.name));
            text.push_str(&render(written.root(), &report));
            values.extend(files(written.root()).values().map(|t| values_only(t)));
        }
        assert!(
            !has_angle_template(&values),
            "{name} keeps a <<template>> in a value:\n{values}"
        );
        if name == "request-vars-coll.json" {
            for kept in [
                "?<<queryParamKey>>={{queryParamValue}}",
                "<<customHeaderKey>>",
            ] {
                assert!(
                    text.contains(kept),
                    "{name}: the name {kept} is not kept verbatim:\n{text}"
                );
            }
        }
        insta::assert_snapshot!(format!("hoppscotch_{}", stem(&path)), text);
        imported += 1;
    }
    assert_eq!(imported, 38, "41 collection fixtures, 3 refused");
}

#[test]
fn every_environment_fixture_imports_and_matches_its_snapshot() {
    let mut imported = 0;
    for path in fixtures("environments") {
        let name = file_name(&path);
        let result = parse_environments(
            Some(EnvironmentFormat::Hoppscotch),
            &fs::read(&path).expect("read"),
        );
        if let Some((_, want)) = REFUSED_ENVIRONMENTS.iter().find(|(f, _)| *f == name) {
            let e = result.expect_err(&name);
            assert_eq!(refusal(&e), *want, "{name}: {e}");
            continue;
        }
        let envs = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        let dir = TempDir::new().expect("tempdir");
        let mut collection = Collection::create(&dir.path().join("c"), "c").expect("create");
        let mut report = ImportReport::default();
        for env in &envs {
            let (_, r) = add_environment(&mut collection, env, &MemoryStore::new())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            report.notes.extend(r.notes);
            report.secrets_to_enter.extend(r.secrets_to_enter);
        }
        let root = collection.root().join("environments");
        let text = render(&root, &report);
        let values: String = files(&root).values().map(|t| values_only(t)).collect();
        assert!(
            !has_angle_template(&values),
            "{name} keeps a <<template>> in a value:\n{values}"
        );
        insta::assert_snapshot!(format!("hoppscotch_env_{}", stem(&path)), text);
        imported += 1;
    }
    assert_eq!(imported, 15, "17 environment fixtures, 2 refused");
}

#[test]
fn detection_reaches_this_importer() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hoppscotch");
    let input = fs::read(dir.join("collections/coll-v2-req-v3.json")).expect("read");
    let auto = parse_collections(None, &input, &CollectionOptions::default()).expect("detected");
    assert_eq!(
        format!("{auto:?}"),
        format!("{:?}", collections(&input).expect("named"))
    );
    let env = fs::read(dir.join("environments/env-v2.json")).expect("read");
    assert_eq!(
        parse_environments(None, &env).expect("detected")[0].name,
        "env-v2"
    );
}

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

fn insert(node: &mut Value) {
    node.as_object_mut()
        .expect("object")
        .insert("zzApinataProbe".into(), json!(1));
}

fn plant_folder(collection: &mut Value) -> bool {
    collection
        .get_mut("folders")
        .and_then(Value::as_array_mut)
        .and_then(|f| f.first_mut())
        .map(insert)
        .is_some()
}

/// Whether an auth object maps with its members read (Decision 12); an unsupported or
/// `authActive: false` auth reports no member, so it holds no probe.
fn mapped_auth(auth: &Value) -> bool {
    let kind = auth.get("authType").and_then(Value::as_str);
    matches!(
        kind,
        Some("none" | "inherit" | "basic" | "bearer" | "api-key")
    ) && auth.get("authActive") != Some(&Value::Bool(false))
}

/// Plant the probe in the first REST request, depth first, or in its `member` object.
fn plant_request(collection: &mut Value, member: Option<&str>) -> bool {
    if let Some(requests) = collection.get_mut("requests").and_then(Value::as_array_mut) {
        for r in requests.iter_mut() {
            let rest = r.get("v").is_none_or(Value::is_string) && r.get("query").is_none();
            match member {
                None if rest => {
                    insert(r);
                    return true;
                }
                Some(key)
                    if rest
                        && r.get(key).is_some_and(|m| {
                            m.is_object() && (key != "auth" || mapped_auth(m))
                        }) =>
                {
                    insert(&mut r[key]);
                    return true;
                }
                _ => {}
            }
        }
    }
    collection
        .get_mut("folders")
        .and_then(Value::as_array_mut)
        .is_some_and(|fs| fs.iter_mut().any(|f| plant_request(f, member)))
}

#[test]
fn unknown_keys_are_reported_at_every_level() {
    type Plant = fn(&mut Value) -> bool;
    let levels: [(&str, Plant, &str); 5] = [
        (
            "collection",
            |c| {
                insert(c);
                true
            },
            "collection / zzApinataProbe",
        ),
        ("folder", plant_folder, " / zzApinataProbe"),
        ("request", |c| plant_request(c, None), " / zzApinataProbe"),
        (
            "auth",
            |c| plant_request(c, Some("auth")),
            " / auth.zzApinataProbe",
        ),
        (
            "body",
            |c| plant_request(c, Some("body")),
            " / body.zzApinataProbe",
        ),
    ];
    let mut planted = [0usize; 5];
    for path in fixtures("collections") {
        let name = file_name(&path);
        if REFUSED_COLLECTIONS.iter().any(|(f, _)| *f == name) {
            continue;
        }
        let original: Value =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        for (i, (level, plant, suffix)) in levels.iter().enumerate() {
            let mut doc = original.clone();
            let first = if doc.is_array() {
                &mut doc[0]
            } else {
                &mut doc
            };
            if !plant(first) {
                continue;
            }
            let bundles = collections(&serde_json::to_vec(&doc).expect("serialise"))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let dropped: Vec<String> = bundles
                .iter()
                .flat_map(|b| notes(&b.collection))
                .filter(|n| n.kind == NoteKind::Dropped)
                .map(|n| n.source.clone())
                .collect();
            let found = dropped.iter().any(|s| {
                s.ends_with(suffix) && (*level == "collection" || !s.starts_with("collection / "))
            });
            assert!(
                found,
                "{name}: the {level} probe is not reported: {dropped:?}"
            );
            planted[i] += 1;
        }
    }
    assert!(
        planted.iter().all(|&n| n > 0),
        "a level was never probed: {planted:?}"
    );
}

/// A folder chain `depth` folders below the root collection.
fn nested(depth: usize) -> Value {
    let mut node = json!({"v": 12, "name": format!("f{depth}"), "folders": [], "requests": []});
    for d in (0..depth).rev() {
        node = json!({"v": 12, "name": format!("f{d}"), "folders": [node], "requests": []});
    }
    node
}

#[test]
fn deep_folders_import_to_the_depth_limit_and_fail_cleanly_past_it() {
    let bundles =
        collections(&serde_json::to_vec(&nested(60)).expect("serialise")).expect("parses");
    let (mut depth, mut items) = (0, &bundles[0].collection.items);
    while let Some(ImportedItem::Folder(f)) = items.first() {
        depth += 1;
        items = &f.items;
    }
    assert_eq!(depth, 60);
    let e = collections(&serde_json::to_vec(&nested(70)).expect("serialise")).unwrap_err();
    assert!(
        matches!(e, FormatError::Hoppscotch(DocumentError::Parse { .. })),
        "{e:?}"
    );
}

#[test]
fn a_hundred_thousand_requests_import() {
    let requests: Vec<Value> = (0..100_000)
        .map(|i| json!({"v": "17", "name": format!("r{i}"), "method": "GET", "endpoint": "u"}))
        .collect();
    let input =
        serde_json::to_vec(&json!({"v": 12, "name": "big", "folders": [], "requests": requests}))
            .expect("serialise");
    let started = std::time::Instant::now();
    let bundles = collections(&input).expect("parses");
    assert_eq!(bundles[0].collection.items.len(), 100_000);
    assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
}

#[test]
fn no_note_or_error_names_another_tool() {
    let mut texts: Vec<String> = Vec::new();
    let take =
        |result: Result<Vec<ImportBundle>, FormatError>, texts: &mut Vec<String>| match result {
            Ok(bundles) => texts.extend(
                bundles
                    .iter()
                    .flat_map(|b| notes(&b.collection))
                    .map(|n| n.detail.clone()),
            ),
            Err(e) => texts.push(e.to_string()),
        };
    for path in fixtures("collections") {
        take(collections(&fs::read(&path).expect("read")), &mut texts);
    }
    let odd = br#"{"v":13,"name":"c","folders":[],"requests":[{"v":10,"name":"q","url":"u","query":"{a}"},{"v":"x","name":"x"}]}"#;
    for input in [&b"[]"[..], b"{}", b"{", odd] {
        take(collections(input), &mut texts);
    }
    for path in fixtures("environments") {
        match parse_environments(
            Some(EnvironmentFormat::Hoppscotch),
            &fs::read(&path).expect("read"),
        ) {
            Ok(envs) => texts.extend(
                envs.iter()
                    .flat_map(|e| e.notes.iter())
                    .map(|n| n.detail.clone()),
            ),
            Err(e) => texts.push(e.to_string()),
        }
    }
    assert!(texts.len() > 50, "only {} texts", texts.len());
    let named: Vec<&String> = texts
        .iter()
        .filter(|t| t.to_lowercase().contains("bruno"))
        .collect();
    assert!(named.is_empty(), "{named:?}");
}
