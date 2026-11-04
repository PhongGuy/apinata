use crate::support;

use apinata_app::session::Session;
use apinata_app::shell::Shell;
use apinata_app::watch::set::{WatchFactory, notify_factory};
use apinata_app::workspace::Workspace;
use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};
use support::*;

fn shell_on(
    cx: &mut TestAppContext,
    root: &std::path::Path,
    factory: WatchFactory,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    cx.executor().allow_parking();
    support::init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.set_watch_factory(Some(factory), cx));
    let shell_ws = ws.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Shell::new(shell_ws, window, cx))
            })
        })
        .unwrap();
    ws.update(cx, |w, cx| w.open(root.to_path_buf(), cx));
    cx.run_until_parked();
    (state, win, ws)
}

fn files() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("users/u.yml", &request("u", "http://x/0")),
    ])
}

#[gpui_kit::test]
fn a_watch_failure_shows_a_notification_and_a_status_marker_that_reloads(cx: &mut TestAppContext) {
    let col = files();
    let root = col.path().to_path_buf();
    let (_state, win, ws) = shell_on(cx, &root, failing_on(root.join("users")));
    wait_for(cx, "the failure", |cx| {
        ws.read_with(cx, |w, _| w.watch_failed(&root).is_some())
    });
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert_eq!(
        cx.update_window(win, |_, w, cx| w.notifications(cx).len())
            .unwrap(),
        1
    );
    assert!(
        cx.update_window(win, |_, w, _| w.try_find(("unwatched", 0usize)).is_some())
            .unwrap()
    );
    std::fs::write(root.join("a.yml"), request("a", "http://x/disk")).unwrap();
    cx.update_window(win, |_, w, cx| w.click(("reload-collection", 0usize), cx))
        .unwrap();
    wait_for(cx, "the manual reload", |cx| {
        url(&ws_snap(cx, &ws, &root), "a.yml") == "http://x/disk"
    });
}

#[gpui_kit::test]
fn a_watched_collection_shows_no_marker(cx: &mut TestAppContext) {
    let col = files();
    let root = col.path().to_path_buf();
    let (_state, win, ws) = shell_on(cx, &root, notify_factory());
    wait_for(cx, "watching to start", |cx| {
        ws_snap(cx, &ws, &root).seq >= 2
    });
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let found = cx
        .update_window(win, |_, w, _| {
            (
                w.try_find("status-bar").is_some(),
                w.try_find(("unwatched", 0usize)).is_some(),
            )
        })
        .unwrap();
    assert_eq!(found, (true, false));
    assert_eq!(
        cx.update_window(win, |_, w, cx| w.notifications(cx).len())
            .unwrap(),
        0
    );
}

// A pipe beside the items: the first watch lists it, and the collection must stay in service.
#[cfg(unix)]
#[gpui_kit::test]
fn a_pipe_in_the_collection_leaves_it_in_service(cx: &mut TestAppContext) {
    let col = files();
    let root = col.path().to_path_buf();
    let pipe = root.join("t.yml");
    let made = std::process::Command::new("mkfifo")
        .arg(&pipe)
        .status()
        .unwrap();
    assert!(made.success());
    // The test's executor waits on a stuck actor, so writers let it go, as often as it reopens the
    // pipe, and the clock fails the test instead. With no reader the writer only waits.
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(10));
        while std::fs::OpenOptions::new().write(true).open(&pipe).is_ok() {}
    });
    let started = std::time::Instant::now();
    let (_state, _win, ws) = shell_on(cx, &root, notify_factory());
    wait_for(cx, "watching to start", |cx| {
        ws_snap(cx, &ws, &root).seq >= 2
    });
    std::fs::write(root.join("a.yml"), request("a", "http://x/disk")).unwrap();
    wait_for(cx, "the change on disk", |cx| {
        url(&ws_snap(cx, &ws, &root), "a.yml") == "http://x/disk"
    });
    assert!(
        started.elapsed() < std::time::Duration::from_secs(9),
        "the actor waited on the pipe for {:?}",
        started.elapsed()
    );
}
