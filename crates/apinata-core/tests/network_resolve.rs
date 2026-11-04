//! The acknowledged network settings are the values a send would use for the selected
//! environment, and never a credential: user names, passwords and secret values stay out of
//! the stored file, the diff wording and Debug output.

use crate::common;

use apinata_core::{
    AckState, Collection, CollectionNetwork, CollectionProxy, ContextOptions, HostOverride,
    NetworkAck, NetworkChange, NetworkSnapshot, ProxyConfig, VarContext,
};
use common::write_tree;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const SALT: &str = "salt";

fn proxy_net(host: &str) -> CollectionNetwork {
    let mut net = CollectionNetwork::default();
    net.proxy = CollectionProxy::Custom(ProxyConfig::new("http", host, "3128"));
    net
}

fn texts(changes: &[NetworkChange]) -> Vec<String> {
    changes.iter().map(NetworkChange::describe).collect()
}

fn dir_with(root: &str, envs: &[(&str, &str)]) -> TempDir {
    let mut files = vec![("opencollection.yml".to_owned(), root.to_owned())];
    for (name, vars) in envs {
        files.push((
            format!("environments/{name}.yml"),
            format!("name: {name}\nvariables:\n{vars}"),
        ));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    write_tree(&refs)
}

fn resolved(dir: &Path, env: Option<&str>, net: &CollectionNetwork) -> NetworkSnapshot {
    let c = Collection::open(dir).unwrap();
    let mut options = ContextOptions::new();
    if let Some(env) = env {
        options = options.env(env);
    }
    let ctx = VarContext::new(&c, options).unwrap();
    NetworkSnapshot::resolve(net, &ctx, SALT)
}

#[test]
fn userinfo_in_a_proxy_host_is_never_stored_or_shown() {
    let before = NetworkSnapshot::from_network(&proxy_net("old.test"), SALT);
    let a = NetworkSnapshot::from_network(&proxy_net("user:secretpw@proxy.test"), SALT);
    let b = NetworkSnapshot::from_network(&proxy_net("http://user:otherpw@proxy.test"), SALT);
    for snapshot in [&a, &b] {
        assert!(!format!("{snapshot:?}").contains("secretpw"));
        assert!(!format!("{snapshot:?}").contains("otherpw"));
        assert!(!serde_json::to_string(snapshot).unwrap().contains("pw"));
    }
    let shown = texts(&NetworkSnapshot::diff(&before, &a)).join("|");
    assert!(
        !shown.contains("secretpw") && !shown.contains("user"),
        "{shown}"
    );
    assert!(shown.contains("proxy.test"), "{shown}");
    // Same host and a different password: only the credentials differ.
    let c = NetworkSnapshot::from_network(&proxy_net("user:changed@proxy.test"), SALT);
    assert_eq!(
        texts(&NetworkSnapshot::diff(&a, &c)),
        ["Proxy credentials changed"]
    );
}

#[test]
fn userinfo_in_a_stored_ack_file_never_reaches_disk() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("network-ack.json");
    let mut net = proxy_net("user:secretpw@proxy.test");
    net.host_overrides
        .push(HostOverride::new("bob:hostpw@api.test", "10.0.0.1"));
    NetworkAck::update(&file, |a| a.acknowledge(dir.path(), &net)).unwrap();
    let bytes = fs::read_to_string(&file).unwrap();
    for secret in ["secretpw", "hostpw", "user:", "bob"] {
        assert!(!bytes.contains(secret), "{secret} in {bytes}");
    }
    assert!(
        bytes.contains("proxy.test") && bytes.contains("api.test"),
        "{bytes}"
    );
}

#[test]
fn userinfo_in_an_override_host_is_hashed_and_a_change_is_named_without_it() {
    let row = |host: &str| {
        let mut net = CollectionNetwork::default();
        net.host_overrides.push(HostOverride::new(host, "10.0.0.1"));
        NetworkSnapshot::from_network(&net, SALT)
    };
    let changes = NetworkSnapshot::diff(&row("bob:one@api.test"), &row("bob:two@api.test"));
    assert_eq!(
        texts(&changes),
        ["Host override credentials changed: api.test"]
    );
}

#[test]
fn a_variable_in_the_proxy_host_shows_a_change_of_its_value() {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\n";
    let dir = dir_with(root, &[("dev", "  - name: p\n    value: a.test\n")]);
    let net = proxy_net("{{p}}");
    let first = resolved(dir.path(), Some("dev"), &net);
    fs::write(
        dir.path().join("environments/dev.yml"),
        "name: dev\nvariables:\n  - name: p\n    value: b.test\n",
    )
    .unwrap();
    let second = resolved(dir.path(), Some("dev"), &net);
    assert_eq!(
        texts(&NetworkSnapshot::diff(&first, &second)),
        ["Proxy changed to http://b.test:3128"]
    );
}

#[test]
fn another_environment_with_other_values_counts_as_a_change() {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\n";
    let dir = dir_with(
        root,
        &[
            ("dev", "  - name: p\n    value: dev.test\n"),
            ("prod", "  - name: p\n    value: prod.test\n"),
        ],
    );
    let net = proxy_net("{{p}}");
    let dev = resolved(dir.path(), Some("dev"), &net);
    let prod = resolved(dir.path(), Some("prod"), &net);
    assert_ne!(dev.fingerprint(), prod.fingerprint());
    assert_eq!(
        texts(&NetworkSnapshot::diff(&dev, &prod)),
        ["Proxy changed to http://prod.test:3128"]
    );
}

#[test]
fn an_unresolved_variable_is_a_marker_not_an_error() {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\n";
    let dir = dir_with(root, &[("dev", "  - name: p\n    value: a.test\n")]);
    let net = proxy_net("{{p}}");
    let none = resolved(dir.path(), None, &net);
    let dev = resolved(dir.path(), Some("dev"), &net);
    let shown = texts(&NetworkSnapshot::diff(&dev, &none)).join("|");
    assert!(shown.contains("unresolved: p)"), "{shown}");
    assert_ne!(none.fingerprint(), dev.fingerprint());
}

#[test]
fn a_secret_value_is_hashed_and_its_change_still_shows() {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\n";
    let dir = dir_with(root, &[("dev", "  - name: p\n    secret: true\n")]);
    let net = proxy_net("{{p}}");
    let with = |value: &str| {
        let c = Collection::open(dir.path()).unwrap();
        let options = ContextOptions::new()
            .env("dev")
            .sensitive_env_var("p", value);
        let ctx = VarContext::new(&c, options).unwrap();
        NetworkSnapshot::resolve(&net, &ctx, SALT)
    };
    let (a, b) = (with("hunter2.test"), with("other.test"));
    assert!(!format!("{a:?}").contains("hunter2"));
    assert!(!serde_json::to_string(&a).unwrap().contains("hunter2"));
    let shown = texts(&NetworkSnapshot::diff(&a, &b)).join("|");
    assert!(
        !shown.contains("hunter2") && !shown.contains("other.test"),
        "{shown}"
    );
    assert!(shown.starts_with("Proxy changed"), "{shown}");
}

#[test]
fn a_reset_moves_an_unreadable_record_aside_and_never_deletes_it() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("network-ack.json");
    fs::write(&file, "not json").unwrap();
    let first = NetworkAck::reset_unreadable(&file).unwrap();
    assert_eq!(first, dir.path().join("network-ack.json.bad"));
    assert_eq!(fs::read_to_string(&first).unwrap(), "not json");
    assert!(!file.exists());
    assert!(NetworkAck::load(&file).is_ok());
    // A second bad file does not clobber the first.
    fs::write(&file, "worse").unwrap();
    let second = NetworkAck::reset_unreadable(&file).unwrap();
    assert_ne!(first, second);
    assert_eq!(fs::read_to_string(&first).unwrap(), "not json");
    assert_eq!(fs::read_to_string(&second).unwrap(), "worse");
}

#[test]
fn a_reset_leaves_a_readable_record_alone() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("network-ack.json");
    NetworkAck::update(&file, |a| {
        a.acknowledge(dir.path(), &CollectionNetwork::default())
    })
    .unwrap();
    assert!(NetworkAck::reset_unreadable(&file).is_err());
    assert!(file.exists());
}

#[test]
fn the_check_with_a_resolved_snapshot_follows_the_environment() {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\n";
    let dir = dir_with(
        root,
        &[
            ("dev", "  - name: p\n    value: dev.test\n"),
            ("prod", "  - name: p\n    value: prod.test\n"),
        ],
    );
    let file = dir.path().join("ack.json");
    let net = proxy_net("{{p}}");
    let state = |env: &str| {
        let net = &net;
        NetworkAck::check_file_with(&file, dir.path(), |salt| {
            let c = Collection::open(dir.path()).unwrap();
            let ctx = VarContext::new(&c, ContextOptions::new().env(env)).unwrap();
            NetworkSnapshot::resolve(net, &ctx, salt)
        })
    };
    NetworkAck::update(&file, |a| {
        let c = Collection::open(dir.path()).unwrap();
        let ctx = VarContext::new(&c, ContextOptions::new().env("dev")).unwrap();
        let snapshot = NetworkSnapshot::resolve(&net, &ctx, a.salt());
        a.acknowledge_snapshot(dir.path(), &snapshot);
    })
    .unwrap();
    assert_eq!(state("dev"), AckState::Unchanged);
    assert!(matches!(state("prod"), AckState::Changed(_)));
}

// Passwords that hold the characters an authority parser would stop at.
const TRICKY: [(&str, &str); 6] = [
    ("u:pa/ss@h.test", "pa/ss"),
    ("u:p?w@h.test", "p?w"),
    ("u:p#w@h.test", "p#w"),
    ("http://u:pa/ss@h.test:3128", "pa/ss"),
    ("u:p%40w@h.test", "p%40w"),
    ("u:p@w@h.test", "p@w"),
];

fn nowhere(snapshot: &NetworkSnapshot, secret: &str) {
    assert!(
        !format!("{snapshot:?}").contains(secret),
        "{secret} in Debug"
    );
    assert!(
        !serde_json::to_string(snapshot).unwrap().contains(secret),
        "{secret} in JSON"
    );
}

#[test]
fn a_password_with_a_delimiter_never_reaches_the_record() {
    let empty = NetworkSnapshot::from_network(&CollectionNetwork::default(), SALT);
    for (host, secret) in TRICKY {
        let snapshot = NetworkSnapshot::from_network(&proxy_net(host), SALT);
        nowhere(&snapshot, secret);
        let shown = texts(&NetworkSnapshot::diff(&empty, &snapshot)).join("|");
        assert!(
            !shown.contains(secret) && shown.contains("h.test"),
            "{host}: {shown}"
        );
        let mut net = CollectionNetwork::default();
        net.host_overrides.push(HostOverride::new(host, "10.0.0.1"));
        nowhere(&NetworkSnapshot::from_network(&net, SALT), secret);
    }
}

#[test]
fn a_variable_that_expands_to_such_a_host_is_stripped_too() {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\n";
    let net = proxy_net("{{p}}");
    for (host, secret) in TRICKY {
        let env = format!("  - name: p\n    value: \"{host}\"\n");
        let dir = dir_with(root, &[("dev", &env)]);
        let snapshot = resolved(dir.path(), Some("dev"), &net);
        nowhere(&snapshot, secret);
        let plain = NetworkSnapshot::from_network(&proxy_net(host), SALT);
        assert_eq!(snapshot.fingerprint(), plain.fingerprint(), "{host}");
    }
}

#[test]
fn userinfo_is_stripped_from_every_text_field() {
    use apinata_core::ClientCertificate;
    let mut net = CollectionNetwork::default();
    let mut proxy = ProxyConfig::new("u:SECRETX@http", "h.test", "u:SECRETX@3128");
    proxy.bypass = "a.test,u:SECRETX@b.test".into();
    net.proxy = CollectionProxy::Custom(proxy);
    net.host_overrides
        .push(HostOverride::new("h.test", "u:SECRETX@10.0.0.1"));
    let mut cert = ClientCertificate::pem("u:SECRETX@dom.test", "c.pem", "u:SECRETX@c.key");
    cert.enabled = true;
    net.certificates.push(cert);
    let snapshot = NetworkSnapshot::from_network(&net, SALT);
    nowhere(&snapshot, "SECRETX");
    let empty = NetworkSnapshot::from_network(&CollectionNetwork::default(), SALT);
    let shown = texts(&NetworkSnapshot::diff(&empty, &snapshot)).join("|");
    assert!(!shown.contains("SECRETX"), "{shown}");
    // A change of one of those credentials still shows.
    net.host_overrides[0] = HostOverride::new("h.test", "u:OTHER@10.0.0.1");
    let changed = NetworkSnapshot::from_network(&net, SALT);
    assert_ne!(changed.fingerprint(), snapshot.fingerprint());
}

#[test]
fn an_unresolved_marker_names_the_variable_and_nothing_else() {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\n";
    let dir = dir_with(root, &[]);
    let net = proxy_net("u:LITERAL-SECRET@{{missing}}.test");
    let snapshot = resolved(dir.path(), None, &net);
    nowhere(&snapshot, "LITERAL-SECRET");
    let empty = NetworkSnapshot::from_network(&CollectionNetwork::default(), SALT);
    let shown = texts(&NetworkSnapshot::diff(&empty, &snapshot)).join("|");
    assert!(
        shown.contains("unresolved") && shown.contains("missing"),
        "{shown}"
    );
    assert!(!shown.contains("LITERAL-SECRET"), "{shown}");
    // The literal part still counts: changing it is a change.
    let other = resolved(dir.path(), None, &proxy_net("u:OTHER@{{missing}}.test"));
    assert_ne!(snapshot.fingerprint(), other.fingerprint());
}
