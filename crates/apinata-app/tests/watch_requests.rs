use crate::support;

use apinata_app::command::{Command, Target};
use apinata_app::snapshot::DiskState;
use apinata_app::workspace::WorkspaceEvent;
use gpui_kit::TestAppContext;
use support::*;

fn files() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
    ])
}

fn draft(cx: &mut TestAppContext, w: &Watched, rel: &str, to: &str) {
    w.ws.update(cx, |ws, cx| ws.send(&w.root, edit_url(rel, to), cx));
    cx.run_until_parked();
}

fn disk_of(cx: &mut TestAppContext, w: &Watched, rel: &str) -> DiskState {
    ws_snap(cx, &w.ws, &w.root).content.requests[&ip(rel)].disk
}

#[gpui_kit::test]
fn a_clean_item_takes_the_disk_content(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    std::fs::write(dir.path().join("a.yml"), request("a", "http://x/disk")).unwrap();
    wait_for(cx, "the disk url", |cx| {
        url(&ws_snap(cx, &w.ws, &w.root), "a.yml") == "http://x/disk"
    });
}

#[gpui_kit::test]
fn an_added_file_becomes_an_item(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    std::fs::write(dir.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "b.yml", |cx| {
        ws_snap(cx, &w.ws, &w.root)
            .content
            .requests
            .contains_key(&ip("b.yml"))
    });
}

#[gpui_kit::test]
fn a_seq_only_change_keeps_the_draft(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    draft(cx, &w, "a.yml", "http://x/draft");
    std::fs::write(
        dir.path().join("a.yml"),
        request("a", "http://x/0").replace("seq: 1", "seq: 7"),
    )
    .unwrap();
    wait_for(cx, "seq 7", |cx| {
        ws_snap(cx, &w.ws, &w.root).content.requests[&ip("a.yml")].seq == Some(7)
    });
    let s = ws_snap(cx, &w.ws, &w.root);
    let a = &s.content.requests[&ip("a.yml")];
    assert_eq!(
        (a.url.as_str(), a.dirty, a.disk),
        ("http://x/draft", true, DiskState::Present)
    );
}

#[gpui_kit::test]
fn a_changed_file_under_a_draft_is_marked_changed(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    draft(cx, &w, "a.yml", "http://x/draft");
    std::fs::write(dir.path().join("a.yml"), request("a", "http://x/disk")).unwrap();
    wait_for(cx, "Changed", |cx| {
        disk_of(cx, &w, "a.yml") == DiskState::Changed
    });
    assert_eq!(url(&ws_snap(cx, &w.ws, &w.root), "a.yml"), "http://x/draft");
}

#[gpui_kit::test]
fn a_deleted_file_under_a_draft_is_marked_removed(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    draft(cx, &w, "a.yml", "http://x/draft");
    std::fs::remove_file(dir.path().join("a.yml")).unwrap();
    wait_for(cx, "Removed", |cx| {
        disk_of(cx, &w, "a.yml") == DiskState::Removed
    });
    assert_eq!(url(&ws_snap(cx, &w.ws, &w.root), "a.yml"), "http://x/draft");
}

#[gpui_kit::test]
fn a_broken_file_under_a_draft_is_marked_broken(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    draft(cx, &w, "a.yml", "http://x/draft");
    std::fs::write(dir.path().join("a.yml"), "info: [").unwrap();
    wait_for(cx, "Broken", |cx| {
        disk_of(cx, &w, "a.yml") == DiskState::Broken
    });
}

#[gpui_kit::test]
fn the_apps_own_save_comes_back_as_no_change(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    let seen = events(cx, &w.ws);
    draft(cx, &w, "a.yml", "http://x/draft");
    w.ws.update(cx, |ws, cx| {
        ws.send(&w.root, Command::Save(Target::Request(ip("a.yml"))), cx)
    });
    cx.run_until_parked();
    let saved = ws_snap(cx, &w.ws, &w.root).seq;
    // The echo's reload publishes one more snapshot; wait for it, then check it changed nothing.
    wait_for(cx, "the echo", |cx| ws_snap(cx, &w.ws, &w.root).seq > saved);
    let s = ws_snap(cx, &w.ws, &w.root);
    let a = &s.content.requests[&ip("a.yml")];
    assert_eq!(
        (a.url.as_str(), a.dirty, a.disk),
        ("http://x/draft", false, DiskState::Present)
    );
    assert!(s.stale.is_empty());
    assert!(
        !seen
            .borrow()
            .iter()
            .any(|e| matches!(e, WorkspaceEvent::Notices { .. }))
    );
}
