//! The Import dialog's update mode: a file as the source, the root's label, which target looks
//! selected, and changing the source from the preview.

use crate::import_update::{Rig, V2, phase, preview, rig, serve};
use crate::import_update_folder::place;
use crate::support::*;

use apinata_app::import_link::{ImportLinkDialog, Phase};
use apinata_core::{Collection, ItemPath};
use gpui_kit::{AppContext, Entity, TestAppContext};
use std::path::PathBuf;

/// A collection whose directory name differs from its display name, with the v1 spec in a
/// folder `a` and in a folder `b`.
fn named(r: &Rig) -> PathBuf {
    let mut c = Collection::create(&r.parent.path().join("dir-x"), "Pretty Name").unwrap();
    place(&mut c, "a");
    place(&mut c, "b");
    c.root().to_path_buf()
}

fn selected(cx: &mut TestAppContext, d: &Entity<ImportLinkDialog>) -> Vec<bool> {
    d.read_with(cx, |d, _| d.scopes_selected())
}

#[gpui_kit::test]
fn the_picker_names_a_collection_by_its_display_name(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = named(&r);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let names: Vec<String> =
        d.read_with(cx, |d, _| d.targets().into_iter().map(|t| t.name).collect());
    assert_eq!(names, ["Pretty Name"]);
}

#[gpui_kit::test]
fn the_root_row_does_not_repeat_the_display_name(cx: &mut TestAppContext) {
    let r = rig(cx);
    let mut c = Collection::create(&r.parent.path().join("dir-y"), "Pretty Root").unwrap();
    place(&mut c, "a");
    let root = c.root().to_path_buf();
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let label = d.read_with(cx, |d, _| d.scope_name(None));
    assert_eq!(label, "Whole collection");
}

#[gpui_kit::test]
fn only_the_chosen_scope_is_selected(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = named(&r);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    // No root baseline: scopes are [a, b]; the active tab decides nothing, so none is chosen.
    assert_eq!(selected(cx, &d), [false, false]);
    d.update(cx, |d, cx| {
        d.select_scope(Some(ItemPath::new("b").unwrap()), cx)
    });
    assert_eq!(selected(cx, &d), [false, true]);
    d.update(cx, |d, cx| {
        d.select_scope(Some(ItemPath::new("a").unwrap()), cx)
    });
    assert_eq!(selected(cx, &d), [true, false]);
}

#[gpui_kit::test]
fn a_file_is_a_source_like_the_link(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let file = r.parent.path().join("v2.json");
    std::fs::write(&file, V2).unwrap();
    r.click(cx, "import-update-browse");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(move |o| {
        assert!(o.files && !o.directories && !o.multiple);
        Some(vec![file])
    });
    let from_file = preview(cx, &d);
    // The same spec through the link gives the same plan.
    d.update(cx, |d, cx| d.change_source(cx));
    assert_eq!(phase(cx, &d), Phase::Idle);
    r.click(cx, "import-link-import");
    assert_eq!(format!("{:?}", preview(cx, &d)), format!("{from_file:?}"));
}

/// Types `text` into the field of an update-mode dialog and runs the import.
fn type_source(cx: &mut TestAppContext, r: &Rig, d: &Entity<ImportLinkDialog>, text: &str) {
    let (v, text) = (d.clone(), text.to_owned());
    cx.update_window(r.win, |_, w, cx| {
        v.update(cx, |d, cx| d.set_link(&text, w, cx))
    })
    .unwrap();
    r.click(cx, "import-link-import");
}

#[gpui_kit::test]
fn a_typed_path_or_file_url_is_a_source_like_the_link(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-link-import");
    let from_link = format!("{:?}", preview(cx, &d));
    let file = r.parent.path().join("typed v2.json");
    std::fs::write(&file, V2).unwrap();
    let url = url::Url::from_file_path(&file).unwrap();
    for typed in [file.to_str().unwrap(), url.as_str()] {
        d.update(cx, |d, cx| d.change_source(cx));
        assert_eq!(phase(cx, &d), Phase::Idle);
        type_source(cx, &r, &d, typed);
        assert_eq!(format!("{:?}", preview(cx, &d)), from_link, "{typed}");
    }
}

#[gpui_kit::test]
fn a_typed_missing_path_fails_at_once_in_update_mode(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let gone = r.parent.path().join("gone.json");
    type_source(cx, &r, &d, gone.to_str().unwrap());
    match phase(cx, &d) {
        Phase::Failed(m) => assert!(m.contains("No file at that path"), "{m}"),
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[gpui_kit::test]
fn a_bad_file_fails_in_the_dialog(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let file = r.parent.path().join("junk.json");
    std::fs::write(&file, b"not a spec").unwrap();
    r.click(cx, "import-update-browse");
    cx.simulate_path_prompt_response(move |_| Some(vec![file]));
    wait_for(cx, "the failure", |cx| {
        matches!(phase(cx, &d), Phase::Failed(_))
    });
}

#[gpui_kit::test]
fn the_source_changes_from_the_preview(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = named(&r);
    r.open(cx, root);
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    r.click(cx, "import-update-folder-0");
    r.click(cx, "import-link-import");
    preview(cx, &d);
    r.click(cx, "import-update-change-source");
    assert_eq!(phase(cx, &d), Phase::Idle);
    assert!(present(cx, r.win, "import-link"), "the field is back");
    assert!(d.read_with(cx, |d, cx| d.link(cx)).contains("spec.json"));
    assert_eq!(
        d.read_with(cx, |d, _| d.folder()),
        Some(Some(ItemPath::new("a").unwrap())),
        "the choice survives"
    );
}

#[gpui_kit::test]
fn folders_are_listed_in_the_sidebars_order(cx: &mut TestAppContext) {
    let r = rig(cx);
    let mut c = Collection::create(&r.parent.path().join("dir-z"), "Z").unwrap();
    place(&mut c, "a");
    place(&mut c, "b");
    // Re-recording a's entry puts it after b's in the file, while the tree keeps a first.
    let a = ItemPath::new("a").unwrap();
    let kept = c.file().spec_baseline_in(&a).unwrap();
    c.file_mut().clear_spec_baseline_in(&a);
    c.file_mut().set_spec_baseline_in(&a, &kept);
    c.file_mut().save().unwrap();
    assert_eq!(c.file().spec_baseline_folders()[0].as_str(), "b");
    r.open(cx, c.root().to_path_buf());
    let server = serve(cx, V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    let order: Vec<String> = d.read_with(cx, |d, _| {
        d.scopes()
            .into_iter()
            .flatten()
            .map(|f| f.to_string())
            .collect()
    });
    assert_eq!(order, ["a", "b"]);
}
