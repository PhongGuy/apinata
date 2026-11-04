use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_app::sidebar::drop::{DropZone, drop_target, drop_zone};
use apinata_app::sidebar::model::NodeKind;
use apinata_core::{DropTarget, ItemPath};
use gpui_kit::AppContext;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Bounds, Focusable, TestAppContext, point, px, size};
use support::*;

fn drag_onto(
    cx: &mut TestAppContext,
    h: &SidebarHarness,
    from: gpui_kit::SharedString,
    onto: &str,
    fraction: f32,
) {
    cx.update_window(h.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(from).bounds().center();
        let b = w.find(row(onto)).bounds();
        w.drag(
            start,
            point(b.center().x, b.origin.y + b.size.height * fraction),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

fn top_level(cx: &mut TestAppContext, h: &SidebarHarness) -> Vec<String> {
    let s = h.ws.read_with(cx, |w, cx| w.snapshot(&h.root, cx)).unwrap();
    s.content
        .rows
        .iter()
        .filter(|r| r.depth == 0)
        .map(|r| r.path.as_str().to_string())
        .collect()
}

#[test]
fn zones_split_the_row_in_quarters() {
    let b = Bounds::new(point(px(0.), px(100.)), size(px(200.), px(40.)));
    let zones: Vec<DropZone> = [104., 109., 110., 120., 130., 131.]
        .map(|y| drop_zone(px(y), b))
        .to_vec();
    use DropZone::*;
    assert_eq!(zones, [Before, Before, Inside, Inside, Inside, After]);
}

#[test]
fn targets_follow_the_row_kind_and_refuse_loops() {
    let (a, users, nested) = (ip("a.yml"), ip("users"), ip("users/list.yml"));
    let root = ItemPath::root();
    assert_eq!(
        drop_target(&a, &users, NodeKind::Folder, DropZone::Inside),
        Some(DropTarget::Inside(users.clone()))
    );
    assert_eq!(
        drop_target(&a, &users, NodeKind::Folder, DropZone::Before),
        Some(DropTarget::Before(users.clone()))
    );
    assert_eq!(
        drop_target(&a, &ip("b.yml"), NodeKind::Request, DropZone::Inside),
        Some(DropTarget::After(ip("b.yml")))
    );
    assert_eq!(
        drop_target(&a, &ip("b.yml"), NodeKind::Opaque, DropZone::Before),
        Some(DropTarget::Before(ip("b.yml")))
    );
    assert_eq!(
        drop_target(&nested, &root, NodeKind::Collection, DropZone::After),
        Some(DropTarget::Inside(root.clone()))
    );
    assert_eq!(
        drop_target(&a, &root, NodeKind::Collection, DropZone::Inside),
        None
    );
    assert_eq!(
        drop_target(&nested, &users, NodeKind::Folder, DropZone::Inside),
        None
    );
    assert_eq!(
        drop_target(&users, &users, NodeKind::Folder, DropZone::Before),
        None
    );
    assert_eq!(
        drop_target(
            &users,
            &ip("users/admin"),
            NodeKind::Folder,
            DropZone::Inside
        ),
        None
    );
    assert_eq!(
        drop_target(&users, &ip("usersx"), NodeKind::Folder, DropZone::Inside),
        Some(DropTarget::Inside(ip("usersx")))
    );
    assert_eq!(
        drop_target(&a, &ip("bad"), NodeKind::Invalid, DropZone::Inside),
        None
    );
}

#[gpui_kit::test]
fn the_middle_of_a_folder_moves_the_item_inside(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    drag_onto(cx, &h, row("health.yml"), "users", 0.5);
    assert!(h.root.join("users/health.yml").exists());
    assert!(!h.root.join("health.yml").exists());
    assert_eq!(h.view.read_with(cx, |s, _| s.hover()), None);
}

#[gpui_kit::test]
fn a_request_dropped_on_a_folder_takes_a_seq_past_its_highest(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    for (rel, text) in &mut files {
        if *rel == "users/list.yml" {
            *text = text.replace("seq: 1", "seq: 3");
        }
    }
    let h = open_sidebar_with(cx, files);
    drag_onto(cx, &h, row("health.yml"), "users", 0.5);
    let seq = |rel: &str| {
        let text = std::fs::read_to_string(h.root.join(rel)).unwrap();
        let line = text.lines().find(|l| l.starts_with("  seq: ")).unwrap();
        line["  seq: ".len()..].parse::<u64>().unwrap()
    };
    assert_eq!((seq("users/list.yml"), seq("users/health.yml")), (3, 4));
}

#[gpui_kit::test]
fn the_top_quarter_of_a_request_drops_before_it(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    drag_onto(cx, &h, row("status.yml"), "health.yml", 0.1);
    assert_eq!(
        top_level(cx, &h),
        ["users", "status.yml", "health.yml", "ws.yml"]
    );
}

#[gpui_kit::test]
fn the_middle_of_a_request_drops_after_it(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    drag_onto(cx, &h, row("health.yml"), "status.yml", 0.5);
    assert_eq!(
        top_level(cx, &h),
        ["users", "status.yml", "health.yml", "ws.yml"]
    );
}

#[gpui_kit::test]
fn a_moved_expanded_folder_stays_expanded(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("admin/folder.yml", folder("Admin")));
    let h = open_sidebar_with(cx, files);
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    assert!(shown(cx, h.win, row("users/list.yml")));
    // Sent as a command, so the move is tested apart from the drag gesture.
    let mv = TreeOp::Move {
        item: ip("users"),
        to: DropTarget::Inside(ip("admin")),
    };
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Tree(mv), cx));
    cx.run_until_parked();
    assert!(h.root.join("admin/users/list.yml").exists());
    // The selected folder follows the move, so its new parent opens to show it.
    assert!(shown(cx, h.win, row("admin/users/list.yml")));
}

#[gpui_kit::test]
fn a_drop_across_collections_does_nothing(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let other = collection();
    h.ws.update(cx, |w, cx| w.open(other.path().to_path_buf(), cx));
    frame(cx, h.win);
    let foreign = apinata_app::sidebar::model::node_id(1, &ip("a.yml"));
    drag_onto(cx, &h, foreign, "users", 0.5);
    assert!(other.path().join("a.yml").exists());
    assert!(!h.root.join("users/a.yml").exists());
}

#[gpui_kit::test]
fn dragging_a_folder_does_not_toggle_it(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("admin/folder.yml", folder("Admin")));
    let h = open_sidebar_with(cx, files);
    assert!(
        !shown(cx, h.win, row("users/list.yml")),
        "folders start collapsed"
    );
    cx.update_window(h.win, |_, w, cx| w.drag_to(row("users"), row("admin"), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        h.root.join("admin/users/list.yml").exists(),
        "the drag still moves the folder"
    );
    // A folder moved while expanded stays expanded, so open its new parent to look.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.update_window(h.win, |_, w, cx| w.click(row("admin"), cx))
        .unwrap();
    assert!(shown(cx, h.win, row("admin/users")));
    assert!(
        !shown(cx, h.win, row("admin/users/list.yml")),
        "a drag does not expand the folder"
    );
}

#[gpui_kit::test]
fn clicking_a_folder_focuses_the_tree(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |f, cx| f.focus(w, cx)))
        .unwrap();
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    let filter_focused = cx
        .update_window(h.win, |_, w, cx| filter.focus_handle(cx).is_focused(w))
        .unwrap();
    assert!(!filter_focused, "the click moves focus off the filter");
    // The tree answers arrow keys only while focused.
    cx.simulate_keystrokes(h.win, "down");
    let selected = h.view.read_with(cx, |s, cx| {
        s.tree().read(cx).selected_item().map(|i| i.id.clone())
    });
    assert_eq!(selected, Some(row("users/list.yml")));
}

#[gpui_kit::test]
fn a_click_still_toggles_and_selects_a_folder(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    assert!(shown(cx, h.win, row("users/list.yml")));
    let selected = h.view.read_with(cx, |s, cx| {
        s.tree().read(cx).selected_item().map(|i| i.id.clone())
    });
    assert_eq!(selected, Some(row("users")));
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    assert!(!shown(cx, h.win, row("users/list.yml")));
}
