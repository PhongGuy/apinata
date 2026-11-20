//! Updating, from the Import dialog, a collection whose only OpenAPI record is the reference
//! app's: the dialog offers the recorded source and the first update adopts the collection.

use crate::import_update::{V1, V2, phase, preview, rig, serve};
use crate::support::*;

use apinata_app::import_link::Phase;
use apinata_core::MemoryStore;
use apinata_core::import::{
    CollectionFormat, CollectionOptions, parse_collections, write_collection,
};
use gpui_kit::TestAppContext;
use std::fs;
use std::path::{Path, PathBuf};

/// Write the v1 import under `parent` with its `extensions` swapped for a reference record.
fn reference_only(parent: &Path, source: &str, group_by: &str) -> PathBuf {
    reference_only_from(V1, parent, source, group_by)
}

/// Write the import of `spec` under `parent` with its `extensions` swapped for a reference record.
fn reference_only_from(spec: &[u8], parent: &Path, source: &str, group_by: &str) -> PathBuf {
    let b = parse_collections(
        Some(CollectionFormat::OpenApi),
        spec,
        &CollectionOptions::default(),
    )
    .unwrap()
    .remove(0);
    let (c, _) =
        write_collection(parent, &b.collection, &b.environments, &MemoryStore::new()).unwrap();
    let file = c.root().join("opencollection.yml");
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
    fs::write(
        &file,
        format!(
            "{kept}extensions:\n  bruno:\n    openapi:\n      - sourceUrl: {source}\n        groupBy: {group_by}\n"
        ),
    )
    .unwrap();
    c.root().to_path_buf()
}

#[gpui_kit::test]
fn the_recorded_file_is_offered_and_its_first_update_adopts(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = reference_only(r.parent.path(), "spec.json", "path");
    fs::write(root.join("spec.json"), V2).unwrap();
    r.open(cx, root.clone());
    let d = r.dialog(cx, "");
    let targets = d.read_with(cx, |d, _| d.targets());
    assert_eq!(targets[0].baseline, Some(true), "{targets:?}");
    r.click(cx, "import-update-target-0");
    let link = d.read_with(cx, |d, cx| d.link(cx));
    assert_eq!(
        link,
        root.join("spec.json").to_string_lossy(),
        "the source is offered"
    );
    r.click(cx, "import-link-import");
    let s = preview(cx, &d);
    let added: Vec<_> = s.added.iter().map(|p| p.as_str().to_owned()).collect();
    assert_eq!(added, ["users/listUsers.yml"], "grouped by path");
    assert_eq!(s.differs.len(), 1);
    assert_eq!(s.differs[0].path.as_str(), "people/getUser.yml");
    assert!(present(cx, r.win, "import-update-section-differs"));
    let counts = d.read_with(cx, |d, _| d.counts_text());
    assert!(counts.contains("1 differs from the spec"), "{counts}");
    let row = |cx: &mut TestAppContext| d.read_with(cx, |d, _| d.differs_rows());
    assert!(row(cx)[0].1.ends_with(", not refreshed"), "{:?}", row(cx));
    r.click(cx, "import-update-take-differs-0");
    assert!(
        row(cx)[0].1.ends_with(", spec version will be taken"),
        "{:?}",
        row(cx)
    );
}

#[gpui_kit::test]
fn two_differing_requests_read_in_the_plural(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = reference_only_from(V2, r.parent.path(), "spec.json", "tags");
    fs::write(root.join("spec.json"), V2).unwrap();
    // Docs edited by hand, which a request's spec key does not depend on.
    let get = root.join("people/getUser.yml");
    let text = fs::read_to_string(&get).unwrap();
    fs::write(&get, text.replace("Fetch a user", "Fetch someone")).unwrap();
    let list = root.join("people/listUsers.yml");
    let text = fs::read_to_string(&list).unwrap();
    fs::write(&list, format!("{text}docs: Lists everyone\n")).unwrap();
    r.open(cx, root);
    let d = r.dialog(cx, "");
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    let s = preview(cx, &d);
    assert_eq!(s.differs.len(), 2, "{s:?}");
    let counts = d.read_with(cx, |d, _| d.counts_text());
    assert!(counts.contains("2 differ from the spec"), "{counts}");
}

#[gpui_kit::test]
fn a_recorded_local_link_is_offered_and_fetched(cx: &mut TestAppContext) {
    let r = rig(cx);
    let server = serve(cx, V2);
    let url = format!("{}/spec.json", server.uri());
    let root = reference_only(r.parent.path(), &url, "tags");
    r.open(cx, root);
    let d = r.dialog(cx, "");
    r.click(cx, "import-update-target-0");
    assert_eq!(d.read_with(cx, |d, cx| d.link(cx)), url);
    r.click(cx, "import-link-import");
    let s = preview(cx, &d);
    let added: Vec<_> = s.added.iter().map(|p| p.as_str().to_owned()).collect();
    assert_eq!(added, ["people/listUsers.yml"]);
}

#[gpui_kit::test]
fn a_missing_recorded_file_fails_naming_it(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = reference_only(r.parent.path(), "gone.json", "tags");
    r.open(cx, root);
    let d = r.dialog(cx, "");
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    wait_for(cx, "the failure", |cx| {
        matches!(phase(cx, &d), Phase::Failed(_))
    });
    let text = d.read_with(cx, |d, _| d.error().unwrap_or_default().to_owned());
    assert!(text.contains("gone.json"), "{text}");
}

#[gpui_kit::test]
fn a_typed_link_is_not_replaced_by_the_recorded_one(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = reference_only(r.parent.path(), "spec.json", "tags");
    r.open(cx, root);
    let d = r.dialog(cx, "http://127.0.0.1:9/mine.json");
    r.click(cx, "import-update-target-0");
    assert_eq!(
        d.read_with(cx, |d, cx| d.link(cx)),
        "http://127.0.0.1:9/mine.json"
    );
}

#[gpui_kit::test]
fn a_folder_target_is_not_offered_the_recorded_source(cx: &mut TestAppContext) {
    use apinata_core::import::SpecBaseline;
    use apinata_core::{Collection, ItemPath};
    let r = rig(cx);
    let root = reference_only(r.parent.path(), "spec.json", "tags");
    // `people` carries a baseline of its own, so it is a scope of its own.
    let mut c = Collection::open(&root).unwrap();
    let people = ItemPath::new("people").unwrap();
    c.file_mut()
        .set_spec_baseline_in(&people, &SpecBaseline::default());
    c.file_mut().save().unwrap();
    r.open(cx, root.clone());
    let d = r.dialog(cx, "");
    r.click(cx, "import-update-target-0");
    d.update(cx, |d, cx| d.select_scope(Some(people.clone()), cx));
    frame(cx, r.win);
    assert_eq!(
        d.read_with(cx, |d, cx| d.link(cx)),
        "",
        "no offer for a folder"
    );
    d.update(cx, |d, cx| d.select_scope(None, cx));
    frame(cx, r.win);
    let offered = root.join("spec.json").to_string_lossy().into_owned();
    assert_eq!(d.read_with(cx, |d, cx| d.link(cx)), offered);
    d.update(cx, |d, cx| d.select_scope(Some(people), cx));
    frame(cx, r.win);
    assert_eq!(
        d.read_with(cx, |d, cx| d.link(cx)),
        "",
        "an untouched offer is withdrawn for a folder"
    );
}
