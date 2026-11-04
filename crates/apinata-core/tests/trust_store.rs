//! Host normalisation, loopback detection and the per-user trust file.

use apinata_core::{
    TrustError, TrustStore, bare_host_refusal, is_loopback, normalize_bare_host, normalize_host,
};
use std::fs;
use std::net::IpAddr;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn loopback_is_127_slash_8_ipv6_one_and_mapped_127() {
    for ip in [
        "127.0.0.1",
        "127.255.0.9",
        "::1",
        "::ffff:127.0.0.1",
        "::ffff:127.1.2.3",
    ] {
        assert!(is_loopback(ip.parse::<IpAddr>().unwrap()), "{ip}");
    }
    for ip in [
        "10.0.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "0.0.0.0",
        "::",
        "::ffff:10.0.0.1",
        "fe80::1",
        "192.0.2.1",
    ] {
        assert!(!is_loopback(ip.parse::<IpAddr>().unwrap()), "{ip}");
    }
}

#[test]
fn hosts_normalise_as_the_url_parser_reads_them() {
    let cases = [
        ("Example.COM", "example.com"),
        ("example.com.", "example.com"),
        ("https://API.example.com:8443/v1?x=1", "api.example.com"),
        ("user:pw@api.example.com", "api.example.com"),
        ("BÜCHER.example", "xn--bcher-kva.example"),
        ("ab%43d.example", "abcd.example"),
        ("0x7f.1", "127.0.0.1"),
        ("2130706433", "127.0.0.1"),
        ("127.0.0.1.", "127.0.0.1"),
        ("::1", "[::1]"),
        ("[::1]:8080", "[::1]"),
        ("[::FFFF:127.0.0.1]", "[::ffff:7f00:1]"),
    ];
    for (input, expected) in cases {
        assert_eq!(normalize_host(input).as_deref(), Some(expected), "{input}");
    }
    for bad in ["", "   ", "http://", "exa mple.com", "[::1"] {
        assert_eq!(normalize_host(bad), None, "{bad:?}");
    }
}

#[test]
fn a_missing_file_is_an_empty_store_and_save_round_trips_normalised_entries() {
    let dir = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    let path = dir.path().join("nested/trusted-hosts.json");
    let mut store = TrustStore::load(&path).expect("missing file loads empty");
    assert!(store.trusted(coll.path()).is_empty());

    store.trust(coll.path(), "API.Example.com.").unwrap();
    store
        .trust(coll.path(), "https://api.example.com:443/")
        .unwrap();
    store.deny(coll.path(), "Evil.Example").unwrap();
    store.save().expect("saves, creating the directory");

    let loaded = TrustStore::load(&path).unwrap();
    assert_eq!(loaded.trusted(coll.path()), ["api.example.com"]);
    assert_eq!(loaded.denied(coll.path()), ["evil.example"]);
    assert!(loaded.is_trusted(coll.path(), "API.EXAMPLE.COM"));
    assert!(loaded.is_trusted(coll.path(), "api.example.com."));
    assert!(
        !loaded.is_trusted(coll.path(), "sub.api.example.com"),
        "no wildcards"
    );
    assert!(!loaded.is_trusted(coll.path(), "evil.example"));
    assert!(loaded.is_denied(coll.path(), "EVIL.example."));

    let other = TempDir::new().unwrap();
    assert!(
        !loaded.is_trusted(other.path(), "api.example.com"),
        "per collection"
    );

    // The file holds the documented shape under the canonical root, and no temporary is left.
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let root = fs::canonicalize(coll.path()).unwrap();
    let key = root.to_string_lossy();
    assert_eq!(json["version"], 1);
    assert_eq!(
        json["collections"][key.as_ref()]["trusted"][0],
        "api.example.com"
    );
    assert_eq!(
        json["collections"][key.as_ref()]["denied"][0],
        "evil.example"
    );
    let names: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["trusted-hosts.json"]);
}

#[test]
fn trust_and_deny_move_a_host_between_lists_and_remove_clears_it() {
    let dir = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    let mut store = TrustStore::empty(dir.path().join("t.json"));
    store.deny(coll.path(), "api.example.com").unwrap();
    store.trust(coll.path(), "api.example.com").unwrap();
    assert!(store.is_trusted(coll.path(), "api.example.com"));
    assert!(!store.is_denied(coll.path(), "api.example.com"));
    store.deny(coll.path(), "API.example.com").unwrap();
    assert!(!store.is_trusted(coll.path(), "api.example.com"));
    assert!(store.is_denied(coll.path(), "api.example.com"));
    assert!(store.remove(coll.path(), "api.example.com."));
    assert!(!store.remove(coll.path(), "api.example.com"));
    assert!(!store.is_denied(coll.path(), "api.example.com"));
    assert_eq!(
        store.trust(coll.path(), "exa mple"),
        Err(TrustError::InvalidHost {
            host: "exa mple".into()
        })
    );
}

#[test]
fn collection_keys_are_canonical_roots() {
    let dir = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    fs::create_dir(coll.path().join("sub")).unwrap();
    let spelled = coll.path().join("sub/..");
    let mut store = TrustStore::empty(dir.path().join("t.json"));
    store.trust(&spelled, "api.example.com").unwrap();
    assert!(store.is_trusted(coll.path(), "api.example.com"));
}

#[test]
fn hand_written_entries_are_normalised_and_bad_files_are_errors() {
    let dir = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    let root = fs::canonicalize(coll.path()).unwrap();
    let path = dir.path().join("t.json");
    let file = serde_json::json!({
        "version": 1,
        "collections": { root.to_string_lossy(): { "trusted": ["API.Example.COM.", "not a host"] } }
    });
    fs::write(&path, file.to_string()).unwrap();
    let store = TrustStore::load(&path).unwrap();
    assert_eq!(store.trusted(coll.path()), ["api.example.com"]);
    assert!(store.denied(coll.path()).is_empty());

    fs::write(&path, "{ not json").unwrap();
    assert!(matches!(
        TrustStore::load(&path),
        Err(TrustError::Malformed { .. })
    ));
    fs::write(&path, r#"{"version": 2, "collections": {}}"#).unwrap();
    assert!(matches!(
        TrustStore::load(&path),
        Err(TrustError::Malformed { .. })
    ));
}

#[test]
fn the_default_path_is_under_the_config_dir() {
    let expected = dirs::config_dir().map(|d| d.join("apinata/trusted-hosts.json"));
    assert_eq!(TrustStore::default_path(), expected);
}

#[test]
fn a_bare_host_is_normalised_and_anything_else_is_refused() {
    for (input, expected) in [
        ("api.example.com", "api.example.com"),
        ("API.Example.com.", "api.example.com"),
        ("[::1]", "[::1]"),
        ("10.0.0.1", "10.0.0.1"),
    ] {
        assert_eq!(
            normalize_bare_host(input).as_deref(),
            Some(expected),
            "{input}"
        );
    }
    for bad in [
        "http://good.example",
        "good.example@evil.example",
        "evil.example#.good.example",
        "evil.example/.good",
        "good.example:8080",
        "ex\tample.com",
        "exa mple",
        "",
    ] {
        assert_eq!(normalize_bare_host(bad), None, "{bad:?}");
    }
}

#[test]
fn concurrent_saves_never_collide_on_a_temporary_file() {
    // Several processes (the app, the CLI) may save at once; each save must succeed and the
    // file must always read as a whole document.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("trusted-hosts.json");
    let writers: Vec<_> = (0..8)
        .map(|n| {
            let path = path.clone();
            std::thread::spawn(move || {
                for i in 0..50 {
                    let mut store = TrustStore::empty(&path);
                    store
                        .trust(Path::new("/c"), &format!("h{n}-{i}.example"))
                        .unwrap();
                    store.save().unwrap();
                    TrustStore::load(&path).unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().expect("a save or load failed");
    }
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n != "trusted-hosts.json")
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[cfg(unix)]
#[test]
fn saving_through_a_symlink_writes_its_target_and_keeps_the_link() {
    let dir = tempfile::TempDir::new().unwrap();
    let dotfiles = dir.path().join("dotfiles");
    std::fs::create_dir(&dotfiles).unwrap();
    let real = dotfiles.join("trusted-hosts.json");
    let link = dir.path().join("trusted-hosts.json");
    std::os::unix::fs::symlink("dotfiles/trusted-hosts.json", &link).unwrap();
    let mut store = TrustStore::empty(&link);
    store.trust(Path::new("/c"), "a.example").unwrap();
    store.save().unwrap();
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        TrustStore::load(&real)
            .unwrap()
            .is_trusted(Path::new("/c"), "a.example")
    );
}

#[test]
fn wildcards_and_empty_labels_are_refused_with_a_reason() {
    for (bad, reason) in [
        ("*.example.com", "wildcard"),
        ("api.*.example", "wildcard"),
        ("example.com..", "empty label"),
        (".example.com", "empty label"),
        ("api..example.com", "empty label"),
    ] {
        assert_eq!(normalize_bare_host(bad), None, "{bad:?}");
        let message = bare_host_refusal(bad);
        assert!(message.contains(reason), "{bad:?}: {message}");
    }
}

#[test]
fn a_user_at_host_refusal_never_names_the_host_after_the_at() {
    let message = bare_host_refusal("good.example@evil.example");
    assert!(!message.contains("evil.example"), "{message}");
    assert!(message.contains("bare host name"), "{message}");
}

#[test]
fn display_host_shows_an_ipv4_mapped_address_dotted_and_leaves_the_rest() {
    use apinata_core::{display_host, normalize_host};
    let mapped = normalize_host("[::ffff:10.0.0.1]").unwrap();
    assert_eq!(display_host(&mapped), "[::ffff:10.0.0.1]");
    for same in ["api.example.com", "10.0.0.1", "[::1]", "[2001:db8::1]"] {
        assert_eq!(display_host(same), same);
    }
}

#[test]
fn display_host_handles_case_brackets_and_mapped_forms() {
    use apinata_core::display_host;
    // Hex digits in any case parse; the dotted form is lower case digits anyway.
    assert_eq!(display_host("[::FFFF:A00:1]"), "[::ffff:10.0.0.1]");
    assert_eq!(display_host("[::ffff:a00:1]"), "[::ffff:10.0.0.1]");
    // The dotted input form is shown as itself.
    assert_eq!(display_host("[::ffff:127.0.0.1]"), "[::ffff:127.0.0.1]");
    assert_eq!(display_host("[::FFFF:127.0.0.1]"), "[::ffff:127.0.0.1]");
    // Without brackets it is not an IPv6 host form, so it is left alone.
    assert_eq!(display_host("::ffff:127.0.0.1"), "::ffff:127.0.0.1");
    assert_eq!(display_host("[::ffff:7f00:1"), "[::ffff:7f00:1");
    // Names keep their case: display never rewrites a name.
    assert_eq!(display_host("API.Example.COM"), "API.Example.COM");
    // An IPv6 address that is not IPv4-mapped is unchanged, as is junk in brackets.
    assert_eq!(display_host("[2001:DB8::1]"), "[2001:DB8::1]");
    assert_eq!(display_host("[not-an-ip]"), "[not-an-ip]");
}
