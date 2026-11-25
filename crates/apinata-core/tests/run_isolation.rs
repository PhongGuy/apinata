//! What a run can isolate: an executor on its own cookie jar, and a context whose runtime
//! variables and script edits are its own.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, CookieInfo, CookieJar, EnvOverlay, ExecError, ExecOptions,
    Executor, OverlayLayer, VarContext,
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

#[tokio::test]
async fn a_host_policy_still_blocks_through_an_executor_on_its_own_jar() {
    let exec =
        Executor::new(ExecOptions::new().host_policy(Arc::new(|_: &str| false))).expect("client");
    let run = exec.with_cookie_jar(Arc::new(CookieJar::default()));
    // A documentation address the policy refuses before any connection is tried.
    let req = resolved_one("GET", "http://192.0.2.1/", "", "");
    let err = run.send(&req).await.expect_err("refused");
    assert!(matches!(err, ExecError::HostNotAllowed { .. }), "{err:?}");
}

fn context_with_secret(value: &str) -> (tempfile::TempDir, VarContext) {
    let dir = write_tree(&[(
        "opencollection.yml",
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )]);
    let c = Collection::open(dir.path()).expect("opens");
    let options = ContextOptions::new().sensitive_env_var("key", value);
    let ctx = VarContext::new(&c, options).expect("builds");
    (dir, ctx)
}

#[test]
fn a_grafted_context_takes_the_forks_variables_over_its_own_fresh_layers() {
    let (_old_dir, old) = context_with_secret("old-secret-value");
    let fork = old.fork_variables();
    fork.runtime_vars().set_text("id", "captured");
    fork.env_overlay()
        .set_text(OverlayLayer::Collection, "c", "edited");
    let (_dir, fresh) = context_with_secret("fresh-secret-value");
    let grafted = fresh.with_variables_of(&fork);
    assert_eq!(
        runtime(&grafted),
        [("id".to_owned(), Some("captured".to_owned()))]
    );
    assert_eq!(
        overlay(&grafted),
        [("c".to_owned(), Some("edited".to_owned()))]
    );
    let secrets = grafted.secret_values();
    assert!(
        secrets.contains(&"fresh-secret-value".to_owned()),
        "{secrets:?}"
    );
    assert!(!secrets.contains(&"old-secret-value".to_owned()));
    // Writes through the graft land in the fork's own stores, never in `fresh`.
    grafted.runtime_vars().set_text("later", "1");
    assert!(runtime(&fork).iter().any(|(n, _)| n == "later"));
    assert!(fresh.runtime_vars().is_empty());
}

const ENV_A: &str = "name: a\nvariables:\n  - name: host\n    value: a.test\n";
const ENV_B: &str = "name: b\nvariables:\n  - name: host\n    value: b.test\n";
const ENV_C: &str = "name: c\nvariables:\n  - name: host\n    value: c.test\n";

/// A collection with environments `a`, `b` and `c`, a live overlay shared like the app's, and
/// a forked first run in `a` whose scripts set `token` there and a sensitive `session`.
fn forked_run_in_a() -> (tempfile::TempDir, Collection, EnvOverlay, VarContext) {
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        ),
        ("environments/a.yml", ENV_A),
        ("environments/b.yml", ENV_B),
        ("environments/c.yml", ENV_C),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let live = EnvOverlay::new();
    let first = VarContext::new(&c, ContextOptions::new().env("a").env_overlay(live.clone()))
        .expect("builds");
    let fork = first.fork_variables();
    fork.env_overlay()
        .set_text(OverlayLayer::Environment, "token", "run-set-token");
    fork.runtime_vars().set_text("session", "run-set-session");
    fork.runtime_vars().mark_sensitive("session");
    (dir, c, live, fork)
}

/// The re-run's context: planned afresh in `env` on the live overlay, then grafted onto `fork`.
fn rerun_in(c: &Collection, live: &EnvOverlay, env: &str, fork: &VarContext) -> VarContext {
    let fresh = VarContext::new(c, ContextOptions::new().env(env).env_overlay(live.clone()))
        .expect("builds");
    fresh.with_variables_of(fork)
}

fn env_values(ctx: &VarContext) -> Vec<(String, Option<String>)> {
    (ctx.env_overlay().entries().iter())
        .filter(|v| v.layer() == OverlayLayer::Environment)
        .map(|v| (v.name().to_owned(), v.value().map(str::to_owned)))
        .collect()
}

fn token() -> Vec<(String, Option<String>)> {
    vec![("token".to_owned(), Some("run-set-token".to_owned()))]
}

#[test]
fn a_rerun_in_the_same_environment_keeps_the_forks_environment_values() {
    let (_dir, c, live, fork) = forked_run_in_a();
    let grafted = rerun_in(&c, &live, "a", &fork);
    assert_eq!(env_values(&grafted), token());
}

// A rename carries script values to the new name, for the fork as for the shared store.
#[test]
fn a_rerun_after_a_rename_keeps_the_forks_values_under_the_new_name() {
    let (_dir, c, live, fork) = forked_run_in_a();
    live.rename_environment("a", "c");
    let grafted = rerun_in(&c, &live, "c", &fork);
    assert_eq!(env_values(&grafted), token());
    assert!(
        grafted.runtime_vars().is_sensitive("session"),
        "the sensitive mark stays"
    );
    let shared = VarContext::new(&c, ContextOptions::new().env("c").env_overlay(live.clone()))
        .expect("builds");
    assert!(
        env_values(&shared).is_empty(),
        "the fork's values stay in the fork"
    );
}

// A deleted environment's script values are gone; one recreated under that name starts
// empty, and the re-run's scripts can still set values in it.
#[test]
fn a_rerun_after_a_delete_starts_the_environment_empty_and_can_write_it() {
    let (_dir, c, live, fork) = forked_run_in_a();
    live.clear_environment("a");
    let grafted = rerun_in(&c, &live, "a", &fork);
    assert!(env_values(&grafted).is_empty());
    grafted
        .env_overlay()
        .set_text(OverlayLayer::Environment, "later", "1");
    assert_eq!(
        env_values(&rerun_in(&c, &live, "a", &fork)),
        [("later".to_owned(), Some("1".to_owned()))]
    );
    assert!(live.is_empty(), "the write stays in the fork");
}

// A switch leaves the values with the environment that set them, as it does without a fork.
#[test]
fn a_rerun_after_a_switch_reads_the_new_environment_and_keeps_the_old_ones_values() {
    let (_dir, c, live, fork) = forked_run_in_a();
    live.select_environment(Some("b"));
    assert!(env_values(&rerun_in(&c, &live, "b", &fork)).is_empty());
    live.select_environment(Some("a"));
    assert_eq!(env_values(&rerun_in(&c, &live, "a", &fork)), token());
}
