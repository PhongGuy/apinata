use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::snapshot::DiskState;
use gpui_kit::TestAppContext;
use support::*;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n";

fn files() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/u.yml", &request("u", "http://x/0")),
        ("users/v.yml", &request("v", "http://x/0")),
        ("environments/dev.yml", DEV),
    ])
}

fn apply(cx: &mut TestAppContext, w: &Watched, command: Command) {
    w.ws.update(cx, |ws, cx| ws.send(&w.root, command, cx));
    cx.run_until_parked();
}

fn folder_disk(cx: &mut TestAppContext, w: &Watched) -> Option<DiskState> {
    ws_snap(cx, &w.ws, &w.root)
        .content
        .rows
        .iter()
        .find(|r| r.path == ip("users"))
        .map(|r| r.disk)
}

#[gpui_kit::test]
fn a_new_folder_is_walked_and_then_watched(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    std::fs::create_dir(dir.path().join("n")).unwrap();
    std::fs::write(dir.path().join("n/r.yml"), request("r", "http://x/0")).unwrap();
    wait_for(cx, "n/r.yml", |cx| {
        ws_snap(cx, &w.ws, &w.root)
            .content
            .requests
            .contains_key(&ip("n/r.yml"))
    });
    // The scheduler has run the directory update and its gap reload by now; settle, so only the live watch can deliver this write.
    settle(cx);
    std::fs::write(dir.path().join("n/r.yml"), request("r", "http://x/later")).unwrap();
    wait_for(cx, "the later url", |cx| {
        url(&ws_snap(cx, &w.ws, &w.root), "n/r.yml") == "http://x/later"
    });
}

#[gpui_kit::test]
fn a_removed_directory_keeps_dirty_drafts_and_is_watched_again_after_a_save(
    cx: &mut TestAppContext,
) {
    let dir = files();
    let w = watched(cx, &dir);
    apply(cx, &w, edit_url("users/u.yml", "http://x/draft"));
    std::fs::remove_dir_all(dir.path().join("users")).unwrap();
    wait_for(cx, "a removed folder", |cx| {
        folder_disk(cx, &w) == Some(DiskState::Removed)
    });
    let s = ws_snap(cx, &w.ws, &w.root);
    assert_eq!(url(&s, "users/u.yml"), "http://x/draft");
    assert!(
        !s.content.requests.contains_key(&ip("users/v.yml")),
        "the clean item drops"
    );
    apply(cx, &w, Command::Save(Target::Request(ip("users/u.yml"))));
    wait_for(cx, "the folder back", |cx| {
        folder_disk(cx, &w) == Some(DiskState::Present)
    });
    settle(cx);
    std::fs::write(
        dir.path().join("users/u.yml"),
        request("u", "http://x/outside"),
    )
    .unwrap();
    wait_for(cx, "the outside url", |cx| {
        url(&ws_snap(cx, &w.ws, &w.root), "users/u.yml") == "http://x/outside"
    });
}

#[gpui_kit::test]
fn a_directory_replaced_within_one_window_is_reread(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    let users = dir.path().join("users");
    std::fs::remove_dir_all(&users).unwrap();
    std::fs::create_dir(&users).unwrap();
    std::fs::write(users.join("u.yml"), request("u", "http://x/new")).unwrap();
    wait_for(cx, "the new url", |cx| {
        ws_snap(cx, &w.ws, &w.root)
            .content
            .requests
            .get(&ip("users/u.yml"))
            .is_some_and(|u| u.url == "http://x/new")
    });
    let s = ws_snap(cx, &w.ws, &w.root);
    assert!(
        !s.content.requests.contains_key(&ip("users/v.yml")),
        "a file the replacement dropped is gone"
    );
}

#[gpui_kit::test]
fn a_broken_folder_file_under_a_draft_is_marked_broken(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    apply(
        cx,
        &w,
        Command::Edit {
            target: Target::Folder(ip("users")),
            edit: Edit::Docs("mine".into()),
        },
    );
    std::fs::write(dir.path().join("users/folder.yml"), "info: [").unwrap();
    wait_for(cx, "Broken", |cx| {
        ws_snap(cx, &w.ws, &w.root).content.folders[&ip("users")].disk == DiskState::Broken
    });
    assert_eq!(
        ws_snap(cx, &w.ws, &w.root).content.folders[&ip("users")].docs,
        "mine"
    );
}

#[gpui_kit::test]
fn an_environment_removed_under_a_draft_is_marked_removed(cx: &mut TestAppContext) {
    let dir = files();
    let w = watched(cx, &dir);
    let color = Edit::Color(Some("#ff0000".into()));
    apply(
        cx,
        &w,
        Command::Edit {
            target: Target::Environment("dev".into()),
            edit: color,
        },
    );
    std::fs::remove_file(dir.path().join("environments/dev.yml")).unwrap();
    wait_for(cx, "Removed", |cx| {
        ws_snap(cx, &w.ws, &w.root)
            .content
            .environments
            .iter()
            .any(|e| e.name == "dev" && e.disk == DiskState::Removed)
    });
}

#[gpui_kit::test]
fn an_environments_directory_created_later_is_watched(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let w = watched(cx, &dir);
    std::fs::create_dir(dir.path().join("environments")).unwrap();
    std::fs::write(dir.path().join("environments/dev.yml"), DEV).unwrap();
    wait_for(cx, "dev", |cx| {
        ws_snap(cx, &w.ws, &w.root)
            .content
            .environments
            .iter()
            .any(|e| e.name == "dev")
    });
    std::fs::write(
        dir.path().join("environments/stage.yml"),
        DEV.replace("dev", "stage"),
    )
    .unwrap();
    wait_for(cx, "stage", |cx| {
        ws_snap(cx, &w.ws, &w.root)
            .content
            .environments
            .iter()
            .any(|e| e.name == "stage")
    });
}
