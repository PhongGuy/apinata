use crate::support;

use apinata_app::command::Command;
use apinata_app::watch::set::notify_factory;
use apinata_app::workspace::WorkspaceEvent;
use gpui_kit::TestAppContext;
use std::sync::Arc;
use support::*;

fn files() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("users/u.yml", &request("u", "http://x/0")),
    ])
}

fn disk_edit(dir: &tempfile::TempDir) {
    std::fs::write(dir.path().join("a.yml"), request("a", "http://x/disk")).unwrap();
}

#[gpui_kit::test]
fn an_outside_edit_reaches_the_snapshot(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    disk_edit(&dir);
    wait_for(cx, "the disk url", |cx| {
        url(&ws_snap(cx, &w.ws, &w.root), "a.yml") == "http://x/disk"
    });
}

#[gpui_kit::test]
fn without_a_factory_nothing_is_watched(cx: &mut TestAppContext) {
    let dir = files();
    let w = watching(cx, &dir, None);
    disk_edit(&dir);
    settle(cx);
    let s = ws_snap(cx, &w.ws, &w.root);
    assert_eq!((s.seq, url(&s, "a.yml")), (1, "http://x/0".to_string()));
}

#[gpui_kit::test]
fn watching_starts_for_a_collection_already_open(cx: &mut TestAppContext) {
    let dir = files();
    let w = watching(cx, &dir, None);
    w.ws.update(cx, |ws, cx| {
        ws.set_watch_factory(Some(notify_factory()), cx)
    });
    wait_for(cx, "watching to start", |cx| {
        ws_snap(cx, &w.ws, &w.root).seq >= 2
    });
    disk_edit(&dir);
    wait_for(cx, "the disk url", |cx| {
        url(&ws_snap(cx, &w.ws, &w.root), "a.yml") == "http://x/disk"
    });
}

#[gpui_kit::test]
fn a_watch_that_cannot_be_placed_stops_watching_and_says_so(cx: &mut TestAppContext) {
    let dir = files();
    let seen_root = dir.path().to_path_buf();
    let w = watching(cx, &dir, None);
    let seen = events(cx, &w.ws);
    w.ws.update(cx, |ws, cx| {
        ws.set_watch_factory(Some(failing_on(seen_root.join("users"))), cx)
    });
    wait_for(cx, "the failure", |cx| {
        w.ws.read_with(cx, |ws, _| ws.watch_failed(&w.root).is_some())
    });
    let failed: Vec<_> = seen
        .borrow()
        .iter()
        .filter(|e| matches!(e, WorkspaceEvent::WatchFailed { .. }))
        .cloned()
        .collect();
    assert_eq!(
        failed,
        [WorkspaceEvent::WatchFailed {
            root: seen_root,
            message: "OS file watch limit reached.".into()
        }]
    );
    // The root's watch was placed before the failure; it must have stopped too.
    disk_edit(&dir);
    settle(cx);
    assert_eq!(url(&ws_snap(cx, &w.ws, &w.root), "a.yml"), "http://x/0");
    w.ws.update(cx, |ws, cx| ws.send(&w.root, Command::ReloadAll, cx));
    cx.run_until_parked();
    assert_eq!(url(&ws_snap(cx, &w.ws, &w.root), "a.yml"), "http://x/disk");
}

#[gpui_kit::test]
fn a_factory_error_is_a_watch_failure(cx: &mut TestAppContext) {
    let dir = files();
    let broken: apinata_app::watch::set::WatchFactory = Arc::new(|_| Err("no inotify".to_string()));
    let w = watching(cx, &dir, Some(broken));
    wait_for(cx, "the failure", |cx| {
        w.ws.read_with(cx, |ws, _| ws.watch_failed(&w.root) == Some("no inotify"))
    });
}
