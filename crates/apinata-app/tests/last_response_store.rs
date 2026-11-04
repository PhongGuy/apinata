use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::last_response::{BODY_LIMIT, Limits, ResponseStore, Stored, keeps};
use apinata_core::{
    Assertion, AssertionOutcome, ItemPath, Outcome, RequestResult, Response, Verdict,
};

fn ip(s: &str) -> ItemPath {
    ItemPath::new(s).unwrap()
}

fn at(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn result(item: &str, response: Response) -> RequestResult {
    let outcomes = vec![
        AssertionOutcome::new(
            Assertion::new("res.status", "eq", Some("200".into())),
            Verdict::Pass,
        ),
        AssertionOutcome::new(
            Assertion::new("res.body", "isJson", None),
            Verdict::fail("\"x\"", Some("d".into())),
        ),
        AssertionOutcome::new(
            Assertion::new("res.x", "eq", Some("{{y}}".into())),
            Verdict::skipped("needs js"),
        ),
    ];
    RequestResult::from_parts(ip(item), "A", Outcome::Failed, Some(response), outcomes)
}

fn json_response() -> Response {
    Response::new(200)
        .with_status_text("OK")
        .with_header("Content-Type", "application/json")
        .with_header("x-a", "1")
        .with_header("x-a", "2")
        .with_text_body("{\"a\":1}")
        .with_response_time(Duration::from_millis(42))
        .with_url("http://127.0.0.1:9/ok")
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_saved_response_loads_back_whole() {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("responses"));
    let root = Path::new("/c");
    let saved = result("a.yml", json_response());
    store
        .save(root, &ip("a.yml"), &saved, at(1_760_000_000_123))
        .unwrap();
    let got = store.load(root, &ip("a.yml")).expect("kept");
    assert_eq!(got.saved_at, at(1_760_000_000_123));
    assert!(!got.body_dropped);
    assert_eq!(got.result.path, ip("a.yml"));
    assert_eq!(got.result.name, "A");
    assert_eq!(got.result.outcome, Outcome::Failed);
    assert_eq!(got.result.response, saved.response);
    assert_eq!(got.result.assertions, saved.assertions);
    assert!(store.load(root, &ip("b.yml")).is_none());
    assert!(store.load(Path::new("/other"), &ip("a.yml")).is_none());
}

#[test]
fn the_send_file_keeps_the_version_1_entry_format() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("responses");
    let store = ResponseStore::new(dir.clone());
    store
        .save(
            Path::new("/c"),
            &ip("a.yml"),
            &result("a.yml", json_response()),
            at(5),
        )
        .unwrap();
    let index: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("index.json")).unwrap()).unwrap();
    assert_eq!(index["version"], 2);
    let entry = &index["entries"][0];
    assert_eq!(entry["root"], "/c");
    assert_eq!(entry["item"], "a.yml");
    let id = entry["id"].as_str().unwrap().to_owned();
    assert_eq!(id.len(), 16);
    assert!(
        id.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{id}.0.json"))).unwrap())
            .unwrap();
    assert_eq!(meta["version"], 1);
    assert_eq!(meta["saved_at_ms"], 5);
    assert_eq!(meta["status"], 200);
    assert_eq!(meta["outcome"], "failed");
    assert_eq!(meta["text"], "{\"a\":1}");
    assert_eq!(meta["has_body_file"], false);
    assert_eq!(meta["headers"][1], serde_json::json!(["x-a", "1"]));
    assert_eq!(meta["assertions"][1]["verdict"], "fail");
    assert_eq!(meta["assertions"][2]["reason"], "needs js");
    assert_eq!(
        listing(&dir),
        vec![format!("{id}.0.json"), "index.json".to_owned()]
    );
}

#[test]
fn raw_bytes_that_differ_from_the_text_go_to_a_body_file() {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("r"));
    let binary = Response::new(200).with_body(vec![0xff, 0x41], "\u{fffd}A");
    let root = Path::new("/c");
    store
        .save(root, &ip("b.yml"), &result("b.yml", binary.clone()), at(1))
        .unwrap();
    assert!(
        listing(&data.path().join("r"))
            .iter()
            .any(|n| n.ends_with(".body"))
    );
    let got = store.load(root, &ip("b.yml")).unwrap();
    assert_eq!(got.result.response.unwrap().body(), binary.body());
}

#[test]
fn a_text_save_with_history_off_leaves_no_body_file() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    let binary = Response::new(200).with_body(vec![0xff], "\u{fffd}");
    let off = Limits {
        per_request: 1,
        total_bytes: u64::MAX,
    };
    store
        .save_with(root, &ip("b.yml"), &result("b.yml", binary), at(1), off)
        .unwrap();
    store
        .save_with(
            root,
            &ip("b.yml"),
            &result("b.yml", json_response()),
            at(2),
            off,
        )
        .unwrap();
    assert!(!listing(&dir).iter().any(|n| n.ends_with(".body")));
    assert_eq!(store.load(root, &ip("b.yml")).unwrap().saved_at, at(2));
}

#[test]
fn a_body_over_the_limit_is_dropped_but_the_rest_is_kept() {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("r"));
    let root = Path::new("/c");
    let big = Response::new(200)
        .with_header("x-a", "1")
        .with_text_body("x".repeat(BODY_LIMIT + 1))
        .with_response_time(Duration::from_millis(7));
    store
        .save(root, &ip("a.yml"), &result("a.yml", big.clone()), at(1))
        .unwrap();
    let got = store.load(root, &ip("a.yml")).unwrap();
    assert!(got.body_dropped);
    let r = got.result.response.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers(), big.headers());
    assert_eq!(r.response_time(), Duration::from_millis(7));
    assert_eq!(r.size(), big.size());
    assert!(r.body().is_empty());
    let at_limit = Response::new(200).with_text_body("y".repeat(BODY_LIMIT));
    store
        .save(root, &ip("a.yml"), &result("a.yml", at_limit), at(2))
        .unwrap();
    assert!(!store.load(root, &ip("a.yml")).unwrap().body_dropped);
}

#[test]
fn a_truncated_response_keeps_its_mark_and_size() {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("r"));
    let cut = Response::new(200).with_text_body("abc").with_truncated(99);
    store
        .save(
            Path::new("/c"),
            &ip("a.yml"),
            &result("a.yml", cut.clone()),
            at(1),
        )
        .unwrap();
    let r = store
        .load(Path::new("/c"), &ip("a.yml"))
        .unwrap()
        .result
        .response
        .unwrap();
    assert!(r.truncated());
    assert_eq!(r.size(), 99);
    assert_eq!(r, cut);
}

#[test]
fn a_result_without_a_response_writes_nothing() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    let none = RequestResult::from_parts(ip("a.yml"), "A", Outcome::Errored, None, Vec::new());
    store
        .save(Path::new("/c"), &ip("a.yml"), &none, at(1))
        .unwrap();
    assert!(!dir.exists());
}

#[test]
fn corrupt_or_foreign_files_read_as_missing() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    store
        .save(root, &ip("a.yml"), &result("a.yml", json_response()), at(1))
        .unwrap();
    store
        .save(root, &ip("b.yml"), &result("b.yml", json_response()), at(1))
        .unwrap();
    let index: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("index.json")).unwrap()).unwrap();
    let id_of = |item: &str| {
        index["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["item"] == item)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    // A garbled entry file is a miss, and the entry leaves the index.
    std::fs::write(dir.join(format!("{}.0.json", id_of("a.yml"))), "{not json").unwrap();
    assert!(store.load(root, &ip("a.yml")).is_none());
    let after = std::fs::read_to_string(dir.join("index.json")).unwrap();
    assert!(!after.contains("\"a.yml\""));
    // An entry written by a future version is a miss too.
    let b = dir.join(format!("{}.0.json", id_of("b.yml")));
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&b).unwrap()).unwrap();
    meta["version"] = serde_json::json!(2);
    std::fs::write(&b, meta.to_string()).unwrap();
    assert!(store.load(root, &ip("b.yml")).is_none());
    // A garbled index reads as empty, and the next save rewrites it.
    std::fs::write(dir.join("index.json"), "]]").unwrap();
    assert!(store.load(root, &ip("b.yml")).is_none());
    store
        .save(root, &ip("c.yml"), &result("c.yml", json_response()), at(3))
        .unwrap();
    assert_eq!(store.load(root, &ip("c.yml")).unwrap().saved_at, at(3));
}

#[test]
fn a_missing_body_file_is_a_miss() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    let binary = Response::new(200).with_body(vec![0xff], "\u{fffd}");
    store
        .save(
            Path::new("/c"),
            &ip("b.yml"),
            &result("b.yml", binary),
            at(1),
        )
        .unwrap();
    for name in listing(&dir).into_iter().filter(|n| n.ends_with(".body")) {
        std::fs::remove_file(dir.join(name)).unwrap();
    }
    assert!(store.load(Path::new("/c"), &ip("b.yml")).is_none());
}

#[test]
fn nothing_is_written_beside_the_collection() {
    let col = tempfile::tempdir().unwrap();
    std::fs::write(col.path().join("a.yml"), "x").unwrap();
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("r"));
    store
        .save(
            col.path(),
            &ip("a.yml"),
            &result("a.yml", json_response()),
            at(1),
        )
        .unwrap();
    assert_eq!(listing(col.path()), vec!["a.yml".to_owned()]);
}

#[test]
fn only_an_existing_request_file_keeps_a_response() {
    let col = tempfile::tempdir().unwrap();
    std::fs::write(col.path().join("a.yml"), "x").unwrap();
    std::fs::create_dir(col.path().join("users")).unwrap();
    assert!(keeps(col.path(), &ip("a.yml")));
    assert!(!keeps(col.path(), &ip("draft-1.yml")));
    assert!(!keeps(col.path(), &ip("users")));
}

// Draft numbering restarts each run, so a kept draft response would surface under a later draft.
#[test]
fn a_draft_never_keeps_a_response_even_when_a_file_shares_its_name() {
    let col = tempfile::tempdir().unwrap();
    std::fs::write(col.path().join("untitled-1.draft"), "x").unwrap();
    assert!(!keeps(col.path(), &ip("untitled-1.draft")));
}

#[test]
fn a_sub_millisecond_response_time_survives() {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("r"));
    let quick = Response::new(200).with_response_time(Duration::from_micros(300));
    store
        .save(
            Path::new("/c"),
            &ip("a.yml"),
            &result("a.yml", quick),
            at(1),
        )
        .unwrap();
    let time = store
        .load(Path::new("/c"), &ip("a.yml"))
        .unwrap()
        .result
        .response
        .unwrap()
        .response_time();
    assert!(!time.is_zero());
    assert!(time.abs_diff(Duration::from_micros(300)) <= Duration::from_micros(1));
}

/// The result of `result` plus one script test per verdict and a script-error entry.
fn scripted(item: &str) -> RequestResult {
    use apinata_core::{ConsoleEntry, ConsoleLevel, ScriptPhase, TestOutcome};
    let mut r = result(item, json_response());
    let mut thrown = TestOutcome::new(
        ScriptPhase::PostResponse,
        "post-response script",
        Verdict::fail("TypeError: x", None),
    );
    thrown.script_error = true;
    r.tests = vec![
        TestOutcome::new(ScriptPhase::Tests, "status ok", Verdict::Pass),
        TestOutcome::new(
            ScriptPhase::Tests,
            "body",
            Verdict::fail("expected 1 to equal 2", Some("d".into())),
        ),
        TestOutcome::new(ScriptPhase::PreRequest, "later", Verdict::skipped("bail")),
        thrown,
    ];
    r.console = vec![ConsoleEntry::new(
        ScriptPhase::Tests,
        ConsoleLevel::Log,
        "logged-value",
    )];
    r
}

#[test]
fn script_tests_load_back_with_the_response() {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("responses"));
    let root = Path::new("/c");
    let saved = scripted("a.yml");
    store.save(root, &ip("a.yml"), &saved, at(7)).unwrap();
    let got = store.load(root, &ip("a.yml")).expect("kept");
    assert_eq!(got.result.tests, saved.tests);
    let seq = store.history(root, &ip("a.yml"))[0].seq;
    let picked = store.load_send(root, &ip("a.yml"), seq).expect("kept");
    assert_eq!(picked.result.tests, saved.tests);
}

#[test]
fn console_output_is_never_written_to_disk() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("responses");
    let store = ResponseStore::new(dir.clone());
    store
        .save(Path::new("/c"), &ip("a.yml"), &scripted("a.yml"), at(7))
        .unwrap();
    for name in listing(&dir) {
        let text = std::fs::read_to_string(dir.join(&name)).unwrap();
        assert!(!text.contains("logged-value"), "{name}");
    }
    let got = store.load(Path::new("/c"), &ip("a.yml")).unwrap();
    assert!(got.result.console.is_empty());
}

#[test]
fn a_send_file_without_tests_still_loads() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("responses");
    let store = ResponseStore::new(dir.clone());
    let root = Path::new("/c");
    store
        .save(root, &ip("a.yml"), &result("a.yml", json_response()), at(5))
        .unwrap();
    // Rewrite the send file as an older build wrote it, with no tests key.
    let file = listing(&dir)
        .into_iter()
        .find(|n| n.ends_with(".0.json"))
        .unwrap();
    let path = dir.join(file);
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    meta.as_object_mut().unwrap().remove("tests");
    std::fs::write(&path, meta.to_string()).unwrap();
    let got = store
        .load(root, &ip("a.yml"))
        .expect("an old entry still loads");
    assert!(got.result.tests.is_empty());
    assert_eq!(got.result.assertions.len(), 3);
}

const KEPT_SECRET: &str = "Echo-S3cret-42";

// Every file the store wrote, as bytes, so a non-UTF-8 body file is searched too.
fn store_holds(dir: &Path, needle: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let bytes = std::fs::read(p).unwrap();
            bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
        })
        .map(|p| p.display().to_string())
        .collect()
}

#[test]
fn every_kept_response_field_is_scrubbed() {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("responses"));
    let response = Response::new(200)
        .with_status_text(format!("OK {KEPT_SECRET}"))
        .with_header("x-echo", format!("Bearer {KEPT_SECRET}"))
        .with_text_body(format!(r#"{{"auth":"Bearer {KEPT_SECRET}"}}"#));
    let assertions = vec![AssertionOutcome::new(
        Assertion::new("res.body.auth", "eq", Some(KEPT_SECRET.into())),
        Verdict::fail(format!("Bearer {KEPT_SECRET}"), Some(KEPT_SECRET.into())),
    )];
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Failed,
        Some(response),
        assertions,
    );
    let secrets = vec![KEPT_SECRET.to_owned()];
    store
        .save_redacted(
            dir.path(),
            &ip("a.yml"),
            &result,
            &secrets,
            UNIX_EPOCH,
            Limits::default(),
        )
        .unwrap();
    assert!(store_holds(store.dir(), KEPT_SECRET).is_empty());
    let kept = store.load(dir.path(), &ip("a.yml")).expect("kept");
    let kept = kept.result.response.expect("a response");
    assert_eq!(kept.text(), r#"{"auth":"Bearer [redacted]"}"#);
    assert!(
        kept.headers()
            .iter()
            .any(|(n, v)| n == "x-echo" && v == "Bearer [redacted]")
    );
}

#[test]
fn a_raw_body_holding_a_secret_is_not_kept() {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("responses"));
    let mut bytes = vec![0xff_u8];
    bytes.extend_from_slice(format!("key={KEPT_SECRET}").as_bytes());
    let response = Response::new(200).with_body(bytes, format!("\u{fffd}key={KEPT_SECRET}"));
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    store
        .save_redacted(
            dir.path(),
            &ip("a.yml"),
            &result,
            &[KEPT_SECRET.to_owned()],
            UNIX_EPOCH,
            Limits::default(),
        )
        .unwrap();
    assert!(store_holds(store.dir(), KEPT_SECRET).is_empty());
    let kept = store.load(dir.path(), &ip("a.yml")).expect("kept");
    assert_eq!(
        kept.result.response.expect("a response").text(),
        "\u{fffd}key=[redacted]"
    );
}

#[test]
fn a_clean_raw_body_keeps_its_body_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("responses"));
    let bytes = vec![0xff_u8, b'o', b'k'];
    let response = Response::new(200).with_body(bytes.clone(), "\u{fffd}ok");
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    store
        .save_redacted(
            dir.path(),
            &ip("a.yml"),
            &result,
            &[KEPT_SECRET.to_owned()],
            UNIX_EPOCH,
            Limits::default(),
        )
        .unwrap();
    let kept = store.load(dir.path(), &ip("a.yml")).expect("kept");
    assert_eq!(
        kept.result.response.expect("a response").body(),
        bytes.as_slice()
    );
}

const URL_PASSWORD: &str = "hunter2pw";

// The headers whose value can carry a url, each holding a password the request never sent.
fn url_headers() -> [(&'static str, String); 3] {
    let url = format!("http://joe:{URL_PASSWORD}@127.0.0.1:1/ok");
    [
        ("Location", url.clone()),
        ("Content-Location", url.clone()),
        ("Refresh", format!("0; url={url}")),
    ]
}

// Keeps one response with `headers` and no secrets, then loads it back.
fn kept_with_headers(dir: &Path, headers: &[(&str, String)]) -> (ResponseStore, Stored) {
    let store = ResponseStore::new(dir.join("responses"));
    let response = headers
        .iter()
        .fold(Response::new(302), |r, (n, v)| r.with_header(*n, v.clone()));
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    store.save(dir, &ip("a.yml"), &result, UNIX_EPOCH).unwrap();
    let stored = store.load(dir, &ip("a.yml")).expect("kept");
    (store, stored)
}

#[test]
fn a_url_password_in_a_response_header_is_not_kept() {
    for (name, value) in url_headers() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = kept_with_headers(dir.path(), &[(name, value)]);
        let leaks = store_holds(store.dir(), URL_PASSWORD);
        assert!(leaks.is_empty(), "{name} kept the password in {leaks:?}");
    }
}

#[test]
fn a_kept_url_header_is_the_core_mask_and_marked() {
    for (name, value) in url_headers() {
        let dir = tempfile::tempdir().unwrap();
        let (_, stored) = kept_with_headers(dir.path(), &[(name, value.clone())]);
        let response = stored.result.response.expect("a response");
        assert_eq!(
            response.headers()[0].1,
            apinata_core::mask_response_header(name, &value),
            "{name}"
        );
        assert_eq!(stored.masked_headers, vec![0], "{name}");
    }
}

#[test]
fn a_kept_set_cookie_keeps_only_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let headers = [("Set-Cookie", "sid=cookievalue1".to_owned())];
    let (store, stored) = kept_with_headers(dir.path(), &headers);
    assert!(store_holds(store.dir(), "cookievalue1").is_empty());
    let response = stored.result.response.expect("a response");
    assert_eq!(response.headers()[0].1, "sid=[redacted]");
    assert_eq!(stored.masked_headers, vec![0]);
}

#[test]
fn a_plain_kept_header_is_unchanged_and_unmarked() {
    let dir = tempfile::tempdir().unwrap();
    let headers = [("Content-Type", "application/json".to_owned())];
    let (_, stored) = kept_with_headers(dir.path(), &headers);
    let response = stored.result.response.expect("a response");
    assert_eq!(response.headers()[0].1, "application/json");
    assert!(stored.masked_headers.is_empty());
}

// The raw values a kept send held on disk before its headers took the mask.
const OLD_RAW: [&str; 3] = [URL_PASSWORD, "cookievalue1", "tok123secret"];

fn old_headers() -> Vec<(&'static str, String)> {
    vec![
        (
            "Location",
            format!("http://joe:{URL_PASSWORD}@127.0.0.1:1/ok"),
        ),
        ("Set-Cookie", "sid=cookievalue1".to_owned()),
        ("Authorization", "Bearer tok123secret".to_owned()),
        ("Content-Type", "text/plain".to_owned()),
    ]
}

// Writes by hand a store holding one send of `a.yml` in the format kept sends had before their
// headers were masked: raw values, no `masked_headers`, a snapshot url with its password.
// Returns the store and the send's json path.
fn old_store(dir: &Path) -> (ResponseStore, std::path::PathBuf) {
    let responses = dir.join("responses");
    std::fs::create_dir_all(&responses).unwrap();
    let snapshot = serde_json::json!({
        "http": {"method": "GET", "url": format!("http://joe:{URL_PASSWORD}@127.0.0.1:1/x")}
    })
    .to_string();
    let headers: Vec<[String; 2]> = old_headers()
        .into_iter()
        .map(|(n, v)| [n.to_owned(), v])
        .collect();
    let entry = serde_json::json!({
        "version": 1, "saved_at_ms": 1000, "name": "a", "outcome": "passed",
        "status": 302, "status_text": "Found", "url": "http://joe@127.0.0.1:1/x",
        "headers": headers, "time_ms": 5.0, "size": 0, "truncated": false,
        "body_dropped": false, "text": "", "has_body_file": false,
        "assertions": [], "tests": [], "request": snapshot, "masked": false
    });
    let json = responses.join("k.0.json");
    std::fs::write(&json, serde_json::to_vec(&entry).unwrap()).unwrap();
    let index = serde_json::json!({
        "version": apinata_app::last_response::FORMAT_VERSION,
        "entries": [{
            "root": dir.to_string_lossy(), "item": "a.yml", "id": "k",
            "sends": [{
                "seq": 0, "saved_at_ms": 1000, "status": 302, "time_ms": 5.0, "size": 0,
                "bytes": 10, "method": "GET", "url": "http://joe@127.0.0.1:1/x", "order": 1
            }]
        }]
    });
    std::fs::write(
        responses.join("index.json"),
        serde_json::to_vec(&index).unwrap(),
    )
    .unwrap();
    (ResponseStore::new(responses), json)
}

// Every old header is shown as core masks it, and the three it changed are marked.
fn assert_remasked(stored: &Stored) {
    let response = stored.result.response.as_ref().expect("a response");
    for (ix, (name, raw)) in old_headers().into_iter().enumerate() {
        assert_eq!(
            response.headers()[ix].1,
            apinata_core::mask_response_header(name, &raw),
            "{name}"
        );
    }
    assert_eq!(stored.masked_headers, vec![0, 1, 2]);
}

#[test]
fn an_old_kept_send_loads_masked_and_is_rewritten_without_raw_values() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = old_store(dir.path());
    let stored = store.load(dir.path(), &ip("a.yml")).expect("kept");
    assert_remasked(&stored);
    for raw in OLD_RAW {
        let leaks = store_holds(store.dir(), raw);
        assert!(leaks.is_empty(), "{raw} still in {leaks:?}");
    }
    let snapshot = store.request_json(dir.path(), &ip("a.yml"), 0).unwrap();
    assert!(snapshot.contains("joe@127.0.0.1"), "{snapshot}");
    assert_eq!(store.history(dir.path(), &ip("a.yml")).len(), 1);
    assert_remasked(&store.load(dir.path(), &ip("a.yml")).expect("still kept"));
}

#[test]
fn peeking_an_old_kept_send_masks_it_but_leaves_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let (store, json) = old_store(dir.path());
    let before = std::fs::read(&json).unwrap();
    assert_remasked(&store.peek(dir.path(), &ip("a.yml")).expect("kept"));
    assert_remasked(&store.peek_send(dir.path(), &ip("a.yml"), 0).expect("kept"));
    assert_eq!(std::fs::read(&json).unwrap(), before);
}

#[test]
fn a_send_kept_by_the_current_code_is_not_rewritten_on_load() {
    let dir = tempfile::tempdir().unwrap();
    let mut headers = url_headers().to_vec();
    headers.push(("Set-Cookie", "sid=cookievalue1".to_owned()));
    let (store, _) = kept_with_headers(dir.path(), &headers);
    let json = std::fs::read_dir(store.dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap() != "index.json" && p.extension().unwrap() == "json")
        .expect("the send file");
    // A known old mtime shows whether the load wrote the file again.
    let old = UNIX_EPOCH + Duration::from_secs(1_000_000);
    std::fs::File::options()
        .write(true)
        .open(&json)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let before = std::fs::read(&json).unwrap();
    store.load(dir.path(), &ip("a.yml")).expect("kept");
    assert_eq!(std::fs::read(&json).unwrap(), before);
    assert_eq!(std::fs::metadata(&json).unwrap().modified().unwrap(), old);
}

#[test]
fn an_old_snapshot_read_alone_has_no_url_password_and_stays_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let (store, json) = old_store(dir.path());
    let before = std::fs::read(&json).unwrap();
    let snapshot = store.request_json(dir.path(), &ip("a.yml"), 0).unwrap();
    assert!(!snapshot.contains(URL_PASSWORD), "{snapshot}");
    assert!(snapshot.contains("joe@127.0.0.1"), "{snapshot}");
    assert_eq!(std::fs::read(&json).unwrap(), before);
}

#[test]
fn two_stores_on_one_dir_saving_at_once_lose_no_send() {
    // Two stores share no Mutex, as two app processes would not.
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let root = data.path().to_path_buf();
    let per = 40;
    let writers: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|side| {
            let store = ResponseStore::new(dir.clone());
            let root = root.clone();
            std::thread::spawn(move || {
                for i in 0..per {
                    let item = format!("{side}{i}.yml");
                    store
                        .save(&root, &ip(&item), &result(&item, json_response()), at(1000))
                        .unwrap();
                }
            })
        })
        .collect();
    for w in writers {
        w.join().unwrap();
    }
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("index.json")).unwrap()).unwrap();
    let entries = index["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2 * per, "{index}");
    for entry in entries {
        let id = entry["id"].as_str().unwrap();
        for send in entry["sends"].as_array().unwrap() {
            let json = dir.join(format!("{id}.{}.json", send["seq"]));
            assert!(json.exists(), "{} missing", json.display());
        }
    }
}

#[test]
fn reading_a_store_that_does_not_exist_creates_nothing() {
    let data = tempfile::tempdir().unwrap();
    let parent = data.path().join("data");
    let store = ResponseStore::new(parent.join("r"));
    let root = data.path().to_path_buf();
    let item = ip("a.yml");
    assert!(store.recent(std::slice::from_ref(&root), 10).is_empty());
    assert!(store.history(&root, &item).is_empty());
    assert!(store.peek(&root, &item).is_none());
    assert!(store.peek_send(&root, &item, 0).is_none());
    assert!(store.load(&root, &item).is_none());
    assert!(store.load_send(&root, &item, 0).is_none());
    assert!(store.request_json(&root, &item, 0).is_none());
    assert!(!parent.exists(), "{:?}", listing(data.path()));
}

#[test]
fn a_first_save_leaves_its_lock_file_beside_the_store() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let store = ResponseStore::new(dir.clone());
    store
        .save(
            data.path(),
            &ip("a.yml"),
            &result("a.yml", json_response()),
            at(1000),
        )
        .unwrap();
    assert!(
        data.path().join("r.lock").is_file(),
        "{:?}",
        listing(data.path())
    );
    store.clear().unwrap();
    assert!(!dir.exists());
    assert!(data.path().join("r.lock").is_file());
}

// Runs `write` on a thread while the test holds `<dir>.lock`, and checks it waits for the lock.
fn waits_for_the_lock_file(lock: &Path, write: impl FnOnce() + Send + 'static) {
    let held = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock)
        .unwrap();
    held.lock().unwrap();
    let (done, finished) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        write();
        done.send(()).unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        finished.try_recv().is_err(),
        "wrote while the lock was held"
    );
    drop(held);
    finished.recv_timeout(Duration::from_secs(10)).unwrap();
    writer.join().unwrap();
}

#[test]
fn every_write_takes_the_lock_file_even_on_a_missing_store() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("r");
    let lock = data.path().join("r.lock");
    let root = data.path().to_path_buf();
    let other = ResponseStore::new(dir.clone());
    let store = ResponseStore::new(dir.clone());
    let saved = result("a.yml", json_response());
    other.save(&root, &ip("a.yml"), &saved, at(1000)).unwrap();
    other.clear().unwrap();
    assert!(!dir.exists());

    let (s, r) = (store.clone(), root.clone());
    waits_for_the_lock_file(&lock, move || {
        s.rename(&r, &ip("a.yml"), &ip("b.yml")).unwrap();
    });
    let (s, r) = (store.clone(), root.clone());
    waits_for_the_lock_file(&lock, move || s.remove(&r, &ip("a.yml")).unwrap());
    let (s, r) = (store.clone(), root.clone());
    waits_for_the_lock_file(&lock, move || {
        s.save(&r, &ip("a.yml"), &saved, at(2000)).unwrap();
    });
    assert!(store.peek(&root, &ip("a.yml")).is_some());
}

// The kept response is a record of a send, so its url keeps no username, which can be a token.
#[test]
fn a_kept_response_url_holds_no_username() {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("responses"));
    for url in [
        "http://acct-7781@127.0.0.1:9/ok",
        "http://acct-7781:pw-55@127.0.0.1:9/ok",
    ] {
        let response = Response::new(200).with_url(url);
        store
            .save_redacted(
                dir.path(),
                &ip("a.yml"),
                &result("a.yml", response),
                &[],
                UNIX_EPOCH,
                Limits::default(),
            )
            .unwrap();
        let kept = store.load(dir.path(), &ip("a.yml")).expect("kept");
        assert_eq!(
            kept.result.response.expect("a response").url(),
            "http://127.0.0.1:9/ok"
        );
        let sends = store.history(dir.path(), &ip("a.yml"));
        assert!(sends.iter().all(|s| !s.url.contains("acct-7781")), "{url}");
        assert!(store_holds(store.dir(), "acct-7781").is_empty(), "{url}");
    }
}

// A send kept by an older version holds its url's username; loading it rewrites the file without one.
#[test]
fn loading_an_old_send_drops_its_url_username_from_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let (store, json) = old_store(dir.path());
    let stored = store.load(dir.path(), &ip("a.yml")).expect("kept");
    let response = stored.result.response.expect("a response");
    assert_eq!(response.url(), "http://127.0.0.1:1/x");
    let entry: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(entry["url"], "http://127.0.0.1:1/x");
}
