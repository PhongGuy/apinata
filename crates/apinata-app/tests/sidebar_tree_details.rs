//! Where the tree selection goes when a draft closes, where a folder's New request puts its
//! draft, and where an inline rename field sits in its row.

use crate::support;

use apinata_app::sidebar::marker_id;
use apinata_app::sidebar::menu::MenuEntry;
use apinata_app::sidebar::model::NodeKind;
use apinata_app::tabs::TabKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, ElementId, Focusable, Pixels, SharedString};
use gpui_kit::{TestAppContext, VisualTestContext};
use support::*;

fn focus_shell(cx: &mut TestAppContext, s: &Started) {
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| handle.focus(w, cx));
    frame(cx, s.win);
}

fn active(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .map(|p| p.as_str().to_owned())
    })
}

fn new_draft(cx: &mut TestAppContext, s: &Started) -> String {
    focus_shell(cx, s);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    active(cx, s).expect("a draft")
}

fn select_row(cx: &mut TestAppContext, s: &Started, id: SharedString) {
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&id);
        assert!(ix.is_some(), "{id} is a row");
        t.set_selected_index(ix, cx)
    });
    frame(cx, s.win);
}

fn selected_id(cx: &mut TestAppContext, s: &Started) -> Option<SharedString> {
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    tree.read_with(cx, |t, _| t.selected_item().map(|i| i.id.clone()))
}

fn activate(cx: &mut TestAppContext, s: &Started, rel: &str) {
    let id =
        s.ws.read_with(cx, |w, _| {
            w.tabs()
                .entries()
                .iter()
                .find(|e| e.item().map(|p| p.as_str()) == Some(rel))
                .map(|e| e.id)
        })
        .expect("an open tab");
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(id)));
    frame(cx, s.win);
}

#[gpui_kit::test]
fn closing_a_selected_draft_selects_the_active_tabs_row(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    let draft = new_draft(cx, &s);
    activate(cx, &s, "a.yml");
    assert_eq!(selected_id(cx, &s), Some(row("a.yml")));
    // The draft row is selected while another tab stays active, as Up or Down leaves it.
    select_row(cx, &s, row(&draft));
    choose_menu(cx, s.win, &draft, NodeKind::Draft, MenuEntry::CloseDraft);
    frame(cx, s.win);
    assert!(!shown(cx, s.win, row(&draft)));
    assert_eq!(active(cx, &s).as_deref(), Some("a.yml"));
    assert_eq!(selected_id(cx, &s), Some(row("a.yml")));
}

#[gpui_kit::test]
fn closing_a_draft_before_a_tab_without_a_row_clears_the_selection(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Environments, cx)
    })
    .expect("an environments tab");
    frame(cx, s.win);
    let draft = new_draft(cx, &s);
    assert_eq!(selected_id(cx, &s), Some(row(&draft)));
    choose_menu(cx, s.win, &draft, NodeKind::Draft, MenuEntry::CloseDraft);
    frame(cx, s.win);
    assert!(!shown(cx, s.win, row(&draft)));
    assert_eq!(active(cx, &s), None, "the environments tab is active");
    assert_eq!(selected_id(cx, &s), None);
}

fn with_folder() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("users/folder.yml", &folder("users")),
        ("users/list.yml", &request("List users", "http://x/users")),
    ])
}

#[gpui_kit::test]
fn new_request_from_a_folder_menu_drafts_and_saves_in_that_folder(cx: &mut TestAppContext) {
    let col = with_folder();
    let s = started(cx, &col);
    // The selection sits elsewhere, so only the menu's own row can place the draft.
    select_row(cx, &s, row("a.yml"));
    choose_menu(cx, s.win, "users", NodeKind::Folder, MenuEntry::NewRequest);
    frame(cx, s.win);
    let draft = active(cx, &s).expect("a draft");
    assert_eq!(ip(&draft).parent(), Some(ip("users")), "{draft}");
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("s"));
    frame(cx, s.win);
    assert!(present(cx, s.win, "save-draft-dialog"));
    cx.update_window(s.win, |_, w, cx| {
        w.input("Create user", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_for(cx, "the save", |_| {
        col.path().join("users/Create user.yml").exists()
    });
}

fn bounds(cx: &mut TestAppContext, h: &SidebarHarness, id: impl Into<ElementId>) -> Bounds<Pixels> {
    let id = id.into();
    cx.update_window(h.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

fn start_rename(cx: &mut TestAppContext, h: &SidebarHarness, path: &str) {
    let (view, root) = (h.view.clone(), h.root.clone());
    cx.update_window(h.win, |_, w, cx| {
        view.update(cx, |s, cx| s.start_rename(&root, &ip(path), w, cx))
    })
    .unwrap();
    frame(cx, h.win);
}

#[gpui_kit::test]
fn a_request_rename_field_starts_where_its_name_did_beside_its_method(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let id = row("health.yml");
    let name = bounds(cx, &h, marker_id("name", &id));
    let method = bounds(cx, &h, marker_id("method", &id));
    start_rename(cx, &h, "health.yml");
    let field = bounds(cx, &h, marker_id("rename", &id));
    assert_eq!(field.left(), name.left());
    assert!(
        shown(cx, h.win, marker_id("method", &id)),
        "the badge stays"
    );
    assert_eq!(bounds(cx, &h, marker_id("method", &id)), method);
}

#[gpui_kit::test]
fn a_folder_rename_field_starts_after_its_icon(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let id = row("users");
    let name = bounds(cx, &h, marker_id("name", &id));
    start_rename(cx, &h, "users");
    let field = bounds(cx, &h, marker_id("rename", &id));
    assert_eq!(field.left(), name.left());
    assert!(shown(cx, h.win, marker_id("icon", &id)), "the icon stays");
}
