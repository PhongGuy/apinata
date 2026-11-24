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
    use apinata_app::env_picker::{EnvPicker, NO_ENVIRONMENT};
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
                cx.new(|cx| EnvPicker::new(ws, win, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        sel.read_with(cx, |s, cx| s.checked(cx).0),
        SharedString::from("dev")
    );
    std::fs::remove_file(col.path().join("environments/dev.yml")).unwrap();
    wait_for(cx, "the select to drop dev", |cx| {
        sel.read_with(cx, |s, cx| s.checked(cx).0) == NO_ENVIRONMENT
    });
    let label = sel.read_with(cx, |s, cx| s.label(cx));
    assert_eq!(label, NO_ENVIRONMENT, "the button shows it too");
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

fn with_preset(default: &str, envs: &[&str]) -> tempfile::TempDir {
    let root =
        format!("{ROOT}extensions:\n  bruno:\n    presets:\n      defaultEnvironment: {default}\n");
    let files: Vec<(String, String)> = envs
        .iter()
        .map(|e| (format!("environments/{e}.yml"), format!("name: {e}\n")))
        .collect();
    let mut tree = vec![
        ("opencollection.yml", root.as_str()),
        (
            "a.yml",
            "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n",
        ),
    ];
    tree.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    write_tree(&tree)
}

// With nothing saved, the collection's preset default environment is selected on open.
#[gpui_kit::test]
fn the_preset_default_environment_is_selected_on_first_open(cx: &mut TestAppContext) {
    let col = with_preset("prod", &["dev", "prod"]);
    let (_state, ws, root) = open_dir(cx, &col);
    assert_eq!(chosen(cx, &ws, &root).as_deref(), Some("prod"));
    assert_eq!(saved(cx, &ws, &root).as_deref(), Some("prod"));
}

#[gpui_kit::test]
fn a_saved_choice_beats_the_preset_default(cx: &mut TestAppContext) {
    let col = with_preset("prod", &["dev", "prod"]);
    let (_state, ws, root) = start_with(cx, &col, "dev");
    assert_eq!(chosen(cx, &ws, &root).as_deref(), Some("dev"));
}

#[gpui_kit::test]
fn choosing_no_environment_beats_the_preset_default(cx: &mut TestAppContext) {
    let col = with_preset("prod", &["dev", "prod"]);
    let state = tempfile::tempdir().unwrap();
    let root = col.path().to_path_buf();
    let mut s = Session::default();
    let c = CollectionSession {
        no_environment: true,
        ..Default::default()
    };
    s.collections.insert(root.clone(), c);
    s.open.push(root.clone());
    let session = session_at(cx, &state, s);
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(chosen(cx, &ws, &root), None);
}

// A preset naming an environment the collection lacks is ignored, so the sole-one rule applies.
#[gpui_kit::test]
fn a_preset_naming_a_missing_environment_falls_back_to_the_sole_one(cx: &mut TestAppContext) {
    let col = with_preset("staging", &["dev"]);
    let (_state, ws, root) = open_dir(cx, &col);
    assert_eq!(chosen(cx, &ws, &root).as_deref(), Some("dev"));
}

// An import records its default environment, so the app opens in the one a run uses.
#[gpui_kit::test]
fn an_imported_collection_opens_in_the_environment_a_run_uses(cx: &mut TestAppContext) {
    use apinata_core::import::{CollectionOptions, parse_collections, write_collection};
    let input = serde_json::json!({"_type": "export", "__export_format": 4, "resources": [
        {"_id": "wrk_1", "_type": "workspace", "parentId": null, "name": "Pets"},
        {"_id": "env_1", "_type": "environment", "parentId": "wrk_1", "name": "Base Environment",
         "data": {"host": "http://127.0.0.1:1"}},
        {"_id": "env_2", "_type": "environment", "parentId": "env_1", "name": "staging",
         "data": {"host": "http://127.0.0.1:2"}},
        {"_id": "req_1", "_type": "request", "parentId": "wrk_1", "name": "Ping",
         "method": "GET", "url": "{{ _.host }}/ping"}
    ]})
    .to_string();
    let parent = tempfile::tempdir().unwrap();
    let bundles = parse_collections(None, input.as_bytes(), &CollectionOptions::default()).unwrap();
    let store = apinata_core::MemoryStore::new();
    let (c, report) = write_collection(
        parent.path(),
        &bundles[0].collection,
        &bundles[0].environments,
        &store,
    )
    .unwrap();
    assert_eq!(report.environments, ["Base Environment", "staging"]);
    let run_uses = c.choose_environment(None).name().map(str::to_owned);
    assert_eq!(run_uses.as_deref(), Some("Base Environment"));
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    let root = c.root().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    assert_eq!(chosen(cx, &ws, &root), run_uses);
}
