//! Opening a folder with `workspace.yml`: its collections, and its environments in the picker.

use crate::support;

use apinata_app::command::Command;
use apinata_app::global_env_selector::GlobalEnvSelector;
use apinata_app::workspace::{Workspace, WorkspaceEvent};
use apinata_core::{VarScope, VariableSet};
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowOptions};
use std::path::{Path, PathBuf};
use support::*;

const WS: &str = "opencollection: 1.0.0\ninfo:\n  name: Team\n  type: workspace\ncollections:\n  - name: One\n    path: one\n  - name: Gone\n    path: gone\n  - name: Two\n    path: two\n";
const SHARED: &str = "name: shared\nvariables:\n  - name: host\n    value: from-workspace\n";
const PROFILE: &str = "name: mine\nvariables:\n  - name: host\n    value: from-profile\n";

fn col_files(prefix: &str) -> Vec<(String, String)> {
    vec![
        (format!("{prefix}/opencollection.yml"), ROOT.to_string()),
        (format!("{prefix}/a.yml"), request("a", "http://{{host}}/0")),
    ]
}

fn fixture(ws_yml: &str, collections: &[&str], envs: &[(&str, &str)]) -> tempfile::TempDir {
    let mut files: Vec<(String, String)> = vec![("workspace.yml".into(), ws_yml.into())];
    for c in collections {
        files.extend(col_files(c));
    }
    for (name, text) in envs {
        files.push((format!("environments/{name}.yml"), (*text).into()));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    write_tree(&refs)
}

struct Started {
    ws: Entity<Workspace>,
    session: Entity<apinata_app::persisted::Persisted<apinata_app::session::Session>>,
    _state: tempfile::TempDir,
    _profile: tempfile::TempDir,
}

fn start(cx: &mut TestAppContext) -> Started {
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = workspace(cx, &session);
    let profile = write_tree(&[("mine.yml", PROFILE)]);
    let dir = profile.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    Started {
        ws,
        session,
        _state: state,
        _profile: profile,
    }
}

fn open_path(cx: &mut TestAppContext, ws: &Entity<Workspace>, dir: &Path) {
    let dir = dir.to_path_buf();
    ws.update(cx, |w, cx| w.open_path(dir, cx));
    cx.run_until_parked();
}

fn roots(cx: &mut TestAppContext, ws: &Entity<Workspace>) -> Vec<PathBuf> {
    ws.read_with(cx, |w, _| {
        w.collections()
            .iter()
            .map(|c| c.root().to_path_buf())
            .collect()
    })
}

fn host(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
) -> Option<(VarScope, String)> {
    let (reply, answer) = async_channel::bounded(1);
    let item = ip("a.yml");
    ws.update(cx, |w, cx| {
        w.send(root, Command::Variables { item, reply }, cx)
    });
    cx.run_until_parked();
    let set: VariableSet = answer.try_recv().expect("replied").expect("built");
    set.get("host")
        .map(|i| (i.scope(), i.value().unwrap_or_default().to_owned()))
}

fn items(cx: &mut TestAppContext, ws: &Entity<Workspace>) -> Vec<String> {
    ws.read_with(cx, |w, _| w.global_picker_items())
}

#[gpui_kit::test]
fn a_workspace_folder_opens_its_existing_collections(cx: &mut TestAppContext) {
    let dir = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let s = start(cx);
    let seen = events(cx, &s.ws);
    open_path(cx, &s.ws, dir.path());
    let opened = roots(cx, &s.ws);
    assert_eq!(opened.len(), 2);
    assert!(opened.contains(&absolute(&dir.path().join("one"))));
    assert!(opened.contains(&absolute(&dir.path().join("two"))));
    let notices: Vec<String> = seen
        .borrow()
        .iter()
        .filter_map(|e| match e {
            WorkspaceEvent::FolderNotice(m) => Some(m.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("Gone"), "{notices:?}");
}

#[gpui_kit::test]
fn launch_roots_open_workspace_folders_and_collections(cx: &mut TestAppContext) {
    let dir = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let plain = collection();
    let s = start(cx);
    let (ws_dir, plain_dir) = (dir.path().to_path_buf(), plain.path().to_path_buf());
    s.ws.update(cx, |w, cx| w.open_roots(vec![ws_dir, plain_dir], cx));
    cx.run_until_parked();
    let opened = roots(cx, &s.ws);
    assert_eq!(opened.len(), 3, "{opened:?}");
    assert!(opened.contains(&absolute(&dir.path().join("one"))));
    assert!(opened.contains(&absolute(plain.path())));
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap()
}

#[gpui_kit::test]
fn workspace_environments_follow_the_profile_ones(cx: &mut TestAppContext) {
    let dir = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let s = start(cx);
    assert_eq!(items(cx, &s.ws), ["mine"]);
    open_path(cx, &s.ws, dir.path());
    assert_eq!(items(cx, &s.ws), ["mine", "shared (workspace)"]);
}

#[gpui_kit::test]
fn selecting_a_workspace_environment_broadcasts_without_persisting(cx: &mut TestAppContext) {
    let dir = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let s = start(cx);
    s.ws.update(cx, |w, cx| {
        w.select_global_environment(Some("mine".into()), cx)
    });
    open_path(cx, &s.ws, dir.path());
    s.ws.update(cx, |w, cx| w.select_picker_item("shared (workspace)", cx));
    cx.run_until_parked();
    for root in roots(cx, &s.ws) {
        assert_eq!(
            host(cx, &s.ws, &root),
            Some((VarScope::GlobalEnvironment, "from-workspace".into()))
        );
    }
    assert_eq!(
        s.ws.read_with(cx, |w, _| w.selected_picker_item())
            .as_deref(),
        Some("shared (workspace)")
    );
    let saved = session_value(cx, &s.session);
    assert_eq!(saved.global_environment.as_deref(), Some("mine"));

    s.ws.update(cx, |w, cx| w.select_picker_item("mine", cx));
    cx.run_until_parked();
    let root = roots(cx, &s.ws).remove(0);
    assert_eq!(
        host(cx, &s.ws, &root),
        Some((VarScope::GlobalEnvironment, "from-profile".into()))
    );
}

#[gpui_kit::test]
fn closing_the_last_workspace_collection_drops_its_environments(cx: &mut TestAppContext) {
    let dir = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let s = start(cx);
    open_path(cx, &s.ws, dir.path());
    s.ws.update(cx, |w, cx| w.select_picker_item("shared (workspace)", cx));
    cx.run_until_parked();
    let all = roots(cx, &s.ws);
    s.ws.update(cx, |w, cx| w.close(&all[0], cx));
    assert_eq!(items(cx, &s.ws), ["mine", "shared (workspace)"]);
    s.ws.update(cx, |w, cx| w.close(&all[1], cx));
    cx.run_until_parked();
    assert_eq!(items(cx, &s.ws), ["mine"]);
    assert_eq!(s.ws.read_with(cx, |w, _| w.selected_picker_item()), None);
}

#[gpui_kit::test]
fn opening_another_workspace_replaces_the_first(cx: &mut TestAppContext) {
    let first = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let second_yml =
        "info: {name: Other, type: workspace}\ncollections:\n  - {name: Three, path: three}\n";
    let second = fixture(second_yml, &["three"], &[("other", "name: other\n")]);
    let s = start(cx);
    open_path(cx, &s.ws, first.path());
    open_path(cx, &s.ws, second.path());
    assert_eq!(items(cx, &s.ws), ["mine", "other (workspace)"]);
}

#[gpui_kit::test]
fn a_malformed_workspace_file_opens_nothing_and_says_why(cx: &mut TestAppContext) {
    let dir = write_tree(&[("workspace.yml", "- not\n- a mapping\n")]);
    let s = start(cx);
    let seen = events(cx, &s.ws);
    open_path(cx, &s.ws, dir.path());
    assert!(roots(cx, &s.ws).is_empty());
    assert!(
        seen.borrow()
            .iter()
            .any(|e| matches!(e, WorkspaceEvent::FolderNotice(m) if m.contains("workspace.yml")))
    );
}

#[gpui_kit::test]
fn a_folder_with_both_files_opens_as_a_collection(cx: &mut TestAppContext) {
    let dir = fixture(WS, &[], &[]);
    std::fs::write(dir.path().join("opencollection.yml"), ROOT).unwrap();
    let s = start(cx);
    open_path(cx, &s.ws, dir.path());
    assert_eq!(roots(cx, &s.ws), [absolute(dir.path())]);
    assert_eq!(items(cx, &s.ws), ["mine"]);
}

#[gpui_kit::test]
fn the_picker_lists_workspace_environments(cx: &mut TestAppContext) {
    support::init(cx);
    let dir = fixture(WS, &["one"], &[("shared", SHARED)]);
    let s = start(cx);
    open_path(cx, &s.ws, dir.path());
    let shared = s.ws.clone();
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let sel = cx
        .update_window(win, |_, w, cx| {
            cx.new(|cx| GlobalEnvSelector::new(shared, w, cx))
        })
        .unwrap();
    let listed: Vec<String> = sel
        .read_with(cx, |s, cx| s.items(cx))
        .iter()
        .map(|i| i.to_string())
        .collect();
    assert!(
        listed.contains(&"shared (workspace)".to_string()),
        "{listed:?}"
    );
}

struct Blank;
impl gpui_kit::Render for Blank {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

#[gpui_kit::test]
fn the_effective_global_is_the_workspace_one_while_selected(cx: &mut TestAppContext) {
    let secret = "name: shared\nvariables:\n  - name: tok\n    value: s\n    secret: true\n";
    let dir = fixture(WS, &["one"], &[("shared", secret)]);
    let s = start(cx);
    s.ws.update(cx, |w, cx| {
        w.select_global_environment(Some("mine".into()), cx)
    });
    open_path(cx, &s.ws, dir.path());
    let name = |cx: &mut TestAppContext| {
        s.ws.read_with(cx, |w, _| w.effective_global().map(|(_, n)| n))
    };
    assert_eq!(name(cx).as_deref(), Some("mine"));
    s.ws.update(cx, |w, cx| w.select_picker_item("shared (workspace)", cx));
    let (envs, n) = s.ws.read_with(cx, |w, _| w.effective_global()).unwrap();
    assert_eq!(n, "shared");
    assert!(envs.environment("shared").unwrap().variables()[0].secret);
}

#[gpui_kit::test]
fn a_workspace_with_no_usable_collection_is_dropped(cx: &mut TestAppContext) {
    let yml = "info: {name: Empty, type: workspace}\ncollections:\n  - {name: Gone, path: gone}\n";
    let dir = fixture(yml, &[], &[("shared", SHARED)]);
    let s = start(cx);
    let seen = events(cx, &s.ws);
    open_path(cx, &s.ws, dir.path());
    assert_eq!(items(cx, &s.ws), ["mine"]);
    assert_eq!(
        s.ws.read_with(cx, |w, _| w.reference_workspace_name().map(str::to_owned)),
        None
    );
    assert!(
        seen.borrow()
            .iter()
            .any(|e| matches!(e, WorkspaceEvent::FolderNotice(_)))
    );
}

#[gpui_kit::test]
fn a_colliding_label_selects_the_workspace_environment(cx: &mut TestAppContext) {
    let dir = fixture(
        WS,
        &["one"],
        &[("x", "name: x\nvariables:\n  - name: host\n    value: ws\n")],
    );
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let w = workspace(cx, &session);
    let profile = write_tree(&[("x (workspace).yml", PROFILE)]);
    let p = profile.path().to_path_buf();
    w.update(cx, |w, cx| w.set_global_dir(p, cx));
    cx.run_until_parked();
    open_path(cx, &w, dir.path());
    w.update(cx, |w, cx| w.select_picker_item("x (workspace)", cx));
    let (_, n) = w.read_with(cx, |w, _| w.effective_global()).unwrap();
    assert_eq!(n, "x");
}

fn saved_workspaces(cx: &mut TestAppContext, s: &Started) -> Vec<PathBuf> {
    session_value(cx, &s.session).workspaces
}

#[gpui_kit::test]
fn restore_reopens_a_saved_workspace_with_its_environments(cx: &mut TestAppContext) {
    let dir = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let ws_dir = absolute(dir.path());
    let state = tempfile::tempdir().unwrap();
    let start = apinata_app::session::Session {
        workspaces: vec![ws_dir.clone()],
        open: vec![absolute(&dir.path().join("one"))],
        ..Default::default()
    };
    let session = session_at(cx, &state, start);
    let w = workspace(cx, &session);
    let profile = write_tree(&[("mine.yml", PROFILE)]);
    let p = profile.path().to_path_buf();
    w.update(cx, |w, cx| w.set_global_dir(p, cx));
    cx.run_until_parked();
    w.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(roots(cx, &w).len(), 2);
    assert_eq!(items(cx, &w), ["mine", "shared (workspace)"]);
    assert_eq!(session_value(cx, &session).workspaces, [ws_dir]);
}

#[test]
fn a_session_file_without_workspaces_loads_unchanged() {
    let old = r#"{"recent":["/a"],"open":["/a"],"sidebar_width":200.0}"#;
    let s: apinata_app::session::Session = serde_json::from_str(old).unwrap();
    assert!(s.workspaces.is_empty());
    assert_eq!(s.open, [PathBuf::from("/a")]);
    let out = serde_json::to_string(&s).unwrap();
    assert!(!out.contains("workspaces"), "{out}");
}

#[gpui_kit::test]
fn opening_a_workspace_records_it_and_closing_or_replacing_forgets_it(cx: &mut TestAppContext) {
    let a = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let b = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let s = start(cx);
    open_path(cx, &s.ws, a.path());
    assert_eq!(saved_workspaces(cx, &s), [absolute(a.path())]);
    open_path(cx, &s.ws, b.path());
    assert_eq!(saved_workspaces(cx, &s), [absolute(b.path())]);
    for root in roots(cx, &s.ws) {
        s.ws.update(cx, |w, cx| w.close(&root, cx));
    }
    cx.run_until_parked();
    assert!(saved_workspaces(cx, &s).is_empty());
}

#[gpui_kit::test]
fn closing_the_last_workspace_collection_leaves_nothing_in_the_session(cx: &mut TestAppContext) {
    let dir = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let s = start(cx);
    open_path(cx, &s.ws, dir.path());
    for root in roots(cx, &s.ws) {
        s.ws.update(cx, |w, cx| w.close(&root, cx));
    }
    cx.run_until_parked();
    let saved = session_value(cx, &s.session);
    assert!(saved.workspaces.is_empty());
    assert!(saved.open.is_empty(), "{:?}", saved.open);
    // A relaunch from this session restores nothing.
    let w = workspace(cx, &s.session);
    w.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert!(roots(cx, &w).is_empty());
}

#[gpui_kit::test]
fn restore_warns_once_about_a_vanished_workspace_and_opens_nothing_from_it(
    cx: &mut TestAppContext,
) {
    let state = tempfile::tempdir().unwrap();
    let gone = state.path().join("wcopy");
    let start = apinata_app::session::Session {
        workspaces: vec![gone.clone()],
        open: vec![gone.join("a"), gone.clone()],
        ..Default::default()
    };
    let session = session_at(cx, &state, start);
    let w = workspace(cx, &session);
    let seen = events(cx, &w);
    w.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert!(roots(cx, &w).is_empty());
    let seen = seen.borrow();
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, WorkspaceEvent::OpenFailed { .. }))
    );
    let notices: Vec<&String> = seen
        .iter()
        .filter_map(|e| match e {
            WorkspaceEvent::FolderNotice(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("wcopy"), "{notices:?}");
    let saved = session_value(cx, &session);
    assert!(saved.workspaces.is_empty());
    assert!(saved.open.is_empty(), "{:?}", saved.open);
}

#[gpui_kit::test]
fn closing_all_after_a_restore_leaves_nothing_in_the_session(cx: &mut TestAppContext) {
    let dir = fixture(WS, &["one", "two"], &[("shared", SHARED)]);
    let ws_dir = absolute(dir.path());
    let state = tempfile::tempdir().unwrap();
    let start = apinata_app::session::Session {
        workspaces: vec![ws_dir.clone()],
        open: vec![ws_dir.clone(), absolute(&dir.path().join("one"))],
        ..Default::default()
    };
    let session = session_at(cx, &state, start);
    let w = workspace(cx, &session);
    w.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    for root in roots(cx, &w) {
        w.update(cx, |w, cx| w.close(&root, cx));
    }
    cx.run_until_parked();
    let saved = session_value(cx, &session);
    assert!(saved.workspaces.is_empty());
    assert!(saved.open.is_empty(), "{:?}", saved.open);
}
