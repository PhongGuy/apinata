//! Sends that failed on the network are kept in their own short list that History shows
//! beside the kept responses, with their reason, and its search matches them too.

use crate::support;

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::app::AppState;
use apinata_app::history_palette::{HistoryEntry, filter_history, history_rows};
use apinata_app::last_response::{Limits, ResponseStore};
use apinata_core::{ExecError, Outcome, RequestResult, Response, RunError};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn at(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn failed(item: &str, error: ExecError) -> RequestResult {
    let mut r = RequestResult::from_parts(ip(item), "x", Outcome::Errored, None, Vec::new());
    r.error = Some(RunError::Exec(error));
    r
}

fn refused(item: &str, url: &str) -> RequestResult {
    failed(
        item,
        ExecError::connect(url, "detail", apinata_core::ConnectCause::Refused),
    )
}

fn limits(per_request: usize) -> Limits {
    Limits {
        per_request,
        ..Limits::default()
    }
}

fn store() -> (tempfile::TempDir, ResponseStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(dir.path().join("responses"));
    (dir, store)
}

#[test]
fn failures_list_newest_first_with_method_url_and_reason() {
    let (_dir, store) = store();
    let root = PathBuf::from("/c");
    let other = PathBuf::from("/elsewhere");
    let timeout = failed("b.yml", ExecError::timeout("http://h/b"));
    let save = |root: &PathBuf, item: &str, r: &RequestResult, ms| {
        store
            .save_failure(root, &ip(item), r, "POST", &[], at(ms), limits(20), None)
            .unwrap()
    };
    assert!(save(&root, "a.yml", &refused("a.yml", "http://h/a"), 1_000));
    assert!(save(&root, "b.yml", &timeout, 2_000));
    assert!(save(&other, "z.yml", &timeout, 3_000));
    let got: Vec<_> = store
        .failures(std::slice::from_ref(&root))
        .into_iter()
        .map(|f| (f.item.as_str().to_owned(), f.at, f.method, f.url, f.reason))
        .collect();
    let row = |item: &str, ms, url: &str, reason: &str| {
        let s = |t: &str| t.to_owned();
        (s(item), at(ms), s("POST"), s(url), s(reason))
    };
    assert_eq!(
        got,
        [
            row("b.yml", 2_000, "http://h/b", "Timed out"),
            row("a.yml", 1_000, "http://h/a", "Connection refused"),
        ]
    );
}

#[test]
fn a_cancelled_send_or_one_with_a_response_is_not_a_failure() {
    let (_dir, store) = store();
    let root = PathBuf::from("/c");
    let cancelled = failed("a.yml", ExecError::Cancelled);
    let mut answered = refused("a.yml", "http://h/a");
    answered.response = Some(Response::new(500));
    for r in [&cancelled, &answered] {
        let kept = store
            .save_failure(&root, &ip("a.yml"), r, "GET", &[], at(1), limits(20), None)
            .unwrap();
        assert!(!kept);
    }
    assert!(store.failures(&[root]).is_empty());
}

#[test]
fn a_secret_never_reaches_the_kept_url() {
    let (dir, store) = store();
    let root = PathBuf::from("/c");
    let r = refused(
        "a.yml",
        "http://user:pw-77@h/a?token=SeKrEt-41&api_key=k-9&q=1",
    );
    let secrets = ["SeKrEt-41".to_owned()];
    store
        .save_failure(
            &root,
            &ip("a.yml"),
            &r,
            "GET",
            &secrets,
            at(1),
            limits(20),
            None,
        )
        .unwrap();
    let url = store.failures(&[root]).remove(0).url;
    for secret in ["SeKrEt-41", "pw-77", "k-9"] {
        assert!(!url.contains(secret), "{url}");
    }
    assert!(url.contains("q=1"), "{url}");
    let on_disk = std::fs::read_dir(dir.path().join("responses")).unwrap();
    for file in on_disk {
        let text = std::fs::read_to_string(file.unwrap().path()).unwrap();
        assert!(!text.contains("SeKrEt-41") && !text.contains("pw-77"));
    }
}

#[test]
fn each_request_keeps_its_newest_failures_up_to_the_history_limit() {
    let (_dir, store) = store();
    let root = PathBuf::from("/c");
    for ms in 1..=4 {
        let r = refused("a.yml", &format!("http://h/{ms}"));
        store
            .save_failure(&root, &ip("a.yml"), &r, "GET", &[], at(ms), limits(2), None)
            .unwrap();
    }
    let b = refused("b.yml", "http://h/b");
    store
        .save_failure(&root, &ip("b.yml"), &b, "GET", &[], at(5), limits(2), None)
        .unwrap();
    let urls: Vec<String> = store.failures(&[root]).into_iter().map(|f| f.url).collect();
    assert_eq!(urls, ["http://h/b", "http://h/4", "http://h/3"]);
}

#[test]
fn renaming_moves_failures_and_removing_or_clearing_drops_them() {
    let (_dir, store) = store();
    let root = PathBuf::from("/c");
    let r = refused("f/a.yml", "http://h/a");
    store
        .save_failure(
            &root,
            &ip("f/a.yml"),
            &r,
            "GET",
            &[],
            at(1),
            limits(20),
            None,
        )
        .unwrap();
    store.rename(&root, &ip("f"), &ip("g")).unwrap();
    let items: Vec<String> = (store.failures(std::slice::from_ref(&root)).into_iter())
        .map(|f| f.item.as_str().to_owned())
        .collect();
    assert_eq!(items, ["g/a.yml"]);
    store.remove(&root, &ip("g")).unwrap();
    assert!(store.failures(std::slice::from_ref(&root)).is_empty());
    store
        .save_failure(&root, &ip("a.yml"), &r, "GET", &[], at(2), limits(20), None)
        .unwrap();
    store.clear().unwrap();
    assert!(store.failures(&[root]).is_empty());
}

#[gpui_kit::test]
fn a_refused_send_is_kept_as_a_failure(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection_for(&format!("http://127.0.0.1:{}", dead_port()), "");
    let s = started(cx, &col);
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let model = sender(cx, &s.ws);
    let r = s.root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.response.is_none());
    let roots = [s.root.clone()];
    wait_real(cx, |_| !store.failures(&roots).is_empty());
    let got = store.failures(&roots).remove(0);
    assert_eq!(got.item, ip("a.yml"));
    assert_eq!(got.method, "GET");
    assert_eq!(got.reason, "Connection refused");
    assert!(got.url.ends_with("/ok"), "{}", got.url);
    assert!(
        store.load(&s.root, &ip("a.yml")).is_none(),
        "no response kept"
    );
}

#[gpui_kit::test]
fn with_remembering_off_a_failure_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection_for(&format!("http://127.0.0.1:{}", dead_port()), "");
    let s = started(cx, &col);
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |v| v.remember_responses = false));
    });
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let model = sender(cx, &s.ws);
    let r = s.root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    finished(cx, &model);
    cx.run_until_parked();
    assert!(store.failures(std::slice::from_ref(&s.root)).is_empty());
    assert!(!store.dir().exists());
}

fn kept(item: &str, status: u16, url: &str) -> RequestResult {
    let response = Response::new(status).with_text_body("k").with_url(url);
    RequestResult::from_parts(ip(item), "x", Outcome::Passed, Some(response), Vec::new())
}

// Rows, newest first: b refused at 3s, a 500 at 2s, a timed out at 1s.
fn seeded(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let root = col.path().to_path_buf();
    started_with(cx, col, move |paths| {
        let store = ResponseStore::new(paths.responses.clone());
        let timeout = failed("a.yml", ExecError::timeout("https://api.example.com/slow"));
        let refused = refused("b.yml", "https://api.example.com/down");
        store
            .save_failure(
                &root,
                &ip("a.yml"),
                &timeout,
                "GET",
                &[],
                at(1_000),
                limits(20),
                None,
            )
            .unwrap();
        store
            .save(
                &root,
                &ip("a.yml"),
                &kept("a.yml", 500, "https://api.example.com/err"),
                at(2_000),
            )
            .unwrap();
        store
            .save_failure(
                &root,
                &ip("b.yml"),
                &refused,
                "POST",
                &[],
                at(3_000),
                limits(20),
                None,
            )
            .unwrap();
    })
}

#[gpui_kit::test]
fn history_rows_put_failures_among_the_sends_by_time(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let roots = [s.root.clone()];
    let recent = store.recent(&roots, 100);
    let failures = store.failures(&roots);
    let rows: Vec<HistoryEntry> =
        s.ws.read_with(cx, |w, cx| history_rows(&recent, &failures, w, cx));
    let got: Vec<(&str, Option<&str>, &str)> = rows
        .iter()
        .map(|e| (e.label.as_str(), e.reason.as_deref(), e.target.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (
                "b",
                Some("Connection refused"),
                "POST https://api.example.com/down"
            ),
            ("a", None, "https://api.example.com/err"),
            ("a", Some("Timed out"), "GET https://api.example.com/slow"),
        ]
    );
    assert!(rows[0].detail.starts_with("Connection refused · "));
}

#[gpui_kit::test]
fn the_history_search_matches_failures_by_reason_and_url(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let roots = [s.root.clone()];
    let (recent, failures) = (store.recent(&roots, 100), store.failures(&roots));
    let rows =
        s.ws.read_with(cx, |w, cx| history_rows(&recent, &failures, w, cx));
    let hits = |q: &str| -> Vec<usize> {
        filter_history(&rows, q)
            .into_iter()
            .map(|m| m.index)
            .collect()
    };
    assert_eq!(hits("refused"), [0]);
    assert_eq!(hits("TIMED"), [2]);
    assert_eq!(hits("/slow"), [2]);
    assert_eq!(hits("5xx"), [1], "a failure has no status class");
    assert_eq!(hits("0"), [1], "a failure has no status digits");
    let refused = &filter_history(&rows, "refused")[0];
    let reason = 11.."Connection refused".len();
    assert_eq!(refused.detail, std::slice::from_ref(&reason));
}

#[gpui_kit::test]
fn the_history_palette_shows_a_failure_with_its_reason(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("History", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        present(cx, s.win, "history-palette")
    });
    let detail = |cx: &mut TestAppContext, row: usize| {
        cx.update_window(s.win, |_, w, _| {
            w.within("history-palette")
                .find(("history-detail", row))
                .label()
                .map(str::to_owned)
        })
        .unwrap()
        .unwrap()
    };
    assert!(detail(cx, 0).starts_with("Connection refused"));
    assert!(detail(cx, 1).starts_with("500"));
    assert!(detail(cx, 2).starts_with("Timed out"));
    cx.update_window(s.win, |_, w, cx| w.input("refused", cx))
        .unwrap();
    frame(cx, s.win);
    let rows = cx
        .update_window(s.win, |_, w, _| {
            let palette = w.within("history-palette");
            (0usize..)
                .take_while(|r| palette.try_find(("history-method", *r)).is_some())
                .count()
        })
        .unwrap();
    assert_eq!(rows, 1);
    // Choosing the failure opens its request.
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws), ["b.yml"]);
}

// `api/down` refuses (nothing listens on its port) and `api/up` answers 200.
fn refusing_suite(up: &str) -> tempfile::TempDir {
    let req = |name: &str, seq: u32, url: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n"
        )
    };
    let down = format!("http://127.0.0.1:{}/down", dead_port());
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api")),
        ("api/down.yml", &req("down", 1, &down)),
        ("api/up.yml", &req("up", 2, &format!("{up}/up"))),
    ])
}

fn run_api(
    cx: &mut TestAppContext,
    s: &Started,
) -> gpui_kit::Entity<apinata_app::runner_model::RunnerModel> {
    use apinata_app::runner_model::RunnerModel;
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let m = cx.new(|_| RunnerModel::new(ws, root, ip("api")));
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    m
}

// The History rows of `s`'s collection, sends and failures merged.
fn rows_of(cx: &mut TestAppContext, s: &Started) -> Vec<HistoryEntry> {
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let roots = [s.root.clone()];
    let (recent, failures) = (store.recent(&roots, 100), store.failures(&roots));
    s.ws.read_with(cx, |w, cx| history_rows(&recent, &failures, w, cx))
}

fn runner_failures(rows: &[HistoryEntry]) -> Vec<(String, Option<&'static str>, String)> {
    rows.iter()
        .filter_map(|e| Some((e.label.clone(), e.tag, e.reason.clone()?)))
        .collect()
}

#[gpui_kit::test]
fn a_run_with_a_refused_request_leaves_one_runner_failure(cx: &mut TestAppContext) {
    use apinata_app::history_palette::RUN_TAG;
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = refusing_suite(&server.uri());
    let s = started(cx, &col);
    run_api(cx, &s);
    wait_real(cx, |cx| rows_of(cx, &s).len() == 2);
    let rows = rows_of(cx, &s);
    let refused = (
        "down".to_owned(),
        Some(RUN_TAG),
        "Connection refused".to_owned(),
    );
    assert_eq!(runner_failures(&rows), [refused]);
    let up = rows
        .iter()
        .find(|e| e.label == "up")
        .expect("the answered send");
    assert_eq!(
        (up.status, up.tag, up.reason.as_deref()),
        (200, Some(RUN_TAG), None)
    );
}

// Each send is its own History row: a re-run that fails again adds a second, later failure
// rather than replacing the first, as a re-run's answered send adds a kept send.
#[gpui_kit::test]
fn a_rerun_that_fails_again_adds_its_own_failure(cx: &mut TestAppContext) {
    use apinata_app::history_palette::RUN_TAG;
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = refusing_suite(&server.uri());
    let s = started(cx, &col);
    let m = run_api(cx, &s);
    wait_real(cx, |cx| rows_of(cx, &s).len() == 2);
    assert!(m.update(cx, |m, cx| m.rerun_failed(cx)));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    wait_real(cx, |cx| rows_of(cx, &s).len() == 3);
    let rows = rows_of(cx, &s);
    let refused = (
        "down".to_owned(),
        Some(RUN_TAG),
        "Connection refused".to_owned(),
    );
    assert_eq!(runner_failures(&rows), [refused.clone(), refused]);
    assert!(
        rows[0].reason.is_some(),
        "the re-run's failure is the newest row"
    );
}
