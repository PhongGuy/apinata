//! What a run can isolate: an executor on its own cookie jar, and a context whose runtime
//! variables and script edits are its own.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, CookieInfo, CookieJar, ExecOptions, Executor, OverlayLayer,
    VarContext,
};
use common::{resolved_one, write_tree};
use std::sync::Arc;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn cookie_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", "run=1; Path=/"))
        .mount(&server)
        .await;
    server
}

async fn cookie_headers(server: &MockServer) -> Vec<Option<String>> {
    let got = server.received_requests().await.unwrap_or_default();
    got.iter()
        .map(|r| {
            r.headers
                .get("cookie")
                .map(|v| v.to_str().unwrap().to_owned())
        })
        .collect()
}

#[tokio::test]
async fn an_executor_on_its_own_jar_neither_sends_nor_fills_the_shared_one() {
    let server = cookie_server().await;
    let shared = Arc::new(CookieJar::default());
    shared
        .insert(CookieInfo::new("127.0.0.1", "sid", "shared"))
        .unwrap();
    let exec = Executor::new(ExecOptions::new().cookie_jar(Arc::clone(&shared))).expect("client");
    let own = Arc::new(CookieJar::default());
    let run = exec.with_cookie_jar(Arc::clone(&own));
    let req = resolved_one("GET", &format!("{}/login", server.uri()), "", "");
    run.send(&req).await.expect("sent");
    assert_eq!(cookie_headers(&server).await, [None]);
    let names =
        |jar: &CookieJar| -> Vec<String> { jar.list().into_iter().map(|c| c.name).collect() };
    assert_eq!(names(&shared), ["sid"], "the shared jar is unchanged");
    assert_eq!(names(&own), ["run"]);
    assert_eq!(names(run.cookies()), ["run"]);
}

#[tokio::test]
async fn an_executor_on_its_own_jar_keeps_the_cookie_switches() {
    let exec =
        Executor::new(ExecOptions::new().store_cookies(false).send_cookies(false)).expect("client");
    let run = exec.with_cookie_jar(Arc::new(CookieJar::default()));
    assert!(!run.store_cookies_enabled() && !run.send_cookies_enabled());
}

fn context() -> (tempfile::TempDir, VarContext) {
    let dir = write_tree(&[(
        "opencollection.yml",
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("builds");
    (dir, ctx)
}

fn runtime(ctx: &VarContext) -> Vec<(String, Option<String>)> {
    (ctx.runtime_vars().entries().iter())
        .map(|v| (v.name().to_owned(), v.value().map(str::to_owned)))
        .collect()
}

fn overlay(ctx: &VarContext) -> Vec<(String, Option<String>)> {
    (ctx.env_overlay().entries().iter())
        .map(|v| (v.name().to_owned(), v.value().map(str::to_owned)))
        .collect()
}

#[test]
fn a_value_set_in_a_fork_is_invisible_in_the_original() {
    let (_dir, ctx) = context();
    ctx.runtime_vars().set_text("before", "1");
    ctx.env_overlay()
        .set_text(OverlayLayer::Collection, "c", "old");
    let fork = ctx.fork_variables();
    assert_eq!(runtime(&fork), runtime(&ctx), "the fork starts as a copy");
    assert_eq!(overlay(&fork), overlay(&ctx));
    fork.runtime_vars().set_text("during", "2");
    fork.env_overlay()
        .set_text(OverlayLayer::Collection, "c", "new");
    assert_eq!(runtime(&ctx), [("before".to_owned(), Some("1".to_owned()))]);
    assert_eq!(overlay(&ctx), [("c".to_owned(), Some("old".to_owned()))]);
    ctx.runtime_vars().set_text("after", "3");
    assert!(!runtime(&fork).iter().any(|(n, _)| n == "after"));
}

#[test]
fn a_sensitive_mark_survives_the_fork() {
    let (_dir, ctx) = context();
    ctx.runtime_vars().set_text("token", "hidden-value");
    ctx.runtime_vars().mark_sensitive("token");
    let fork = ctx.fork_variables();
    assert!(fork.runtime_vars().is_sensitive("token"));
    fork.runtime_vars().mark_sensitive("other");
    assert!(!ctx.runtime_vars().is_sensitive("other"));
}
