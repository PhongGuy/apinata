//! Secrets a runner send, its scripts and its nested sends carry never reach the log.

use crate::script_support;
use crate::session_log_support;

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, ExecutionMode, Executor, ItemPath,
    LogLimits, MemoryStore, RequestFile, RunOptions, ScriptOptions, SendSource, SessionLog,
    VarContext, send_one,
};
use script_support::{ROOT, on, script, scripted, send_full, write_tree};
use session_log_support::{assert_absent, entries, lines, only};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "S3cr/et val-77";
const LATE: &str = "LATE-fresh-value-42";

async fn route(s: &MockServer, at: &str, response: ResponseTemplate) {
    Mock::given(path(at)).respond_with(response).mount(s).await;
}

async fn sent(dir: &std::path::Path, ctx: ContextOptions, log: &SessionLog) {
    send_full(
        dir,
        "r.yml",
        on(),
        ctx,
        ExecOptions::new().session_log(log.clone()),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn script_and_nested_sends_carrying_a_secret_are_redacted() {
    let s = MockServer::start().await;
    for at in ["/main", "/s", "/t"] {
        route(&s, at, ResponseTemplate::new(200)).await;
    }
    let code = format!(
        "console.log('tok is ' + bru.getEnvVar('tok'));\nawait bru.sendRequest({{ method: 'get', url: '{}/s?key=' + bru.getEnvVar('tok') }});\nawait bru.runRequest('t');",
        s.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "t.yml",
            &format!(
                "info:\n  name: T\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{}/t?key={{{{tok}}}}\"\n",
                s.uri()
            ),
        ),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main?key={{{{tok}}}}", s.uri()),
                &script("before-request", &code),
                "",
            ),
        ),
    ]);
    let log = SessionLog::new();
    sent(
        dir.path(),
        ContextOptions::new().sensitive_env_var("tok", SECRET),
        &log,
    )
    .await;
    assert_eq!(entries(&log).len(), 3);
    assert_absent(&log, SECRET);
    assert!(lines(&log).iter().any(|l| l.text == "tok is [redacted]"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_the_response_echoes_and_a_script_reads_late_is_redacted() {
    let s = MockServer::start().await;
    route(
        &s,
        "/main",
        ResponseTemplate::new(200).set_body_string(format!("{{\"v\":\"{SECRET}\"}}")),
    )
    .await;
    // An OS process value is neither substituted nor in the send-time set, so only the
    // hold's learned values (the script's read) can hide it in the held entry.
    let code = "bru.getProcessEnv('ECHO');";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("after-response", code),
                "",
            ),
        ),
    ]);
    let log = SessionLog::new();
    sent(
        dir.path(),
        ContextOptions::new().process_env([("ECHO", SECRET)]),
        &log,
    )
    .await;
    let e = only(&log);
    assert_eq!(e.status, Some(200));
    assert_eq!(e.response_body.text, "{\"v\":\"[redacted]\"}");
    assert_absent(&log, SECRET);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_set_from_the_response_and_read_is_redacted_across_the_body_cut() {
    let s = MockServer::start().await;
    // `{"pad":"` is 8 bytes and `","fresh":"` is 11, so LATE starts 6 bytes before the 64 KiB cut.
    let body = format!(
        "{{\"pad\":\"{}\",\"fresh\":\"{LATE}\"}}",
        "a".repeat(64 * 1024 - 6 - 19)
    );
    route(
        &s,
        "/main",
        ResponseTemplate::new(200).set_body_string(body),
    )
    .await;
    let code = "bru.setEnvVar('fresh', res.body.fresh);\nconsole.log(bru.getEnvVar('fresh'));";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - secret: true\n    name: fresh\n",
        ),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("after-response", code),
                "",
            ),
        ),
    ]);
    let log = SessionLog::new();
    let ctx = ContextOptions::new()
        .env("dev")
        .secrets(Arc::new(MemoryStore::new()));
    sent(dir.path(), ctx, &log).await;
    assert_absent(&log, LATE);
    let e = &entries(&log)[0];
    assert!(e.response_body.truncated);
    let kept = e
        .response_body
        .text
        .split("\n[truncated:")
        .next()
        .unwrap_or_default();
    for k in 1..LATE.len() {
        assert!(!kept.ends_with(&LATE[..k]), "ends in {}", &LATE[..k]);
    }
    assert_eq!(lines(&log)[0].text, "[redacted]");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_sending_in_a_loop_stays_within_the_budget() {
    let s = MockServer::start().await;
    route(
        &s,
        "/big",
        ResponseTemplate::new(200).set_body_string("b".repeat(100 * 1024)),
    )
    .await;
    route(&s, "/main", ResponseTemplate::new(200)).await;
    let code = format!(
        "for (let i = 0; i < 30; i++) {{ await bru.sendRequest({{ method: 'get', url: '{}/big' }}); }}",
        s.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main", s.uri()),
                &script("before-request", &code),
                "",
            ),
        ),
    ]);
    let mut limits = LogLimits::default();
    limits.budget_bytes = 512 * 1024;
    let log = SessionLog::with_limits(limits);
    sent(dir.path(), ContextOptions::new(), &log).await;
    assert!(log.retained_bytes() <= limits.budget_bytes);
    let all = entries(&log);
    assert!(all.len() < 31, "{}", all.len());
    assert_eq!(all.last().map(|e| e.source), Some(SendSource::Runner));
    assert!(all.windows(2).all(|w| w[1].seq == w[0].seq + 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reader_polling_through_interleaved_sends_sees_each_entry_once() {
    let s = MockServer::start().await;
    route(
        &s,
        "/held",
        ResponseTemplate::new(200).set_delay(Duration::from_millis(300)),
    )
    .await;
    route(&s, "/fast", ResponseTemplate::new(200)).await;
    let fast_file = format!(
        "info:\n  name: F\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{}/fast\"\n",
        s.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f.yml", &fast_file),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/held", s.uri()),
                &script("before-request", "console.log('held')"),
                "",
            ),
        ),
    ]);
    let log = SessionLog::new();
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec =
        Arc::new(Executor::new(ExecOptions::new().session_log(log.clone())).expect("client"));
    let request = RequestFile::load(dir.path().join("r.yml")).expect("loads");
    let fast = apinata_core::resolve(
        &RequestFile::load(dir.path().join("f.yml")).expect("loads"),
        &ctx,
    )
    .expect("resolves");
    let done = AtomicBool::new(false);
    let mut seen = Vec::new();
    let mut next = 0;
    // Console first, then network filtered below the console slice's `next`, so an entry
    // published between the two calls is read on the next poll, never twice.
    let mut poll = |seen: &mut Vec<u64>| {
        let c = log.console_since(next);
        let n = log.network_since(next);
        seen.extend(c.entries.iter().map(|l| l.seq));
        seen.extend(n.entries.iter().map(|e| e.seq).filter(|seq| *seq < c.next));
        next = c.next;
    };
    let (opts, cancel) = (on(), CancelHandle::new());
    let held = send_one(
        ItemPath::new("r.yml").expect("path"),
        &request,
        &ctx,
        &exec,
        &opts,
        &cancel,
    );
    let direct = async {
        for _ in 0..3 {
            let _ = exec.send(&fast).await;
        }
    };
    let both = async {
        tokio::join!(held, direct);
        done.store(true, Ordering::SeqCst);
    };
    let poller = async {
        while !done.load(Ordering::SeqCst) {
            poll(&mut seen);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::join!(both, poller);
    poll(&mut seen);
    poll(&mut seen);
    seen.sort();
    assert_eq!(seen, (0..next).collect::<Vec<_>>());
    // One console line, the held runner entry and three direct entries.
    assert_eq!(next, 5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_scripts_off_entries_are_tagged_by_mode_and_carry_no_secret() {
    let s = MockServer::start().await;
    route(
        &s,
        "/main",
        ResponseTemplate::new(200).set_body_string(SECRET),
    )
    .await;
    let code = "console.log('tok is ' + bru.getEnvVar('tok'));";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/main?key={{{{tok}}}}", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    for (mode, source) in [
        (ExecutionMode::Standalone, SendSource::Request),
        (ExecutionMode::Runner, SendSource::Runner),
    ] {
        let log = SessionLog::new();
        let opts = RunOptions::new().execution_mode(mode);
        let ctx = ContextOptions::new().sensitive_env_var("tok", SECRET);
        send_full(
            dir.path(),
            "r.yml",
            opts,
            ctx,
            ExecOptions::new().session_log(log.clone()),
        )
        .await;
        let all = entries(&log);
        assert_eq!(all.iter().map(|e| e.source).collect::<Vec<_>>(), [source]);
        // The script never ran, so no console line was recorded to leak through.
        assert!(lines(&log).is_empty());
        assert_absent(&log, SECRET);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dotenv_value_is_hidden_but_an_unused_process_value_is_not() {
    let s = MockServer::start().await;
    route(
        &s,
        "/main",
        ResponseTemplate::new(200).set_body_string("dotenv-echo-9 Plain"),
    )
    .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (".env", "DOT=dotenv-echo-9\n"),
        (
            "r.yml",
            &scripted("R", &format!("{}/main", s.uri()), "", ""),
        ),
    ]);
    let log = SessionLog::new();
    // The process value is never substituted or read, so it is ordinary text in the log.
    let ctx = ContextOptions::new().process_env([("SHELLISH", "Plain")]);
    sent(dir.path(), ctx, &log).await;
    assert_absent(&log, "dotenv-echo-9");
    assert_eq!(entries(&log)[0].response_body.text, "[redacted] Plain");
}

/// A request whose after-response script prints the body the server echoes, with `auth` under `http`.
fn printing(url: &str, auth: &str) -> String {
    let scripts = script("after-response", "console.log(res.body);");
    format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{auth}runtime:\n  scripts:\n{scripts}"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_substituted_os_value_the_script_prints_is_redacted() {
    let s = MockServer::start().await;
    route(
        &s,
        "/main",
        ResponseTemplate::new(200).set_body_string("echo os-secret-777"),
    )
    .await;
    let url = format!("{}/main?k={{{{process.env.OSTOK}}}}", s.uri());
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &printing(&url, ""))]);
    let log = SessionLog::new();
    sent(
        dir.path(),
        ContextOptions::new().process_env([("OSTOK", "os-secret-777")]),
        &log,
    )
    .await;
    assert_eq!(lines(&log).len(), 1);
    assert_absent(&log, "os-secret-777");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_literal_bearer_token_the_script_prints_is_redacted() {
    let s = MockServer::start().await;
    route(
        &s,
        "/main",
        ResponseTemplate::new(200).set_body_string("echo lit-bearer-555"),
    )
    .await;
    let auth = "  auth:\n    type: bearer\n    token: lit-bearer-555\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &printing(&format!("{}/main", s.uri()), auth)),
    ]);
    let log = SessionLog::new();
    sent(dir.path(), ContextOptions::new(), &log).await;
    assert_eq!(lines(&log).len(), 1);
    assert_absent(&log, "lit-bearer-555");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_loop_of_signed_sends_hides_each_signature_without_saturating_the_hold() {
    let s = MockServer::start().await;
    route(&s, "/o", ResponseTemplate::new(200)).await;
    route(&s, "/main", ResponseTemplate::new(200)).await;
    // Each send places a fresh signature in its query; its own entry hides it, and the hold
    // learns none of them, so 1100 sends leave the console line readable.
    let auth = "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: cs-oauth-3391\n    accessToken: tk-oauth-1180\n    accessTokenSecret: ts-oauth-7713\n    signatureMethod: HMAC-SHA1\n    placement: query\n";
    let signed = format!(
        "info:\n  name: O\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{}/o\"\n{auth}",
        s.uri()
    );
    let code =
        "for (let i = 0; i < 1100; i++) { await bru.runRequest('o'); }\nconsole.log('done');";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("o.yml", &signed),
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
    let log = SessionLog::new();
    // The loop is a throughput test, not a time-limit test: give the script a limit no loaded
    // machine reaches, so only the redaction behaviour decides the outcome.
    send_full(
        dir.path(),
        "r.yml",
        RunOptions::new().scripts(
            ScriptOptions::new()
                .enabled(true)
                .timeout(Duration::from_secs(600)),
        ),
        ContextOptions::new(),
        ExecOptions::new().session_log(log.clone()),
    )
    .await;
    let texts: Vec<String> = lines(&log).iter().map(|l| l.text.clone()).collect();
    assert_eq!(texts, ["done"]);
    let signed: Vec<_> = entries(&log)
        .into_iter()
        .filter(|e| e.url.ends_with("/o") || e.url.contains("/o?"))
        .collect();
    assert!(signed.len() > 400, "{}", signed.len());
    for e in &signed {
        let fields = session_log_support::texts(e);
        assert!(
            fields.iter().any(|t| t.contains("oauth_signature=")),
            "{fields:?}"
        );
        for text in fields {
            for (i, _) in text.match_indices("oauth_signature=") {
                assert!(
                    text[i + "oauth_signature=".len()..].starts_with("[redacted]"),
                    "{text}"
                );
            }
        }
    }
    for secret in ["cs-oauth-3391", "tk-oauth-1180", "ts-oauth-7713"] {
        assert_absent(&log, secret);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_a_folder_variable_shadows_is_still_redacted_from_the_log() {
    let s = MockServer::start().await;
    // The server echoes the shadowed environment secret, which the request never substituted.
    route(
        &s,
        "/f/main",
        ResponseTemplate::new(200).set_body_string(SECRET),
    )
    .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "f/folder.yml",
            "info:\n  name: f\n  type: folder\nrequest:\n  variables:\n    - name: tok\n      value: plain-folder-value\n",
        ),
        (
            "f/r.yml",
            &scripted("R", &format!("{}/f/main?k={{{{tok}}}}", s.uri()), "", ""),
        ),
    ]);
    let log = SessionLog::new();
    send_full(
        dir.path(),
        "f/r.yml",
        on(),
        ContextOptions::new().sensitive_env_var("tok", SECRET),
        ExecOptions::new().session_log(log.clone()),
    )
    .await;
    assert_eq!(entries(&log).len(), 1);
    assert_absent(&log, SECRET);
}
