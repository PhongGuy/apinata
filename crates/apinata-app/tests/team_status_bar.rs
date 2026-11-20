//! The status bar shows each team collection's sync status, and a click opens its fix.

use crate::git_support::Sandbox;
use crate::support::team::*;
use crate::support::*;
use apinata_core::team::{Block, Role, TeamState};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use std::time::Duration;

const FAST: Duration = Duration::from_millis(50);

fn app(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    http_ready(cx);
    team_started(cx, sb, col, |_, _| {}).0
}

fn label(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.within("status-bar")
            .try_find(("team-status", 0usize))
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn name(s: &Started) -> String {
    s.root.file_name().unwrap().to_string_lossy().into_owned()
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn a_team_collection_shows_synced(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let want = format!("{}: synced", name(&s));
    wait_for(cx, "synced", |cx| {
        label(cx, &s).as_deref() == Some(want.as_str())
    });
}

#[gpui_kit::test]
fn offline_counts_the_unpushed_commits(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    sb.git(
        col.path(),
        &[
            "remote",
            "set-url",
            "origin",
            "http://127.0.0.1:1/git/t/w.git",
        ],
    );
    let s = app(cx, &sb, &col);
    let root = s.root.clone();
    let config = apinata_core::team::TeamConfig::new(
        root.clone(),
        copies_of(&s),
        apinata_core::team::TeamAuth::new("http://127.0.0.1:1", "plk_test"),
        apinata_core::team::Identity::new("Ada", "ada@example.com"),
        Role::Write,
    )
    .base_options(sb.options())
    .debounce_unclamped(FAST);
    s.ws.update(cx, |w, cx| w.start_team(&root, config, None, cx));
    // The start pull commits whatever is on disk, so the file arrives whole through a rename,
    // as the app saves: a half-written file would be committed twice.
    let staged = sb.path().join("b.yml");
    std::fs::write(&staged, request("b", "http://x/b")).unwrap();
    std::fs::rename(&staged, col.path().join("b.yml")).unwrap();
    let want = format!("{}: offline, 1 unpushed", name(&s));
    wait_for(cx, "offline", |cx| {
        label(cx, &s).as_deref() == Some(want.as_str())
    });
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_problem_toasts_once_and_the_status_opens_its_fix(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let synced = format!("{}: synced", name(&s));
    wait_for(cx, "synced", |cx| {
        label(cx, &s).as_deref() == Some(synced.as_str())
    });
    let before = toasts(cx, &s);
    let hook = col.path().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho nope >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    let problem = format!("{}: problem: commit hook refused", name(&s));
    wait_for(cx, "the problem", |cx| {
        label(cx, &s).as_deref() == Some(problem.as_str())
    });
    settle(cx);
    assert_eq!(toasts(cx, &s), before + 1);
    cx.update_window(s.win, |_, w, cx| {
        w.within("status-bar").click(("team-status", 0usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    let view = cx
        .update(|cx| apinata_app::team_sync_dialog::current(cx))
        .expect("the dialog");
    assert_eq!(view.read_with(cx, |v, _| v.root().to_path_buf()), s.root);
    assert!(matches!(
        s.ws.read_with(cx, |w, _| w.team_status(&s.root).map(|t| t.state)),
        Some(TeamState::Blocked(Block::Hook { .. }))
    ));
}

#[gpui_kit::test]
fn a_refused_edit_in_a_read_only_collection_toasts_once(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Read, FAST);
    let before = toasts(cx, &s);
    let root = s.root.clone();
    for url in ["http://x/1", "http://x/2"] {
        s.ws.update(cx, |w, cx| w.send(&root, edit_url("a.yml", url), cx));
        frame(cx, s.win);
    }
    assert_eq!(toasts(cx, &s), before + 1);
    let want = format!("{}: synced, read only", name(&s));
    assert_eq!(label(cx, &s).as_deref(), Some(want.as_str()));
}

#[gpui_kit::test]
fn a_plain_collection_has_no_sync_status(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    assert_eq!(label(cx, &s), None);
}

// The editor's own line says the folder is read only, so typing there is not the first cue.
#[gpui_kit::test]
fn a_read_only_collection_marks_the_request_breadcrumb(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    assert!(!shown(cx, s.win, "breadcrumb-read-only"));
    start_team_as(cx, &sb, &s, Role::Read, FAST);
    assert!(shown(cx, s.win, "breadcrumb-read-only"));
    let mark = cx
        .update_window(s.win, |_, w, _| {
            w.find("breadcrumb-read-only").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(mark.as_deref(), Some("Read only"));
}

fn synced(cx: &mut TestAppContext) -> (Sandbox, tempfile::TempDir, Started) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let want = format!("{}: synced", name(&s));
    wait_for(cx, "synced", |cx| {
        label(cx, &s).as_deref() == Some(want.as_str())
    });
    (sb, col, s)
}

fn dialog_root(cx: &mut TestAppContext) -> Option<std::path::PathBuf> {
    let view = cx.update(|cx| apinata_app::team_sync_dialog::current(cx))?;
    Some(view.read_with(cx, |v, _| v.root().to_path_buf()))
}

#[gpui_kit::test]
fn the_sync_status_takes_tab_and_enter_opens_its_dialog(cx: &mut TestAppContext) {
    use crate::keyboard_reach::{press, tab_to};
    let (_sb, _col, s) = synced(cx);
    let tip = cx
        .update_window(s.win, |_, w, _| {
            (w.within("status-bar").find(("team-status", 0usize)))
                .description()
                .map(str::to_owned)
        })
        .unwrap();
    let command = apinata_app::commands::label("collection.team_sync");
    assert_eq!(tip.as_deref(), Some(command));
    assert!(tab_to(cx, s.win, ("team-status", 0usize)), "Tab reaches it");
    press(cx, s.win, "enter");
    assert_eq!(dialog_root(cx), Some(s.root.clone()));
}

#[gpui_kit::test]
fn the_palette_opens_the_current_collections_sync_dialog(cx: &mut TestAppContext) {
    let (_sb, _col, s) = synced(cx);
    let label = apinata_app::commands::label("collection.team_sync");
    assert_eq!(label, "Team sync…");
    let entry = (apinata_app::palette::palette_items().into_iter())
        .find(|e| e.label == label)
        .expect("Team sync is in the palette");
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(entry.action.boxed_clone(), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(dialog_root(cx), Some(s.root.clone()));
}
