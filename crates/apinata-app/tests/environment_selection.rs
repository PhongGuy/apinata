use crate::support;

use apinata_app::command::{Command, EnvOp};
use apinata_app::session::{CollectionSession, Session};
use apinata_app::workspace::Workspace;
use gpui_kit::{Entity, TestAppContext};
use std::path::{Path, PathBuf};
use support::*;

fn with_envs() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
        ("environments/prod.yml", "name: prod\n"),
    ])
}

fn saved(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> Option<String> {
    let session = ws.read_with(cx, |w, _| w.session().clone());
    session_value(cx, &session)
        .collections
        .get(root)
        .and_then(|c| c.environment.clone())
}

fn chosen(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> Option<String> {
    ws.read_with(cx, |w, cx| w.selected_environment(root, cx))
}

fn start_with(
    cx: &mut TestAppContext,
    col: &tempfile::TempDir,
    env: &str,
) -> (tempfile::TempDir, Entity<Workspace>, PathBuf) {
    let state = tempfile::tempdir().unwrap();
    let root = col.path().to_path_buf();
    let mut s = Session::default();
    let c = CollectionSession {
        environment: Some(env.into()),
        ..Default::default()
    };
    s.collections.insert(root.clone(), c);
    s.open.push(root.clone());
    let session = session_at(cx, &state, s);
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    (state, ws, root)
}

#[gpui_kit::test]
fn a_choice_reaches_the_actor_and_the_session(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = open_dir(cx, &col);
    assert_eq!(
        ws.read_with(cx, |w, cx| w.environments(&root, cx)),
        ["dev", "prod"]
    );
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("dev".into()), cx)
    })
    .expect("open");
    assert_eq!(
        saved(cx, &ws, &root),
        None,
        "the session waits for the acknowledging snapshot"
    );
    cx.run_until_parked();
    assert_eq!(chosen(cx, &ws, &root).as_deref(), Some("dev"));
    assert_eq!(saved(cx, &ws, &root).as_deref(), Some("dev"));
    ws.update(cx, |w, cx| w.select_environment(&root, None, cx));
    cx.run_until_parked();
    assert_eq!(
        (chosen(cx, &ws, &root), saved(cx, &ws, &root)),
        (None, None)
    );
}

#[gpui_kit::test]
fn the_saved_choice_is_restored_on_open(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = start_with(cx, &col, "prod");
    assert_eq!(chosen(cx, &ws, &root).as_deref(), Some("prod"));
    assert_eq!(saved(cx, &ws, &root).as_deref(), Some("prod"));
}

#[gpui_kit::test]
fn a_saved_choice_whose_file_is_gone_is_dropped(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = start_with(cx, &col, "staging");
    assert_eq!(
        (chosen(cx, &ws, &root), saved(cx, &ws, &root)),
        (None, None)
    );
}

#[gpui_kit::test]
fn renaming_or_deleting_the_chosen_environment_updates_the_session(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = start_with(cx, &col, "dev");
    let rename = Command::Env(EnvOp::Rename {
        from: "dev".into(),
        to: "qa".into(),
    });
    ws.update(cx, |w, cx| w.send(&root, rename, cx));
    cx.run_until_parked();
    assert_eq!(saved(cx, &ws, &root).as_deref(), Some("qa"));
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Env(EnvOp::Delete { name: "qa".into() }), cx)
    });
    cx.run_until_parked();
    assert_eq!(saved(cx, &ws, &root), None);
}

#[gpui_kit::test]
fn the_current_collection_is_the_active_tabs(cx: &mut TestAppContext) {
    let first = with_envs();
    let second = two_requests();
    let (_state, ws, root) = open_dir(cx, &first);
    let other = second.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(other.clone(), cx));
    cx.run_until_parked();
    assert_eq!(ws.read_with(cx, |w, _| w.current_root()), Some(root));
    ws.update(cx, |w, cx| w.open_item(&other, ip("b.yml"), false, cx))
        .expect("a request");
    assert_eq!(ws.read_with(cx, |w, _| w.current_root()), Some(other));
}

// Records the session's environment for `root` on every session change, so a transient
// rewrite between the opening snapshot and the acknowledging one is visible.
#[gpui_kit::test]
fn an_unacknowledged_snapshot_never_rewrites_the_saved_choice(cx: &mut TestAppContext) {
    let col = with_envs();
    let state = tempfile::tempdir().unwrap();
    let root = col.path().to_path_buf();
    let c = CollectionSession {
        environment: Some("prod".into()),
        ..Default::default()
    };
    let mut s = Session::default();
    s.collections.insert(root.clone(), c);
    s.open.push(root.clone());
    let session = session_at(cx, &state, s);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (log, key) = (seen.clone(), root.clone());
    let _watch = cx.update(|cx| {
        cx.observe(&session, move |p, cx| {
            let env = p
                .read(cx)
                .get()
                .collections
                .get(&key)
                .and_then(|c| c.environment.clone());
            log.borrow_mut().push(env);
        })
    });
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(chosen(cx, &ws, &root).as_deref(), Some("prod"));
    let seen = seen.borrow();
    assert!(!seen.is_empty(), "the session changed while opening");
    assert!(
        seen.iter().all(|e| e.as_deref() == Some("prod")),
        "saved choice rewritten: {seen:?}"
    );
}

#[gpui_kit::test]
fn a_selected_environment_deleted_on_disk_shows_no_environment(cx: &mut TestAppContext) {
    use apinata_app::env_selector::{EnvSelector, NO_ENVIRONMENT};
    use gpui_kit::{AppContext as _, SharedString, WindowOptions};
    support::init(cx);
    let col = with_envs();
    let w = watched(cx, &col);
    w.ws.update(cx, |ws, cx| {
        ws.select_environment(&w.root, Some("dev".into()), cx)
    })
    .expect("open");
    cx.run_until_parked();
    let ws = w.ws.clone();
    let (_win, sel) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |win, cx| {
                cx.new(|cx| EnvSelector::new(ws, win, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        sel.read_with(cx, |s, _| s.shown()),
        Some(SharedString::from("dev"))
    );
    std::fs::remove_file(col.path().join("environments/dev.yml")).unwrap();
    wait_for(cx, "the select to drop dev", |cx| {
        sel.read_with(cx, |s, _| s.shown()) == Some(SharedString::from(NO_ENVIRONMENT))
    });
    let value = sel.read_with(cx, |s, cx| s.select().read(cx).selected_value().cloned());
    assert_eq!(
        value,
        Some(SharedString::from(NO_ENVIRONMENT)),
        "the kit shows it too"
    );
}

fn with_one_env() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("environments/dev.yml", "name: dev\n"),
    ])
}

// However it is opened, a collection with one environment and no saved choice selects it.
#[gpui_kit::test]
fn a_sole_environment_is_selected_when_nothing_was_chosen(cx: &mut TestAppContext) {
    let col = with_one_env();
    let (_state, ws, root) = open_dir(cx, &col);
    assert_eq!(chosen(cx, &ws, &root).as_deref(), Some("dev"));
    assert_eq!(saved(cx, &ws, &root).as_deref(), Some("dev"));
}

// Picking No environment is a choice, so the next start keeps it instead of re-selecting.
#[gpui_kit::test]
fn choosing_no_environment_is_kept_over_the_sole_one(cx: &mut TestAppContext) {
    let col = with_one_env();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| w.select_environment(&root, None, cx));
    cx.run_until_parked();
    assert_eq!(chosen(cx, &ws, &root), None);
    let session = ws.read_with(cx, |w, _| w.session().clone());
    let mut kept = session_value(cx, &session);
    kept.open = vec![root.clone()];

    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, kept);
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(chosen(cx, &ws, &root), None);
}

// Two environments give no obvious pick, so none is selected.
#[gpui_kit::test]
fn several_environments_open_with_none_selected(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = open_dir(cx, &col);
    assert_eq!(chosen(cx, &ws, &root), None);
}

fn chose_none(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> bool {
    let session = ws.read_with(cx, |w, _| w.session().clone());
    (session_value(cx, &session).collections.get(root)).is_some_and(|c| c.no_environment)
}

// Deleting the selected environment clears it, but that is not the user picking No environment.
#[gpui_kit::test]
fn deleting_the_selected_environment_is_not_recorded_as_choosing_none(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = start_with(cx, &col, "dev");
    ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Env(EnvOp::Delete { name: "dev".into() }),
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(saved(cx, &ws, &root), None);
    assert!(!chose_none(cx, &ws, &root));
}

// Choosing No environment in the app is recorded as the user's pick.
#[gpui_kit::test]
fn picking_no_environment_is_recorded_as_a_choice(cx: &mut TestAppContext) {
    let col = with_envs();
    let (_state, ws, root) = start_with(cx, &col, "dev");
    ws.update(cx, |w, cx| w.select_environment(&root, None, cx));
    cx.run_until_parked();
    assert!(chose_none(cx, &ws, &root));
}
