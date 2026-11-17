//! Team mode in the app: watched saves push, a conflict opens under the changed-on-disk
//! banner, copies from an earlier run open at start, and closing the window or a
//! collection pushes first.

use crate::git_support::{Sandbox, write};
use crate::support::*;

use std::path::PathBuf;
use std::time::Duration;

use apinata_app::command::{Command, Target};
use apinata_app::snapshot::DiskState;
use apinata_app::tabs::TabKind;
use apinata_app::watch::set::notify_factory;
use apinata_core::GitOptions;
use apinata_core::team::{Identity, Role, TeamAuth, TeamConfig, TeamState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};

/// `col` as main of a new bare origin, plus a teammate clone at `<sandbox>/mate`.
fn shared(sb: &Sandbox, col: &tempfile::TempDir) -> PathBuf {
    let bare = sb.path().join("origin.git");
    sb.git(
        sb.path(),
        &["init", "-q", "--bare", "-b", "main", "origin.git"],
    );
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(col.path(), "base");
    sb.git(
        col.path(),
        &["remote", "add", "origin", bare.to_str().unwrap()],
    );
    sb.git(col.path(), &["push", "-q", "-u", "origin", "main"]);
    sb.git(sb.path(), &["clone", "-q", bare.to_str().unwrap(), "mate"]);
    bare
}

fn origin_has(sb: &Sandbox, bare: &std::path::Path, rel: &str, text: &str) -> bool {
    let out = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(bare)
        .args(["show", &format!("main:{rel}")])
        .envs(sb.env())
        .output()
        .unwrap();
    out.status.success() && String::from_utf8_lossy(&out.stdout).contains(text)
}

fn copies_dir(s: &Started) -> PathBuf {
    s.files.path().join("conflicts").join("b1")
}
/// The real app over `col` with git and the watcher on, team mode not yet started.
fn app(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    cx.executor().allow_parking();
    let s = started(cx, col);
    s.ws.update(cx, |w, cx| {
        w.set_git(Some(sb.options()), cx);
        w.set_watch_factory(Some(notify_factory()), cx);
    });
    s
}

/// Start team mode on `s` and wait for its start pull to settle.
fn start_team(cx: &mut TestAppContext, sb: &Sandbox, s: &Started, debounce: Duration) {
    start_team_with(cx, sb.options(), s, debounce);
}

/// As `start_team`, with the sync loop's git calls run under `options`.
fn start_team_with(cx: &mut TestAppContext, options: GitOptions, s: &Started, debounce: Duration) {
    let config = TeamConfig::new(
        s.root.clone(),
        copies_dir(s),
        TeamAuth::new("http://127.0.0.1:9", "plk_test"),
        Identity::new("Ada", "ada@example.com"),
        Role::Write,
    )
    .base_options(options)
    .debounce_unclamped(debounce);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.start_team(&root, config, None, cx));
    wait_for(cx, "the start pull", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.team_status(&root)
                .is_some_and(|t| t.state == TeamState::Idle)
        })
    });
}

fn team_app(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    col: &tempfile::TempDir,
    debounce: Duration,
) -> Started {
    let s = app(cx, sb, col);
    start_team(cx, sb, &s, debounce);
    s
}

fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
    ])
}

#[gpui_kit::test]
fn a_watched_save_is_committed_and_pushed_as_the_user(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = collection();
    let bare = shared(&sb, &col);
    let _s = team_app(cx, &sb, &col, Duration::from_millis(50));
    std::fs::write(col.path().join("a.yml"), request("a", "http://x/mine")).unwrap();
    wait_for(cx, "the push", |_| {
        origin_has(&sb, &bare, "a.yml", "x/mine")
    });
}

#[gpui_kit::test]
fn a_conflict_opens_the_local_copy_under_the_changed_on_disk_banner(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = collection();
    let bare = shared(&sb, &col);
    let s = team_app(cx, &sb, &col, Duration::from_millis(50));
    let root = s.root.clone();
    let mate = sb.path().join("mate");
    write(&mate, "a.yml", &request("a", "http://x/theirs"));
    sb.commit_all(&mate, "theirs");
    sb.git(&mate, &["push", "-q"]);
    std::fs::write(col.path().join("a.yml"), request("a", "http://x/mine")).unwrap();
    wait_for(cx, "the copy", |_| copies_dir(&s).join("a.yml").is_file());
    // The copy is kept before the pull resets the tree, so the request can still be out of the
    // tree from the rebase's conflict markers; open it once the adopted copy is in the snapshot.
    wait_for(cx, "the copy adopted", |cx| {
        s.ws.read_with(cx, |w, cx| {
            w.request(&root, &ip("a.yml"), cx)
                .is_some_and(|r| r.disk == DiskState::Changed)
        })
    });
    let opened =
        s.ws.update(cx, |w, cx| w.open_item(&root, ip("a.yml"), false, cx));
    assert!(opened.is_some(), "a.yml did not open");
    wait_for(cx, "the banner", |cx| {
        frame(cx, s.win);
        shown(cx, s.win, "banner-changed")
    });
    assert!(
        std::fs::read_to_string(col.path().join("a.yml"))
            .unwrap()
            .contains("x/theirs")
    );
    cx.update_window(s.win, |_, w, cx| w.click("banner-overwrite", cx))
        .unwrap();
    wait_for(cx, "mine pushed", |_| {
        origin_has(&sb, &bare, "a.yml", "x/mine")
    });
    wait_for(cx, "the copy settled", |_| {
        !copies_dir(&s).join("a.yml").exists()
    });
}

/// A `git` first on the PATH that holds `rebase --abort` until `<gate>/go` exists, touching
/// `<gate>/waiting` once it holds, so a test can look at the tree a stopped rebase leaves.
#[cfg(unix)]
fn held_abort(gate: &std::path::Path) -> std::ffi::OsString {
    use std::os::unix::fs::PermissionsExt as _;
    let path = std::env::var_os("PATH").unwrap();
    let real = std::env::split_paths(&path)
        .map(|d| d.join("git"))
        .find(|g| g.is_file())
        .unwrap();
    let bin = gate.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = format!(
        "#!/bin/sh\ncase \" $* \" in *\" rebase --abort \"*)\n  touch '{g}/waiting'\n  i=0\n  while [ ! -e '{g}/go' ] && [ $i -lt 600 ]; do sleep 0.05; i=$((i+1)); done;;\nesac\nexec '{real}' \"$@\"\n",
        g = gate.display(),
        real = real.display(),
    );
    let wrapper = bin.join("git");
    std::fs::write(&wrapper, script).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&path))).unwrap()
}

fn tab_open(cx: &mut TestAppContext, s: &Started, item: &str) -> bool {
    s.ws.read_with(cx, |w, _| {
        (w.tabs().entries().iter())
            .any(|e| e.root == s.root && e.kind == TabKind::Request(ip(item)))
    })
}

// The rebase is held where it stopped on the conflict until the workspace has seen the
// conflict markers, so the half-merged tree has every chance to reach the open tab.
#[cfg(unix)]
#[gpui_kit::test]
fn an_open_request_keeps_its_tab_through_a_conflicted_pull(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = collection();
    shared(&sb, &col);
    let s = app(cx, &sb, &col);
    let root = s.root.clone();
    let gate = sb.path().join("gate");
    let options = sb.options().env("PATH", held_abort(&gate));
    start_team_with(cx, options, &s, Duration::from_millis(50));
    let opened =
        s.ws.update(cx, |w, cx| w.open_item(&root, ip("a.yml"), false, cx));
    assert!(opened.is_some(), "a.yml did not open");
    let mate = sb.path().join("mate");
    write(&mate, "a.yml", &request("a", "http://x/theirs"));
    sb.commit_all(&mate, "theirs");
    sb.git(&mate, &["push", "-q"]);
    std::fs::write(col.path().join("a.yml"), request("a", "http://x/mine")).unwrap();
    wait_for(cx, "the rebase to stop", |_| gate.join("waiting").exists());
    wait_for(cx, "the markers to reach the workspace", |cx| {
        s.ws.read_with(cx, |w, cx| {
            w.reloads_held(&root) || w.request(&root, &ip("a.yml"), cx).is_none()
        })
    });
    std::fs::write(gate.join("go"), "").unwrap();
    wait_for(cx, "the copy adopted", |cx| {
        s.ws.read_with(cx, |w, cx| {
            w.request(&root, &ip("a.yml"), cx)
                .is_some_and(|r| r.disk == DiskState::Changed)
        })
    });
    assert!(tab_open(cx, &s, "a.yml"), "the tab on a.yml closed");
    wait_for(cx, "the banner", |cx| {
        frame(cx, s.win);
        shown(cx, s.win, "banner-changed")
    });
}

// Unsaved edits on top of the conflicted save must outlive the half-merged tree too.
#[cfg(unix)]
#[gpui_kit::test]
fn unsaved_edits_survive_a_conflicted_pull(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = collection();
    shared(&sb, &col);
    let s = app(cx, &sb, &col);
    let root = s.root.clone();
    let gate = sb.path().join("gate");
    let options = sb.options().env("PATH", held_abort(&gate));
    start_team_with(cx, options, &s, Duration::from_millis(50));
    let opened =
        s.ws.update(cx, |w, cx| w.open_item(&root, ip("a.yml"), false, cx));
    assert!(opened.is_some(), "a.yml did not open");
    let mate = sb.path().join("mate");
    write(&mate, "a.yml", &request("a", "http://x/theirs"));
    sb.commit_all(&mate, "theirs");
    sb.git(&mate, &["push", "-q"]);
    s.ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://x/mine"), cx);
        w.send(&root, Command::Save(Target::Request(ip("a.yml"))), cx);
        w.send(&root, edit_url("a.yml", "http://x/draft"), cx);
    });
    wait_for(cx, "the rebase to stop", |_| gate.join("waiting").exists());
    wait_for(cx, "the markers to reach the workspace", |cx| {
        s.ws.read_with(cx, |w, cx| {
            w.reloads_held(&root)
                || (w.request(&root, &ip("a.yml"), cx)).is_none_or(|r| r.disk == DiskState::Broken)
        })
    });
    std::fs::write(gate.join("go"), "").unwrap();
    wait_for(cx, "the pull to settle", |cx| {
        s.ws.read_with(cx, |w, cx| {
            w.request(&root, &ip("a.yml"), cx)
                .is_some_and(|r| r.disk == DiskState::Changed)
        })
    });
    assert!(tab_open(cx, &s, "a.yml"), "the tab on a.yml closed");
    let view =
        s.ws.read_with(cx, |w, cx| w.request(&root, &ip("a.yml"), cx).unwrap());
    assert!(view.dirty, "the unsaved edit was lost");
    assert_eq!(view.url, "http://x/draft");
}

#[gpui_kit::test]
fn copies_left_from_an_earlier_run_open_when_team_mode_starts(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = collection();
    shared(&sb, &col);
    let s = app(cx, &sb, &col);
    let root = s.root.clone();
    std::fs::create_dir_all(copies_dir(&s)).unwrap();
    std::fs::write(copies_dir(&s).join("a.yml"), request("a", "http://x/mine")).unwrap();
    start_team(cx, &sb, &s, Duration::from_millis(50));
    s.ws.update(cx, |w, cx| {
        w.open_item(&root, ip("a.yml"), false, cx);
    });
    wait_for(cx, "the banner", |cx| {
        frame(cx, s.win);
        shown(cx, s.win, "banner-changed")
    });
}

#[gpui_kit::test]
fn a_settled_copy_in_a_folder_leaves_no_empty_folder_behind(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/a.yml", &request("a", "http://x/0")),
    ]);
    shared(&sb, &col);
    let s = app(cx, &sb, &col);
    let root = s.root.clone();
    std::fs::create_dir_all(copies_dir(&s).join("f")).unwrap();
    std::fs::write(
        copies_dir(&s).join("f").join("a.yml"),
        request("a", "http://x/mine"),
    )
    .unwrap();
    start_team(cx, &sb, &s, Duration::from_millis(50));
    s.ws.update(cx, |w, cx| {
        w.open_item(&root, ip("f/a.yml"), false, cx);
    });
    wait_for(cx, "the banner", |cx| {
        frame(cx, s.win);
        shown(cx, s.win, "banner-changed")
    });
    cx.update_window(s.win, |_, w, cx| w.click("banner-overwrite", cx))
        .unwrap();
    wait_for(cx, "the copy settled", |_| {
        !copies_dir(&s).join("f").join("a.yml").exists()
    });
    assert!(!copies_dir(&s).join("f").exists());
}

#[gpui_kit::test]
fn closing_the_window_pushes_before_it_closes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = collection();
    let bare = shared(&sb, &col);
    let s = team_app(cx, &sb, &col, Duration::from_secs(60));
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    let closed_at_once = VisualTestContext::from_window(s.win, cx).simulate_close();
    assert!(!closed_at_once, "the close waits for the flush");
    wait_for(cx, "the window to close", |cx| {
        cx.update(|cx| cx.windows().is_empty())
    });
    assert!(
        origin_has(&sb, &bare, "b.yml", "x/b"),
        "pushed before the window went"
    );
}

#[gpui_kit::test]
fn a_close_that_saves_unsaved_edits_pushes_them_before_it_closes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = collection();
    let bare = shared(&sb, &col);
    let s = team_app(cx, &sb, &col, Duration::from_secs(60));
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/saved"), cx)
    });
    cx.run_until_parked();
    assert!(!VisualTestContext::from_window(s.win, cx).simulate_close());
    cx.run_until_parked();
    assert!(confirm_open(cx), "the unsaved edit prompts");
    answer_confirm(cx, "confirm-save");
    wait_for(cx, "the window to close", |cx| {
        cx.update(|cx| cx.windows().is_empty())
    });
    assert!(
        origin_has(&sb, &bare, "a.yml", "x/saved"),
        "the saved edit was pushed first"
    );
}

#[gpui_kit::test]
fn closing_a_collection_pushes_its_pending_save(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = collection();
    let bare = shared(&sb, &col);
    let s = team_app(cx, &sb, &col, Duration::from_secs(60));
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.close(&root, cx));
    wait_for(cx, "the closed collection's push", |_| {
        origin_has(&sb, &bare, "b.yml", "x/b")
    });
}
