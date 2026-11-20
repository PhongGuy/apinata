//! Updating a collection whose only OpenAPI record is the reference app's sync entry.

use crate::import_update_support::{FakeBin, bundle, imported, ip, paths, spec};
use apinata_core::import::{
    ChangedField, GroupBy, SpecLocation, TakeSpec, UpdateOptions, apply_update, can_update,
    file_field_hashes, plan_update, recorded_spec,
};
use apinata_core::{Collection, ItemPath};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const REFERENCE: &str = "extensions:\n  bruno:\n    openapi:\n      - sourceUrl: spec.json\n        groupBy: path\n        autoCheck: false\n        autoCheckInterval: 5\n        lastSyncDate: 2026-01-02T03:04:05.000Z\n        specHash: abc123\n";

/// Swap the root file's `extensions` block for `block`, as a collection synced only by the
/// reference app would have it.
fn with_extensions(root: &Path, block: &str) {
    let file = root.join("opencollection.yml");
    let text = fs::read_to_string(&file).unwrap();
    let mut kept = String::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.starts_with("extensions:") {
            skipping = true;
            continue;
        }
        if skipping && (line.starts_with(' ') || line.is_empty()) {
            continue;
        }
        skipping = false;
        kept.push_str(line);
        kept.push('\n');
    }
    fs::write(&file, format!("{kept}{block}")).unwrap();
}

/// A `fixture` import that carries only the reference app's record.
fn reference_only(parent: &Path, fixture: &str) -> Collection {
    let c = imported(parent, fixture);
    with_extensions(c.root(), REFERENCE);
    Collection::open(c.root()).unwrap()
}

fn reference_block(root: &Path) -> String {
    let text = fs::read_to_string(root.join("opencollection.yml")).unwrap();
    let start = text.find("  bruno:\n").expect("reference block");
    let rest = &text[start..];
    let end = rest
        .find("  apinata:")
        .or_else(|| rest.find("\n\n").map(|n| n + 1))
        .unwrap_or(rest.len());
    rest[..end].to_owned()
}

#[test]
fn the_reference_source_resolves_against_the_root_with_its_grouping() {
    let dir = TempDir::new().unwrap();
    let c = reference_only(dir.path(), "update-v1.json");
    let recorded = recorded_spec(&c).expect("recorded");
    assert_eq!(recorded.group_by, GroupBy::Path);
    match recorded.location {
        SpecLocation::File(path) => assert_eq!(path, c.root().join("spec.json")),
        other => panic!("expected a file, got {other:?}"),
    }
    assert!(can_update(&c, &ItemPath::root()));
}

#[test]
fn a_link_source_is_a_link_and_the_first_usable_entry_wins_with_tags_by_default() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    with_extensions(
        c.root(),
        "extensions:\n  bruno:\n    openapi:\n      - sourceUrl: \"\"\n      - sourceUrl: http://127.0.0.1:9/spec.json\n      - sourceUrl: other.json\n",
    );
    let c = Collection::open(c.root()).unwrap();
    let recorded = recorded_spec(&c).expect("recorded");
    assert_eq!(recorded.group_by, GroupBy::Tags);
    match recorded.location {
        SpecLocation::Link(url) => assert_eq!(url, "http://127.0.0.1:9/spec.json"),
        other => panic!("expected a link, got {other:?}"),
    }
}

#[test]
fn our_own_record_wins_over_the_reference_one() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    let ours = fs::read_to_string(c.root().join("opencollection.yml")).unwrap();
    let ours = &ours[ours.find("extensions:").unwrap()..];
    // The import may already write a `bruno` section; the record goes first inside it.
    let block = if ours.contains("\n  bruno:\n") {
        ours.replacen(
            "\n  bruno:\n",
            "\n  bruno:\n    openapi:\n      - sourceUrl: spec.json\n",
            1,
        )
    } else {
        ours.replacen(
            "extensions:\n",
            "extensions:\n  bruno:\n    openapi:\n      - sourceUrl: spec.json\n",
            1,
        )
    };
    with_extensions(c.root(), &block);
    let mut c = Collection::open(c.root()).unwrap();
    assert!(c.file().spec_baseline().is_some());
    // Our record holds no source, so the reference one is still offered.
    let offered = recorded_spec(&c).expect("the reference source");
    assert_eq!(
        offered.location,
        SpecLocation::File(c.root().join("spec.json"))
    );
    let s = apply_update(
        &mut c,
        &bundle(&spec("update-v2.json")),
        &UpdateOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(
        paths(&s.changed.iter().map(|c| c.path.clone()).collect::<Vec<_>>()),
        ["people/getUser.yml"]
    );
    assert!(s.differs.is_empty());
}

#[test]
fn a_collection_with_neither_record_is_not_updatable() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    with_extensions(c.root(), "");
    let c = Collection::open(c.root()).unwrap();
    assert!(recorded_spec(&c).is_none());
    assert!(!can_update(&c, &ItemPath::root()));
}

#[test]
fn the_first_update_reports_a_differing_request_and_leaves_it_alone() {
    let dir = TempDir::new().unwrap();
    let mut c = reference_only(dir.path(), "update-v1.json");
    let before = fs::read(c.root().join("people/getUser.yml")).unwrap();
    let v2 = bundle(&spec("update-v2.json"));
    let planned = plan_update(&c, &v2, &UpdateOptions::default()).unwrap();
    let s = apply_update(&mut c, &v2, &UpdateOptions::default(), None).unwrap();
    assert_eq!(format!("{planned:?}"), format!("{s:?}"));
    assert!(s.changed.is_empty(), "{s:?}");
    assert!(s.edited.is_empty(), "differing is not edited: {s:?}");
    assert_eq!(s.differs.len(), 1);
    assert_eq!(s.differs[0].path.as_str(), "people/getUser.yml");
    assert!(s.differs[0].fields.contains(&ChangedField::Target));
    assert!(s.differs[0].fields.contains(&ChangedField::Docs));
    assert_eq!(
        fs::read(c.root().join("people/getUser.yml")).unwrap(),
        before
    );
    let baseline = Collection::open(c.root())
        .unwrap()
        .file()
        .spec_baseline()
        .expect("our record is written");
    let get = baseline.get("GET /users/{}").expect("adopted");
    assert!(
        get.get(ChangedField::Target).is_none(),
        "no hash for a differing field"
    );
    assert!(get.get(ChangedField::Docs).is_none());
    assert!(
        get.get(ChangedField::Method).is_some(),
        "equal fields are recorded"
    );
    assert_eq!(baseline.file("GET /users/{}"), Some("people/getUser.yml"));
}

#[test]
fn the_first_update_adds_new_operations_and_prunes_nothing() {
    let dir = TempDir::new().unwrap();
    let mut c = reference_only(dir.path(), "update-v1.json");
    let mut o = UpdateOptions::default();
    o.prune = true;
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &bundle(&spec("update-v2.json")), &o, Some(&bin)).unwrap();
    assert_eq!(paths(&s.added), ["people/listUsers.yml"]);
    assert!(s.pruned.is_empty() && s.orphaned.is_empty(), "{s:?}");
    assert_eq!(bin.taken(), 0);
    assert!(c.root().join("legacy/removeLegacy.yml").exists());
    assert!(paths(&s.kept).contains(&"people/createUser.yml"));
}

#[test]
fn the_second_update_is_a_normal_three_way_merge() {
    let dir = TempDir::new().unwrap();
    let mut c = reference_only(dir.path(), "update-v1.json");
    // A local URL edit before adoption stays the user's; the docs are still as generated.
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_url("{{baseUrl}}/users/:id?mine=1");
    f.save().unwrap();
    let first = apply_update(
        &mut c,
        &bundle(&spec("update-v1.json")),
        &UpdateOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(first.differs.len(), 1, "{first:?}");
    assert_eq!(first.differs[0].fields, [ChangedField::Target]);
    let mut again = Collection::open(c.root()).unwrap();
    let second = apply_update(
        &mut again,
        &bundle(&spec("update-v2.json")),
        &UpdateOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(second.changed.len(), 1, "{second:?}");
    assert_eq!(second.changed[0].fields, [ChangedField::Docs]);
    assert_eq!(second.differs.len(), 1);
    assert_eq!(second.differs[0].fields, [ChangedField::Target]);
    let get = Collection::open(c.root()).unwrap();
    let get = get.request(&ip("people/getUser.yml")).unwrap();
    assert_eq!(get.docs(), "Fetch a user");
    assert!(get.url().contains("mine=1") && !get.url().contains("include"));
}

#[test]
fn the_reference_keys_survive_the_update_byte_for_byte() {
    let dir = TempDir::new().unwrap();
    let mut c = reference_only(dir.path(), "update-v1.json");
    let before = reference_block(c.root());
    apply_update(
        &mut c,
        &bundle(&spec("update-v2.json")),
        &UpdateOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        Collection::open(c.root())
            .unwrap()
            .file()
            .spec_baseline()
            .is_some()
    );
    assert_eq!(reference_block(c.root()), before);
}

#[test]
fn the_reference_source_is_still_offered_after_the_first_update() {
    let dir = TempDir::new().unwrap();
    let mut c = reference_only(dir.path(), "update-v1.json");
    apply_update(
        &mut c,
        &bundle(&spec("update-v2.json")),
        &UpdateOptions::default(),
        None,
    )
    .unwrap();
    let c = Collection::open(c.root()).unwrap();
    assert!(c.file().spec_baseline().is_some());
    let recorded = recorded_spec(&c).expect("still offered");
    assert_eq!(
        recorded.location,
        SpecLocation::File(c.root().join("spec.json"))
    );
    assert_eq!(recorded.group_by, GroupBy::Path);
}

#[test]
fn a_folder_scope_does_not_adopt_from_the_root_record() {
    let dir = TempDir::new().unwrap();
    let c = reference_only(dir.path(), "update-v1.json");
    assert!(!can_update(&c, &ip("people")));
}

/// The v1 spec with an example on getUser's response, which the v1 import does not have.
fn v1_with_example() -> apinata_core::import::ImportBundle {
    let text = String::from_utf8(spec("update-v1.json")).unwrap().replacen(
        r#""200":{"description":"ok"}"#,
        r#""200":{"description":"ok","content":{"application/json":{"example":{"id":"1"}}}}"#,
        1,
    );
    bundle(text.as_bytes())
}

#[test]
fn a_request_without_examples_is_not_reported_as_differing_and_gets_them_next_time() {
    let dir = TempDir::new().unwrap();
    let mut c = reference_only(dir.path(), "update-v1.json");
    assert!(
        c.request(&ip("people/getUser.yml"))
            .unwrap()
            .examples()
            .is_empty()
    );
    let first = apply_update(&mut c, &v1_with_example(), &UpdateOptions::default(), None).unwrap();
    assert!(first.differs.is_empty(), "{first:?}");
    assert!(paths(&first.kept).contains(&"people/getUser.yml"));
    assert!(
        c.request(&ip("people/getUser.yml"))
            .unwrap()
            .examples()
            .is_empty()
    );
    let mut again = Collection::open(c.root()).unwrap();
    let second = apply_update(
        &mut again,
        &v1_with_example(),
        &UpdateOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(second.changed.len(), 1, "{second:?}");
    assert_eq!(second.changed[0].fields, [ChangedField::Examples]);
}

#[test]
fn a_differing_request_named_on_the_first_update_takes_the_spec_and_then_updates_clean() {
    let dir = TempDir::new().unwrap();
    let mut c = reference_only(dir.path(), "update-v1.json");
    let v2 = bundle(&spec("update-v2.json"));
    let mut o = UpdateOptions::default();
    o.take_spec = vec![TakeSpec::request(ip("people/getUser.yml"))];
    let s = apply_update(&mut c, &v2, &o, None).unwrap();
    assert!(s.differs.is_empty(), "{s:?}");
    assert_eq!(paths(&[s.changed[0].path.clone()]), ["people/getUser.yml"]);
    let mut reopened = Collection::open(c.root()).unwrap();
    let get = reopened.request(&ip("people/getUser.yml")).unwrap();
    assert!(get.url().contains("include"));
    let recorded = reopened.file().spec_baseline().expect("our record");
    assert_eq!(
        recorded.get("GET /users/{}").unwrap(),
        &file_field_hashes(get)
    );
    let next = apply_update(&mut reopened, &v2, &UpdateOptions::default(), None).unwrap();
    assert!(
        next.differs.is_empty() && next.changed.is_empty(),
        "{next:?}"
    );
}

/// Make `dir` read-only so a save inside it fails, run `f`, then make it writable again.
#[cfg(unix)]
fn with_read_only<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
    let out = f();
    fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
    out
}

#[cfg(unix)]
#[test]
fn a_failed_take_on_the_first_update_records_the_request_as_an_untaken_update_would() {
    let dir = TempDir::new().unwrap();
    let mut c = reference_only(dir.path(), "update-v1.json");
    let v2 = bundle(&spec("update-v2.json"));
    let mut o = UpdateOptions::default();
    o.take_spec = vec![TakeSpec::request(ip("people/getUser.yml"))];
    let people = c.root().join("people");
    let s = with_read_only(&people, || apply_update(&mut c, &v2, &o, None).unwrap());
    assert!(
        s.notes.iter().any(|n| n.detail.contains("not updated")),
        "{s:?}"
    );
    let reopened = Collection::open(c.root()).unwrap();
    let next = plan_update(&reopened, &v2, &UpdateOptions::default()).unwrap();
    let differs: Vec<_> = next.differs.iter().map(|d| d.path.clone()).collect();
    assert_eq!(paths(&differs), ["people/getUser.yml"], "{next:?}");
    assert!(
        !next
            .notes
            .iter()
            .any(|n| n.detail.contains("not generated by the spec")),
        "{next:?}"
    );
}
