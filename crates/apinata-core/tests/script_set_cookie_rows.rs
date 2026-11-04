//! A cookie a script stores is recorded on the response beside the hops' Set-Cookie rows: the
//! pre-request phase's before the hops, the later phases' after them, each with the jar's verdict.

use crate::script_support;
use crate::session_log_support as logs;

use apinata_core::{
    CancelHandle, Collection, ContextOptions, CookieOrigin, CookieOutcome, CookieRejection,
    ExecOptions, Executor, ItemPath, RequestResult, ScriptPhase, SessionLog, VarContext, send_one,
};
use script_support::*;
use std::sync::Arc;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Sends a GET to `/r`, which sets `h=1`, running `scripts` through a fresh in-memory executor.
async fn send_scripts(scripts: &str) -> RequestResult {
    send_beside(scripts, None).await.0
}

/// Sends a GET to a closed 127.0.0.1 port running `scripts`, so no response arrives.
async fn send_unanswered(scripts: &str) -> RequestResult {
    let text = scripted("R", "http://127.0.0.1:1/r", scripts, "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let request = apinata_core::RequestFile::load(dir.path().join("r.yml")).expect("loads");
    let item = ItemPath::new("r.yml").expect("item path");
    send_one(item, &request, &ctx, &exec, &on(), &CancelHandle::new()).await
}

/// [`send_scripts`], with `c.yml` (a GET to `/c` running `child`) beside `r.yml` when given;
/// the executor is returned so a test can read its jar.
async fn send_beside(scripts: &str, child: Option<&str>) -> (RequestResult, Arc<Executor>) {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", "h=1; Path=/"))
        .mount(&s)
        .await;
    Mock::given(path("/c"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let text = scripted("R", &format!("{}/r", s.uri()), scripts, "");
    let mut files = vec![("opencollection.yml", ROOT.to_owned()), ("r.yml", text)];
    if let Some(child) = child {
        files.push(("c.yml", scripted("C", &format!("{}/c", s.uri()), child, "")));
    }
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let dir = write_tree(&files);
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let request = apinata_core::RequestFile::load(dir.path().join("r.yml")).expect("loads");
    let item = ItemPath::new("r.yml").expect("item path");
    let result = send_one(item, &request, &ctx, &exec, &on(), &CancelHandle::new()).await;
    (result, exec)
}

/// (origin, name, outcome) of each Set-Cookie row: the script phase, or `hop`.
fn rows(result: &RequestResult) -> Vec<(&'static str, String, CookieOutcome)> {
    result
        .set_cookies()
        .map(|(origin, c)| {
            let from = match origin {
                CookieOrigin::Script(phase) => {
                    assert_eq!(origin.phase(), Some(phase));
                    phase.as_str()
                }
                _ => {
                    assert!(origin.hop().is_some());
                    assert_eq!(origin.phase(), None);
                    "hop"
                }
            };
            (from, c.name.clone(), c.outcome)
        })
        .collect()
}

fn row(
    from: &'static str,
    name: &str,
    outcome: CookieOutcome,
) -> (&'static str, String, CookieOutcome) {
    (from, name.to_owned(), outcome)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_post_response_cookie_follows_the_hop_rows() {
    let after = script(
        "after-response",
        r#"await bru.cookies.add({ key: "s", value: "1" });"#,
    );
    let result = send_scripts(&after).await;
    assert_eq!(
        rows(&result),
        [
            row("hop", "h", CookieOutcome::Stored),
            row("post-response", "s", CookieOutcome::Stored),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pre_request_cookie_comes_before_the_hop_rows_and_tests_after() {
    let before = script(
        "before-request",
        r#"await bru.cookies.add({ key: "p", value: "1" });"#,
    );
    let tests = script(
        "tests",
        r#"await bru.cookies.jar().setCookies(req.getUrl(), [{ key: "t1", value: "1" }, { key: "t2", value: "2" }]);"#,
    );
    let result = send_scripts(&format!("{before}{tests}")).await;
    assert_eq!(
        rows(&result),
        [
            row("pre-request", "p", CookieOutcome::Stored),
            row("hop", "h", CookieOutcome::Stored),
            row("tests", "t1", CookieOutcome::Stored),
            row("tests", "t2", CookieOutcome::Stored),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cookie_for_another_domain_is_rejected_as_a_domain_mismatch() {
    let before = script(
        "before-request",
        r#"await bru.cookies.add({ key: "e", value: "1", domain: "evil.example" });"#,
    );
    let result = send_scripts(&before).await;
    assert_eq!(
        rows(&result),
        [
            row(
                "pre-request",
                "e",
                CookieOutcome::Rejected(CookieRejection::DomainMismatch)
            ),
            row("hop", "h", CookieOutcome::Stored),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_cookie_row_never_shows_its_value_in_debug() {
    let after = script(
        "after-response",
        r#"await bru.cookies.add({ key: "s", value: "row-secret-77" });"#,
    );
    let result = send_scripts(&after).await;
    let res = result.response.as_ref().expect("a response");
    let (_, c) = res.set_cookies().last().expect("a script row");
    assert_eq!(c.value, "row-secret-77");
    assert!(!format!("{c:?}").contains("row-secret-77"), "{c:?}");
    assert!(!format!("{res:?}").contains("row-secret-77"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_nested_requests_script_cookie_is_not_the_callers_row() {
    let caller = script(
        "after-response",
        r#"const r = await bru.runRequest("c");
console.log(r.status);"#,
    );
    let child = script(
        "after-response",
        r#"await bru.cookies.add({ key: "child", value: "1" });"#,
    );
    let (result, exec) = send_beside(&caller, Some(&child)).await;
    assert_eq!(console(&result), ["log:200"], "the child ran");
    let names: Vec<String> = exec.cookies().list().into_iter().map(|c| c.name).collect();
    assert!(names.contains(&"child".to_owned()), "{names:?}");
    assert_eq!(rows(&result), [row("hop", "h", CookieOutcome::Stored)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_delete_records_a_removed_row_without_a_value() {
    let after = script(
        "after-response",
        r#"await bru.cookies.add({ key: "s", value: "gone-value-31" });
await bru.cookies.remove("s");
await bru.cookies.remove("never-set");"#,
    );
    let result = send_scripts(&after).await;
    assert_eq!(
        rows(&result),
        [
            row("hop", "h", CookieOutcome::Stored),
            row("post-response", "s", CookieOutcome::Stored),
            row("post-response", "s", CookieOutcome::Removed),
        ]
    );
    let (_, removed) = result.set_cookies().last().expect("the removed row");
    assert_eq!(removed.value, "");
    assert_eq!(removed.domain.as_deref(), Some("127.0.0.1"));
    assert_eq!(removed.path.as_deref(), Some("/"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clearing_records_one_removed_row_per_cookie_the_jar_dropped() {
    let after = script(
        "after-response",
        r#"await bru.cookies.add({ key: "s", value: "1" });
await bru.cookies.clear();
await bru.cookies.add({ key: "t", value: "2" });
await bru.cookies.jar().clear();
await bru.cookies.jar().clear();"#,
    );
    let result = send_scripts(&after).await;
    assert_eq!(
        rows(&result),
        [
            row("hop", "h", CookieOutcome::Stored),
            row("post-response", "s", CookieOutcome::Stored),
            row("post-response", "h", CookieOutcome::Removed),
            row("post-response", "s", CookieOutcome::Removed),
            row("post-response", "t", CookieOutcome::Stored),
            row("post-response", "t", CookieOutcome::Removed),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_send_keeps_its_pre_request_cookie_rows() {
    let before = script(
        "before-request",
        r#"await bru.cookies.add({ key: "p", value: "1" });"#,
    );
    let result = send_unanswered(&before).await;
    assert!(result.response.is_none(), "{result:?}");
    assert!(result.error.is_some(), "{result:?}");
    assert_eq!(
        rows(&result),
        [row("pre-request", "p", CookieOutcome::Stored)]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_post_response_cookie_is_a_post_response_row() {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let after = script(
        "after-response",
        r#"await bru.cookies.add({ key: "f", value: "1" });"#,
    );
    let own = script("before-request", "const unused = 1;");
    let text = scripted("R", &format!("{}/r", s.uri()), &own, "");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder(&after)),
        ("f/r.yml", &text),
    ]);
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let request = apinata_core::RequestFile::load(dir.path().join("f/r.yml")).expect("loads");
    let item = ItemPath::new("f/r.yml").expect("item path");
    let result = send_one(item, &request, &ctx, &exec, &on(), &CancelHandle::new()).await;
    let origins: Vec<_> = result
        .set_cookies()
        .map(|(o, c)| (o, c.name.clone()))
        .collect();
    assert_eq!(
        origins,
        [(
            CookieOrigin::Script(ScriptPhase::PostResponse),
            "f".to_owned()
        )]
    );
}

/// Sends `r.yml`, a GET of `url` running `scripts`, through an executor recording into a fresh
/// session log; the result and the log.
async fn send_logged(url: &str, scripts: &str) -> (RequestResult, SessionLog) {
    let text = scripted("R", url, scripts, "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let (exec, log) = logs::logged(ExecOptions::new());
    let exec = Arc::new(exec);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let request = apinata_core::RequestFile::load(dir.path().join("r.yml")).expect("loads");
    let item = ItemPath::new("r.yml").expect("item path");
    let result = send_one(item, &request, &ctx, &exec, &on(), &CancelHandle::new()).await;
    (result, log)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_cookie_leaves_no_trace_in_the_session_log() {
    // Stored before the send, the cookie rides in the request's Cookie header, which the log
    // records; only a match on that header answers 200.
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .and(header("cookie", "s=LogSecret-668a"))
        .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", "h=1; Path=/"))
        .mount(&s)
        .await;
    let before = script(
        "before-request",
        r#"await bru.cookies.add({ key: "s", value: "LogSecret-668a" });"#,
    );
    let (result, log) = send_logged(&format!("{}/r", s.uri()), &before).await;
    assert_eq!(
        rows(&result),
        [
            row("pre-request", "s", CookieOutcome::Stored),
            row("hop", "h", CookieOutcome::Stored),
        ]
    );
    let entry = logs::only(&log);
    assert_eq!(entry.status, Some(200), "{entry:?}");
    assert!(logs::lines(&log).is_empty(), "{:?}", logs::lines(&log));
    logs::assert_absent(&log, "LogSecret-668a");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_sends_script_cookie_leaves_no_trace_in_the_session_log() {
    let before = script(
        "before-request",
        r#"await bru.cookies.add({ key: "p", value: "LogSecret-668b" });"#,
    );
    let (result, log) = send_logged("http://127.0.0.1:1/r", &before).await;
    assert!(result.response.is_none(), "{result:?}");
    assert_eq!(
        rows(&result),
        [row("pre-request", "p", CookieOutcome::Stored)]
    );
    let entry = logs::only(&log);
    assert!(entry.error.is_some(), "{entry:?}");
    assert!(logs::lines(&log).is_empty(), "{:?}", logs::lines(&log));
    logs::assert_absent(&log, "LogSecret-668b");
}

/// A child `c.yml` that stores `cp` before its send and `cr` after it.
fn cookie_child() -> String {
    let before = script(
        "before-request",
        r#"await bru.cookies.add({ key: "cp", value: "1" });"#,
    );
    let after = script(
        "after-response",
        r#"await bru.cookies.add({ key: "cr", value: "2" });"#,
    );
    format!("{before}{after}")
}

/// A `kind` script that runs `c` and then stores `after` itself.
fn calling(kind: &str) -> String {
    script(
        kind,
        r#"const r = await bru.runRequest("c");
console.log(r.status);
await bru.cookies.add({ key: "after", value: "3" });"#,
    )
}

/// The names the executor's jar holds, sorted.
fn jar_names(exec: &Executor) -> Vec<String> {
    let mut names: Vec<String> = exec.cookies().list().into_iter().map(|c| c.name).collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pre_request_nested_call_keeps_the_childs_rows_off_the_caller() {
    let (result, exec) = send_beside(&calling("before-request"), Some(&cookie_child())).await;
    assert_eq!(console(&result), ["log:200"], "the child ran");
    assert_eq!(jar_names(&exec), ["after", "cp", "cr", "h"]);
    assert_eq!(
        rows(&result),
        [
            row("pre-request", "after", CookieOutcome::Stored),
            row("hop", "h", CookieOutcome::Stored),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tests_nested_call_keeps_the_childs_rows_off_the_caller() {
    let (result, exec) = send_beside(&calling("tests"), Some(&cookie_child())).await;
    assert_eq!(console(&result), ["log:200"], "the child ran");
    assert_eq!(jar_names(&exec), ["after", "cp", "cr", "h"]);
    assert_eq!(
        rows(&result),
        [
            row("hop", "h", CookieOutcome::Stored),
            row("tests", "after", CookieOutcome::Stored),
        ]
    );
}
