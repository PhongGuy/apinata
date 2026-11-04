//! Updating a spec imported into a subfolder from the Import dialog: the dialog lists the root
//! and every folder with a baseline, and only the chosen one is planned and written.

use crate::import_update::{Rig, V1, V2, phase, preview, rig, serve};
use crate::support::*;

use apinata_app::import_link::{ImportLinkDialog, Phase};
use apinata_core::import::{
    CollectionFormat, CollectionOptions, ImportedItem, add_request, parse_collections,
    record_spec_baseline, write_collection,
};
use apinata_core::{Collection, ItemPath, MemoryStore};
use gpui_kit::{Entity, TestAppContext};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Places the v1 spec's requests under a new folder `name` and records that folder's baseline.
pub(crate) fn place(c: &mut Collection, name: &str) {
    fn walk(
        c: &mut Collection,
        parent: &ItemPath,
        items: &[ImportedItem],
        out: &mut Vec<ItemPath>,
    ) {
        for item in items {
            match item {
                ImportedItem::Request(r) => {
                    out.push(add_request(c, parent, &r.name, &r.request).unwrap())
                }
                ImportedItem::Folder(f) => {
                    let sub = c.create_folder(parent, &f.name).unwrap();
                    walk(c, &sub, &f.items, out);
                }
                _ => {}
            }
        }
    }
    let bundle = parse_collections(
        Some(CollectionFormat::OpenApi),
        V1,
        &CollectionOptions::default(),
    )
    .unwrap()
    .remove(0);
    let folder = c.create_folder(&ItemPath::root(), name).unwrap();
    let mut placed = Vec::new();
    walk(c, &folder, &bundle.collection.items, &mut placed);
    record_spec_baseline(c, &folder, &placed).unwrap();
}

/// The v1 spec imported at the root and again into each of the folders `names`.
fn root_and_folders(parent: &Path, names: &[&str]) -> PathBuf {
    let b = parse_collections(
        Some(CollectionFormat::OpenApi),
        V1,
        &CollectionOptions::default(),
    )
    .unwrap()
    .remove(0);
    let (mut c, _) =
        write_collection(parent, &b.collection, &b.environments, &MemoryStore::new()).unwrap();
    for n in names {
        place(&mut c, n);
    }
    c.root().to_path_buf()
}

/// A collection with the v1 spec imported into each of the folders `names`; no root baseline.
fn folders_only(parent: &Path, names: &[&str]) -> PathBuf {
    let mut c = Collection::create(&parent.join("shop"), "shop").unwrap();
    for n in names {
        place(&mut c, n);
    }
    c.root().to_path_buf()
}

fn bytes_under(root: &Path, dir: &str) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.join(dir)];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().display().to_string(),
                    fs::read(&p).unwrap(),
                );
            }
        }
    }
    out
}

fn scopes(cx: &mut TestAppContext, d: &Entity<ImportLinkDialog>) -> Vec<Option<String>> {
    d.read_with(cx, |d, _| {
        d.scopes()
            .into_iter()
            .map(|f| f.map(|p| p.to_string()))
            .collect()
    })
}

fn chosen(cx: &mut TestAppContext, d: &Entity<ImportLinkDialog>) -> Option<Option<String>> {
    d.read_with(cx, |d, _| d.folder().map(|f| f.map(|p| p.to_string())))
}

fn paths(items: &[ItemPath]) -> Vec<String> {
    items.iter().map(|p| p.to_string()).collect()
}

#[gpui_kit::test]
fn the_root_and_every_baseline_folder_are_listed(cx: &mut TestAppContext) {
    let r: Rig = rig(cx);
    let root = root_and_folders(r.parent.path(), &["a", "b"]);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    assert_eq!(
        scopes(cx, &d),
        [None, Some("a".to_owned()), Some("b".to_owned())]
    );
    for n in 0..3 {
        let id: &'static str = Box::leak(format!("import-update-folder-{n}").into_boxed_str());
        assert!(present(cx, r.win, id), "{id} is visible");
    }
    assert_eq!(chosen(cx, &d), Some(None), "the root is the default");
}

#[gpui_kit::test]
fn a_single_baseline_folder_is_preselected(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = folders_only(r.parent.path(), &["a"]);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    assert_eq!(scopes(cx, &d), [Some("a".to_owned())]);
    assert_eq!(chosen(cx, &d), Some(Some("a".to_owned())));
}

#[gpui_kit::test]
fn several_folders_and_no_root_wait_for_a_choice(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = folders_only(r.parent.path(), &["a", "b"]);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    assert_eq!(chosen(cx, &d), None);
    let why = d.read_with(cx, |d, cx| d.primary_blocked(cx));
    assert!(why.is_some_and(|w| w.contains("folder")));
}

#[gpui_kit::test]
fn the_active_tab_folder_is_the_default(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = root_and_folders(r.parent.path(), &["a", "b"]);
    r.open(cx, root.clone());
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    open_tab(cx, &ws, &root, "b/people/getUser.yml");
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    assert_eq!(chosen(cx, &d), Some(Some("b".to_owned())));
}

#[gpui_kit::test]
fn a_user_choice_is_never_overridden(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = root_and_folders(r.parent.path(), &["a", "b"]);
    r.open(cx, root.clone());
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    open_tab(cx, &ws, &root, "b/people/getUser.yml");
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-folder-1");
    assert_eq!(chosen(cx, &d), Some(Some("a".to_owned())));
    // A late answer for the same collection re-runs the preselect; it must leave the choice.
    d.update(cx, |d, cx| d.relearn_for_test(cx));
    cx.run_until_parked();
    assert_eq!(chosen(cx, &d), Some(Some("a".to_owned())));
}

#[gpui_kit::test]
fn picking_a_folder_previews_and_applies_only_that_folder(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = folders_only(r.parent.path(), &["a", "b"]);
    r.open(cx, root.clone());
    let b_before = bytes_under(&root, "b");
    let a_before = bytes_under(&root, "a");
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-folder-0");
    r.click(cx, "import-link-import");
    let s = preview(cx, &d);
    assert_eq!(paths(&s.added), ["a/people/listUsers.yml"]);
    assert!(s.changed.iter().all(|c| c.path.as_str().starts_with("a/")));
    assert_eq!(
        bytes_under(&root, "a"),
        a_before,
        "a preview writes nothing"
    );
    let label = d.read_with(cx, |d, _| d.scope_label());
    assert_eq!(label.as_deref(), Some("Updating shop / Folder a"));
    r.click(cx, "import-update-apply");
    wait_for(cx, "the result", |cx| {
        matches!(phase(cx, &d), Phase::Done(_))
    });
    assert_eq!(bytes_under(&root, "b"), b_before, "b is untouched");
    assert_ne!(bytes_under(&root, "a"), a_before, "a was updated");
    assert!(root.join("a/people/listUsers.yml").exists());
    assert!(!root.join("b/people/listUsers.yml").exists());
    let label = d.read_with(cx, |d, _| d.scope_label());
    assert_eq!(label.as_deref(), Some("Updated shop / Folder a"));
}

#[gpui_kit::test]
fn changing_the_folder_in_the_preview_plans_again(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = folders_only(r.parent.path(), &["a", "b"]);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-folder-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-folder-1");
    wait_for(cx, "the b preview", |cx| match phase(cx, &d) {
        Phase::Preview { summary, .. } => {
            summary.0.added.iter().all(|p| p.as_str().starts_with("b/"))
        }
        _ => false,
    });
    assert_eq!(
        d.read_with(cx, |d, _| d.scope_label()).as_deref(),
        Some("Updating shop / Folder b")
    );
}

#[gpui_kit::test]
fn the_root_scope_row_reads_whole_collection(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    assert_eq!(
        d.read_with(cx, |d, _| d.scope_name(&None)),
        "Whole collection"
    );
    r.click(cx, "import-link-import");
    preview(cx, &d);
    let label = d.read_with(cx, |d, _| d.scope_label()).unwrap();
    assert!(
        label.starts_with("Updating ") && !label.contains("Folder"),
        "{label}"
    );
}

#[gpui_kit::test]
fn only_the_chosen_collection_carries_the_marker(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let other = tempfile::tempdir().unwrap();
    r.open(cx, folders_only(other.path(), &["a"]));
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let marks =
        |cx: &mut TestAppContext| d.read_with(cx, |d, _| (d.target_marker(0), d.target_marker(1)));
    let first = d
        .read_with(cx, |d, _| d.selected())
        .expect("a default choice");
    let want = |n: usize| (n == 0, n == 1);
    assert_eq!(marks(cx), want(first), "before any click");
    let second = 1 - first;
    r.click(
        cx,
        ["import-update-target-0", "import-update-target-1"][second],
    );
    assert_eq!(d.read_with(cx, |d, _| d.selected()), Some(second));
    assert_eq!(marks(cx), want(second), "after choosing the other one");
}

#[gpui_kit::test]
fn the_label_keeps_the_previewed_folder_while_planning_again(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = folders_only(r.parent.path(), &["a", "b"]);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-folder-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    d.update(cx, |d, cx| {
        d.select_scope(Some(ItemPath::new("b").unwrap()), cx)
    });
    assert_eq!(
        d.read_with(cx, |d, _| d.scope_label()).as_deref(),
        Some("Updating shop / Folder a"),
        "the old preview is still on screen"
    );
    wait_for(cx, "the b preview", |cx| {
        d.read_with(cx, |d, _| d.scope_label()).as_deref() == Some("Updating shop / Folder b")
    });
}

#[gpui_kit::test]
fn another_collection_cannot_be_chosen_during_a_preview(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = r.open_v1(cx);
    let other = tempfile::tempdir().unwrap();
    r.open(cx, folders_only(other.path(), &["a"]));
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-link-import");
    preview(cx, &d);
    let before = d.read_with(cx, |d, _| d.selected());
    d.update(cx, |d, cx| d.select_target(1, cx));
    assert_eq!(d.read_with(cx, |d, _| d.selected()), before);
    assert!(root.exists());
}

fn outside(root: &Path, dirs: &[&str]) -> BTreeMap<String, Vec<u8>> {
    bytes_under(root, "")
        .into_iter()
        // opencollection.yml holds every folder's baseline, so an update rewrites its own entry.
        .filter(|(p, _)| p != "opencollection.yml")
        .filter(|(p, _)| !dirs.iter().any(|d| p.starts_with(&format!("{d}/"))))
        .collect()
}

#[gpui_kit::test]
fn a_folder_update_leaves_the_root_and_the_other_folder_alone(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = root_and_folders(r.parent.path(), &["a", "b"]);
    r.open(cx, root.clone());
    let (root_files, b_before) = (outside(&root, &["a", "b"]), bytes_under(&root, "b"));
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-folder-1");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-apply");
    wait_for(cx, "the result", |cx| {
        matches!(phase(cx, &d), Phase::Done(_))
    });
    assert!(root.join("a/people/listUsers.yml").exists());
    assert_eq!(outside(&root, &["a", "b"]), root_files);
    assert_eq!(bytes_under(&root, "b"), b_before);
}

#[gpui_kit::test]
fn a_root_update_leaves_the_folders_alone(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = root_and_folders(r.parent.path(), &["a", "b"]);
    r.open(cx, root.clone());
    let (a, b) = (bytes_under(&root, "a"), bytes_under(&root, "b"));
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-folder-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-apply");
    wait_for(cx, "the result", |cx| {
        matches!(phase(cx, &d), Phase::Done(_))
    });
    assert!(root.join("people/listUsers.yml").exists());
    assert_eq!(bytes_under(&root, "a"), a);
    assert_eq!(bytes_under(&root, "b"), b);
}

#[gpui_kit::test]
fn a_picked_folder_is_the_only_primary_control(cx: &mut TestAppContext) {
    use apinata_app::import_link::PickStyle;
    let r: Rig = rig(cx);
    let root = root_and_folders(r.parent.path(), &["a", "b"]);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-folder-1");
    let (target, scopes) = d.read_with(cx, |d, _| (d.target_style(0), d.scope_styles()));
    assert_eq!(
        target,
        PickStyle::Marked,
        "the collection yields to the folder but stays marked"
    );
    assert_eq!(
        scopes,
        [PickStyle::Plain, PickStyle::Primary, PickStyle::Plain]
    );
    let primaries = scopes.iter().filter(|s| **s == PickStyle::Primary).count()
        + usize::from(target == PickStyle::Primary);
    assert_eq!(primaries, 1, "exactly one primary across both lists");
    for id in [
        "import-update-collection-heading",
        "import-update-folder-heading",
    ] {
        assert!(present(cx, r.win, id), "{id} is visible");
    }
}

#[gpui_kit::test]
fn without_a_scope_picker_the_collection_is_primary(cx: &mut TestAppContext) {
    use apinata_app::import_link::PickStyle;
    let r: Rig = rig(cx);
    let root = root_and_folders(r.parent.path(), &[]);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let (target, other) = d.read_with(cx, |d, _| (d.target_style(0), d.target_style(1)));
    assert_eq!(target, PickStyle::Primary);
    assert_eq!(other, PickStyle::Plain);
    assert!(present(cx, r.win, "import-update-collection-heading"));
    assert!(!present(cx, r.win, "import-update-folder-heading"));
}

/// The screen's target line: its text, and whether it is drawn.
fn target_line(cx: &mut TestAppContext, r: &Rig, d: &Entity<ImportLinkDialog>) -> String {
    assert!(
        present(cx, r.win, "import-update-scope"),
        "the line is drawn"
    );
    d.read_with(cx, |d, _| d.scope_label()).expect("a line")
}

#[gpui_kit::test]
fn the_preview_names_the_collection_for_a_whole_collection_update(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let name = d.read_with(cx, |d, _| d.targets()[0].name.clone());
    r.click(cx, "import-link-import");
    preview(cx, &d);
    frame(cx, r.win);
    assert_eq!(target_line(cx, &r, &d), format!("Updating {name}"));
}

#[gpui_kit::test]
fn the_result_names_the_collection_for_a_whole_collection_update(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let name = d.read_with(cx, |d, _| d.targets()[0].name.clone());
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-apply");
    wait_for(cx, "the result", |cx| {
        matches!(phase(cx, &d), Phase::Done(_))
    });
    frame(cx, r.win);
    assert_eq!(target_line(cx, &r, &d), format!("Updated {name}"));
}

#[gpui_kit::test]
fn the_preview_names_collection_and_folder_for_a_folder_update(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = folders_only(r.parent.path(), &["a", "b"]);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-folder-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    frame(cx, r.win);
    assert_eq!(target_line(cx, &r, &d), "Updating shop / Folder a");
}

#[gpui_kit::test]
fn the_result_names_collection_and_folder_for_a_folder_update(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = folders_only(r.parent.path(), &["a", "b"]);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-folder-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-apply");
    wait_for(cx, "the result", |cx| {
        matches!(phase(cx, &d), Phase::Done(_))
    });
    frame(cx, r.win);
    assert_eq!(target_line(cx, &r, &d), "Updated shop / Folder a");
}
