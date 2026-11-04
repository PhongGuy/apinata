use crate::support;

use apinata_app::command::Command;
use apinata_app::sidebar::model::invalid_id;
use apinata_app::sidebar::{SidebarEvent, marker_id};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use support::*;

#[gpui_kit::test]
fn method_labels_and_the_dirty_dot_follow_the_snapshot(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    assert!(shown(cx, h.win, marker_id("method", &row("health.yml"))));
    assert!(!shown(cx, h.win, marker_id("method", &row("ws.yml"))));
    let dot = marker_id("dirty", &row("health.yml"));
    assert!(!shown(cx, h.win, dot.clone()));
    h.ws.update(cx, |w, cx| {
        w.send(&h.root, edit_url("health.yml", "http://x/9"), cx)
    });
    assert!(shown(cx, h.win, dot));
}

#[gpui_kit::test]
fn dirty_items_gone_or_broken_on_disk_are_marked(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    for rel in ["health.yml", "status.yml"] {
        h.ws.update(cx, |w, cx| {
            w.send(&h.root, edit_url(rel, "http://x/mine"), cx)
        });
    }
    cx.run_until_parked();
    std::fs::remove_file(h.root.join("health.yml")).unwrap();
    std::fs::write(h.root.join("status.yml"), "info: [\n").unwrap();
    let changed = vec![h.root.join("health.yml"), h.root.join("status.yml")];
    h.ws.update(cx, |w, cx| w.send(&h.root, Command::Reload(changed), cx));
    assert!(shown(cx, h.win, marker_id("removed", &row("health.yml"))));
    assert!(shown(cx, h.win, marker_id("broken", &row("status.yml"))));
    assert!(!shown(cx, h.win, marker_id("broken", &row("health.yml"))));
}

#[gpui_kit::test]
fn clicking_a_request_opens_it_and_a_double_click_keeps_it(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("broken.yml", "info: [\n".to_string()));
    let h = open_sidebar_with(cx, files);
    let events = sidebar_events(cx, &h.view);
    for id in [row("ws.yml"), row("users"), invalid_id(0, 0)] {
        cx.update_window(h.win, |_, w, cx| w.click(id, cx)).unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
    }
    cx.run_until_parked();
    assert!(events.borrow().is_empty());
    cx.update_window(h.win, |_, w, cx| w.click(row("health.yml"), cx))
        .unwrap();
    cx.run_until_parked();
    let open = |rel: &str, permanent| SidebarEvent::OpenItem {
        root: h.root.clone(),
        path: ip(rel),
        permanent,
    };
    assert_eq!(events.borrow().as_slice(), [open("health.yml", false)]);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.update_window(h.win, |_, w, cx| w.double_click(row("status.yml"), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(events.borrow().last(), Some(&open("status.yml", true)));
    assert_eq!(
        h.view.read_with(cx, |s, cx| s.selected(cx)),
        Some((h.root.clone(), ip("status.yml")))
    );
}
