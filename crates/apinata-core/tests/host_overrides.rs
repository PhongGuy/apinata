//! `extensions.apinata.hostOverrides`: read and written beside the file's other extensions,
//! and resolved into each request's network with typed errors for a bad or repeated row.

use crate::network_support::resolve_in;

use apinata_core::{Collection, ContextOptions, HostOverride, ResolveError};
use std::fs;
use tempfile::TempDir;

const PLAIN: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// A root whose `hostOverrides` hold `rows`, each `(host, address, disabled)`.
fn root(rows: &[(&str, &str, bool)]) -> String {
    let mut out = format!("{PLAIN}extensions:\n  apinata:\n    hostOverrides:\n");
    for (host, address, disabled) in rows {
        out.push_str(&format!(
            "      - host: \"{host}\"\n        address: \"{address}\"\n"
        ));
        if *disabled {
            out.push_str("        disabled: true\n");
        }
    }
    out
}

fn resolve(
    rows: &[(&str, &str, bool)],
    url: &str,
) -> Result<Vec<(String, Vec<String>)>, ResolveError> {
    resolve_in(&root(rows), url, ContextOptions::new())
        .result
        .map(|req| {
            req.network()
                .host_overrides()
                .iter()
                .map(|o| {
                    let addrs = o.addresses().iter().map(|a| a.to_string()).collect();
                    (o.host().to_owned(), addrs)
                })
                .collect()
        })
}

#[test]
fn enabled_rows_resolve_to_normalised_hosts_and_socket_addresses() {
    let got = resolve(
        &[
            ("API.Fake.Test", "127.0.0.1", false),
            ("v6.test", "[::1]:8443", false),
            ("off.test", "127.0.0.1", true),
            ("two.test", "127.0.0.1, 127.0.0.2:81", false),
        ],
        "http://api.fake.test/",
    )
    .expect("resolves");
    assert_eq!(
        got,
        [
            ("api.fake.test".to_owned(), vec!["127.0.0.1:0".to_owned()]),
            ("v6.test".to_owned(), vec!["[::1]:8443".to_owned()]),
            (
                "two.test".to_owned(),
                vec!["127.0.0.1:0".to_owned(), "127.0.0.2:81".to_owned()]
            ),
        ]
    );
}

#[test]
fn variables_in_a_row_are_expanded_before_the_lookup() {
    let text = format!(
        "{PLAIN}extensions:\n  apinata:\n    hostOverrides:\n      - host: \"{{{{h}}}}\"\n        address: \"{{{{a}}}}\"\n"
    );
    let options = ContextOptions::new()
        .env_var("h", "Var.Test")
        .env_var("a", "127.0.0.9");
    let r = resolve_in(&text, "http://var.test/", options);
    let overrides = r.request().network().host_overrides();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].host(), "var.test");
    assert_eq!(overrides[0].addresses()[0].to_string(), "127.0.0.9:0");
}

#[test]
fn a_bad_row_is_a_typed_error_naming_the_row() {
    for (host, address) in [
        ("fake.test", "not-an-ip"),
        ("fake.test", ""),
        ("", "127.0.0.1"),
        ("fake.test", "0.0.0.0"),
        ("fake.test", "[::]:80"),
        ("fake.test", "127.0.0.1:99999"),
        ("127.0.0.1", "127.0.0.2"),
        ("*.test", "127.0.0.1"),
        ("fake.test:80", "127.0.0.1"),
        ("http://fake.test", "127.0.0.1"),
    ] {
        let err = resolve(&[(host, address, false)], "http://x.test/").unwrap_err();
        match &err {
            ResolveError::InvalidHostOverride {
                host: h,
                address: a,
                ..
            } => {
                assert_eq!((h.as_str(), a.as_str()), (host, address), "{err}");
            }
            other => panic!("{host} -> {address}: {other:?}"),
        }
    }
}

#[test]
fn too_many_addresses_in_one_row_are_refused() {
    let many = (1..=20)
        .map(|n| format!("127.0.0.{n}"))
        .collect::<Vec<_>>()
        .join(",");
    let err = resolve(&[("fake.test", &many, false)], "http://x.test/").unwrap_err();
    assert!(
        matches!(err, ResolveError::InvalidHostOverride { .. }),
        "{err:?}"
    );
}

#[test]
fn a_disabled_row_is_never_validated() {
    let got = resolve(&[("fake.test", "not-an-ip", true)], "http://x.test/").expect("resolves");
    assert!(got.is_empty());
}

#[test]
fn two_enabled_rows_for_one_host_are_an_error_naming_both() {
    let err = resolve(
        &[
            ("Fake.test", "127.0.0.1", false),
            ("fake.TEST", "127.0.0.2", false),
        ],
        "http://x.test/",
    )
    .unwrap_err();
    match err {
        ResolveError::DuplicateHostOverride {
            host,
            first,
            second,
        } => {
            assert_eq!(host, "fake.test");
            assert_eq!(
                (first.as_str(), second.as_str()),
                ("127.0.0.1", "127.0.0.2")
            );
        }
        other => panic!("{other:?}"),
    }
    // A disabled twin is not a duplicate.
    resolve(
        &[
            ("fake.test", "127.0.0.1", false),
            ("fake.test", "127.0.0.2", true),
        ],
        "http://x.test/",
    )
    .expect("one enabled row");
}

fn open(text: &str) -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("opencollection.yml"), text).expect("root");
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn saved(dir: &TempDir) -> String {
    fs::read_to_string(dir.path().join("opencollection.yml")).expect("read")
}

const WITH_EXTENSIONS: &str = r#"opencollection: "1.0.0"
info:
  name: t
extensions:
  bruno:
    scripts:
      flow: sequential
  apinata:
    openapi:
      ops: {}
    hostOverrides:
      - host: fake.test
        address: 127.0.0.1
        x-note: kept
      - host: off.test
        address: "[::1]:81"
        disabled: true
      - just a string
"#;

#[test]
fn rows_read_in_file_order_with_their_enabled_flag() {
    let (_dir, c) = open(WITH_EXTENSIONS);
    let rows = c.file().network().host_overrides;
    let shown: Vec<_> = rows
        .iter()
        .map(|r| (r.host.as_str(), r.address.as_str(), r.enabled))
        .collect();
    assert_eq!(
        shown,
        [
            ("fake.test", "127.0.0.1", true),
            ("off.test", "[::1]:81", false)
        ]
    );
}

#[test]
fn an_unchanged_set_saves_byte_for_byte() {
    let (dir, mut c) = open(WITH_EXTENSIONS);
    let rows = c.file().network().host_overrides;
    c.file_mut().set_host_overrides(&rows);
    c.file_mut().save().expect("saves");
    assert_eq!(saved(&dir), WITH_EXTENSIONS);
}

#[test]
fn editing_a_row_keeps_unknown_keys_and_the_neighbouring_extensions() {
    let (dir, mut c) = open(WITH_EXTENSIONS);
    let mut rows = c.file().network().host_overrides;
    rows[0].address = "127.0.0.2".into();
    rows.push(HostOverride::new("new.test", "127.0.0.3:9"));
    c.file_mut().set_host_overrides(&rows);
    c.file_mut().save().expect("saves");
    let text = saved(&dir);
    assert!(text.contains("flow: sequential"), "{text}");
    assert!(text.contains("ops: {}"), "{text}");
    assert!(text.contains("x-note: kept"), "{text}");
    assert!(text.contains("just a string"), "{text}");
    assert!(text.contains("address: 127.0.0.2"), "{text}");
    assert!(text.contains("host: new.test"), "{text}");
    let (_d2, again) = open(&text);
    assert_eq!(again.file().network().host_overrides.len(), 3);
}

#[test]
fn clearing_the_rows_removes_only_the_sections_it_emptied() {
    let (dir, mut c) = open(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nextensions:\n  apinata:\n    hostOverrides:\n      - host: a.test\n        address: 127.0.0.1\n",
    );
    c.file_mut().set_host_overrides(&[]);
    c.file_mut().save().expect("saves");
    // The writer normalises a file it saves, so compare with Bruno's form of the same document.
    assert_eq!(saved(&dir), "opencollection: 1.0.0\n\ninfo:\n  name: t\n");

    let (dir, mut c) = open(&WITH_EXTENSIONS.replace("      - just a string\n", ""));
    c.file_mut().set_host_overrides(&[]);
    c.file_mut().save().expect("saves");
    let text = saved(&dir);
    assert!(!text.contains("hostOverrides"), "{text}");
    assert!(
        text.contains("ops: {}") && text.contains("flow: sequential"),
        "{text}"
    );
}

#[test]
fn an_ipv4_mapped_unspecified_address_is_refused_like_0_0_0_0() {
    for address in ["::ffff:0.0.0.0", "[::ffff:0.0.0.0]:80", "::ffff:0:0"] {
        let err = resolve(&[("fake.test", address, false)], "http://x.test/").unwrap_err();
        match &err {
            ResolveError::InvalidHostOverride { reason, .. } => {
                assert!(reason.contains("unspecified"), "{address}: {reason}");
            }
            other => panic!("{address}: {other:?}"),
        }
    }
}

#[test]
fn a_zone_id_in_an_override_address_is_a_typed_error_naming_it() {
    for address in ["fe80::1%2", "[fe80::1%2]:80", "fe80::1%eth0"] {
        let err = resolve(&[("fake.test", address, false)], "http://x.test/").unwrap_err();
        match &err {
            ResolveError::InvalidHostOverride { reason, .. } => {
                assert!(reason.contains("zone id"), "{address}: {reason}");
            }
            other => panic!("{address}: {other:?}"),
        }
    }
}

#[test]
fn an_incomplete_row_in_the_file_fails_naming_the_setting_and_the_file() {
    for (host, address) in [("", ""), ("fake.test", ""), ("", "127.0.0.1")] {
        let err = resolve(&[(host, address, false)], "http://x.test/").unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("Network settings") && text.contains("opencollection.yml"),
            "{text}"
        );
    }
}
