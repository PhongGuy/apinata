//! Network edits on the collection: literal secrets go to `.env`, never into the file.

use crate::support;

use apinata_app::command::{Command, Edit, Field, NoticeKind, Target};
use apinata_app::model::CollectionModel;
use apinata_core::{ClientCertificate, CollectionNetwork, CollectionProxy, ProxyAuth, ProxyConfig};
use gpui_kit::{Entity, TestAppContext};
use std::path::Path;
use std::process::Command as Process;
use support::*;
use tempfile::TempDir;

fn root_with_password(password: &str) -> String {
    format!(
        "{ROOT}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: proxy.local\n      port: \"3128\"\n      auth:\n        username: u\n        password: \"{password}\"\n      bypassProxy: \"\"\n"
    )
}

fn custom(host: &str, password: &str) -> CollectionProxy {
    let mut p = ProxyConfig::new("http", host, "3128");
    p.auth = Some(ProxyAuth::new("u", password));
    CollectionProxy::Custom(p)
}

fn password_of(proxy: &CollectionProxy) -> String {
    match proxy {
        CollectionProxy::Custom(p) => p
            .auth
            .as_ref()
            .map(|a| a.password.clone())
            .unwrap_or_default(),
        other => panic!("not a custom proxy: {other:?}"),
    }
}

fn network(cx: &mut TestAppContext, model: &Entity<CollectionModel>) -> CollectionNetwork {
    snap(cx, model)
        .content
        .collection
        .network
        .clone()
        .expect("the collection's network")
}

fn apply(cx: &mut TestAppContext, model: &Entity<CollectionModel>, edit: Edit) {
    send(
        cx,
        model,
        Command::Edit {
            target: Target::Collection,
            edit,
        },
    );
    cx.run_until_parked();
}

fn save(cx: &mut TestAppContext, model: &Entity<CollectionModel>) {
    send(cx, model, Command::Save(Target::Collection));
    cx.run_until_parked();
}

fn read(dir: &TempDir, rel: &str) -> String {
    std::fs::read_to_string(dir.path().join(rel)).unwrap_or_default()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Process::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}");
}

#[gpui_kit::test]
fn a_new_literal_moves_to_dotenv_and_dotenv_is_ignored(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let model = open(cx, &dir);
    apply(cx, &model, Edit::Proxy(custom("proxy.local", "hunter2")));
    assert_eq!(
        password_of(&network(cx, &model).proxy),
        "{{process.env.APINATA_PROXY_PASSWORD}}"
    );
    // Nothing reaches the disk before the save.
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
    assert!(notices(cx, &model).is_empty());
    assert!(!snap(cx, &model).content.collection.plain_secrets);
    save(cx, &model);
    assert_eq!(read(&dir, ".env"), "APINATA_PROXY_PASSWORD='hunter2'\n");
    assert_eq!(read(&dir, ".gitignore"), ".env\n");
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::Warnings(vec![
            "Added .env to .gitignore".into()
        ])]
    );
    let saved = read(&dir, "opencollection.yml");
    assert!(
        saved.contains("{{process.env.APINATA_PROXY_PASSWORD}}") && !saved.contains("hunter2"),
        "{saved}"
    );
}

#[gpui_kit::test]
fn a_template_is_written_as_is(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let model = open(cx, &dir);
    apply(cx, &model, Edit::Proxy(custom("proxy.local", "{{pw}}")));
    assert_eq!(password_of(&network(cx, &model).proxy), "{{pw}}");
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
    assert!(notices(cx, &model).is_empty());
}

#[gpui_kit::test]
fn an_unclosed_brace_pair_is_a_secret_not_a_template(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let model = open(cx, &dir);
    apply(cx, &model, Edit::Proxy(custom("proxy.local", "ab{{c")));
    assert_eq!(
        password_of(&network(cx, &model).proxy),
        "{{process.env.APINATA_PROXY_PASSWORD}}"
    );
    save(cx, &model);
    assert_eq!(read(&dir, ".env"), "APINATA_PROXY_PASSWORD='ab{{c'\n");
}

#[gpui_kit::test]
fn an_existing_literal_stays_until_moved(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", &root_with_password("hunter2"))]);
    let model = open(cx, &dir);
    assert!(snap(cx, &model).content.collection.plain_secrets);
    apply(cx, &model, Edit::Proxy(custom("proxy2.local", "hunter2")));
    assert_eq!(password_of(&network(cx, &model).proxy), "hunter2");
    assert!(!dir.path().join(".env").exists());
    apply(cx, &model, Edit::MoveNetworkSecrets);
    assert_eq!(
        password_of(&network(cx, &model).proxy),
        "{{process.env.APINATA_PROXY_PASSWORD}}"
    );
    assert!(!snap(cx, &model).content.collection.plain_secrets);
    assert!(!dir.path().join(".env").exists());
    save(cx, &model);
    assert!(read(&dir, ".env").contains("APINATA_PROXY_PASSWORD='hunter2'"));
}

#[gpui_kit::test]
fn a_reference_keeps_its_name_and_taken_names_get_a_suffix(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        (
            "opencollection.yml",
            &root_with_password("{{process.env.MY_PW}}"),
        ),
        (".env", "APINATA_PROXY_PASSWORD=other\nMY_PW=old\n"),
        (".gitignore", "/.env\n"),
    ]);
    let model = open(cx, &dir);
    apply(cx, &model, Edit::Proxy(custom("proxy.local", "new")));
    assert_eq!(
        password_of(&network(cx, &model).proxy),
        "{{process.env.MY_PW}}"
    );
    assert_eq!(
        read(&dir, ".env"),
        "APINATA_PROXY_PASSWORD=other\nMY_PW=old\n"
    );
    save(cx, &model);
    assert_eq!(
        read(&dir, ".env"),
        "APINATA_PROXY_PASSWORD=other\nMY_PW='new'\n"
    );
    assert_eq!(read(&dir, ".gitignore"), "/.env\n");
    assert!(notices(cx, &model).is_empty());
    let mut locked = ClientCertificate::pem("a.test", "c.pem", "k.pem");
    locked.passphrase = "pp".into();
    let open_cert = ClientCertificate::pem("b.test", "c2.pem", "k2.pem");
    apply(
        cx,
        &model,
        Edit::ClientCertificates(vec![open_cert, locked]),
    );
    let certs = network(cx, &model).certificates;
    assert_eq!(certs[0].passphrase, "");
    assert_eq!(
        certs[1].passphrase,
        "{{process.env.APINATA_CERT_PASSPHRASE_2}}"
    );
    save(cx, &model);
    assert!(read(&dir, ".env").ends_with("APINATA_CERT_PASSPHRASE_2='pp'\n"));
    let dir2 = write_tree(&[
        ("opencollection.yml", ROOT),
        (".env", "APINATA_PROXY_PASSWORD=other\n"),
        (".gitignore", ".env\n"),
    ]);
    let model2 = open(cx, &dir2);
    apply(cx, &model2, Edit::Proxy(custom("proxy.local", "x")));
    assert_eq!(
        password_of(&network(cx, &model2).proxy),
        "{{process.env.APINATA_PROXY_PASSWORD_2}}"
    );
}

#[gpui_kit::test]
fn a_tracked_dotenv_refuses_and_writes_nothing(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT), (".env", "X=1\n")]);
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", ".env"]);
    let model = open(cx, &dir);
    apply(cx, &model, Edit::Proxy(custom("proxy.local", "hunter2")));
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::Failed {
            target: Some(Target::Collection),
            message: ".env is tracked by git, so nothing was saved; untrack .env first".into(),
            row: None
        }]
    );
    assert_eq!(network(cx, &model).proxy, CollectionProxy::Inherit);
    assert_eq!(read(&dir, ".env"), "X=1\n");
    assert!(!dir.path().join(".gitignore").exists());
}

#[gpui_kit::test]
fn a_stale_network_edit_is_held_and_writes_no_dotenv(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let model = open(cx, &dir);
    std::fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: renamed\n",
    )
    .unwrap();
    send(
        cx,
        &model,
        Command::Reload(vec![dir.path().join("opencollection.yml")]),
    );
    apply(cx, &model, Edit::Proxy(custom("proxy.local", "hunter2")));
    assert_eq!(notices(cx, &model), [NoticeKind::Stale(Target::Collection)]);
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
}

#[gpui_kit::test]
fn folders_have_no_network(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
    ]);
    let model = open(cx, &dir);
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Folder(ip("users")),
            edit: Edit::Proxy(CollectionProxy::Off),
        },
    );
    cx.run_until_parked();
    assert_eq!(
        notices(cx, &model),
        [NoticeKind::Unsupported {
            target: Target::Folder(ip("users")),
            field: Field::Network
        }]
    );
    assert!(
        snap(cx, &model).content.folders[&ip("users")]
            .network
            .is_none()
    );
}

#[gpui_kit::test]
fn an_unquotable_secret_is_refused_and_writes_nothing(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let model = open(cx, &dir);
    let mut ok = ClientCertificate::pem("a.test", "c.pem", "k.pem");
    ok.passphrase = "fine".into();
    let mut bad = ClientCertificate::pem("b.test", "c2.pem", "k2.pem");
    bad.passphrase = "'`\"".into();
    apply(cx, &model, Edit::ClientCertificates(vec![ok, bad]));
    let notices = notices(cx, &model);
    let [
        NoticeKind::Failed {
            target: Some(Target::Collection),
            message,
            ..
        },
    ] = notices.as_slice()
    else {
        panic!("{notices:?}");
    };
    assert!(
        message.contains(".env") && !message.contains("fine"),
        "{message}"
    );
    assert!(network(cx, &model).certificates.is_empty());
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
}

#[gpui_kit::test]
fn moving_nothing_writes_nothing(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", &root_with_password("{{pw}}"))]);
    let model = open(cx, &dir);
    let before = network(cx, &model);
    apply(cx, &model, Edit::MoveNetworkSecrets);
    assert_eq!(network(cx, &model), before);
    assert!(!snap(cx, &model).content.collection.dirty);
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
}

#[gpui_kit::test]
fn an_inserted_row_never_takes_another_rows_name(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT), (".gitignore", ".env\n")]);
    let model = open(cx, &dir);
    let mut first = ClientCertificate::pem("a.test", "c.pem", "k.pem");
    first.passphrase = "old".into();
    apply(cx, &model, Edit::ClientCertificates(vec![first]));
    let existing = network(cx, &model).certificates[0].clone();
    assert_eq!(
        existing.passphrase,
        "{{process.env.APINATA_CERT_PASSPHRASE_1}}"
    );
    let mut inserted = ClientCertificate::pem("b.test", "c2.pem", "k2.pem");
    inserted.passphrase = "new".into();
    apply(
        cx,
        &model,
        Edit::ClientCertificates(vec![inserted, existing]),
    );
    let certs = network(cx, &model).certificates;
    assert_eq!(
        certs[0].passphrase,
        "{{process.env.APINATA_CERT_PASSPHRASE_1_2}}"
    );
    assert_eq!(
        certs[1].passphrase,
        "{{process.env.APINATA_CERT_PASSPHRASE_1}}"
    );
    save(cx, &model);
    assert_eq!(
        read(&dir, ".env"),
        "APINATA_CERT_PASSPHRASE_1='old'\nAPINATA_CERT_PASSPHRASE_1_2='new'\n"
    );
}

// The reference writer always writes the proxy's auth block, empty when nothing is set, as
// the parity case network-proxy-socks-noauth records.
#[gpui_kit::test]
fn switching_an_unauthenticated_proxy_on_writes_the_empty_auth_block(cx: &mut TestAppContext) {
    let off = format!(
        "{ROOT}config:\n  proxy:\n    inherit: false\n    disabled: true\n    config:\n      protocol: http\n      hostname: proxy.local\n      port: \"3128\"\n      bypassProxy: \"\"\n"
    );
    let dir = write_tree(&[("opencollection.yml", &off)]);
    let model = open(cx, &dir);
    assert_eq!(network(cx, &model).proxy, CollectionProxy::Off);
    let proxy = ProxyConfig::new("http", "proxy.local", "3128");
    apply(cx, &model, Edit::Proxy(CollectionProxy::Custom(proxy)));
    send(cx, &model, Command::Save(Target::Collection));
    cx.run_until_parked();
    assert_eq!(
        read(&dir, "opencollection.yml"),
        format!(
            "{}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: proxy.local\n      port: \"3128\"\n      auth:\n        username: \"\"\n        password: \"\"\n      bypassProxy: \"\"\n",
            // The reference writer also puts a blank line after the version.
            ROOT.replacen('\n', "\n\n", 1)
        )
    );
}

#[gpui_kit::test]
fn dont_save_leaves_dotenv_untouched(cx: &mut TestAppContext) {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let model = open(cx, &dir);
    apply(cx, &model, Edit::Proxy(custom("proxy.local", "hunter2")));
    let mut cert = ClientCertificate::pem("a.test", "c.pem", "k.pem");
    cert.passphrase = "pp-secret".into();
    apply(cx, &model, Edit::ClientCertificates(vec![cert]));
    send(cx, &model, Command::Revert(Target::Collection));
    cx.run_until_parked();
    assert_eq!(network(cx, &model).proxy, CollectionProxy::Inherit);
    save(cx, &model);
    assert!(!dir.path().join(".env").exists() && !dir.path().join(".gitignore").exists());
}

#[gpui_kit::test]
fn a_proxy_reference_that_is_no_env_key_is_never_reused(cx: &mut TestAppContext) {
    let dir = write_tree(&[
        (
            "opencollection.yml",
            &root_with_password("{{process.env.APINATA_PRmy-xOXY_PASSWORD}}"),
        ),
        (".gitignore", ".env\n"),
    ]);
    let model = open(cx, &dir);
    apply(cx, &model, Edit::Proxy(custom("proxy.local", "topsecret")));
    assert_eq!(
        password_of(&network(cx, &model).proxy),
        "{{process.env.APINATA_PROXY_PASSWORD}}"
    );
    save(cx, &model);
    assert_eq!(read(&dir, ".env"), "APINATA_PROXY_PASSWORD='topsecret'\n");
}
