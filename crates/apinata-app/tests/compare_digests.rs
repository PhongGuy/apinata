//! Compare matches header values by their true values, then by kept digests under the current
//! key, then by the masked text; a live value keeps its true value for the reveal.

use crate::support::*;

use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use apinata_app::compare_source::{
    CompareSide, CompareSource, HeaderValue, at_rest, compare_sides, side_of_example,
    side_of_response, side_of_stored,
};
use apinata_app::last_response::{
    DIGEST_KEY_NAME, KeptDigests, Limits, ResponseStore, Stored, install,
};
use apinata_core::{
    DigestKey, ExampleResponse, ItemPath, MemoryStore, Outcome, RequestResult, Response,
    ResponseSnapshot,
};
use gpui_kit::TestAppContext;

fn ip(s: &str) -> ItemPath {
    ItemPath::new(s).unwrap()
}

fn fresh_key() -> DigestKey {
    DigestKey::load(&MemoryStore::new(), DIGEST_KEY_NAME).unwrap()
}

fn cookies(values: &[&str]) -> Response {
    let r = Response::new(200).with_header("content-type", "application/json");
    let r = values
        .iter()
        .fold(r, |r, v| r.with_header("set-cookie", *v));
    r.with_text_body("{}")
}

fn cookie(value: &str) -> Response {
    cookies(&[value])
}

// `response` kept by a store holding `key`, and read back.
fn kept(response: Response, key: Option<&DigestKey>) -> Stored {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("responses"));
    if let Some(key) = key {
        store.set_digest_key(key.clone());
    }
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    store
        .save(Path::new("/c"), &ip("a.yml"), &result, UNIX_EPOCH)
        .unwrap();
    store.load(Path::new("/c"), &ip("a.yml")).expect("kept")
}

fn cookie_changed(old: &CompareSide, new: &CompareSide) -> bool {
    (compare_sides(old, new).headers.iter()).any(|h| h.name.eq_ignore_ascii_case("set-cookie"))
}

#[test]
fn a_live_value_keeps_its_true_value_and_shows_it_scrubbed() {
    let r = Response::new(200).with_header("x-echo", "tok-66-secret");
    let side = side_of_response(&r, &["tok-66-secret".to_owned()], &[], None, None);
    let h = &side.headers[0];
    assert_eq!(h.shown, "[redacted]");
    assert_eq!(h.raw.as_deref(), Some("tok-66-secret"));
    assert!(h.masked());
    assert_eq!(h.digest, None, "no key, no digest");
}

#[test]
fn a_live_digest_is_the_keys_digest_of_the_true_value() {
    let key = fresh_key();
    let r = Response::new(200).with_header("X-Echo", "tok-66-secret");
    let side = side_of_response(&r, &["tok-66-secret".to_owned()], &[], None, Some(&key));
    assert_eq!(
        side.headers[0].digest,
        Some(key.digest("X-Echo", "tok-66-secret"))
    );
}

#[test]
fn two_kept_sends_under_one_key_find_a_changed_masked_cookie() {
    let key = fresh_key();
    let old = side_of_stored(&kept(cookie("sid=aaa"), Some(&key)), Some(&key));
    let new = side_of_stored(&kept(cookie("sid=bbb"), Some(&key)), Some(&key));
    let same = side_of_stored(&kept(cookie("sid=aaa"), Some(&key)), Some(&key));
    assert_eq!(old.headers[1].raw, None, "the kept side has no true value");
    assert!(cookie_changed(&old, &new));
    assert!(!cookie_changed(&old, &same));
}

#[test]
fn two_cookies_one_changed_is_a_change() {
    let key = fresh_key();
    let side = |v: &[&str]| side_of_stored(&kept(cookies(v), Some(&key)), Some(&key));
    let old = side(&["sid=aaa", "theme=dark"]);
    assert!(cookie_changed(&old, &side(&["sid=bbb", "theme=dark"])));
    assert!(!cookie_changed(&old, &side(&["sid=aaa", "theme=dark"])));
}

#[test]
fn a_live_send_and_a_kept_one_agree_on_the_digest() {
    let key = fresh_key();
    let old = side_of_stored(&kept(cookie("sid=aaa"), Some(&key)), Some(&key));
    let same = side_of_response(&cookie("sid=aaa"), &[], &[], None, Some(&key));
    let changed = side_of_response(&cookie("sid=bbb"), &[], &[], None, Some(&key));
    assert!(!cookie_changed(&old, &same));
    assert!(cookie_changed(&old, &changed));
}

#[test]
fn digests_under_another_key_are_ignored() {
    let (saved, now) = (fresh_key(), fresh_key());
    let old = side_of_stored(&kept(cookie("sid=aaa"), Some(&saved)), Some(&now));
    let new = side_of_stored(&kept(cookie("sid=bbb"), Some(&saved)), Some(&now));
    assert_eq!(old.headers[1].digest, None);
    assert!(
        !cookie_changed(&old, &new),
        "both sides fall back to equal masked text"
    );
}

#[test]
fn a_send_kept_without_a_key_compares_by_masked_text() {
    let key = fresh_key();
    let old = side_of_stored(&kept(cookie("sid=aaa"), None), Some(&key));
    let new = side_of_response(&cookie("sid=bbb"), &[], &[], None, Some(&key));
    assert_eq!(old.headers[1].digest, None);
    assert!(!cookie_changed(&old, &new));
}

#[test]
fn a_restored_live_side_uses_its_kept_digests() {
    let key = fresh_key();
    let old = side_of_stored(&kept(cookie("sid=aaa"), Some(&key)), Some(&key));
    let shown = kept(cookie("sid=bbb"), Some(&key));
    let response = shown.result.response.as_ref().unwrap();
    let digests = shown.header_digests.as_ref();
    let new = at_rest(side_of_response(
        response,
        &[],
        &shown.masked_headers,
        digests,
        Some(&key),
    ));
    assert_eq!(new.headers[1].raw, None);
    assert_eq!(
        new.headers[1].digest,
        Some(key.digest("set-cookie", "sid=bbb"))
    );
    assert!(cookie_changed(&old, &new));
}

#[test]
fn without_a_key_no_side_has_a_digest() {
    let key = fresh_key();
    let side = side_of_stored(&kept(cookie("sid=aaa"), Some(&key)), None);
    assert!(side.headers.iter().all(|h| h.digest.is_none()));
}

fn one(raw: Option<&str>, digest: Option<&str>) -> CompareSide {
    CompareSide {
        base: ResponseSnapshot::new(200),
        headers: vec![HeaderValue {
            name: "x-a".into(),
            shown: "[redacted]".into(),
            raw: raw.map(str::to_owned),
            digest: digest.map(str::to_owned),
        }],
    }
}

#[test]
fn at_rest_drops_a_digest_made_from_a_value_it_drops() {
    let side = at_rest(one(Some("true"), Some("d")));
    assert_eq!(
        (side.headers[0].raw.clone(), side.headers[0].digest.clone()),
        (None, None)
    );
}

#[test]
fn true_values_win_over_digests_and_digests_over_masked_text() {
    let changed = |o: &CompareSide, n: &CompareSide| {
        compare_sides(o, n).headers.iter().any(|h| h.name == "x-a")
    };
    assert!(!changed(
        &one(Some("v"), Some("d1")),
        &one(Some("v"), Some("d2"))
    ));
    assert!(changed(
        &one(Some("v1"), Some("d")),
        &one(Some("v2"), Some("d"))
    ));
    assert!(changed(&one(None, Some("d1")), &one(Some("v"), Some("d2"))));
    assert!(!changed(&one(None, Some("d1")), &one(Some("v"), None)));
}

// A send kept before masked places were recorded: the scrubbed header holds the placeholder.
fn kept_before_masked_places(value: &str) -> Stored {
    let response = Response::new(200).with_header("x-echo", value);
    Stored {
        result: RequestResult::from_parts(
            ip("a.yml"),
            "a",
            Outcome::Passed,
            Some(response),
            Vec::new(),
        ),
        saved_at: UNIX_EPOCH,
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    }
}

#[test]
fn an_old_kept_placeholder_matches_the_live_secret_it_hid() {
    let secrets = ["tok-704-secret".to_owned()];
    let echo_changed = |old: &CompareSide, new: &CompareSide| {
        (compare_sides(old, new).headers.iter()).any(|h| h.name.eq_ignore_ascii_case("x-echo"))
    };
    let stored = kept_before_masked_places("Bearer [redacted]");
    for key in [None, Some(fresh_key())] {
        let key = key.as_ref();
        let old = side_of_stored(&stored, key);
        let same = side_of_response(
            &Response::new(200).with_header("x-echo", "Bearer tok-704-secret"),
            &secrets,
            &[],
            None,
            key,
        );
        let other = side_of_response(
            &Response::new(200).with_header("x-echo", "Bearer other"),
            &secrets,
            &[],
            None,
            key,
        );
        assert!(!echo_changed(&old, &same), "with key {:?}", key.is_some());
        assert!(echo_changed(&old, &other), "with key {:?}", key.is_some());
    }
}

// A placeholder at a place the file did not name: its digest stays only when the file's
// digests were made under the current key.
#[test]
fn an_old_placeholder_keeps_a_digest_the_file_held_under_the_key() {
    let (key, other) = (fresh_key(), fresh_key());
    let held = |by: &DigestKey| {
        let mut stored = kept_before_masked_places("Bearer [redacted]");
        stored.header_digests = Some(KeptDigests {
            key: by.id().to_owned(),
            values: vec![(0, "held".to_owned())],
        });
        side_of_stored(&stored, Some(&key)).headers[0].clone()
    };
    let same = held(&key);
    assert_eq!((same.raw, same.digest.as_deref()), (None, Some("held")));
    let foreign = held(&other);
    assert_eq!((foreign.raw, foreign.digest), (None, None));
}

const ECHO_SECRET: &str = "tok-721-secret";

// The kept file of `store`'s newest send, rewritten as a send kept before masked places
// were recorded: no masked places and no digests.
fn age_newest_send(store: &ResponseStore) {
    let newest = (std::fs::read_dir(store.dir()).unwrap())
        .map(|f| f.unwrap().path())
        .filter(|p| p.file_name().is_some_and(|n| n != "index.json"))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .max_by_key(|p| {
            let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
            stem.rsplit('.').next().unwrap().parse::<u64>().unwrap()
        })
        .expect("a kept send");
    let mut entry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&newest).unwrap()).unwrap();
    let fields = entry.as_object_mut().unwrap();
    assert!(fields.remove("masked_headers").is_some());
    fields.remove("header_digests");
    std::fs::write(&newest, entry.to_string()).unwrap();
}

// Restores an old-format send of `newest` on screen, after a send of `previous` kept with its
// masked places and digests under one key, and reports the changed header names against Previous.
fn changed_against_previous(
    cx: &mut TestAppContext,
    previous: Response,
    newest: Response,
) -> Vec<String> {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
    ]);
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    store.set_digest_key(fresh_key());
    let secrets = [ECHO_SECRET.to_owned()];
    for (n, response) in [previous, newest].into_iter().enumerate() {
        let result = RequestResult::from_parts(
            ip("a.yml"),
            "a",
            Outcome::Passed,
            Some(response),
            Vec::new(),
        );
        let at = UNIX_EPOCH + Duration::from_secs(1_760_000_000 + n as u64);
        (store.save_redacted(
            &root,
            &ip("a.yml"),
            &result,
            &secrets,
            at,
            Limits::default(),
        ))
        .unwrap();
    }
    age_newest_send(&store);
    cx.update(|cx| install(store.clone(), cx));
    let (_win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.restored().is_some()));
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == 2));
    pane.update(cx, |p, cx| p.compare_with(CompareSource::Previous, cx));
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.comparison().is_some()));
    let comparison = pane.read_with(cx, |p, _| p.comparison()).unwrap();
    drop((col, state));
    (comparison.headers.iter())
        .map(|h| h.name.to_ascii_lowercase())
        .collect()
}

fn echo(value: &str, plain: &str) -> Response {
    Response::new(200)
        .with_header("x-echo", value)
        .with_header("x-plain", plain)
        .with_text_body("{}")
}

#[gpui_kit::test]
fn a_restored_old_placeholder_matches_the_previous_send_that_hid_it(cx: &mut TestAppContext) {
    let bearer = format!("Bearer {ECHO_SECRET}");
    let changed = changed_against_previous(cx, echo(&bearer, "a"), echo(&bearer, "b"));
    assert!(!changed.contains(&"x-echo".to_owned()), "{changed:?}");
    assert!(
        changed.contains(&"x-plain".to_owned()),
        "control: {changed:?}"
    );
}

const EXAMPLE_SECRET: &str = "tok-727-secret";

// A send whose x-echo header held the secret, kept scrubbed under `key` and read back.
fn kept_scrubbed(key: Option<&DigestKey>) -> Stored {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("responses"));
    if let Some(key) = key {
        store.set_digest_key(key.clone());
    }
    let response = Response::new(200).with_header("x-echo", format!("Bearer {EXAMPLE_SECRET}"));
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    (store.save_redacted(
        Path::new("/c"),
        &ip("a.yml"),
        &result,
        &[EXAMPLE_SECRET.to_owned()],
        UNIX_EPOCH,
        Limits::default(),
    ))
    .unwrap();
    store.load(Path::new("/c"), &ip("a.yml")).expect("kept")
}

fn example(headers: &[(&str, &str)]) -> ExampleResponse {
    let mut r = ExampleResponse::new(200);
    r.headers = (headers.iter())
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect();
    r
}

fn echo_changed(old: &CompareSide, new: &CompareSide) -> bool {
    (compare_sides(old, new).headers.iter()).any(|h| h.name.eq_ignore_ascii_case("x-echo"))
}

#[test]
fn a_masked_kept_value_matches_an_example_holding_its_true_value() {
    let key = fresh_key();
    let restored = side_of_stored(&kept_scrubbed(Some(&key)), Some(&key));
    assert_eq!(restored.headers[0].raw, None, "the kept side is masked");
    let same = side_of_example(&example(&[("x-echo", "Bearer tok-727-secret")]), Some(&key));
    let other = side_of_example(&example(&[("x-echo", "Bearer other")]), Some(&key));
    assert!(!echo_changed(&same, &restored));
    assert!(echo_changed(&other, &restored));
}

#[test]
fn without_a_key_an_example_compares_with_a_masked_value_by_shown_text() {
    let restored = side_of_stored(&kept_scrubbed(None), None);
    let same = side_of_example(&example(&[("x-echo", "Bearer tok-727-secret")]), None);
    assert!(same.headers.iter().all(|h| h.digest.is_none()));
    assert!(echo_changed(&same, &restored));
}

#[test]
fn an_example_value_that_cannot_be_true_has_no_digest() {
    let key = fresh_key();
    let side = side_of_example(
        &example(&[
            ("x-echo", "Bearer [redacted]"),
            ("location", "https://u:pw@example.com/"),
            ("x-plain", "a"),
        ]),
        Some(&key),
    );
    let digests: Vec<_> = side.headers.iter().map(|h| h.digest.clone()).collect();
    assert_eq!(
        digests,
        [
            None,
            Some(key.digest("location", "https://u:pw@example.com/")),
            Some(key.digest("x-plain", "a")),
        ]
    );
    assert!(side.headers.iter().all(|h| h.raw.is_some()), "raw stays");
}

// A "Keep values" example holds a set-cookie word for word, as the kept send's digest saw it.
#[test]
fn a_kept_masked_cookie_matches_an_example_holding_the_same_cookie() {
    let key = fresh_key();
    let restored = side_of_stored(&kept(cookie("sid=aaa"), Some(&key)), Some(&key));
    assert_eq!(restored.headers[1].raw, None, "the kept side is masked");
    let same = side_of_example(
        &example(&[
            ("content-type", "application/json"),
            ("set-cookie", "sid=aaa"),
        ]),
        Some(&key),
    );
    let other = side_of_example(
        &example(&[
            ("content-type", "application/json"),
            ("set-cookie", "sid=bbb"),
        ]),
        Some(&key),
    );
    assert!(!cookie_changed(&same, &restored));
    assert!(cookie_changed(&other, &restored));
}

// An example saved without kept values holds a location stripped of its userinfo; the true
// values differ, so it is listed as changed against a send whose location had userinfo.
#[test]
fn an_example_location_without_userinfo_differs_from_a_kept_one_with_it() {
    let key = fresh_key();
    let sent = Response::new(200).with_header("location", "https://u:pw@example.com/");
    let restored = side_of_stored(&kept(sent, Some(&key)), Some(&key));
    let stripped = side_of_example(
        &example(&[("location", "https://example.com/")]),
        Some(&key),
    );
    let changed = (compare_sides(&stripped, &restored).headers.iter())
        .any(|h| h.name.eq_ignore_ascii_case("location"));
    assert!(changed);
}
