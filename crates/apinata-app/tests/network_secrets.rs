//! The pure helpers that name and recognise network secrets.

use apinata_app::network_secrets::{
    dotenv_name, env_name, git_tracks_dotenv, is_template, passphrase_base, plain_secrets,
};
use apinata_core::{ClientCertificate, CollectionNetwork, CollectionProxy, ProxyAuth, ProxyConfig};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

fn custom(password: &str) -> CollectionProxy {
    let mut p = ProxyConfig::new("http", "p", "3128");
    p.auth = Some(ProxyAuth::new("u", password));
    CollectionProxy::Custom(p)
}

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
fn plain_secrets_are_literals_only() {
    let mut n = CollectionNetwork::default();
    assert!(!plain_secrets(&n));
    n.proxy = custom("{{process.env.P}}");
    assert!(!plain_secrets(&n));
    n.proxy = custom("hunter2");
    assert!(plain_secrets(&n));
    n.proxy = CollectionProxy::Off;
    let mut cert = ClientCertificate::pem("a.test", "c.pem", "k.pem");
    cert.passphrase = "pp".into();
    n.certificates = vec![cert];
    assert!(plain_secrets(&n));
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

/// `env_name` must follow the core variable-name rule, so the two cannot drift apart.
#[test]
fn env_name_follows_the_core_variable_name_rule() {
    for inner in ["", " ", "a b", "a}}", "{x", "tok en", "a/b", "é"] {
        let template = format!("{{{{process.env.{inner}}}}}");
        assert!(
            apinata_core::variable_name_problem(inner).is_some(),
            "core accepts {inner:?}, so this case proves nothing"
        );
        assert_eq!(env_name(&template), None, "{template:?}");
    }
    for inner in ["tok", "api.key", "my-var_1"] {
        let template = format!("{{{{process.env.{inner}}}}}");
        assert_eq!(apinata_core::variable_name_problem(inner), None, "{inner}");
        assert_eq!(env_name(&template), Some(inner), "{template:?}");
    }
}
