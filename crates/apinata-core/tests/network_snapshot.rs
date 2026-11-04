//! The snapshot keeps the order a send reads (the first matching certificate wins), the content
//! of the certificate files it names, and a dynamic field as its template; credentials are
//! keyed with scrypt, not one SHA-256.

use crate::common;

use apinata_core::{
    ClientCertificate, Collection, CollectionNetwork, CollectionProxy, ContextOptions,
    HostOverride, NetworkChange, NetworkSnapshot, ProxyConfig, VarContext,
};
use common::write_tree;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

const SALT: &str = "salt";

fn texts(changes: &[NetworkChange]) -> Vec<String> {
    changes.iter().map(NetworkChange::describe).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn certs(rows: &[(&str, &str, &str)]) -> CollectionNetwork {
    let mut net = CollectionNetwork::default();
    net.certificates = rows
        .iter()
        .map(|(d, c, k)| ClientCertificate::pem(*d, *c, *k))
        .collect();
    net
}

fn resolved(dir: &Path, net: &CollectionNetwork) -> NetworkSnapshot {
    let c = Collection::open(dir).unwrap();
    let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
    NetworkSnapshot::resolve(net, &ctx, SALT)
}

#[test]
fn host_override_order_is_kept_and_named() {
    let mut one = CollectionNetwork::default();
    one.host_overrides = vec![
        HostOverride::new("a.test", "10.0.0.1"),
        HostOverride::new("b.test", "10.0.0.2"),
    ];
    let mut two = one.clone();
    two.host_overrides.reverse();
    let (a, b) = (
        NetworkSnapshot::from_network(&one, SALT),
        NetworkSnapshot::from_network(&two, SALT),
    );
    assert_ne!(a.fingerprint(), b.fingerprint());
    assert_eq!(
        texts(&NetworkSnapshot::diff(&a, &b)),
        ["Host override order changed"]
    );
}

#[test]
fn a_certificate_reorder_changes_the_fingerprint_and_says_so() {
    let first = certs(&[("*.test", "a.pem", "a.key"), ("*.test", "b.pem", "b.key")]);
    let mut second = first.clone();
    second.certificates.reverse();
    let (a, b) = (
        NetworkSnapshot::from_network(&first, SALT),
        NetworkSnapshot::from_network(&second, SALT),
    );
    assert_ne!(a.fingerprint(), b.fingerprint());
    assert_eq!(
        texts(&NetworkSnapshot::diff(&a, &b)),
        ["Client certificate order changed"]
    );
}

#[test]
fn a_certificate_file_whose_bytes_change_reads_as_changed() {
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        ),
        ("c.pem", "first certificate"),
        ("c.key", "first key"),
    ]);
    let net = certs(&[("*.test", "c.pem", "c.key")]);
    let before = resolved(dir.path(), &net);
    fs::write(dir.path().join("c.pem"), "swapped certificate").unwrap();
    let after = resolved(dir.path(), &net);
    assert_ne!(before.fingerprint(), after.fingerprint());
    assert_eq!(
        texts(&NetworkSnapshot::diff(&before, &after)),
        ["Client certificate changed for *.test"]
    );
    let json = serde_json::to_string(&after).unwrap();
    assert!(!json.contains("swapped certificate"), "{json}");
}

#[test]
fn a_missing_certificate_file_is_a_marker_and_a_later_file_is_a_change() {
    let dir = write_tree(&[(
        "opencollection.yml",
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
    )]);
    let net = certs(&[("*.test", "c.pem", "c.key")]);
    let missing = resolved(dir.path(), &net);
    fs::write(dir.path().join("c.pem"), "now here").unwrap();
    fs::write(dir.path().join("c.key"), "now here").unwrap();
    let present = resolved(dir.path(), &net);
    assert_ne!(missing.fingerprint(), present.fingerprint());
}

#[test]
fn a_dynamic_field_is_kept_as_its_template() {
    let dir = write_tree(&[(
        "opencollection.yml",
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
    )]);
    let mut net = CollectionNetwork::default();
    net.proxy = CollectionProxy::Custom(ProxyConfig::new("http", "{{$randomInt}}.test", "3128"));
    let a = resolved(dir.path(), &net);
    let b = resolved(dir.path(), &net);
    assert_eq!(a.fingerprint(), b.fingerprint());
    let shown = texts(&NetworkSnapshot::diff(&NetworkSnapshot::default(), &a)).join("|");
    assert!(shown.contains("{{$randomInt}}.test"), "{shown}");
}

#[test]
fn the_keyed_hash_is_scrypt_and_stable() {
    let dir = write_tree(&[(
        "opencollection.yml",
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
    )]);
    let mut net = CollectionNetwork::default();
    net.proxy = CollectionProxy::Custom(ProxyConfig::new("http", "{{nope}}", "3128"));
    let first = serde_json::to_string(&resolved(dir.path(), &net)).unwrap();
    let again = serde_json::to_string(&resolved(dir.path(), &net)).unwrap();
    assert_eq!(first, again);
    let params = scrypt::Params::new(14, 8, 1).unwrap();
    let mut out = [0u8; 32];
    scrypt::scrypt(b"{{nope}}", SALT.as_bytes(), &params, &mut out).unwrap();
    let sha = hex(&Sha256::digest(format!("{SALT}\0{{{{nope}}}}")));
    assert!(first.contains(&hex(&out)), "{first}");
    assert!(!first.contains(&sha), "{first}");
}
