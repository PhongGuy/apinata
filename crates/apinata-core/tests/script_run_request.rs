//! `bru.runRequest`: the nested exchange, its result shape, its depth and per-level queueing.

use crate::script_support;

use apinata_core::{ContextOptions, ExecOptions, Outcome, VarContext};
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A GET request with no scripts.
fn plain(name: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

/// `text` with its `seq` set to `n`.
fn at(text: String, n: u32) -> String {
    text.replacen("seq: 1", &format!("seq: {n}"), 1)
}

fn var(ctx: &VarContext, name: &str) -> Option<String> {
    ctx.runtime_vars()
        .entries()
        .iter()
        .find(|v| v.name() == name)
        .and_then(|v| v.value().map(str::to_owned))
}

async fn ok(s: &MockServer, route: &str, times: u64) {
    Mock::given(path(route))
        .respond_with(ResponseTemplate::new(200))
        .expect(times)
        .mount(s)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_nested_run_resolves_the_response_shape() {
    let s = MockServer::start().await;
    Mock::given(path("/data"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-a", "1")
                .set_body_string(r#"{"n":1}"#),
        )
        .expect(1)
        .mount(&s)
        .await;
    ok(&s, "/main", 1).await;
    let code = "const r = await bru.runRequest('t');\nconsole.log(JSON.stringify([r.status, r.statusText, r.headers['x-a'], r.data.n, r.dataBuffer, r.size, typeof r.duration, r.url.endsWith('/data')]));";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("t.yml", &plain("T", &format!("{}/data", s.uri()))),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(
        console(&result),
        [r#"log:[200,"OK","1",1,"eyJuIjoxfQ==",7,"number",true]"#]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_request_extension_is_optional() {
    let s = MockServer::start().await;
    ok(&s, "/data", 2).await;
    ok(&s, "/main", 1).await;
    let code = "const a = await bru.runRequest('t');\nconst b = await bru.runRequest('t.yml');\nconsole.log(a.status, b.status);";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("t.yml", &plain("T", &format!("{}/data", s.uri()))),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:200 200"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_depth_two_chain_shares_runtime_variables() {
    let s = MockServer::start().await;
    ok(&s, "/main", 1).await;
    ok(&s, "/b", 1).await;
    ok(&s, "/c", 1).await;
    let a = "bru.setVar('seed', 's');\nconst r = await bru.runRequest('b');\nbru.setVar('outer', r.status + bru.getVar('fromC'));";
    let b = "await bru.runRequest('c');";
    let c = "bru.setVar('fromC', 'c' + bru.getVar('seed'));";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &scripted(
                "A",
                &format!("{}/main", s.uri()),
                &script("before-request", a),
                "",
            ),
        ),
        (
            "b.yml",
            &scripted(
                "B",
                &format!("{}/b", s.uri()),
                &script("before-request", b),
                "",
            ),
        ),
        (
            "c.yml",
            &scripted(
                "C",
                &format!("{}/c", s.uri()),
                &script("after-response", c),
                "",
            ),
        ),
    ]);
    let (result, ctx) = send_full(
        dir.path(),
        "a.yml",
        on(),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(var(&ctx, "outer").as_deref(), Some("200cs"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_self_loop_stops_at_depth_16() {
    let s = MockServer::start().await;
    // The top-level send plus sixteen nested ones each reach the server once.
    ok(&s, "/x", 17).await;
    let code = "const r = await bru.runRequest('loop');\nif (r.message) bru.setVar('cap', r.message);\nbru.setVar('depth', String(Number(bru.getVar('depth') || 0) + 1));";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "loop.yml",
            &scripted(
                "L",
                &format!("{}/x", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let (result, ctx) = send_full(
        dir.path(),
        "loop.yml",
        on(),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(
        var(&ctx, "cap").as_deref(),
        Some("bru.runRequest: maximum nesting depth (16) exceeded")
    );
    assert_eq!(var(&ctx, "depth").as_deref(), Some("17"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_calls_queue_per_level() {
    let s = MockServer::start().await;
    ok(&s, "/main", 1).await;
    ok(&s, "/b", 2).await;
    ok(&s, "/c", 3).await;
    // Two of the parallel calls nest again; one lock for every level would deadlock here.
    let code = "const all = await Promise.all([bru.runRequest('b'), bru.runRequest('b'), bru.runRequest('c')]);\nconsole.log(JSON.stringify(all.map((r) => r.status)));";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
        (
            "b.yml",
            &scripted(
                "B",
                &format!("{}/b", s.uri()),
                &script("before-request", "await bru.runRequest('c');"),
                "",
            ),
        ),
        ("c.yml", &plain("C", &format!("{}/c", s.uri()))),
    ]);
    let result = send(dir.path(), "r.yml", limited(10_000)).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:[200,200,200]"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nested_results_stay_out_of_the_report() {
    let s = MockServer::start().await;
    ok(&s, "/a", 1).await;
    ok(&s, "/b", 2).await;
    let a = "const r = await bru.runRequest('b');\nbru.setVar('st', String(r.status));";
    let b = "test('f', () => expect(1).to.equal(2));";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &scripted(
                "A",
                &format!("{}/a", s.uri()),
                &script("before-request", a),
                "",
            ),
        ),
        (
            "b.yml",
            &at(
                scripted("B", &format!("{}/b", s.uri()), &script("tests", b), ""),
                2,
            ),
        ),
    ]);
    let report = run_dir(dir.path(), on()).await;
    let outcomes: Vec<Outcome> = report.results.iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [Outcome::Passed, Outcome::Failed]);
    assert!(report.results[0].tests.is_empty());
    assert_eq!(report.summary.failed_tests, 1);
}
