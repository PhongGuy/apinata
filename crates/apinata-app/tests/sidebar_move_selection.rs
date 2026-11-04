//! A selected request inside a moved folder stays selected at its new path, whether the move
//! comes from the workspace, an inline rename or a drag, and its new folders open to show it.

use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_core::DropTarget;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn click(cx: &mut TestAppContext, h: &SidebarHarness, path: &str) {
    cx.update_window(h.win, |_, w, cx| w.click(row(path), cx))
        .unwrap();
    frame(cx, h.win);
}

fn selected(cx: &mut TestAppContext, h: &SidebarHarness) -> Option<(std::path::PathBuf, String)> {
    h.view
        .read_with(cx, |s, cx| s.selected(cx))
        .map(|(r, p)| (r, p.to_string()))
}

#[gpui_kit::test]
fn a_workspace_move_keeps_the_nested_request_selected(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("admin/folder.yml", folder("Admin")));
    let h = open_sidebar_with(cx, files);
    click(cx, &h, "users");
    click(cx, &h, "users/list.yml");
    let mv = TreeOp::Move {
        item: ip("users"),
        to: DropTarget::Inside(ip("admin")),
    };
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Tree(mv), cx));
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(h.root.join("admin/users/list.yml").exists());
    assert_eq!(
        selected(cx, &h),
        Some((h.root.clone(), ip("admin/users/list.yml").to_string()))
    );
    assert!(shown(cx, h.win, row("admin/users/list.yml")));
}

#[gpui_kit::test]
fn renaming_an_outer_folder_keeps_the_nested_request_selected(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("outer/folder.yml", folder("Outer")));
    files.push(("outer/inner/folder.yml", folder("Inner")));
    files.push(("outer/inner/r.yml", request("R", "http://x/r")));
    let h = open_sidebar_with(cx, files);
    click(cx, &h, "outer");
    click(cx, &h, "outer/inner");
    click(cx, &h, "outer/inner/r.yml");
    let (view, root) = (h.view.clone(), h.root.clone());
    cx.update_window(h.win, |_, w, cx| {
        view.update(cx, |s, cx| s.start_rename(&root, &ip("outer"), w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        w.input("renamed", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(h.root.join("renamed/inner/r.yml").exists());
    assert_eq!(
        selected(cx, &h),
        Some((h.root.clone(), ip("renamed/inner/r.yml").to_string()))
    );
}

#[gpui_kit::test]
fn a_dragged_subfolder_opens_its_collapsed_target_to_its_selected_request(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("p/folder.yml", folder("P")));
    files.push(("p/sub/folder.yml", folder("Sub")));
    files.push(("p/sub/r.yml", request("R", "http://x/r")));
    files.push(("t/folder.yml", folder("T")));
    let h = open_sidebar_with(cx, files);
    click(cx, &h, "p");
    click(cx, &h, "p/sub");
    click(cx, &h, "p/sub/r.yml");
    // Past the double-click window, so the drag's press is not read as a second click.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.update_window(h.win, |_, w, cx| w.drag_to(row("p/sub"), row("t"), cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(h.root.join("t/sub/r.yml").exists());
    let expanded = h.view.read_with(cx, |s, _| s.expanded_paths(&h.root));
    assert!(expanded.contains(&ip("t")), "{expanded:?}");
    assert!(shown(cx, h.win, row("t/sub/r.yml")));
    assert_eq!(
        selected(cx, &h),
        Some((h.root.clone(), ip("t/sub/r.yml").to_string()))
    );
}

// A selection handed on by a rebuild no notice explains is the neighbour's for good, so a
// later move of the lost row's folder leaves the neighbour selected. The target holds a
// request, so a wrong reveal shifts the rows and the next rebuild lands elsewhere.
#[gpui_kit::test]
fn a_selection_lost_to_a_disk_delete_does_not_come_back_on_a_later_move(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("c/folder.yml", folder("C")));
    files.push(("c/k.yml", request("K", "http://x/k")));
    files.push(("e/folder.yml", folder("E")));
    files.push(("e/x.yml", request("X", "http://x/x")));
    let h = open_sidebar_with(cx, files);
    click(cx, &h, "e");
    click(cx, &h, "e/x.yml");
    std::fs::remove_file(h.root.join("e/x.yml")).unwrap();
    let changed = vec![h.root.join("e/x.yml")];
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Reload(changed), cx));
    cx.run_until_parked();
    frame(cx, h.win);
    let near = selected(cx, &h).expect("a neighbour takes the selection");
    assert!(
        !near.1.starts_with('e'),
        "the neighbour is outside e: {near:?}"
    );
    let mv = TreeOp::Move {
        item: ip("e"),
        to: DropTarget::Inside(ip("c")),
    };
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Tree(mv), cx));
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(h.root.join("c/e/folder.yml").exists());
    assert_eq!(selected(cx, &h), Some(near));
}

// The reveal after a move happens once, so collapsing the target afterwards sticks.
#[gpui_kit::test]
fn a_collapse_after_the_reveal_survives_a_later_rebuild(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("p/folder.yml", folder("P")));
    files.push(("p/sub/folder.yml", folder("Sub")));
    files.push(("p/sub/r.yml", request("R", "http://x/r")));
    files.push(("t/folder.yml", folder("T")));
    let h = open_sidebar_with(cx, files);
    click(cx, &h, "p");
    click(cx, &h, "p/sub");
    click(cx, &h, "p/sub/r.yml");
    let mv = TreeOp::Move {
        item: ip("p/sub"),
        to: DropTarget::Inside(ip("t")),
    };
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Tree(mv), cx));
    cx.run_until_parked();
    assert!(shown(cx, h.win, row("t/sub/r.yml")));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    click(cx, &h, "t");
    assert!(!shown(cx, h.win, row("t/sub/r.yml")), "t collapsed");
    let op = TreeOp::CreateFolder {
        parent: apinata_core::ItemPath::root(),
        name: "z".into(),
    };
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Tree(op), cx));
    cx.run_until_parked();
    assert!(h.root.join("z").is_dir());
    let expanded = h.view.read_with(cx, |s, _| s.expanded_paths(&h.root));
    assert!(!expanded.contains(&ip("t")), "{expanded:?}");
    assert!(!shown(cx, h.win, row("t/sub/r.yml")));
}

// The sidebar may rebuild more than once between the workspace's notify and the move's notice
// batch; a second rebuild that still sees the neighbour selected keeps the hand-off for it.
#[gpui_kit::test]
fn a_second_rebuild_before_the_move_notice_keeps_the_moved_request_selected(
    cx: &mut TestAppContext,
) {
    let mut files = sidebar_files();
    files.push(("admin/folder.yml", folder("Admin")));
    let h = open_sidebar_with(cx, files);
    click(cx, &h, "users");
    click(cx, &h, "users/list.yml");
    // Fires once, on the first notify after the move is on disk; this observer runs after the
    // sidebar's own and before the workspace's Notices event reaches it.
    let armed = std::rc::Rc::new(std::cell::Cell::new(true));
    let (win, view, flag) = (h.win, h.view.clone(), armed.clone());
    let moved = h.root.join("admin/users/list.yml");
    let _extra = cx.update(|cx| {
        cx.observe(&h.ws, move |_, cx| {
            if moved.exists() && flag.replace(false) {
                win.update(cx, |_, w, cx| {
                    view.update(cx, |s, cx| {
                        s.expand_all(&apinata_app::sidebar::ExpandAll, w, cx)
                    })
                })
                .unwrap();
            }
        })
    });
    let mv = TreeOp::Move {
        item: ip("users"),
        to: DropTarget::Inside(ip("admin")),
    };
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Tree(mv), cx));
    cx.run_until_parked();
    frame(cx, h.win);
    assert!(!armed.get(), "the extra rebuild ran");
    assert!(h.root.join("admin/users/list.yml").exists());
    assert_eq!(
        selected(cx, &h),
        Some((h.root.clone(), ip("admin/users/list.yml").to_string()))
    );
}
