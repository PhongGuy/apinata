//! The shared secret field: every auth secret says where its value lives, and a literal moves
//! to a secret whose value is written only when the request is saved.

use crate::support;

use apinata_app::auth_section::AuthScope;
use apinata_app::command::{Command, Target};
use apinata_app::request_editor::Section;
use apinata_app::secret_storage::SecretStorage;
use apinata_app::snapshot::DotenvNames;
use apinata_app::workspace::Workspace;
use apinata_core::{AuthSpec, MemoryStore, SecretKey, SecretStore};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::path::PathBuf;
use std::sync::Arc;
use support::*;

struct Open {
    col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<Workspace>,
    root: PathBuf,
    win: AnyWindowHandle,
    store: Arc<MemoryStore>,
}

// A request `a` with Basic auth whose password is `password`.
fn basic(password: &str) -> String {
    format!(
        "{}  auth:\n    type: basic\n    username: u\n    password: \"{password}\"\n",
        request("a", "http://127.0.0.1:9/x")
    )
}

/// Opens `files` with `a.yml` in an editor on its Auth section, `env` selected.
fn open_auth(cx: &mut TestAppContext, files: &[(&str, &str)], env: Option<&str>) -> Open {
    let col = write_tree(files);
    let store = Arc::new(MemoryStore::new());
    let (state, ws, root) = open_dir_with(cx, &col, store.clone());
    if let Some(env) = env {
        let select = Command::SelectEnvironment(Some(env.to_owned()));
        ws.update(cx, |w, cx| w.send(&root, select, cx));
        cx.run_until_parked();
    }
    let (win, _editor) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    frame(cx, win);
    Open {
        col,
        _state: state,
        ws,
        root,
        win,
        store,
    }
}

// The accessible label of element `id`, after a fresh frame.
fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn password(cx: &mut TestAppContext, o: &Open) -> String {
    match &view_of(cx, &o.ws, &o.root, "a.yml").auth {
        AuthSpec::Basic { password, .. } => password.clone(),
        other => panic!("not basic: {other:?}"),
    }
}

fn run(cx: &mut TestAppContext, o: &Open, command: Command) {
    o.ws.update(cx, |w, cx| w.send(&o.root, command, cx));
    cx.run_until_parked();
}

fn read(o: &Open, rel: &str) -> String {
    std::fs::read_to_string(o.col.path().join(rel)).unwrap_or_default()
}

fn stored(o: &Open, env: &str, name: &str) -> Option<String> {
    let root = std::fs::canonicalize(&o.root).unwrap();
    o.store.get(&SecretKey::new(root, env, name)).unwrap()
}

const DEV_SECRET: &str =
    "name: dev\nvariables:\n  - name: tok\n    secret: true\n  - name: plain\n    value: p\n";

#[gpui_kit::test]
fn a_typed_password_says_it_is_in_the_request_file_and_offers_the_move(cx: &mut TestAppContext) {
    let o = open_auth(
        cx,
        &[("opencollection.yml", ROOT), ("a.yml", &basic("hunter22"))],
        None,
    );
    assert_eq!(
        label(cx, o.win, "auth-password-storage").as_deref(),
        Some("Stored in the request file")
    );
    assert!(present(cx, o.win, "auth-password-move"));
}

#[gpui_kit::test]
fn a_secret_variable_says_it_is_kept_in_the_keychain(cx: &mut TestAppContext) {
    let o = open_auth(
        cx,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &basic("{{tok}}")),
            ("environments/dev.yml", DEV_SECRET),
        ],
        Some("dev"),
    );
    assert_eq!(
        label(cx, o.win, "auth-password-storage").as_deref(),
        Some("Secret, kept in your keychain")
    );
    assert!(!present(cx, o.win, "auth-password-move"));
}

#[gpui_kit::test]
fn a_plain_environment_variable_says_it_is_in_the_environment_file(cx: &mut TestAppContext) {
    let o = open_auth(
        cx,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &basic("{{plain}}")),
            ("environments/dev.yml", DEV_SECRET),
        ],
        Some("dev"),
    );
    assert_eq!(
        label(cx, o.win, "auth-password-storage").as_deref(),
        Some("Stored in the environment file")
    );
}

#[gpui_kit::test]
fn a_dotenv_reference_says_it_is_in_dotenv(cx: &mut TestAppContext) {
    let o = open_auth(
        cx,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &basic("{{process.env.PW}}")),
            (".env", "PW=hunter22\n"),
        ],
        None,
    );
    assert_eq!(
        label(cx, o.win, "auth-password-storage").as_deref(),
        Some("Stored in .env")
    );
    assert!(!present(cx, o.win, "auth-password-move"));
}

#[gpui_kit::test]
fn the_hint_names_the_file_of_each_level(_cx: &mut TestAppContext) {
    let none = DotenvNames::default();
    let text = |scope| SecretStorage::of("pw", scope, None, &none).map(SecretStorage::text);
    assert_eq!(
        [
            text(AuthScope::Request),
            text(AuthScope::Folder),
            text(AuthScope::Collection)
        ],
        [
            Some("Stored in the request file"),
            Some("Stored in the folder file"),
            Some("Stored in the collection file")
        ]
    );
    assert_eq!(SecretStorage::of("", AuthScope::Request, None, &none), None);
    let mut held = DotenvNames::default();
    held.held.insert("PW".into());
    assert_eq!(
        SecretStorage::of("{{process.env.PW}}", AuthScope::Request, None, &held)
            .map(SecretStorage::text),
        Some("Saved to .env when you save")
    );
}

#[gpui_kit::test]
fn move_to_a_secret_keeps_the_value_in_the_environment_once_saved(cx: &mut TestAppContext) {
    let o = open_auth(
        cx,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &basic("hunter22")),
            ("environments/dev.yml", "name: dev\n"),
        ],
        Some("dev"),
    );
    cx.update_window(o.win, |_, w, cx| w.click("auth-password-move", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(password(cx, &o), "{{a_password}}");
    // Nothing is written before the save.
    assert!(!read(&o, "environments/dev.yml").contains("a_password"));
    assert_eq!(stored(&o, "dev", "a_password"), None);

    run(cx, &o, Command::Save(Target::Request(ip("a.yml"))));
    assert_eq!(stored(&o, "dev", "a_password").as_deref(), Some("hunter22"));
    let env = read(&o, "environments/dev.yml");
    assert!(
        env.contains("a_password") && !env.contains("hunter22"),
        "{env}"
    );
    assert!(!read(&o, "a.yml").contains("hunter22"));
    assert_eq!(
        label(cx, o.win, "auth-password-storage").as_deref(),
        Some("Secret, kept in your keychain")
    );
}

#[gpui_kit::test]
fn move_to_a_secret_without_an_environment_goes_to_an_ignored_dotenv(cx: &mut TestAppContext) {
    let o = open_auth(
        cx,
        &[("opencollection.yml", ROOT), ("a.yml", &basic("hunter22"))],
        None,
    );
    cx.update_window(o.win, |_, w, cx| w.click("auth-password-move", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(password(cx, &o), "{{process.env.APINATA_A_PASSWORD}}");
    assert_eq!(
        label(cx, o.win, "auth-password-storage").as_deref(),
        Some("Saved to .env when you save")
    );
    assert!(!o.col.path().join(".env").exists());

    run(cx, &o, Command::Save(Target::Request(ip("a.yml"))));
    let dotenv = read(&o, ".env");
    assert!(
        dotenv.contains("APINATA_A_PASSWORD") && dotenv.contains("hunter22"),
        "{dotenv}"
    );
    assert!(
        read(&o, ".gitignore").lines().any(|l| l.trim() == ".env"),
        "{}",
        read(&o, ".gitignore")
    );
    assert!(!read(&o, "a.yml").contains("hunter22"));
}

#[gpui_kit::test]
fn a_reverted_move_keeps_nothing(cx: &mut TestAppContext) {
    let o = open_auth(
        cx,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &basic("hunter22")),
            ("environments/dev.yml", "name: dev\n"),
        ],
        Some("dev"),
    );
    cx.update_window(o.win, |_, w, cx| w.click("auth-password-move", cx))
        .unwrap();
    cx.run_until_parked();
    run(cx, &o, Command::Revert(Target::Request(ip("a.yml"))));
    run(cx, &o, Command::Save(Target::Request(ip("a.yml"))));
    assert_eq!(password(cx, &o), "hunter22");
    assert_eq!(stored(&o, "dev", "a_password"), None);
    assert!(!read(&o, "environments/dev.yml").contains("a_password"));
}
