use crate::support;

use apinata_app::command::Edit;
use apinata_app::draft_store::Drafts;
use apinata_app::save_draft::{self, SaveAs};
use apinata_core::ItemPath;
use std::time::Duration;

use gpui_kit::{Focusable, TestAppContext, VisualTestContext};
use support::*;

fn close_active_tab(cx: &mut TestAppContext, s: &Started) {
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    VisualTestContext::from_window(s.win, cx).update(|w, cx| handle.focus(w, cx));
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("w"));
    frame(cx, s.win);
}

// Runs ready work until none is left; panics instead of hanging when it never parks.
fn settle_bounded(cx: &mut TestAppContext) {
    for _ in 0..10_000 {
        if !cx.executor().tick() {
            return;
        }
    }
    panic!("never parks");
}

// `wait_save` that fails rather than hangs when the store keeps itself busy.
fn wait_save_bounded(cx: &mut TestAppContext) {
    settle_bounded(cx);
    (cx.executor()).advance_clock(apinata_app::persisted::SAVE_DELAY + Duration::from_millis(50));
    settle_bounded(cx);
}

// Opens a second tab of draft `item` under its history key `hk`, as reopening one of its
// history entries does.
fn reopen_same_key(cx: &mut TestAppContext, s: &Started, item: &str, hk: &ItemPath) {
    let snap = ws_snap(cx, &s.ws, &s.root).content.requests[&ip(item)]
        .snapshot
        .clone()
        .expect("a draft snapshot");
    s.ws.update(cx, |w, cx| {
        w.reopen_draft(&s.root, hk.clone(), 0, (*snap).clone(), cx)
    });
    settle_bounded(cx);
}

#[gpui_kit::test]
fn dont_save_and_save_as_remove_the_file(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/x".into()));
    wait_save(cx);
    assert!(draft_path(&s, &hk).exists());
    close_active_tab(cx, &s);
    answer_confirm(cx, "confirm-discard");
    wait_save(cx);
    assert!(!draft_path(&s, &hk).exists(), "Don't save removes it");

    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/z".into()));
    wait_save(cx);
    let save_as = SaveAs {
        name: "Kept".into(),
        parent: ItemPath::root(),
        target: None,
    };
    cx.update(|cx| save_draft::send(&s.ws, &s.root, ip(&item), save_as, cx));
    wait_save(cx);
    assert!(!draft_path(&s, &hk).exists(), "Save-as removes it");
}

#[gpui_kit::test]
fn a_draft_edited_back_to_clean_drops_its_file_and_closes_silently(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/y".into()));
    wait_save(cx);
    assert!(draft_path(&s, &hk).exists());
    edit_item(cx, &s, &item, Edit::Url(String::new()));
    wait_save(cx);
    assert!(
        !draft_path(&s, &hk).exists(),
        "no stale file outlives an undo"
    );
    close_active_tab(cx, &s);
    assert!(!confirm_open(cx));
    assert!(tab_names(cx, &s.ws).is_empty());
}

#[gpui_kit::test]
fn a_clean_tab_sharing_a_dirty_drafts_key_settles_and_keeps_the_file(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let store = cx.update(|cx| cx.global::<Drafts>().0.clone());
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/x".into()));
    wait_save_bounded(cx);
    let fetches = store.read_with(cx, |d, _| d.fetches());
    reopen_same_key(cx, &s, &item, &hk);
    for _ in 0..5 {
        wait_save_bounded(cx);
    }
    assert!(store.read_with(cx, |d, _| d.fetches()) <= fetches + 1);
    assert!(
        draft_path(&s, &hk).exists(),
        "the dirty draft keeps its file"
    );
}

#[gpui_kit::test]
fn closing_a_clean_tab_sharing_a_dirty_drafts_key_keeps_the_file(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/x".into()));
    wait_save_bounded(cx);
    reopen_same_key(cx, &s, &item, &hk);
    let active =
        s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().item().cloned());
    assert_ne!(active, Some(ip(&item)), "the clean duplicate is active");
    close_active_tab(cx, &s);
    settle_bounded(cx);
    assert!(!confirm_open(cx));
    assert!(
        draft_path(&s, &hk).exists(),
        "the dirty original keeps its file"
    );
}

#[gpui_kit::test]
fn two_dirty_tabs_sharing_a_key_settle_and_save_as_of_one_keeps_the_file(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/x".into()));
    wait_save_bounded(cx);
    reopen_same_key(cx, &s, &item, &hk);
    let dup = s.ws.read_with(cx, |w, _| {
        let e = w.tabs().active().unwrap();
        e.item().unwrap().as_str().to_owned()
    });
    edit_item(cx, &s, &dup, Edit::Url("http://127.0.0.1:9/dup".into()));
    for _ in 0..3 {
        wait_save_bounded(cx);
    }
    let save_as = SaveAs {
        name: "Kept".into(),
        parent: ItemPath::root(),
        target: None,
    };
    cx.update(|cx| save_draft::send(&s.ws, &s.root, ip(&item), save_as, cx));
    settle_bounded(cx);
    // Each tab owns a file, so the saved one's goes and the other dirty tab's stays.
    let kept = apinata_app::draft_file::load_dir(&s.files.path().join("drafts")).drafts;
    assert!(
        kept.len() == 1 && kept[0].request.contains("/dup"),
        "the other dirty tab keeps its file"
    );
}

// Edits a reopened duplicate so two tabs on one key are dirty, then returns both item names.
fn two_dirty(cx: &mut TestAppContext, s: &Started, hk: &ItemPath, item: &str) -> String {
    reopen_same_key(cx, s, item, hk);
    let dup = s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .unwrap()
            .item()
            .unwrap()
            .as_str()
            .to_owned()
    });
    edit_item(cx, s, &dup, Edit::Url("http://127.0.0.1:9/dup".into()));
    for _ in 0..3 {
        wait_save_bounded(cx);
    }
    dup
}

fn slot_file(s: &Started, hk: &ItemPath, slot: u32) -> std::path::PathBuf {
    s.files
        .path()
        .join("drafts")
        .join(apinata_app::draft_file::file_name_slot(hk, slot))
}

#[gpui_kit::test]
fn a_tab_that_goes_clean_drops_only_its_own_slot_and_frees_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/x".into()));
    wait_save_bounded(cx);
    let dup = two_dirty(cx, &s, &hk, &item);
    assert!(slot_file(&s, &hk, 0).exists() && slot_file(&s, &hk, 1).exists());
    // The duplicate is the second slot; undoing its edit must drop that file alone.
    let before = std::fs::read_to_string(slot_file(&s, &hk, 0)).unwrap();
    edit_item(cx, &s, &dup, Edit::Url("http://127.0.0.1:9/x".into()));
    for _ in 0..3 {
        wait_save_bounded(cx);
    }
    assert!(
        slot_file(&s, &hk, 0).exists(),
        "the other tab keeps its slot"
    );
    assert!(!slot_file(&s, &hk, 1).exists(), "the clean tab's slot goes");
    assert_eq!(
        std::fs::read_to_string(slot_file(&s, &hk, 0)).unwrap(),
        before
    );
    // The freed slot is the next one a new duplicate takes.
    let again = two_dirty(cx, &s, &hk, &item);
    assert_ne!(again, dup);
    assert!(slot_file(&s, &hk, 1).exists(), "the freed slot is reused");
}

#[gpui_kit::test]
fn a_draft_saved_as_from_a_shared_key_drops_only_its_slot(cx: &mut TestAppContext) {
    // A draft closed with Save as leaves the store through the rekey: the tab drops its key,
    // so rebind finds nothing bound to it and removes the file with no draft left to rebuild.
    let col = two_requests();
    let s = started(cx, &col);
    let (item, hk) = new_draft_at(cx, &s, ItemPath::root());
    edit_item(cx, &s, &item, Edit::Url("http://127.0.0.1:9/x".into()));
    wait_save_bounded(cx);
    let dup = two_dirty(cx, &s, &hk, &item);
    let save_as = SaveAs {
        name: "Kept".into(),
        parent: ItemPath::root(),
        target: None,
    };
    cx.update(|cx| save_draft::send(&s.ws, &s.root, ip(&dup), save_as, cx));
    settle_bounded(cx);
    assert!(
        !slot_file(&s, &hk, 1).exists(),
        "the saved tab's slot is dropped"
    );
    assert!(slot_file(&s, &hk, 0).exists(), "the other tab's stays");
}
