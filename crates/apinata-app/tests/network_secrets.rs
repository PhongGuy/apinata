//! The pure helpers that name and recognise network secrets, and the per-field move.

use apinata_app::command::{Edit, NetworkSlot};
use apinata_app::network_secrets::{
    TRACKED, UNQUOTABLE, apply, dotenv_name, env_name, git_tracks_dotenv, is_template,
    passphrase_base,
};
use apinata_core::{Collection, CollectionNetwork, CollectionProxy};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}");
}

#[test]
fn names_and_templates() {
    assert!(is_template("{{process.env.X}}") && is_template("a{{b}}") && !is_template("hunter2"));
    assert!(!is_template("ab{{c") && !is_template("x}}{{") && !is_template("}}"));
    assert_eq!(env_name("{{process.env.MY_PW}}"), Some("MY_PW"));
    assert_eq!(env_name("{{process.env.MY_PW}}x"), None);
    assert_eq!(env_name("{{pw}}"), None);
    assert_eq!(passphrase_base(0), "APINATA_CERT_PASSPHRASE_1");
    let taken: BTreeSet<String> = [
        "APINATA_PROXY_PASSWORD".to_string(),
        "APINATA_PROXY_PASSWORD_2".to_string(),
    ]
    .into();
    assert_eq!(
        dotenv_name("APINATA_PROXY_PASSWORD", &taken),
        "APINATA_PROXY_PASSWORD_3"
    );
    assert_eq!(dotenv_name("OTHER", &taken), "OTHER");
}

#[test]
fn only_a_tracked_dotenv_counts_as_tracked() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "X=1\n").unwrap();
    assert!(!git_tracks_dotenv(dir.path()));
    git(dir.path(), &["init", "-q"]);
    assert!(!git_tracks_dotenv(dir.path()));
    git(dir.path(), &["add", ".env"]);
    assert!(git_tracks_dotenv(dir.path()));
}

/// `env_name` takes only names a `.env` key and a shell variable can both be: the core
/// variable rule allows `.` and `-`, which would put an unusable key into `.env`.
#[test]
fn env_name_takes_only_portable_env_keys() {
    let refused = [
        "",
        " ",
        "a b",
        "a}}",
        "{x",
        "tok en",
        "a/b",
        "é",
        "api.key",
        "my-var_1",
        "1abc",
        "APINATA_OAmy-clientUTH2_CLIENT_SECRET",
    ];
    for inner in refused {
        let template = format!("{{{{process.env.{inner}}}}}");
        assert_eq!(env_name(&template), None, "{template:?}");
    }
    for inner in ["tok", "API_KEY", "_x1", "APINATA_OAUTH2_CLIENT_SECRET"] {
        let template = format!("{{{{process.env.{inner}}}}}");
        assert_eq!(env_name(&template), Some(inner), "{template:?}");
    }
}

// A collection whose proxy password is `password` and whose two certificates have the
// passphrases `first` and `second`, with `.env` holding `KEEP=1`.
fn network_root(password: &str, first: &str, second: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = format!(
        "opencollection: 1.0.0\ninfo:\n  name: c\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: proxy.local\n      port: \"3128\"\n      auth:\n        username: u\n        password: \"{password}\"\n      bypassProxy: \"\"\n  clientCertificates:\n    - domain: a.test\n      type: pem\n      certificateFilePath: a.pem\n      privateKeyFilePath: a.key\n      passphrase: \"{first}\"\n    - domain: b.test\n      type: pem\n      certificateFilePath: b.pem\n      privateKeyFilePath: b.key\n      passphrase: \"{second}\"\n"
    );
    std::fs::write(dir.path().join("opencollection.yml"), root).unwrap();
    std::fs::write(dir.path().join(".env"), "KEEP=1\n").unwrap();
    dir
}

fn password_of(network: &CollectionNetwork) -> &str {
    match &network.proxy {
        CollectionProxy::Custom(p) => p.auth.as_ref().map_or("", |a| a.password.as_str()),
        other => panic!("not a custom proxy: {other:?}"),
    }
}

#[test]
fn moving_one_network_slot_moves_only_that_literal() {
    let dir = network_root("hunter2", "pp-one", "pp-two");
    let mut c = Collection::open(dir.path()).unwrap();
    let slot = Edit::MoveNetworkSecret(NetworkSlot::Passphrase(1));
    let moved = apply(&mut c, slot, &BTreeSet::new()).unwrap();
    assert_eq!(
        moved,
        [("APINATA_CERT_PASSPHRASE_2".to_owned(), "pp-two".to_owned())]
    );
    let network = c.file().network();
    assert_eq!(password_of(&network), "hunter2");
    assert_eq!(network.certificates[0].passphrase, "pp-one");
    assert_eq!(
        network.certificates[1].passphrase,
        "{{process.env.APINATA_CERT_PASSPHRASE_2}}"
    );

    let slot = Edit::MoveNetworkSecret(NetworkSlot::ProxyPassword);
    let moved = apply(&mut c, slot, &BTreeSet::new()).unwrap();
    assert_eq!(
        moved,
        [("APINATA_PROXY_PASSWORD".to_owned(), "hunter2".to_owned())]
    );
    let network = c.file().network();
    assert_eq!(
        password_of(&network),
        "{{process.env.APINATA_PROXY_PASSWORD}}"
    );
    assert_eq!(network.certificates[0].passphrase, "pp-one");
    // Values reach `.env` only when the collection is saved.
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".env")).unwrap(),
        "KEEP=1\n"
    );
}

#[test]
fn moving_a_reference_or_an_empty_slot_changes_nothing() {
    let dir = network_root("{{process.env.KEEP}}", "", "pp-two");
    let mut c = Collection::open(dir.path()).unwrap();
    let before = c.file().network();
    for slot in [NetworkSlot::ProxyPassword, NetworkSlot::Passphrase(0)] {
        let moved = apply(&mut c, Edit::MoveNetworkSecret(slot), &BTreeSet::new()).unwrap();
        assert!(moved.is_empty());
    }
    assert_eq!(c.file().network(), before);
}

#[test]
fn an_unquotable_value_is_refused_and_leaves_the_draft_and_dotenv_alone() {
    let dir = network_root("a'b\\\"c`", "pp-one", "pp-two");
    let mut c = Collection::open(dir.path()).unwrap();
    let before = c.file().network();
    let slot = Edit::MoveNetworkSecret(NetworkSlot::ProxyPassword);
    let refused = apply(&mut c, slot, &BTreeSet::new()).unwrap_err();
    assert_eq!(refused, UNQUOTABLE);
    assert_eq!(c.file().network(), before);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".env")).unwrap(),
        "KEEP=1\n"
    );
}

#[test]
fn a_tracked_dotenv_is_refused_and_leaves_the_draft_and_dotenv_alone() {
    let dir = network_root("hunter2", "pp-one", "pp-two");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", ".env"]);
    let mut c = Collection::open(dir.path()).unwrap();
    let before = c.file().network();
    let slot = Edit::MoveNetworkSecret(NetworkSlot::Passphrase(0));
    let refused = apply(&mut c, slot, &BTreeSet::new()).unwrap_err();
    assert_eq!(refused, TRACKED);
    assert_eq!(c.file().network(), before);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".env")).unwrap(),
        "KEEP=1\n"
    );
}
