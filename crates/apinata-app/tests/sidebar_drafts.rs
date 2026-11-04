//! An open draft has a row under "Unsaved" for exactly as long as its tab is open.

use crate::support;

use apinata_app::sidebar::menu::MenuEntry;
use apinata_app::sidebar::model::{NodeKind, node_id, unsaved_id};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable, TestAppContext, VisualTestContext};
use support::*;

fn focus_shell(cx: &mut TestAppContext, s: &Started) {
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| handle.focus(w, cx));
    frame(cx, s.win);
}

fn new_draft(cx: &mut TestAppContext, s: &Started) -> String {
    focus_shell(cx, s);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    active(cx, s).expect("a draft")
}

fn active(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .map(|p| p.as_str().to_owned())
    })
}

#[gpui_kit::test]
fn a_new_draft_has_a_row_until_its_tab_closes(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    assert!(!shown(cx, s.win, unsaved_id(0)));
    let draft = new_draft(cx, &s);
    assert!(ip(&draft).is_draft(), "{draft}");
    assert!(shown(cx, s.win, unsaved_id(0)));
    assert!(shown(cx, s.win, row(&draft)));
    choose_menu(cx, s.win, &draft, NodeKind::Draft, MenuEntry::CloseDraft);
    frame(cx, s.win);
    assert!(
        tab_names(cx, &s.ws).is_empty(),
        "an untouched draft closes without asking"
    );
    assert!(!shown(cx, s.win, row(&draft)));
    assert!(!shown(cx, s.win, unsaved_id(0)));
}

#[gpui_kit::test]
fn clicking_a_draft_row_focuses_its_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let draft = new_draft(cx, &s);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    assert_eq!(active(cx, &s).as_deref(), Some("a.yml"));
    cx.update_window(s.win, |_, w, cx| w.click(row(&draft), cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(active(cx, &s), Some(draft.clone()));
    assert_eq!(tab_names(cx, &s.ws).len(), 2, "no second tab for the draft");
}

#[gpui_kit::test]
fn save_on_a_draft_row_asks_for_a_name(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let draft = new_draft(cx, &s);
    choose_menu(cx, s.win, &draft, NodeKind::Draft, MenuEntry::SaveDraft);
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-draft-dialog"));
}

fn select_row(cx: &mut TestAppContext, s: &Started, id: gpui_kit::SharedString) {
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&id);
        assert!(ix.is_some(), "{id} is a row");
        t.set_selected_index(ix, cx)
    });
}

fn selected_id(cx: &mut TestAppContext, s: &Started) -> Option<gpui_kit::SharedString> {
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    tree.read_with(cx, |t, _| t.selected_item().map(|i| i.id.clone()))
}

#[gpui_kit::test]
fn a_selected_unsaved_group_is_not_the_collection(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    new_draft(cx, &s);
    select_row(cx, &s, unsaved_id(0));
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    assert_eq!(sidebar.read_with(cx, |v, cx| v.selected(cx)), None);
}

// The group shares the collection's root path; a rebuild must restore the collection row.
#[gpui_kit::test]
fn a_selected_collection_row_survives_rebuilds_beside_the_unsaved_group(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    for _ in 0..9 {
        new_draft(cx, &s);
    }
    let collection = node_id(0, &apinata_core::ItemPath::root());
    select_row(cx, &s, collection.clone());
    let ids: Vec<_> =
        s.ws.read_with(cx, |w, _| w.tabs().entries().iter().map(|e| e.id).collect());
    for id in &ids[..8] {
        s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.close(*id)));
        frame(cx, s.win);
        assert_eq!(selected_id(cx, &s), Some(collection.clone()));
    }
}

// Left on the selected group makes the tree collapse it itself; the group must stay open.
#[gpui_kit::test]
fn left_on_the_unsaved_group_keeps_its_drafts_shown(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let draft = new_draft(cx, &s);
    select_row(cx, &s, unsaved_id(0));
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| tree.update(cx, |t, cx| t.focus(w, cx)));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.press("left", cx))
        .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(shown(cx, s.win, unsaved_id(0)));
    assert!(shown(cx, s.win, row(&draft)), "the group collapsed");
    assert_eq!(
        selected_id(cx, &s),
        Some(unsaved_id(0)),
        "the selection moved"
    );
}

// A draft tab whose view the snapshot does not hold yet still gets its row at once.
#[gpui_kit::test]
fn a_draft_whose_view_is_not_loaded_has_a_row(cx: &mut TestAppContext) {
    use apinata_app::tabs::TabKind;
    let col = two_requests();
    let s = started(cx, &col);
    let item = ip("untitled-9.draft");
    assert!(item.is_draft());
    s.ws.update(cx, |w, cx| {
        let root = s.root.clone();
        w.update_tabs(cx, |t| {
            t.open(&root, TabKind::Request(item.clone()), false);
        })
    });
    frame(cx, s.win);
    assert!(shown(cx, s.win, unsaved_id(0)));
    assert!(shown(cx, s.win, row("untitled-9.draft")));
}
