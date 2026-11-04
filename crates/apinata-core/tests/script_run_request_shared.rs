//! What `bru.runRequest` shares with its caller: cookies, the host policy, the guard, approvals
//! and the caller's lifetime.

use crate::script_support;

use apinata_core::{
    Collection, ContextOptions, ExecError, ExecOptions, Outcome, RequestFile, ResolvedRequest,
    RunOptions, ScriptOptions, VarContext, script_texts,
};
use script_support::*;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn plain(name: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

fn var(ctx: &VarContext, name: &str) -> Option<String> {
    ctx.runtime_vars()
        .entries()
        .iter()
        .find(|v| v.name() == name)
        .and_then(|v| v.value().map(str::to_owned))
}

async fn mounted(s: &MockServer, route: &str, times: u64) {
    Mock::given(path(route))
        .respond_with(ResponseTemplate::new(200))
        .expect(times)
        .mount(s)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_nested_run_shares_the_cookie_jar() {
    let s = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", "sid=abc; Path=/"))
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(path("/me"))
        .and(header("cookie", "sid=abc"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("login.yml", &plain("L", &format!("{}/login", s.uri()))),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/me", s.uri()),
                &script("before-request", "await bru.runRequest('login');"),
                "",
            ),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_policy_refusals_are_recorded_on_the_caller() {
    let s = MockServer::start().await;
    mounted(&s, "/r", 1).await;
    let inner = "try { await bru.sendRequest('http://other.example.com/y'); } catch (e) {}";
    let outer = "for (const _ of [1, 2]) {\n  const r = await bru.runRequest('ext');\n  console.log(r.error);\n}";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "ext.yml",
            &scripted(
                "E",
                "http://api.example.com/x",
                &script("before-request", inner),
                "",
            ),
        ),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/r", s.uri()),
                &script("before-request", outer),
                "",
            ),
        ),
    ]);
    let exec = ExecOptions::new().host_policy(Arc::new(|host: &str| host == "127.0.0.1"));
    let (result, _) = send_full(dir.path(), "r.yml", on(), ContextOptions::new(), exec).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let lines = console(&result);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines
            .iter()
            .all(|l| l.starts_with("log:host `api.example.com` is not allowed")),
        "{lines:?}"
    );
    let mut hosts: Vec<String> = result
        .script_refusals()
        .iter()
        .map(|e| match e {
            ExecError::HostNotAllowed { host, .. } => host.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    hosts.sort();
    assert_eq!(hosts, ["api.example.com", "other.example.com"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_run_guard_refuses_a_nested_run() {
    let s = MockServer::start().await;
    mounted(&s, "/ok", 1).await;
    mounted(&s, "/guarded", 0).await;
    let code = "console.log(JSON.stringify(await bru.runRequest('guarded')));";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("guarded.yml", &plain("G", &format!("{}/guarded", s.uri()))),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let guard = Arc::new(|r: &ResolvedRequest| {
        r.url()
            .contains("/guarded")
            .then(|| "refused by test".to_owned())
    });
    let result = send(dir.path(), "r.yml", on().guard(guard)).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), [r#"log:{"message":"refused by test"}"#]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unapproved_nested_script_does_not_run() {
    let s = MockServer::start().await;
    mounted(&s, "/r", 1).await;
    mounted(&s, "/t", 0).await;
    let code =
        "const r = await bru.runRequest('t');\nconsole.log(r.message.includes('not approved'));";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/r", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
        (
            "t.yml",
            &scripted(
                "T",
                &format!("{}/t", s.uri()),
                &script("before-request", "bru.setVar('ran', 'yes');"),
                "",
            ),
        ),
    ]);
    // Only the caller's own scripts are approved, as the MCP gate approves them.
    let c = Collection::open(dir.path()).unwrap();
    let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
    let req = RequestFile::load(dir.path().join("r.yml")).unwrap();
    let hashes: BTreeSet<String> = script_texts(&req, &ctx)
        .into_iter()
        .map(|t| t.hash)
        .collect();
    let opts = RunOptions::new().scripts(ScriptOptions::new().enabled(true).only_approved(hashes));
    let (result, ctx) = send_full(
        dir.path(),
        "r.yml",
        opts,
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:true"]);
    assert_eq!(var(&ctx, "ran"), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_nested_run_stops_when_its_caller_ends() {
    let s = MockServer::start().await;
    mounted(&s, "/n", 0).await;
    let ticker = "while (true) {\n  await bru.sleep(50);\n  bru.setVar('ticks', String(Number(bru.getVar('ticks') || 0) + 1));\n}";
    // The nested run starts 300 ms in, so its own 400 ms limit outlives the caller's by 300 ms.
    let caller = "await bru.sleep(300);\nawait bru.runRequest('n');";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "n.yml",
            &scripted(
                "N",
                &format!("{}/n", s.uri()),
                &script("before-request", ticker),
                "",
            ),
        ),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/r", s.uri()),
                &script("before-request", caller),
                "",
            ),
        ),
    ]);
    let (result, ctx) = send_full(
        dir.path(),
        "r.yml",
        limited(400),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert!(script_error(&result).contains("script exceeded the 0.4 s limit"));
    let ticks = |ctx: &VarContext| var(ctx, "ticks").map_or(0, |t| t.parse::<u32>().unwrap());
    let before = ticks(&ctx);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after = ticks(&ctx);
    assert!(after <= before + 1, "{before} -> {after}");
}

/// Sends `r.yml` whose script calls `n` 300 ms in, where `n` reaches a spinning request
/// through `hops` further levels; returns the result and how long the send took.
async fn spin_beneath_a_caller(hops: usize) -> (apinata_core::RequestResult, Duration) {
    let s = MockServer::start().await;
    let mut files = vec![("opencollection.yml".to_owned(), ROOT.to_owned())];
    let levels: Vec<String> = (0..=hops).map(|i| format!("n{i}")).collect();
    for (i, name) in levels.iter().enumerate() {
        let code = match levels.get(i + 1) {
            Some(next) => format!("await bru.runRequest('{next}');"),
            None => "for (;;) {}".to_owned(),
        };
        let url = format!("{}/{name}", s.uri());
        let body = scripted(name, &url, &script("before-request", &code), "");
        files.push((format!("{name}.yml"), body));
    }
    let caller = "await bru.sleep(300);\nawait bru.runRequest('n0');";
    let url = format!("{}/r", s.uri());
    files.push((
        "r.yml".to_owned(),
        scripted("R", &url, &script("before-request", caller), ""),
    ));
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let dir = write_tree(&refs);
    let start = std::time::Instant::now();
    let result = send(dir.path(), "r.yml", limited(400)).await;
    (result, start.elapsed())
}

// One thread, as the CLI runs: the caller can only end on time if the spinning nested run
// gives the thread back, so this also pins that the nested run stops.
#[tokio::test(flavor = "current_thread")]
async fn a_spinning_nested_run_ends_at_its_callers_limit() {
    let (result, took) = spin_beneath_a_caller(0).await;
    assert!(
        script_error(&result).contains("script exceeded the 0.4 s limit"),
        "{:?} {took:?}",
        result.outcome
    );
    assert!(took < Duration::from_millis(650), "{took:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_spinning_nested_run_ends_at_its_callers_limit_on_many_threads() {
    let (result, took) = spin_beneath_a_caller(0).await;
    assert!(
        script_error(&result).contains("script exceeded the 0.4 s limit"),
        "{:?} {took:?}",
        result.outcome
    );
    assert!(took < Duration::from_millis(650), "{took:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn a_spin_two_levels_down_ends_at_the_outermost_limit() {
    let (result, took) = spin_beneath_a_caller(1).await;
    assert!(
        script_error(&result).contains("script exceeded the 0.4 s limit"),
        "{:?} {took:?}",
        result.outcome
    );
    assert!(took < Duration::from_millis(650), "{took:?}");
}

// The late-finish check must not fire on a nested run that ends well inside the caller's limit.
#[tokio::test(flavor = "current_thread")]
async fn a_quick_nested_run_inside_a_generous_limit_passes_intact() {
    let s = MockServer::start().await;
    mounted(&s, "/r", 1).await;
    Mock::given(path("/n"))
        .respond_with(ResponseTemplate::new(201).set_body_string("ok"))
        .expect(1)
        .mount(&s)
        .await;
    let caller = "const r = await bru.runRequest('n');\nconsole.log(r.status, r.data);";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "n.yml",
            &scripted(
                "N",
                &format!("{}/n", s.uri()),
                &script("before-request", "bru.setVar('ran', 'yes');"),
                "",
            ),
        ),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/r", s.uri()),
                &script("before-request", caller),
                "",
            ),
        ),
    ]);
    let (result, ctx) = send_full(
        dir.path(),
        "r.yml",
        limited(10_000),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:201 ok"]);
    assert_eq!(var(&ctx, "ran").as_deref(), Some("yes"));
}
