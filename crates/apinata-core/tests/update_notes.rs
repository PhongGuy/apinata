//! Release notes for an offered update: the signed notes asset, its checks and the selection.
#![cfg(feature = "update")]

use crate::update_support;

use apinata_core::update::{
    Available, GroupKind, NotesGroup, NotesItem, NotesRelease, ReleaseNotes, check, evaluate,
};
use std::io::{Read as _, Write as _};
use std::time::{Duration, Instant};
use update_support::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ART: &str = "https://example.com/apinata";

fn release(version: &str, previous: Option<&str>, item: &str) -> serde_json::Value {
    serde_json::json!({
        "version": version,
        "previous": previous,
        "date": "2026-12-01",
        "groups": [{"kind": "features", "title": "New features",
                    "items": [{"text": item, "pr": 7}]}]
    })
}

fn notes_doc(releases: Vec<serde_json::Value>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"schema": 1, "releases": releases})).unwrap()
}

/// A 4.0.0 manifest whose `notes_asset` lists `listed` at `url`.
fn manifest_with(url: &str, listed: &[u8]) -> String {
    let mut doc: serde_json::Value = serde_json::from_str(&manifest("4.0.0", ART, b"new")).unwrap();
    doc["notes_asset"] = serde_json::json!({
        "url": url, "size": listed.len(), "sha256": sha256_hex(listed)
    });
    doc.to_string()
}

/// Publishes a manifest listing `listed`, serves `served` at `/v4/notes.json`, and checks from
/// `current`.
async fn offer_with(listed: &[u8], served: ResponseTemplate, current: &str) -> Available {
    let s = MockServer::start().await;
    let k = TestKey::generate();
    let url = format!("{}/v4/notes.json", s.uri());
    publish(&s, &k, &manifest_with(&url, listed)).await;
    Mock::given(method("GET"))
        .and(path("/v4/notes.json"))
        .respond_with(served)
        .mount(&s)
        .await;
    check(&config(&s.uri(), &k, current))
        .await
        .unwrap()
        .expect("still offered")
}

async fn offer_notes(doc: &[u8], current: &str) -> Available {
    offer_with(
        doc,
        ResponseTemplate::new(200).set_body_bytes(doc.to_vec()),
        current,
    )
    .await
}

fn versions(a: &Available) -> Vec<String> {
    let notes = a.release_notes.as_ref().expect("notes");
    notes.releases.iter().map(|r| r.version.clone()).collect()
}

#[tokio::test]
async fn only_releases_newer_than_the_installed_one_show_newest_first() {
    let doc = notes_doc(vec![
        release("3.0.0", Some("2.0.0"), "c"),
        release("4.0.0", Some("3.0.0"), "d"),
        release("2.0.0", Some("1.0.0"), "b"),
        release("1.0.0", None, "a"),
    ]);
    let a = offer_notes(&doc, "2.0.0").await;
    assert_eq!(versions(&a), ["4.0.0", "3.0.0"]);
    let notes = a.release_notes.unwrap();
    assert!(!notes.older_not_listed, "3.0.0 follows 2.0.0");
    assert_eq!(a.notes_error, None);
    let item = &notes.releases[0].groups[0].items[0];
    assert_eq!((item.text.as_str(), item.pr), ("d", Some(7)));
    assert_eq!(notes.releases[0].groups[0].kind, GroupKind::Features);
}

#[tokio::test]
async fn a_list_that_stops_short_of_the_installed_version_says_so() {
    let doc = notes_doc(vec![
        release("4.0.0", Some("3.0.0"), "d"),
        release("3.0.0", Some("2.0.0"), "c"),
    ]);
    let a = offer_notes(&doc, "1.0.0").await;
    assert_eq!(versions(&a), ["4.0.0", "3.0.0"]);
    assert!(a.release_notes.unwrap().older_not_listed);
}

#[tokio::test]
async fn an_oldest_entry_without_previous_says_older_versions_may_be_missing() {
    for previous in [
        serde_json::Value::Null,
        serde_json::json!("absent"),
        serde_json::json!("not a version"),
    ] {
        let mut oldest = release("3.0.0", None, "c");
        if previous == "absent" {
            oldest.as_object_mut().unwrap().remove("previous");
        } else {
            oldest["previous"] = previous.clone();
        }
        let doc = notes_doc(vec![release("4.0.0", Some("3.0.0"), "d"), oldest]);
        let a = offer_notes(&doc, "2.0.0").await;
        assert_eq!(versions(&a), ["4.0.0", "3.0.0"]);
        assert!(a.release_notes.unwrap().older_not_listed, "{previous}");
    }
}

#[tokio::test]
async fn unreadable_notes_still_offer_the_update() {
    let doc = notes_doc(vec![release("4.0.0", Some("3.0.0"), "d")]);
    let schema_two = String::from_utf8(doc.clone())
        .unwrap()
        .replace("\"schema\":1", "\"schema\":2")
        .into_bytes();
    let big = vec![b' '; 64 * 1024 + 1];
    let cases: Vec<(&str, Vec<u8>, ResponseTemplate)> = vec![
        (
            "wrong sha",
            doc.clone(),
            ResponseTemplate::new(200)
                .set_body_bytes(doc.iter().rev().copied().collect::<Vec<_>>()),
        ),
        ("404", doc.clone(), ResponseTemplate::new(404)),
        (
            "oversized",
            big.clone(),
            ResponseTemplate::new(200).set_body_bytes(big),
        ),
        (
            "schema 2",
            schema_two.clone(),
            ResponseTemplate::new(200).set_body_bytes(schema_two),
        ),
        (
            "longer than listed",
            doc.clone(),
            ResponseTemplate::new(200).set_body_bytes([doc.clone(), b" ".to_vec()].concat()),
        ),
    ];
    for (name, listed, served) in cases {
        let a = offer_with(&listed, served, "3.0.0").await;
        assert_eq!(a.version, "4.0.0", "{name}");
        assert!(a.release_notes.is_none(), "{name}");
        assert!(a.notes_error.is_some(), "{name}");
    }
}

#[tokio::test]
async fn a_latest_or_insecure_notes_url_is_never_fetched() {
    let doc = notes_doc(vec![release("4.0.0", Some("3.0.0"), "d")]);
    for bad in [
        "/releases/latest/download/notes.json",
        "http://example.invalid/v4/notes.json",
        "not a url",
    ] {
        let s = MockServer::start().await;
        let k = TestKey::generate();
        let url = if bad.starts_with('/') {
            format!("{}{bad}", s.uri())
        } else {
            bad.to_owned()
        };
        publish(&s, &k, &manifest_with(&url, &doc)).await;
        Mock::given(method("GET"))
            .and(path("/releases/latest/download/notes.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(doc.clone()))
            .expect(0)
            .mount(&s)
            .await;
        let a = check(&config(&s.uri(), &k, "3.0.0"))
            .await
            .unwrap()
            .expect("still offered");
        assert!(a.release_notes.is_none(), "{bad}");
        assert!(a.notes_error.is_some(), "{bad}");
        s.verify().await;
    }
}

#[tokio::test]
async fn controls_and_bidi_overrides_are_stripped_and_items_capped() {
    let long = "\u{e9}\u{1F600}\u{4E2D}".repeat(200);
    let mut r = release(
        "4.0.0",
        Some("3.0.0"),
        "a\u{0007}b\u{202E}c\u{0085}d\u{2066}e\nf\u{2028}g\u{2029}h\u{200B}i\u{200C}j\u{200D}k\u{2060}l\u{FEFF}m",
    );
    r["intro"] = serde_json::json!("Hi\u{001B}[31m there\u{200F}");
    r["groups"][0]["title"] = serde_json::json!("New\u{0000} features");
    r["groups"][0]["items"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"text": long}));
    let a = offer_notes(&notes_doc(vec![r]), "3.0.0").await;
    let shown = &a.release_notes.unwrap().releases[0];
    assert_eq!(shown.intro.as_deref(), Some("Hi[31m there"));
    let group = &shown.groups[0];
    assert_eq!(group.title, "New features");
    assert_eq!(group.items[0].text, "abcde f g hijklm");
    let capped = &group.items[1].text;
    assert_eq!(capped.chars().count(), 500);
    assert!(
        long.starts_with(capped.as_str()),
        "cut on a character boundary"
    );
    assert_eq!(group.items[1].pr, None);
}

#[tokio::test]
async fn unknown_or_missing_kinds_are_other() {
    let mut r = release("4.0.0", Some("3.0.0"), "a");
    r["groups"] = serde_json::json!([
        {"kind": "security", "title": "Security", "items": [{"text": "s"}]},
        {"title": "Misc", "items": [{"text": "m"}]},
        {"kind": "fixes", "title": "Fixes", "items": [{"text": "f"}]}
    ]);
    let a = offer_notes(&notes_doc(vec![r]), "3.0.0").await;
    let kinds: Vec<GroupKind> = (a.release_notes.unwrap().releases[0].groups.iter())
        .map(|g| g.kind)
        .collect();
    assert_eq!(
        kinds,
        [GroupKind::Other, GroupKind::Other, GroupKind::Fixes]
    );
}

#[tokio::test]
async fn a_manifest_without_notes_asset_has_no_notes_and_no_error() {
    let s = MockServer::start().await;
    let k = TestKey::generate();
    publish(&s, &k, &manifest("4.0.0", ART, b"new")).await;
    let a = check(&config(&s.uri(), &k, "3.0.0"))
        .await
        .unwrap()
        .expect("offered");
    assert_eq!(a.notes.as_deref(), Some("Release notes."));
    assert!(a.release_notes.is_none());
    assert_eq!(a.notes_error, None);
}

#[test]
fn the_manifest_parser_accepts_any_notes_asset_shape() {
    let cfg = config("https://example.com", &TestKey::generate(), "3.0.0");
    let base: serde_json::Value = serde_json::from_str(&manifest("4.0.0", ART, b"new")).unwrap();
    for (asset, valid) in [
        (
            serde_json::json!({"url": "https://example.com/v4/notes.json", "size": 10,
                               "sha256": "0".repeat(64), "extra": true}),
            true,
        ),
        (serde_json::json!("not an object"), false),
        (serde_json::json!({"url": 3}), false),
    ] {
        let mut doc = base.clone();
        doc["notes_asset"] = asset.clone();
        let a = evaluate(doc.to_string().as_bytes(), &cfg)
            .unwrap()
            .expect("offered");
        assert_eq!(a.version, "4.0.0", "{asset}");
        assert!(a.release_notes.is_none());
        assert_eq!(
            a.notes_error.is_none(),
            valid,
            "{asset}: {:?}",
            a.notes_error
        );
    }
}

#[test]
fn the_notes_types_round_trip_for_a_generator() {
    let notes = ReleaseNotes::new(vec![
        NotesRelease::new(
            "4.0.0",
            Some("3.0.0".into()),
            Some("2026-12-01".into()),
            vec![NotesGroup::new(
                GroupKind::Fixes,
                "Fixes",
                vec![NotesItem::new("Fixed a crash").with_pr(12)],
            )],
        )
        .with_intro("A small release."),
    ]);
    let json = serde_json::to_value(&notes).unwrap();
    assert_eq!(json["schema"], 1);
    assert_eq!(json["releases"][0]["groups"][0]["kind"], "fixes");
    assert_eq!(json["releases"][0]["groups"][0]["items"][0]["pr"], 12);
    let back: ReleaseNotes = serde_json::from_value(json).unwrap();
    assert_eq!(back, notes);
    assert_eq!(
        NotesItem::new("Fixed a crash").with_pr(12).line(),
        "Fixed a crash (#12)"
    );
}

#[tokio::test]
async fn a_stalled_notes_host_costs_only_the_notes_timeout() {
    let doc = notes_doc(vec![release("4.0.0", Some("3.0.0"), "d")]);
    let s = MockServer::start().await;
    let k = TestKey::generate();
    let url = format!("{}/v4/notes.json", s.uri());
    publish(&s, &k, &manifest_with(&url, &doc)).await;
    Mock::given(method("GET"))
        .and(path("/v4/notes.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(doc.clone())
                .set_delay(Duration::from_secs(10)),
        )
        .mount(&s)
        .await;
    let mut cfg = config(&s.uri(), &k, "3.0.0");
    assert_eq!(cfg.limits.notes, Duration::from_secs(10));
    cfg.limits.notes = Duration::from_millis(300);
    let started = Instant::now();
    let a = check(&cfg).await.unwrap().expect("still offered");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(a.release_notes.is_none());
    assert!(a.notes_error.is_some());
}

/// Serves one HTTP response with `body` chunked, so no Content-Length is sent, then holds
/// the connection open; returns the URL it answers on.
fn chunked_server(body: Vec<u8>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let Ok((mut conn, _)) = listener.accept() else {
            return;
        };
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            match conn.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => seen.extend_from_slice(&buf[..n]),
            }
        }
        let head = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
            body.len()
        );
        let _ = conn.write_all(head.as_bytes());
        let _ = conn.write_all(&body);
        let _ = conn.write_all(b"\r\n");
        let _ = conn.flush();
        // The body never ends, so only the read cap can stop the read early.
        std::thread::sleep(Duration::from_secs(8));
    });
    format!("http://127.0.0.1:{port}/v4/notes.json")
}

#[tokio::test]
async fn the_read_stops_at_the_listed_size_without_a_content_length() {
    let doc = notes_doc(vec![release("4.0.0", Some("3.0.0"), "d")]);
    let served = [doc.clone(), vec![b' '; 1024]].concat();
    let s = MockServer::start().await;
    let k = TestKey::generate();
    publish(&s, &k, &manifest_with(&chunked_server(served), &doc)).await;
    let mut cfg = config(&s.uri(), &k, "3.0.0");
    cfg.limits.notes = Duration::from_secs(6);
    let started = Instant::now();
    let a = check(&cfg).await.unwrap().expect("still offered");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "waited for more bytes"
    );
    let why = a.notes_error.expect("refused");
    assert!(
        why.contains(&format!("larger than {} bytes", doc.len())),
        "{why}"
    );
}
