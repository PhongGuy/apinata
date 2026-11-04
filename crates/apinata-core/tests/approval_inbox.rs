//! The approval inbox: file format, one pending request per collection and host, denial,
//! and resolution.

use apinata_core::{
    ApprovalInbox, DISPLAY_MAX_CHARS, Filing, ScriptTrust, TrustError, TrustStore, script_hash,
};
use std::fs;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

struct Fixture {
    _tmp: TempDir,
    coll: TempDir,
    inbox: ApprovalInbox,
    store: TrustStore,
}

fn fixture() -> Fixture {
    let tmp = TempDir::new().unwrap();
    Fixture {
        inbox: ApprovalInbox::new(tmp.path().join("approvals")),
        store: TrustStore::empty(tmp.path().join("trusted-hosts.json")),
        coll: TempDir::new().unwrap(),
        _tmp: tmp,
    }
}

fn written(filing: Filing) -> std::path::PathBuf {
    match filing {
        Filing::Written(path) => path,
        other => panic!("expected a new file, got {other:?}"),
    }
}

#[test]
fn a_request_writes_the_documented_file() {
    let f = fixture();
    let path = written(
        f.inbox
            .request(&f.store, f.coll.path(), "API.Example.com.", "users/get.yml")
            .unwrap(),
    );

    let name = path.file_name().unwrap().to_str().unwrap();
    let (pid, nanos) = name.strip_suffix(".json").unwrap().split_once('-').unwrap();
    assert_eq!(pid, std::process::id().to_string());
    assert!(nanos.parse::<u128>().is_ok(), "{name}");
    assert_eq!(path.parent().unwrap(), f.inbox.dir());

    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let root = fs::canonicalize(f.coll.path()).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["collection"], root.to_str().unwrap());
    assert_eq!(json["host"], "api.example.com");
    assert_eq!(json["request"], "users/get.yml");
    let at = json["requested_at"].as_str().unwrap();
    assert_eq!(
        (at.len(), &at[4..5], &at[10..11], &at[19..]),
        (20, "-", "T", "Z"),
        "{at}"
    );
    assert_eq!(json.as_object().unwrap().len(), 5);

    let pending = f.inbox.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].collection, root);
    assert_eq!(pending[0].host, "api.example.com");
    assert_eq!(pending[0].request, "users/get.yml");
    assert_eq!(pending[0].requested_at, at);
    assert_eq!(pending[0].file, path);
    let names: Vec<_> = fs::read_dir(f.inbox.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "no temporary left: {names:?}");
}

#[test]
fn one_pending_request_per_collection_and_host() {
    let f = fixture();
    written(
        f.inbox
            .request(&f.store, f.coll.path(), "api.example.com", "a.yml")
            .unwrap(),
    );
    assert_eq!(
        f.inbox
            .request(&f.store, f.coll.path(), "API.example.com", "b.yml")
            .unwrap(),
        Filing::Pending
    );
    written(
        f.inbox
            .request(&f.store, f.coll.path(), "other.example.com", "a.yml")
            .unwrap(),
    );
    let other = TempDir::new().unwrap();
    written(
        f.inbox
            .request(&f.store, other.path(), "api.example.com", "a.yml")
            .unwrap(),
    );
    assert_eq!(f.inbox.pending().unwrap().len(), 3);
}

#[test]
fn a_denied_host_files_nothing() {
    let mut f = fixture();
    f.store.deny(f.coll.path(), "evil.example").unwrap();
    assert_eq!(
        f.inbox
            .request(&f.store, f.coll.path(), "EVIL.example.", "a.yml")
            .unwrap(),
        Filing::Denied
    );
    assert!(f.inbox.pending().unwrap().is_empty());
}

#[test]
fn resolve_deletes_only_that_pair() {
    let f = fixture();
    written(
        f.inbox
            .request(&f.store, f.coll.path(), "api.example.com", "a.yml")
            .unwrap(),
    );
    written(
        f.inbox
            .request(&f.store, f.coll.path(), "other.example.com", "a.yml")
            .unwrap(),
    );
    assert_eq!(
        f.inbox.resolve(f.coll.path(), "API.example.com.").unwrap(),
        1
    );
    let left = f.inbox.pending().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].host, "other.example.com");
    assert_eq!(
        f.inbox.resolve(f.coll.path(), "api.example.com").unwrap(),
        0
    );
    written(
        f.inbox
            .request(&f.store, f.coll.path(), "api.example.com", "a.yml")
            .unwrap(),
    );
}

#[test]
fn pending_skips_temporaries_strangers_and_bad_files_and_a_missing_dir_is_empty() {
    let f = fixture();
    assert!(f.inbox.pending().unwrap().is_empty());
    written(
        f.inbox
            .request(&f.store, f.coll.path(), "api.example.com", "a.yml")
            .unwrap(),
    );
    let dir = f.inbox.dir();
    fs::write(dir.join(".1-2.json.apinata-tmp"), "{").unwrap();
    fs::write(dir.join("notes.txt"), "x").unwrap();
    fs::write(dir.join("1-3.json"), "{ not json").unwrap();
    fs::write(
        dir.join("1-4.json"),
        r#"{"version":2,"collection":"/x","host":"h","request":"r","requested_at":"t"}"#,
    )
    .unwrap();
    assert_eq!(f.inbox.pending().unwrap().len(), 1);
}

#[test]
fn an_invalid_host_is_an_error_and_the_default_dir_is_under_local_data() {
    let f = fixture();
    assert!(matches!(
        f.inbox
            .request(&f.store, f.coll.path(), "exa mple", "a.yml"),
        Err(TrustError::InvalidHost { .. })
    ));
    let expected = dirs::data_local_dir().map(|d| d.join("apinata/approvals"));
    assert_eq!(ApprovalInbox::default_dir(), expected);
}

#[test]
fn prune_deletes_only_requests_older_than_the_limit() {
    let f = fixture();
    let old = written(
        f.inbox
            .request(&f.store, f.coll.path(), "old.example", "a.yml")
            .unwrap(),
    );
    written(
        f.inbox
            .request(&f.store, f.coll.path(), "new.example", "a.yml")
            .unwrap(),
    );
    let eight_days = SystemTime::now() - Duration::from_secs(8 * 86_400);
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(eight_days)
        .unwrap();

    let week = Duration::from_secs(7 * 86_400);
    assert_eq!(f.inbox.prune(week).unwrap(), 1);
    let left = f.inbox.pending().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].host, "new.example");
    assert_eq!(f.inbox.prune(week).unwrap(), 0);
    assert_eq!(
        ApprovalInbox::new(f.inbox.dir().join("missing"))
            .prune(week)
            .unwrap(),
        0
    );
}

#[test]
fn a_proxy_request_records_the_proxy_and_plain_requests_read_without_one() {
    let f = fixture();
    let path = written(
        f.inbox
            .request_via(
                &f.store,
                f.coll.path(),
                "proxy.corp.example",
                "a.yml",
                Some("http://proxy.corp.example:3128"),
            )
            .unwrap(),
    );
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["proxy"], "http://proxy.corp.example:3128");
    let other = TempDir::new().unwrap();
    written(
        f.inbox
            .request(&f.store, other.path(), "api.example.com", "b.yml")
            .unwrap(),
    );
    let pending = f.inbox.pending().unwrap();
    let seen: Vec<(&str, Option<&str>)> = pending
        .iter()
        .map(|a| (a.host.as_str(), a.proxy.as_deref()))
        .collect();
    assert!(
        seen.contains(&("proxy.corp.example", Some("http://proxy.corp.example:3128"))),
        "{seen:?}"
    );
    assert!(seen.contains(&("api.example.com", None)), "{seen:?}");
}

#[test]
fn a_file_without_mapped_from_still_loads() {
    let f = fixture();
    let path = written(
        f.inbox
            .request(&f.store, f.coll.path(), "api.example.com", "a.yml")
            .unwrap(),
    );
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(json.get("mapped_from").is_none(), "{json}");
    let pending = f.inbox.pending().unwrap();
    assert_eq!(pending[0].mapped_from, None);
}

#[test]
fn a_mapped_request_round_trips_its_name_and_keys_on_the_normalised_ip() {
    let f = fixture();
    let path = written(
        f.inbox
            .request_mapped(
                &f.store,
                f.coll.path(),
                "203.0.113.5",
                "a.yml",
                "api.example.com",
            )
            .unwrap(),
    );
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(json["mapped_from"], "api.example.com");
    let pending = f.inbox.pending().unwrap();
    assert_eq!(pending[0].host, "203.0.113.5");
    assert_eq!(pending[0].mapped_from.as_deref(), Some("api.example.com"));
}

#[test]
fn the_name_is_never_part_of_the_pending_or_trust_decision() {
    let mut f = fixture();
    written(
        f.inbox
            .request_mapped(&f.store, f.coll.path(), "203.0.113.5", "a.yml", "one.test")
            .unwrap(),
    );
    // A second name for the same address is the same pending pair.
    assert_eq!(
        f.inbox
            .request_mapped(&f.store, f.coll.path(), "203.0.113.5", "b.yml", "two.test")
            .unwrap(),
        Filing::Pending
    );
    // So is a plain request for the address.
    assert_eq!(
        f.inbox
            .request(&f.store, f.coll.path(), "203.0.113.5", "c.yml")
            .unwrap(),
        Filing::Pending
    );
    // Trusting the address decides it whatever name asked.
    f.store.trust(f.coll.path(), "203.0.113.5").unwrap();
    assert!(f.store.is_trusted(f.coll.path(), "203.0.113.5"));
    assert!(!f.store.is_trusted(f.coll.path(), "one.test"));
}

// A capped field is at most the limit, ends in the ellipsis, and the long value is nowhere in
// the file.
fn assert_capped(json: &serde_json::Value, key: &str, long: &str, raw: &str) {
    let shown = json[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} missing"));
    assert!(shown.chars().count() <= DISPLAY_MAX_CHARS, "{key}");
    assert!(shown.ends_with('\u{2026}'), "{key}: {shown}");
    assert!(shown.starts_with(&long[..10]), "{key}");
    assert!(
        !raw.contains(long),
        "{key}: the full value reached the file"
    );
}

fn read_json(path: &std::path::Path) -> (serde_json::Value, String) {
    let raw = fs::read_to_string(path).unwrap();
    (serde_json::from_str(&raw).unwrap(), raw)
}

#[test]
fn a_long_request_is_cut_with_an_ellipsis_in_the_file() {
    let f = fixture();
    let long = format!("users/{}", "a".repeat(10_000));
    let path = written(
        f.inbox
            .request(&f.store, f.coll.path(), "api.example.com", &long)
            .unwrap(),
    );
    let (json, raw) = read_json(&path);
    assert_capped(&json, "request", &long, &raw);
    assert_eq!(json["host"], "api.example.com");
}

#[test]
fn a_long_request_and_proxy_are_cut_when_filed_via_a_proxy() {
    let f = fixture();
    let long = format!("users/{}", "b".repeat(10_000));
    let proxy = format!("http://{}.example:3128", "p".repeat(10_000));
    let path = written(
        f.inbox
            .request_via(
                &f.store,
                f.coll.path(),
                "proxy.example",
                &long,
                Some(&proxy),
            )
            .unwrap(),
    );
    let (json, raw) = read_json(&path);
    assert_capped(&json, "request", &long, &raw);
    assert_capped(&json, "proxy", &proxy, &raw);
}

#[test]
fn a_long_request_and_mapped_name_are_cut_when_filed_mapped() {
    let f = fixture();
    let long = format!("users/{}", "c".repeat(10_000));
    let name = format!("{}.test", "n".repeat(10_000));
    let path = written(
        f.inbox
            .request_mapped(&f.store, f.coll.path(), "203.0.113.5", &long, &name)
            .unwrap(),
    );
    let (json, raw) = read_json(&path);
    assert_capped(&json, "request", &long, &raw);
    assert_capped(&json, "mapped_from", &name, &raw);
    assert_eq!(json["host"], "203.0.113.5");
}

#[test]
fn a_long_script_request_is_cut_with_an_ellipsis() {
    let tmp = TempDir::new().unwrap();
    let coll = TempDir::new().unwrap();
    let inbox = ApprovalInbox::new(tmp.path().join("approvals"));
    let trust = ScriptTrust::empty(tmp.path().join("script-trust.json"));
    let long = format!("users/{}", "d".repeat(10_000));
    let path = written(
        inbox
            .request_scripts(&trust, coll.path(), &long, &[script_hash("a")])
            .unwrap(),
    );
    let (json, raw) = read_json(&path);
    assert_capped(&json, "request", &long, &raw);
    assert_eq!(inbox.pending_scripts().unwrap().len(), 1);
}

#[test]
fn a_cut_never_splits_a_multi_byte_char() {
    let f = fixture();
    // Four-byte chars put byte 120 inside a char; a cut by byte would panic or break UTF-8.
    for text in [
        "\u{1F600}".repeat(500),
        format!("a{}", "\u{1F600}".repeat(500)),
    ] {
        let path = written(
            f.inbox
                .request_mapped(&f.store, f.coll.path(), "203.0.113.5", &text, &text)
                .unwrap(),
        );
        let (json, _) = read_json(&path);
        for key in ["request", "mapped_from"] {
            let shown = json[key].as_str().unwrap();
            assert_eq!(shown.chars().count(), DISPLAY_MAX_CHARS, "{key}");
            assert!(shown.ends_with('\u{2026}'), "{key}");
        }
        f.inbox.resolve(f.coll.path(), "203.0.113.5").unwrap();
    }
}

#[test]
fn a_value_at_the_limit_or_shorter_is_stored_unchanged() {
    let f = fixture();
    let exact = "e".repeat(DISPLAY_MAX_CHARS);
    let path = written(
        f.inbox
            .request_mapped(&f.store, f.coll.path(), "203.0.113.5", &exact, "one.test")
            .unwrap(),
    );
    let (json, _) = read_json(&path);
    assert_eq!(json["request"], exact.as_str());
    assert_eq!(json["mapped_from"], "one.test");
    let over = "e".repeat(DISPLAY_MAX_CHARS + 1);
    let path = written(
        f.inbox
            .request(&f.store, f.coll.path(), "api.example.com", &over)
            .unwrap(),
    );
    let (json, _) = read_json(&path);
    assert_eq!(
        json["request"].as_str().unwrap().chars().count(),
        DISPLAY_MAX_CHARS
    );
}

#[test]
fn capping_leaves_the_host_and_its_dedupe_alone() {
    let f = fixture();
    let host = format!("{}.example.com", "h".repeat(100));
    let long = "x".repeat(10_000);
    let path = written(
        f.inbox
            .request(&f.store, f.coll.path(), &host, &long)
            .unwrap(),
    );
    let (json, _) = read_json(&path);
    assert_eq!(json["host"], host.as_str());
    // A second request with a different long text is still the same pending pair.
    let other = "y".repeat(10_000);
    assert_eq!(
        f.inbox
            .request(&f.store, f.coll.path(), &host, &other)
            .unwrap(),
        Filing::Pending
    );
    assert_eq!(f.inbox.resolve(f.coll.path(), &host).unwrap(), 1);
}
