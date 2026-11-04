//! Saving a scratch draft into a collection.

use crate::support;

use apinata_app::save_draft::SaveDraftForm;
use apinata_core::ItemPath;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn scratch_draft(cx: &mut TestAppContext, s: &Started) -> (std::path::PathBuf, ItemPath) {
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    active_draft(cx, &s.ws)
}

fn names(cx: &mut TestAppContext, s: &Started, root: &std::path::Path) -> Vec<String> {
    let snap = s.ws.read_with(cx, |w, cx| w.snapshot(root, cx)).unwrap();
    snap.content
        .requests
        .values()
        .map(|r| r.name.clone())
        .collect()
}

#[gpui_kit::test]
fn save_writes_into_the_chosen_collection_then_drops_the_draft(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let (from, draft) = scratch_draft(cx, &s);
    let col = collection();
    let target = col.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.open(target.clone(), cx));
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| {
        w.save_draft_into(
            &from,
            draft,
            &target,
            ItemPath::root(),
            "Echo".into(),
            Vec::new(),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(names(cx, &s, &target).contains(&"Echo".to_string()));
    s.ws.read_with(cx, |w, _| {
        assert!(
            w.tabs()
                .entries()
                .iter()
                .all(|e| e.root != from || e.item().is_none())
        );
    });
}

#[gpui_kit::test]
fn a_failed_save_keeps_the_scratch_draft(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let (from, draft) = scratch_draft(cx, &s);
    let col = collection();
    let target = col.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.open(target.clone(), cx));
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| {
        w.save_draft_into(
            &from,
            draft.clone(),
            &target,
            ItemPath::new("missing").unwrap(),
            "Echo".into(),
            Vec::new(),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(!names(cx, &s, &target).contains(&"Echo".to_string()));
    let still = s.ws.read_with(cx, |w, cx| w.request(&from, &draft, cx));
    assert!(still.is_some(), "the draft survives a failed save");
}

fn open_dialog(cx: &mut TestAppContext, s: &Started) {
    use apinata_app::keymap::SaveActive;
    use gpui_kit::component::WindowExt as _;
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(SaveActive), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(
        cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
}

#[gpui_kit::test]
fn the_dialog_for_a_scratch_draft_lists_open_collections_and_the_open_entry(
    cx: &mut TestAppContext,
) {
    let s = started_blank(cx, blank_data_dir());
    let (_from, _draft) = scratch_draft(cx, &s);
    let col = collection();
    let target = col.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.open(target, cx));
    cx.run_until_parked();
    open_dialog(cx, &s);
    assert!(present(cx, s.win, "save-draft-collection"));
}

#[gpui_kit::test]
fn the_dialog_with_no_collection_open_still_offers_the_open_entry_and_will_not_save(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    let s = started_blank(cx, blank_data_dir());
    let (from, draft) = scratch_draft(cx, &s);
    open_dialog(cx, &s);
    assert!(present(cx, s.win, "save-draft-collection"));
    cx.update_window(s.win, |_, w, cx| {
        w.input("Echo", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let still = s.ws.read_with(cx, |w, cx| w.request(&from, &draft, cx));
    assert!(still.is_some(), "nothing was saved without a collection");
}

#[gpui_kit::test]
fn the_dialog_saves_into_the_open_collection(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let s = started_blank(cx, blank_data_dir());
    let (_from, _draft) = scratch_draft(cx, &s);
    let col = collection();
    let target = col.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.open(target.clone(), cx));
    cx.run_until_parked();
    open_dialog(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.input("Echo", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_real(cx, |_| target.join("Echo.yml").exists());
    cx.run_until_parked();
    assert!(names(cx, &s, &target).contains(&"Echo".to_string()));
}

#[gpui_kit::test]
fn a_cross_collection_save_approves_the_typed_scripts_for_the_target(cx: &mut TestAppContext) {
    use apinata_core::{ScriptTrust, script_hash};
    let s = started_blank(cx, blank_data_dir());
    let (from, draft) = scratch_draft(cx, &s);
    let col = collection();
    let target = col.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.open(target.clone(), cx));
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| {
        w.save_draft_into(
            &from,
            draft,
            &target,
            ItemPath::root(),
            "Echo".into(),
            vec!["let h = 8".into()],
            cx,
        )
    });
    wait_real(cx, |_| target.join("Echo.yml").exists());
    settle(cx);
    let trust = s.files.path().join("script-trust.json");
    let t = ScriptTrust::load(trust).unwrap();
    assert!(t.is_approved(&target, &script_hash("let h = 8")));
    assert_eq!(t.decision(&target), None);
}

fn scratch_form(cx: &mut TestAppContext, s: &Started) -> gpui_kit::Entity<SaveDraftForm> {
    cx.update_window(s.win, |_, w, cx| {
        cx.new(|cx| SaveDraftForm::for_scratch(&s.ws, "Echo", w, cx))
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_dialog_with_no_collection_says_so(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let (_from, _draft) = scratch_draft(cx, &s);
    open_dialog(cx, &s);
    assert!(present(cx, s.win, "save-draft-no-collection"));
}

#[gpui_kit::test]
fn the_dialog_with_a_collection_open_has_no_such_line(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let (_from, _draft) = scratch_draft(cx, &s);
    let col = collection();
    s.ws.update(cx, |w, cx| w.open(col.path().to_path_buf(), cx));
    cx.run_until_parked();
    open_dialog(cx, &s);
    assert!(!present(cx, s.win, "save-draft-no-collection"));
}

#[gpui_kit::test]
fn the_open_entry_comes_first_with_no_collection_open(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let form = scratch_form(cx, &s);
    let labels = form.read_with(cx, |f, _| f.collection_labels());
    assert_eq!(
        labels.first().map(|l| l.as_ref()),
        Some("Open or new collection...")
    );
    let picked = form.read_with(cx, |f, cx| f.answer(cx));
    assert!(
        picked.is_none(),
        "Save stays off until a collection is picked"
    );
}

#[gpui_kit::test]
fn the_open_entry_stays_first_beside_open_collections_and_a_pick_still_saves(
    cx: &mut TestAppContext,
) {
    let s = started_blank(cx, blank_data_dir());
    let col = collection();
    s.ws.update(cx, |w, cx| w.open(col.path().to_path_buf(), cx));
    cx.run_until_parked();
    let form = scratch_form(cx, &s);
    let labels = form.read_with(cx, |f, _| f.collection_labels());
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0].as_ref(), "Open or new collection...");
    let answer = form
        .read_with(cx, |f, cx| f.answer(cx))
        .expect("the open collection is picked");
    assert_eq!(answer.target.as_deref(), Some(col.path()));
}
