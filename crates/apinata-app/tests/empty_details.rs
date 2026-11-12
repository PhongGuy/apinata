//! The details of empty places: the no-collection screen, the sidebar with nothing open and
//! empty folders.

use crate::support;

use apinata_app::command::Command;
use apinata_app::keymap::NewRequest;
use apinata_app::sidebar::drop::{DropZone, drop_target};
use apinata_app::sidebar::marker_id;
use apinata_app::sidebar::model::{NodeKind, empty_id};
use apinata_core::DropTarget;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, point, px};
use support::*;

// The index of the empty screen's demo action, found by its label.
fn demo_index(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    cx.update_window(win, |_, w, _| {
        let actions = w.within("no-collection-actions");
        (0usize..)
            .map_while(|ix| actions.try_find(("empty-action", ix)))
            .position(|b| b.label() == Some("Open the demo collection…"))
            .expect("the demo action")
    })
    .unwrap()
}

// A collection with `files` plus the empty folder `empty`, every folder open.
fn with_empty_folder(
    cx: &mut TestAppContext,
    mut files: Vec<(&'static str, String)>,
) -> SidebarHarness {
    files.push(("empty/folder.yml", folder("Nothing here")));
    let h = open_sidebar_with(cx, files);
    cx.update_window(h.win, |_, w, cx| w.click("sidebar-expand-all", cx))
        .unwrap();
    frame(cx, h.win);
    h
}

// The tree id of the selected row, which may be one `Sidebar::selected` leaves out.
fn selected_id(cx: &mut TestAppContext, h: &SidebarHarness) -> Option<String> {
    h.view.read_with(cx, |s, cx| {
        s.tree().read(cx).selected_item().map(|i| i.id.to_string())
    })
}

// Removes the request at `rel` on disk and reloads, as a delete outside the app does.
fn delete_on_disk(cx: &mut TestAppContext, h: &SidebarHarness, rel: &str) {
    std::fs::remove_file(h.root.join(rel)).unwrap();
    let changed = vec![h.root.join(rel)];
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Reload(changed), cx));
    cx.run_until_parked();
    frame(cx, h.win);
}

fn click_row(cx: &mut TestAppContext, h: &SidebarHarness, path: &str) {
    cx.update_window(h.win, |_, w, cx| w.click(row(path), cx))
        .unwrap();
    frame(cx, h.win);
}

const HEADER: [&str; 3] = [
    "sidebar-collapse-all",
    "sidebar-expand-all",
    "sidebar-history",
];

#[gpui_kit::test]
fn new_request_on_the_empty_screen_shows_its_key(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let chip = cx
        .update_window(s.win, |_, w, _| {
            w.within("no-collection-actions")
                .try_find(("empty-kbd", 0usize))
                .is_some()
        })
        .unwrap();
    assert!(chip, "New request shows its key chip");
}

#[gpui_kit::test]
fn the_internet_note_sits_under_the_demo_action(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let ix = demo_index(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let actions = w.within("no-collection-actions");
        let demo = actions.find(("empty-action", ix)).bounds();
        let note = actions.find("demo-hint").bounds();
        assert!(note.top() >= demo.bottom(), "{note:?} vs {demo:?}");
        assert!(note.top() - demo.bottom() < px(8.), "{note:?} vs {demo:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn with_nothing_open_the_sidebar_header_buttons_are_disabled(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    cx.update_window(s.win, |_, w, _| {
        for id in HEADER {
            assert_eq!(w.find(id).disabled(), Some(true), "{id}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn with_a_collection_open_the_sidebar_header_buttons_work(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| {
        for id in HEADER {
            assert_ne!(w.find(id).disabled(), Some(true), "{id}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_empty_folder_has_no_run_button_and_shows_empty_when_open(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("empty/folder.yml", folder("Nothing here")));
    let h = open_sidebar_with(cx, files);
    assert!(present(cx, h.win, marker_id("run-folder", &row("users"))));
    assert!(!present(cx, h.win, marker_id("run-folder", &row("empty"))));
    assert!(!present(cx, h.win, empty_id(0, &ip("empty"))));
    cx.update_window(h.win, |_, w, cx| w.click("sidebar-expand-all", cx))
        .unwrap();
    frame(cx, h.win);
    assert!(present(cx, h.win, empty_id(0, &ip("empty"))));
}

#[gpui_kit::test]
fn deleting_the_last_request_of_an_open_folder_skips_its_empty_row(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("e/folder.yml", folder("E")));
    files.push(("e/x.yml", request("X", "http://x/x")));
    let h = open_sidebar_with(cx, files);
    click_row(cx, &h, "e");
    click_row(cx, &h, "e/x.yml");
    delete_on_disk(cx, &h, "e/x.yml");
    let now = selected_id(cx, &h).expect("a row takes the selection");
    assert_ne!(now, empty_id(0, &ip("e")).to_string());
    let (_, path) = h.view.read_with(cx, |s, cx| s.selected(cx)).unwrap();
    assert!(!path.as_str().starts_with('e'), "{path:?}");
}

#[gpui_kit::test]
fn when_the_folder_ends_the_tree_the_folder_takes_the_selection(cx: &mut TestAppContext) {
    let files = vec![
        ("opencollection.yml", ROOT.to_string()),
        ("e/folder.yml", folder("E")),
        ("e/x.yml", request("X", "http://x/x")),
    ];
    let h = open_sidebar_with(cx, files);
    click_row(cx, &h, "e");
    click_row(cx, &h, "e/x.yml");
    delete_on_disk(cx, &h, "e/x.yml");
    assert_eq!(selected_id(cx, &h), Some(row("e").to_string()));
}

#[gpui_kit::test]
fn a_new_request_from_the_empty_row_goes_in_its_folder(cx: &mut TestAppContext) {
    let h = with_empty_folder(cx, sidebar_files());
    let id = empty_id(0, &ip("empty"));
    h.view.update(cx, |s, cx| {
        s.tree().update(cx, |t, cx| {
            let ix = t.index_of(&id);
            assert!(ix.is_some(), "the Empty row is listed");
            t.set_selected_index(ix, cx)
        })
    });
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| s.new_request(&NewRequest, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let (_, item) = active_draft(cx, &h.ws);
    assert_eq!(item.parent(), Some(ip("empty")));
}

#[test]
fn a_drop_anywhere_on_the_empty_row_lands_inside_its_folder() {
    for zone in [DropZone::Before, DropZone::Inside, DropZone::After] {
        assert_eq!(
            drop_target(&ip("a.yml"), &ip("empty"), NodeKind::Empty, zone),
            Some(DropTarget::Inside(ip("empty"))),
            "{zone:?}"
        );
    }
}

#[gpui_kit::test]
fn a_request_dragged_onto_the_empty_row_moves_into_the_folder(cx: &mut TestAppContext) {
    let h = with_empty_folder(cx, sidebar_files());
    cx.update_window(h.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(row("health.yml")).bounds().center();
        let b = w.find(empty_id(0, &ip("empty"))).bounds();
        w.drag(
            start,
            point(b.center().x, b.origin.y + b.size.height * 0.1),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert!(h.root.join("empty/health.yml").exists());
    assert!(!h.root.join("health.yml").exists());
}

// With nothing open the collection's environment select and Manage environments are locked,
// and each tip says why.
#[gpui_kit::test]
fn with_nothing_open_the_environment_controls_are_locked(cx: &mut TestAppContext) {
    use apinata_app::shell::NO_COLLECTION_ENV_TIP;
    let s = started_blank(cx, blank_data_dir());
    cx.update_window(s.win, |_, w, _| {
        assert_eq!(w.find("env-select").disabled(), Some(true), "select");
        let manage = w.find("environments");
        assert_eq!(manage.disabled(), Some(true), "Manage environments");
        assert_eq!(manage.description(), Some(NO_COLLECTION_ENV_TIP));
    })
    .unwrap();
    cx.update_window(s.win, |_, w, cx| w.hover("env-control", cx))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, s.win);
    let tip = cx
        .update_window(s.win, |_, w, _| {
            w.try_find("env-control-tooltip")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(
        tip.as_deref(),
        Some(NO_COLLECTION_ENV_TIP),
        "the select's tip"
    );
}

#[gpui_kit::test]
fn with_a_collection_open_the_environment_controls_work(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| {
        assert_ne!(w.find("env-select").disabled(), Some(true), "select");
        let manage = w.find("environments");
        assert_ne!(manage.disabled(), Some(true), "Manage environments");
        assert_eq!(
            manage.description(),
            Some(apinata_app::shell::MANAGE_ENVIRONMENTS_TIP)
        );
    })
    .unwrap();
}

// An empty folder's Empty word starts in the column its children's method tags would.
#[gpui_kit::test]
fn the_empty_row_sits_at_the_indent_of_the_folders_children(cx: &mut TestAppContext) {
    let h = with_empty_folder(cx, sidebar_files());
    cx.update_window(h.win, |_, w, _| {
        let child = w.find(marker_id("method", &row("users/list.yml"))).bounds();
        let empty = w
            .find(marker_id("name", &empty_id(0, &ip("empty"))))
            .bounds();
        assert_eq!(empty.left(), child.left(), "{empty:?} vs {child:?}");
    })
    .unwrap();
}
