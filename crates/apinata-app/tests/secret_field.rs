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
    assert_eq!(password(cx, &o), "hunter22");
    // The revert gave the name back, so moving again claims the same one.
    frame(cx, o.win);
    cx.update_window(o.win, |_, w, cx| w.click("auth-password-move", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(password(cx, &o), "{{a_password}}");
    run(cx, &o, Command::Revert(Target::Request(ip("a.yml"))));
    run(cx, &o, Command::Save(Target::Request(ip("a.yml"))));
    assert_eq!(stored(&o, "dev", "a_password"), None);
    assert!(!read(&o, "environments/dev.yml").contains("a_password"));
}

// A collection that holds an environment secret and a `.env` secret keeps both on save.
#[gpui_kit::test]
fn a_save_keeps_an_environment_secret_beside_a_dotenv_one(cx: &mut TestAppContext) {
    use apinata_app::command::{Edit, MOVE_SLOT};
    use apinata_core::{CollectionProxy, ProxyAuth, ProxyConfig};
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &format!("{}  auth: inherit\n", request("a", "http://127.0.0.1:9/x")),
        ),
        ("environments/dev.yml", "name: dev\n"),
    ]);
    let store = Arc::new(MemoryStore::new());
    let model = open_with(cx, &col, store.clone());
    send(cx, &model, Command::SelectEnvironment(Some("dev".into())));
    let edit = |edit| Command::Edit {
        target: Target::Collection,
        edit,
    };
    let bearer = |token: &str| AuthSpec::Bearer {
        token: token.into(),
    };
    send(cx, &model, edit(Edit::Auth(bearer("tok-value"))));
    send(
        cx,
        &model,
        edit(Edit::MoveToSecret(Box::new(bearer(MOVE_SLOT)))),
    );
    let mut proxy = ProxyConfig::new("http", "proxy.local", "3128");
    proxy.auth = Some(ProxyAuth::new("u", "proxy-pw"));
    send(
        cx,
        &model,
        edit(Edit::Proxy(CollectionProxy::Custom(proxy))),
    );
    cx.run_until_parked();
    let token = match snap(cx, &model).content.collection.auth.clone() {
        AuthSpec::Bearer { token } => token,
        other => panic!("not bearer: {other:?}"),
    };
    let name = token
        .trim_start_matches("{{")
        .trim_end_matches("}}")
        .to_owned();
    send(cx, &model, Command::Save(Target::Collection));
    cx.run_until_parked();

    let root = std::fs::canonicalize(col.path()).unwrap();
    let kept = store.get(&SecretKey::new(root, "dev", &name)).unwrap();
    assert_eq!(kept.as_deref(), Some("tok-value"), "{name}");
    let dotenv = std::fs::read_to_string(col.path().join(".env")).unwrap_or_default();
    assert!(dotenv.contains("proxy-pw"), "{dotenv}");
    let (reply, answer) = async_channel::bounded(1);
    send(
        cx,
        &model,
        Command::Variables {
            item: ip("a.yml"),
            reply,
        },
    );
    cx.run_until_parked();
    let vars = answer
        .try_recv()
        .expect("the actor replied")
        .expect("variables");
    assert!(vars.get(&name).is_some(), "{name} is undefined");
    assert!(
        vars.get("process.env.APINATA_PROXY_PASSWORD").is_some(),
        "the proxy password is undefined"
    );
}

// The Network fields: the proxy password and certificate passphrases use the same field, with
// `.env` as the only place a moved value goes.

use apinata_app::auth_section::SecretDestination;
use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::network_section::{CertField, NetworkField, NetworkSection};

// A collection file with a custom proxy whose password is `password` and one certificate
// whose passphrase is `passphrase`.
fn network_root(password: &str, passphrase: &str) -> String {
    format!(
        "{ROOT}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: proxy.local\n      port: \"3128\"\n      auth:\n        username: u\n        password: \"{password}\"\n      bypassProxy: \"\"\n  clientCertificates:\n    - domain: a.test\n      type: pem\n      certificateFilePath: a.pem\n      privateKeyFilePath: a.key\n      passphrase: \"{passphrase}\"\n"
    )
}

struct Net {
    col: tempfile::TempDir,
    _state: tempfile::TempDir,
    win: AnyWindowHandle,
    ed: Entity<ConfigEditor>,
}

/// Opens `files` with the collection settings on the Network tab.
fn open_network(cx: &mut TestAppContext, files: &[(&str, &str)]) -> Net {
    let col = write_tree(files);
    let (state, ws, root) = open_dir(cx, &col);
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (win, ed) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, root, ConfigTarget::Collection, w, cx))
            })
        })
        .unwrap();
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
    frame(cx, win);
    Net {
        col,
        _state: state,
        win: win.into(),
        ed,
    }
}

fn section(cx: &mut TestAppContext, n: &Net) -> Entity<NetworkSection> {
    n.ed.read_with(cx, |e, _| e.network().cloned())
        .expect("a network section")
}

// The text of the proxy password and the first passphrase, as their inputs hold it.
fn shown(cx: &mut TestAppContext, n: &Net) -> (String, String) {
    let s = section(cx, n);
    s.read_with(cx, |s, cx| {
        let pass = s.cert_input(0, CertField::Passphrase).expect("a row");
        (
            s.input(NetworkField::Password).read(cx).value().to_string(),
            pass.read(cx).value().to_string(),
        )
    })
}

fn label_of(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> Option<String> {
    let id = id.into();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn net_read(n: &Net, rel: &str) -> String {
    std::fs::read_to_string(n.col.path().join(rel)).unwrap_or_default()
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}");
}

const IN_FILE: &str = "Stored in the collection file";

#[gpui_kit::test]
fn network_literals_say_where_they_live_and_offer_the_move(cx: &mut TestAppContext) {
    let n = open_network(
        cx,
        &[("opencollection.yml", &network_root("hunter2", "pp-one"))],
    );
    assert_eq!(
        label_of(cx, n.win, "proxy-password-storage").as_deref(),
        Some(IN_FILE)
    );
    assert!(present(cx, n.win, "proxy-password-move"));
    assert_eq!(
        label_of(cx, n.win, ("cert-passphrase-storage", 0usize)).as_deref(),
        Some(IN_FILE)
    );
    assert!(present(cx, n.win, ("cert-passphrase-move", 0usize)));
    // The section-wide notice and its move are gone.
    assert!(!present(cx, n.win, "network-plain-notice"));
    assert!(!present(cx, n.win, "network-move-secrets"));
}

#[gpui_kit::test]
fn a_network_move_shows_the_reference_and_writes_dotenv_on_save(cx: &mut TestAppContext) {
    let n = open_network(
        cx,
        &[("opencollection.yml", &network_root("hunter2", "pp-one"))],
    );
    cx.update_window(n.win, |_, w, cx| w.click("proxy-password-move", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, n.win);
    let reference = "{{process.env.APINATA_PROXY_PASSWORD}}";
    // The field shows the reference, never the value, and only that field moved.
    assert_eq!(shown(cx, &n), (reference.to_owned(), "pp-one".to_owned()));
    assert_eq!(
        label_of(cx, n.win, "proxy-password-storage").as_deref(),
        Some("Saved to .env when you save")
    );
    assert!(!present(cx, n.win, "proxy-password-move"));
    assert!(present(cx, n.win, ("cert-passphrase-move", 0usize)));
    assert!(!n.col.path().join(".env").exists());

    n.ed.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    frame(cx, n.win);
    assert_eq!(net_read(&n, ".env"), "APINATA_PROXY_PASSWORD='hunter2'\n");
    let file = net_read(&n, "opencollection.yml");
    assert!(
        file.contains(reference) && !file.contains("hunter2") && file.contains("pp-one"),
        "{file}"
    );
    assert_eq!(
        label_of(cx, n.win, "proxy-password-storage").as_deref(),
        Some("Stored in .env")
    );
}

#[gpui_kit::test]
fn a_passphrase_moves_on_its_own(cx: &mut TestAppContext) {
    let n = open_network(
        cx,
        &[("opencollection.yml", &network_root("hunter2", "pp-one"))],
    );
    cx.update_window(n.win, |_, w, cx| {
        w.click(("cert-passphrase-move", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, n.win);
    assert_eq!(
        shown(cx, &n),
        (
            "hunter2".to_owned(),
            "{{process.env.APINATA_CERT_PASSPHRASE_1}}".to_owned()
        )
    );
    assert_eq!(
        label_of(cx, n.win, ("cert-passphrase-storage", 0usize)).as_deref(),
        Some("Saved to .env when you save")
    );
}

#[gpui_kit::test]
fn a_refused_network_move_leaves_the_draft_and_dotenv_alone(cx: &mut TestAppContext) {
    let col_files = [
        ("opencollection.yml", network_root("hunter2", "pp-one")),
        (".env", "KEEP=1\n".to_owned()),
    ];
    let files: Vec<(&str, &str)> = col_files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let col = write_tree(&files);
    git(col.path(), &["init", "-q"]);
    git(col.path(), &["add", ".env"]);
    let n = {
        let (state, ws, root) = open_dir(cx, &col);
        support::init(cx);
        cx.update(apinata_app::keymap::init);
        let (win, ed) = cx
            .update(|cx| {
                gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                    cx.new(|cx| ConfigEditor::new(ws, root, ConfigTarget::Collection, w, cx))
                })
            })
            .unwrap();
        ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
        frame(cx, win);
        Net {
            col,
            _state: state,
            win: win.into(),
            ed,
        }
    };
    cx.update_window(n.win, |_, w, cx| w.click("proxy-password-move", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, n.win);
    assert_eq!(shown(cx, &n), ("hunter2".to_owned(), "pp-one".to_owned()));
    let notice = section(cx, &n).read_with(cx, |s, _| s.notice().map(str::to_owned));
    assert_eq!(
        notice.as_deref(),
        Some(apinata_app::network_secrets::TRACKED)
    );
    assert!(present(cx, n.win, "proxy-password-move"));
    n.ed.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    assert_eq!(net_read(&n, ".env"), "KEEP=1\n");
    assert!(net_read(&n, "opencollection.yml").contains("hunter2"));
}

#[gpui_kit::test]
fn a_network_dotenv_reference_says_so_and_shows_the_reference(cx: &mut TestAppContext) {
    let reference = "{{process.env.APINATA_PROXY_PASSWORD}}";
    let n = open_network(
        cx,
        &[
            ("opencollection.yml", &network_root(reference, "")),
            (".env", "APINATA_PROXY_PASSWORD=hunter2\n"),
            (".gitignore", ".env\n"),
        ],
    );
    assert_eq!(
        label_of(cx, n.win, "proxy-password-storage").as_deref(),
        Some("Stored in .env")
    );
    assert!(!present(cx, n.win, "proxy-password-move"));
    assert!(!present(cx, n.win, "proxy-password-stored"));
    assert_eq!(shown(cx, &n).0, reference);
    // An empty passphrase has nothing to place.
    assert!(!present(cx, n.win, ("cert-passphrase-storage", 0usize)));
}

#[gpui_kit::test]
fn the_network_move_tip_names_dotenv_only(_cx: &mut TestAppContext) {
    let tip = SecretDestination::DotenvOnly.move_tip();
    assert!(tip.contains(".env") && !tip.contains("keychain"), "{tip}");
    let tip = SecretDestination::SecretOrDotenv.move_tip();
    assert!(tip.contains("keychain"), "{tip}");
}
