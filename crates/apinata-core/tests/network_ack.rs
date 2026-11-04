//! The network acknowledgement store: what a collection's proxy, host overrides and
//! certificates were when last seen, and the wording of what changed since.

use apinata_core::{
    AckState, ClientCertificate, CollectionNetwork, CollectionProxy, HostOverride, NetworkAck,
    NetworkChange, NetworkSnapshot, ProxyAuth, ProxyConfig, saved_network,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn hosts(rows: &[(&str, &str)]) -> CollectionNetwork {
    let mut net = CollectionNetwork::default();
    net.host_overrides = rows
        .iter()
        .map(|(h, a)| HostOverride::new(*h, *a))
        .collect();
    net
}

fn proxy(host: &str, port: &str) -> CollectionNetwork {
    let mut net = CollectionNetwork::default();
    net.proxy = CollectionProxy::Custom(ProxyConfig::new("http", host, port));
    net
}

fn store() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("network-ack.json");
    (dir, file)
}

fn ack(file: &Path, root: &Path, net: &CollectionNetwork) {
    NetworkAck::update(file, |a| a.acknowledge(root, net)).unwrap();
}

fn state(file: &Path, root: &Path, net: &CollectionNetwork) -> AckState {
    NetworkAck::load(file).unwrap().check(root, net)
}

fn texts(changes: &[NetworkChange]) -> Vec<String> {
    changes.iter().map(NetworkChange::describe).collect()
}

#[test]
fn the_fingerprint_is_stable_and_follows_the_settings() {
    let a = NetworkSnapshot::from_network(&hosts(&[("a.test", "10.0.0.1")]), "salt");
    let again = NetworkSnapshot::from_network(&hosts(&[("a.test", "10.0.0.1")]), "salt");
    let other = NetworkSnapshot::from_network(&hosts(&[("a.test", "10.0.0.2")]), "salt");
    assert_eq!(a.fingerprint().len(), 64);
    assert_eq!(a.fingerprint(), again.fingerprint());
    assert_ne!(a.fingerprint(), other.fingerprint());
}

#[test]
fn row_order_changes_the_fingerprint() {
    let one = hosts(&[("a.test", "10.0.0.1"), ("b.test", "10.0.0.2")]);
    let two = hosts(&[("b.test", "10.0.0.2"), ("a.test", "10.0.0.1")]);
    assert_ne!(
        NetworkSnapshot::from_network(&one, "s").fingerprint(),
        NetworkSnapshot::from_network(&two, "s").fingerprint()
    );
}

#[test]
fn disabled_rows_are_ignored() {
    let mut net = hosts(&[("a.test", "10.0.0.1")]);
    let mut off = HostOverride::new("evil.test", "203.0.113.9");
    off.enabled = false;
    net.host_overrides.push(off);
    let mut cert = ClientCertificate::pem("*.test", "c.pem", "c.key");
    cert.enabled = false;
    net.certificates.push(cert);
    let plain = hosts(&[("a.test", "10.0.0.1")]);
    assert_eq!(
        NetworkSnapshot::from_network(&net, "s"),
        NetworkSnapshot::from_network(&plain, "s")
    );
}

#[test]
fn an_empty_snapshot_is_empty_and_a_row_is_not() {
    let empty = NetworkSnapshot::from_network(&CollectionNetwork::default(), "s");
    assert!(empty.is_empty());
    assert!(!NetworkSnapshot::from_network(&hosts(&[("a.test", "1.1.1.1")]), "s").is_empty());
    assert!(!NetworkSnapshot::from_network(&proxy("p.test", "3128"), "s").is_empty());
}

#[test]
fn a_host_override_diff_names_the_row() {
    let old = NetworkSnapshot::from_network(&CollectionNetwork::default(), "s");
    let new = NetworkSnapshot::from_network(&hosts(&[("api.example.com", "203.0.113.5")]), "s");
    let added = NetworkSnapshot::diff(&old, &new);
    assert_eq!(
        texts(&added),
        ["Host override added: api.example.com -> 203.0.113.5"]
    );
    let removed = NetworkSnapshot::diff(&new, &old);
    assert_eq!(
        texts(&removed),
        ["Host override removed: api.example.com -> 203.0.113.5"]
    );
    let moved = NetworkSnapshot::from_network(&hosts(&[("api.example.com", "198.51.100.1")]), "s");
    assert_eq!(
        texts(&NetworkSnapshot::diff(&new, &moved)),
        ["Host override changed: api.example.com -> 198.51.100.1 (was 203.0.113.5)"]
    );
}

#[test]
fn a_proxy_diff_names_the_new_proxy() {
    let old = NetworkSnapshot::from_network(&CollectionNetwork::default(), "s");
    let new = NetworkSnapshot::from_network(&proxy("proxy.test", "3128"), "s");
    assert_eq!(
        texts(&NetworkSnapshot::diff(&old, &new)),
        ["Proxy changed to http://proxy.test:3128"]
    );
    let mut off = CollectionNetwork::default();
    off.proxy = CollectionProxy::Off;
    let off = NetworkSnapshot::from_network(&off, "s");
    assert_eq!(
        texts(&NetworkSnapshot::diff(&new, &off)),
        ["Proxy changed to off"]
    );
}

#[test]
fn a_credential_change_says_so_without_naming_either_user() {
    let with = |user: &str| {
        let mut net = proxy("proxy.test", "3128");
        if let CollectionProxy::Custom(p) = &mut net.proxy {
            p.auth = Some(ProxyAuth::new(user, "pw"));
        }
        NetworkSnapshot::from_network(&net, "s")
    };
    let changes = NetworkSnapshot::diff(&with("alice"), &with("mallory"));
    assert_eq!(texts(&changes), ["Proxy credentials changed"]);
}

#[test]
fn a_certificate_diff_names_the_domain() {
    let mut net = CollectionNetwork::default();
    net.certificates
        .push(ClientCertificate::pem("*.corp.test", "c.pem", "c.key"));
    let old = NetworkSnapshot::from_network(&CollectionNetwork::default(), "s");
    let new = NetworkSnapshot::from_network(&net, "s");
    assert_eq!(
        texts(&NetworkSnapshot::diff(&old, &new)),
        ["Client certificate added for *.corp.test"]
    );
}

#[test]
fn a_collection_is_first_seen_then_unchanged_then_changed() {
    let (dir, file) = store();
    let root = dir.path().join("col");
    fs::create_dir(&root).unwrap();
    let net = hosts(&[("api.example.com", "203.0.113.5")]);
    match state(&file, &root, &net) {
        AckState::FirstSeen(changes) => assert_eq!(
            texts(&changes),
            ["Host override added: api.example.com -> 203.0.113.5"]
        ),
        other => panic!("{other:?}"),
    }
    ack(&file, &root, &net);
    assert_eq!(state(&file, &root, &net), AckState::Unchanged);
    let edited = hosts(&[("api.example.com", "198.51.100.1")]);
    match state(&file, &root, &edited) {
        AckState::Changed(changes) => assert_eq!(changes.len(), 1),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_first_sight_of_nothing_carries_no_changes() {
    let (dir, file) = store();
    assert_eq!(
        state(&file, dir.path(), &CollectionNetwork::default()),
        AckState::FirstSeen(Vec::new())
    );
}

#[test]
fn no_secret_reaches_the_stored_file() {
    let (dir, file) = store();
    let mut net = proxy("proxy.test", "3128");
    if let CollectionProxy::Custom(p) = &mut net.proxy {
        p.auth = Some(ProxyAuth::new("USERNAME-SENTINEL", "PASSWORD-SENTINEL"));
    }
    let mut cert = ClientCertificate::pem("*.test", "c.pem", "c.key");
    cert.passphrase = "PASSPHRASE-SENTINEL".into();
    net.certificates.push(cert);
    ack(&file, dir.path(), &net);
    let bytes = fs::read_to_string(&file).unwrap();
    assert!(bytes.contains("proxy.test"), "{bytes}");
    for secret in [
        "USERNAME-SENTINEL",
        "PASSWORD-SENTINEL",
        "PASSPHRASE-SENTINEL",
    ] {
        assert!(!bytes.contains(secret), "{secret} leaked into {bytes}");
    }
}

#[cfg(unix)]
#[test]
fn a_symlinked_root_is_the_same_collection() {
    let (dir, file) = store();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let net = hosts(&[("a.test", "10.0.0.1")]);
    ack(&file, &real, &net);
    assert_eq!(state(&file, &link, &net), AckState::Unchanged);
}

#[test]
fn a_store_reloads_from_disk_on_every_load() {
    let (dir, file) = store();
    let net = hosts(&[("a.test", "10.0.0.1")]);
    let before = NetworkAck::load(&file).unwrap();
    ack(&file, dir.path(), &net);
    // A window that loaded earlier is stale; a fresh load sees the other window's write.
    assert!(matches!(
        before.check(dir.path(), &net),
        AckState::FirstSeen(_)
    ));
    assert_eq!(state(&file, dir.path(), &net), AckState::Unchanged);
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".apinata-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_malformed_store_is_an_error_and_is_never_overwritten() {
    let (_dir, file) = store();
    fs::write(&file, "not json").unwrap();
    assert!(NetworkAck::load(&file).is_err());
    assert!(
        NetworkAck::update(&file, |a| a
            .acknowledge(Path::new("/x"), &CollectionNetwork::default()))
        .is_err()
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), "not json");
}

#[test]
fn the_saved_network_is_read_from_the_root_file() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\nextensions:\n  apinata:\n    hostOverrides:\n      - host: api.example.com\n        address: 203.0.113.5\n",
    )
    .unwrap();
    let net = saved_network(dir.path()).unwrap();
    assert_eq!(net.host_overrides.len(), 1);
    assert_eq!(net.host_overrides[0].host, "api.example.com");
    assert!(saved_network(&dir.path().join("missing")).is_err());
}

#[test]
fn an_unparsable_entry_reads_as_changed_not_as_never_seen() {
    let (dir, file) = store();
    let root = dir.path().join("col");
    fs::create_dir(&root).unwrap();
    let net = hosts(&[("a.test", "10.0.0.1")]);
    ack(&file, &root, &net);
    // Damage this collection's entry only.
    let mut json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    let key = fs::canonicalize(&root)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    json["collections"][&key] = serde_json::json!({"fingerprint": 7});
    fs::write(&file, json.to_string()).unwrap();
    match state(&file, &root, &net) {
        AckState::Changed(changes) => assert_eq!(texts(&changes).len(), 1),
        other => panic!("{other:?}"),
    }
    // Acknowledging repairs the entry.
    ack(&file, &root, &net);
    assert_eq!(state(&file, &root, &net), AckState::Unchanged);
}

#[test]
fn an_unreadable_store_cannot_verify_and_says_so() {
    let (dir, file) = store();
    fs::write(&file, "not json").unwrap();
    let net = hosts(&[("a.test", "10.0.0.1")]);
    match NetworkAck::check_file(&file, dir.path(), &net) {
        AckState::Changed(changes) => assert_eq!(
            texts(&changes),
            ["Cannot verify network settings: the record of what was last seen is unreadable"]
        ),
        other => panic!("{other:?}"),
    }
    // A missing file is simply never seen.
    let missing = dir.path().join("missing.json");
    assert_eq!(
        NetworkAck::check_file(&missing, dir.path(), &CollectionNetwork::default()),
        AckState::FirstSeen(Vec::new())
    );
}

#[test]
fn a_relative_root_is_keyed_by_its_absolute_path() {
    let (_dir, file) = store();
    let net = hosts(&[("a.test", "10.0.0.1")]);
    ack(&file, Path::new("no-such-dir-i471/col"), &net);
    let text = fs::read_to_string(&file).unwrap();
    let cwd = std::env::current_dir().unwrap();
    assert!(text.contains(&cwd.to_string_lossy().into_owned()), "{text}");
}

#[test]
fn concurrent_writers_keep_each_others_entries() {
    let (dir, file) = store();
    let net = hosts(&[("a.test", "10.0.0.1")]);
    let roots: Vec<_> = (0..8)
        .map(|i| {
            let root = dir.path().join(format!("col{i}"));
            fs::create_dir(&root).unwrap();
            root
        })
        .collect();
    std::thread::scope(|scope| {
        for root in &roots {
            let (file, net) = (&file, &net);
            scope.spawn(move || ack(file, root, net));
        }
    });
    for root in &roots {
        assert_eq!(state(&file, root, &net), AckState::Unchanged);
    }
}

#[test]
fn acknowledging_what_a_notice_showed_records_that_and_not_a_later_state() {
    let (dir, file) = store();
    let a = hosts(&[("api.example.com", "203.0.113.5")]);
    let (first, shown) = NetworkAck::check_shown(&file, dir.path(), |salt| {
        NetworkSnapshot::from_network(&a, salt)
    });
    assert!(matches!(first, AckState::FirstSeen(_)));
    let shown = shown.expect("a readable record shows its snapshot");
    NetworkAck::update(&file, |ack| ack.acknowledge_shown(dir.path(), &shown)).unwrap();
    assert_eq!(state(&file, dir.path(), &a), AckState::Unchanged);
    let b = hosts(&[("api.example.com", "198.51.100.1")]);
    assert!(matches!(state(&file, dir.path(), &b), AckState::Changed(_)));
}
