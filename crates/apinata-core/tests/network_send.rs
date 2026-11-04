//! The network change check runs on what each send resolves, in memory: a folder, request or
//! run variable that reroutes the proxy asks, as do a reordered or swapped certificate, and the
//! answer records only what the question showed. Every send path the executor offers is checked.

use crate::common;

use apinata_core::{
    AuthorizeOptions, CancelHandle, Collection, ContextOptions, ExecError, ExecOptions, Executor,
    ItemPath, NetworkAck, NetworkAnswer, NetworkCheck, NetworkQuestion, OAuth2Target, Outcome,
    QuestionKind, RequestFile, RequestResult, RunError, RunOptions, ScriptOptions, SessionLog,
    VarContext, fetch_schema, introspection_request, run, send_one,
};
use common::write_tree;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// What the asker saw: each question's kind and summary.
type Seen = Arc<Mutex<Vec<(QuestionKind, String)>>>;

/// A check on `store` whose ask records the question and answers `first` for a first sight and
/// `changed` for a change.
fn check(store: &Path, first: NetworkAnswer, changed: NetworkAnswer) -> (NetworkCheck, Seen) {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let ask = move |q: NetworkQuestion| {
        log.lock().unwrap().push((q.kind(), q.summary()));
        let answer = match q.kind() {
            QuestionKind::FirstSeen => first,
            _ => changed,
        };
        async move { answer }
    };
    (NetworkCheck::new(store, ask), seen)
}

fn changed(seen: &Seen) -> Vec<String> {
    let seen = seen.lock().unwrap();
    seen.iter()
        .filter(|(k, _)| *k == QuestionKind::Changed)
        .map(|(_, s)| s.clone())
        .collect()
}

/// A proxy on 127.0.0.1 that answers every request with a token-shaped JSON body.
async fn proxy() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "{\"access_token\":\"t\",\"token_type\":\"Bearer\",\"p\":\"localhost\"}",
            "application/json",
        ))
        .mount(&s)
        .await;
    s
}

fn root(port: u16, host: &str) -> String {
    format!(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: \"{host}\"\n      port: \"{port}\"\n"
    )
}

fn get(name: &str, seq: u32, url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}

fn vars(name: &str, value: &str) -> String {
    format!("  variables:\n    - name: {name}\n      value: \"{value}\"\n")
}

const ENV: &str = "name: dev\nvariables:\n  - name: ph\n    value: 127.0.0.1\n";

/// A collection whose proxy host is `{{ph}}`: 127.0.0.1 in the environment, `localhost` in
/// folder `f`, with `a.yml` at the root and `f/b.yml` in the folder.
fn shadowed(port: u16) -> TempDir {
    write_tree(&[
        ("opencollection.yml", &root(port, "{{ph}}")),
        ("environments/dev.yml", ENV),
        (
            "f/folder.yml",
            &format!(
                "info:\n  name: f\n  type: folder\nrequest:\n{}",
                vars("ph", "localhost")
            ),
        ),
        ("a.yml", &get("a", 1, "http://example.test/a", "")),
        ("f/b.yml", &get("b", 2, "http://example.test/b", "")),
    ])
}

fn ctx_of(dir: &Path) -> VarContext {
    let c = Collection::open(dir).unwrap();
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    VarContext::new(&c, ContextOptions::new().env("dev")).unwrap()
}

fn exec_with(check: NetworkCheck) -> Arc<Executor> {
    Arc::new(Executor::new(ExecOptions::new().network_check(check)).unwrap())
}

/// Sends the request at `rel` under `dir` in `ctx`.
async fn send_in(
    dir: &Path,
    ctx: &VarContext,
    rel: &str,
    exec: &Arc<Executor>,
    opts: &RunOptions,
) -> RequestResult {
    let request = RequestFile::load(dir.join(rel)).unwrap();
    let path = ItemPath::new(rel).unwrap();
    send_one(path, &request, ctx, exec, opts, &CancelHandle::new()).await
}

async fn send(dir: &Path, rel: &str, check: NetworkCheck) -> RequestResult {
    let ctx = ctx_of(dir);
    send_in(dir, &ctx, rel, &exec_with(check), &RunOptions::new()).await
}

/// A declined question is the person's cancel: the outcome is Cancelled and the error still
/// says why, so reports stay honest.
fn refused(result: &RequestResult) -> bool {
    result.outcome == Outcome::Cancelled
        && matches!(
            result.error,
            Some(RunError::Exec(ExecError::NetworkDeclined))
        )
        && result
            .error
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default()
            == "the network settings changed and the send was not confirmed"
}

async fn paths(s: &MockServer) -> Vec<String> {
    let seen = s.received_requests().await.unwrap_or_default();
    seen.iter().map(|r| r.url.path().to_owned()).collect()
}

use NetworkAnswer::{Cancel, Send};

#[tokio::test]
async fn an_unshadowed_send_is_told_once_then_goes_quietly() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    let first = send(dir.path(), "a.yml", c.clone()).await;
    assert!(first.error.is_none(), "{:?}", first.error);
    let second = send(dir.path(), "a.yml", c).await;
    assert!(second.error.is_none(), "{:?}", second.error);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].0, QuestionKind::FirstSeen);
    assert_eq!(paths(&p).await, ["/a", "/a"]);
}

#[tokio::test]
async fn a_folder_variable_that_shadows_the_proxy_asks_before_the_send() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    let b = send(dir.path(), "f/b.yml", c).await;
    assert!(refused(&b), "{:?}", b.error);
    let asked = changed(&seen);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].contains("localhost"), "{asked:?}");
    assert!(
        asked[0].contains("set by a folder, request or run variable"),
        "{asked:?}"
    );
    assert_eq!(paths(&p).await, ["/a"]);
}

#[tokio::test]
async fn a_request_variable_that_shadows_the_proxy_asks() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let own = get(
        "r",
        3,
        "http://example.test/r",
        &format!("runtime:\n{}", vars("ph", "localhost")),
    );
    fs::write(dir.path().join("r.yml"), own).unwrap();
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    let r = send(dir.path(), "r.yml", c).await;
    assert!(refused(&r), "{:?}", r.error);
    assert_eq!(changed(&seen).len(), 1);
    assert_eq!(paths(&p).await, ["/a"]);
}

#[tokio::test]
async fn a_capture_that_shadows_the_proxy_asks_on_the_next_request_and_stops_the_run() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let capture = "runtime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"res.body.p\"\n        method: jsonq\n      variable:\n        name: \"ph\"\n        scope: runtime\n";
    fs::write(
        dir.path().join("a.yml"),
        get("a", 1, "http://example.test/a", capture),
    )
    .unwrap();
    fs::write(
        dir.path().join("c.yml"),
        get("c", 2, "http://example.test/c", ""),
    )
    .unwrap();
    fs::write(
        dir.path().join("d.yml"),
        get("d", 3, "http://example.test/d", ""),
    )
    .unwrap();
    fs::remove_dir_all(dir.path().join("f")).unwrap();
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    let col = Collection::open(dir.path()).unwrap();
    let plan = col.plan(dir.path()).unwrap();
    let ctx = VarContext::new(&col, ContextOptions::new().env("dev")).unwrap();
    let report = run(
        plan,
        ctx,
        exec_with(c),
        RunOptions::new(),
        CancelHandle::new(),
        Box::new(|_| {}),
    )
    .await;
    let results = &report.results;
    assert!(results[0].error.is_none(), "{:?}", results[0].error);
    assert!(refused(&results[1]), "{:?}", results[1].error);
    assert_eq!(changed(&seen).len(), 1);
    assert_eq!(results[2].outcome, apinata_core::Outcome::Skipped);
    assert_eq!(paths(&p).await, ["/a"]);
}

fn fixture(name: &str) -> String {
    format!(
        "{}/tests/fixtures/network/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn cert_root(entries: &[(&str, &str)]) -> String {
    let mut out = String::from(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  clientCertificates:\n",
    );
    for (cert, key) in entries {
        out.push_str(&format!(
            "    - domain: \"127.0.0.1\"\n      type: pem\n      certificateFilePath: '{cert}'\n      privateKeyFilePath: '{key}'\n"
        ));
    }
    out
}

/// A collection with client certificates for 127.0.0.1 and a request to a closed TLS port.
fn cert_collection(entries: &[(&str, &str)]) -> TempDir {
    write_tree(&[
        ("opencollection.yml", &cert_root(entries)),
        ("environments/dev.yml", ENV),
        ("a.yml", &get("a", 1, "https://127.0.0.1:9/a", "")),
    ])
}

#[tokio::test]
async fn a_certificate_reorder_that_changes_the_chosen_certificate_asks() {
    let (a, b) = (
        (fixture("client.pem"), fixture("client.key")),
        (fixture("server.pem"), fixture("server.key")),
    );
    let order = [(a.0.as_str(), a.1.as_str()), (b.0.as_str(), b.1.as_str())];
    let dir = cert_collection(&order);
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    let reversed = [order[1], order[0]];
    fs::write(dir.path().join("opencollection.yml"), cert_root(&reversed)).unwrap();
    let again = send(dir.path(), "a.yml", c).await;
    assert!(refused(&again), "{:?}", again.error);
    let asked = changed(&seen);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].contains("Client certificate order changed"),
        "{asked:?}"
    );
}

#[tokio::test]
async fn a_swapped_certificate_file_asks() {
    let dir = cert_collection(&[("c.pem", "c.key")]);
    fs::copy(fixture("client.pem"), dir.path().join("c.pem")).unwrap();
    fs::copy(fixture("client.key"), dir.path().join("c.key")).unwrap();
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    fs::copy(fixture("expired-client.pem"), dir.path().join("c.pem")).unwrap();
    let again = send(dir.path(), "a.yml", c).await;
    assert!(refused(&again), "{:?}", again.error);
    let asked = changed(&seen);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].contains("Client certificate changed for 127.0.0.1"),
        "{asked:?}"
    );
}

#[tokio::test]
async fn a_disk_flip_cannot_get_a_send_through_on_the_state_in_memory() {
    let p = proxy().await;
    let port = p.address().port();
    let dir = write_tree(&[
        ("opencollection.yml", &root(port, "127.0.0.1")),
        ("environments/dev.yml", ENV),
        ("a.yml", &get("a", 1, "http://example.test/a", "")),
    ]);
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    // The model in memory is B while the disk is back on the acknowledged A.
    fs::write(
        dir.path().join("opencollection.yml"),
        root(port, "localhost"),
    )
    .unwrap();
    let memory_b = ctx_of(dir.path());
    fs::write(
        dir.path().join("opencollection.yml"),
        root(port, "127.0.0.1"),
    )
    .unwrap();
    let disk = apinata_core::saved_network(dir.path()).unwrap();
    assert_eq!(
        NetworkAck::load(&store).unwrap().check(dir.path(), &disk),
        apinata_core::AckState::Unchanged
    );
    let b = send_in(
        dir.path(),
        &memory_b,
        "a.yml",
        &exec_with(c),
        &RunOptions::new(),
    )
    .await;
    assert!(refused(&b), "{:?}", b.error);
    assert_eq!(changed(&seen).len(), 1);
    assert_eq!(paths(&p).await, ["/a"]);
}

#[tokio::test]
async fn an_answer_records_only_the_state_it_was_shown() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Send);
    send(dir.path(), "a.yml", c.clone()).await;
    send(dir.path(), "f/b.yml", c.clone()).await;
    send(dir.path(), "f/b.yml", c.clone()).await;
    assert_eq!(changed(&seen).len(), 1);
    let other = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n{}",
        vars("ph", "LOCALHOST.")
    );
    fs::write(dir.path().join("f/folder.yml"), other).unwrap();
    send(dir.path(), "f/b.yml", c).await;
    assert_eq!(changed(&seen).len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_sends_of_one_state_ask_once() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let store = dir.path().join("ack.json");
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let ask = move |q: NetworkQuestion| {
        log.lock().unwrap().push((q.kind(), q.summary()));
        async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Send
        }
    };
    let exec = exec_with(NetworkCheck::new(&store, ask));
    let ctx = ctx_of(dir.path());
    let request = RequestFile::load(dir.path().join("f/b.yml")).unwrap();
    let path = ItemPath::new("f/b.yml").unwrap();
    let opts = RunOptions::new();
    let cancel = CancelHandle::new();
    let (one, two) = tokio::join!(
        send_one(path.clone(), &request, &ctx, &exec, &opts, &cancel),
        send_one(path, &request, &ctx, &exec, &opts, &cancel),
    );
    assert!(one.error.is_none() && two.error.is_none());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
}

#[tokio::test]
async fn a_cancel_while_asking_reads_as_cancel() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let store = dir.path().join("ack.json");
    let ask = |_q: NetworkQuestion| std::future::pending::<NetworkAnswer>();
    let exec = exec_with(NetworkCheck::new(&store, ask));
    let ctx = ctx_of(dir.path());
    let request = RequestFile::load(dir.path().join("a.yml")).unwrap();
    let cancel = CancelHandle::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.cancel();
    });
    let opts = RunOptions::new();
    let done = tokio::time::timeout(
        Duration::from_secs(10),
        send_one(
            ItemPath::new("a.yml").unwrap(),
            &request,
            &ctx,
            &exec,
            &opts,
            &cancel,
        ),
    )
    .await
    .expect("the ask ends with the cancel");
    assert!(done.response.is_none());
    assert!(paths(&p).await.is_empty());
}

fn scripts_on() -> RunOptions {
    RunOptions::new().scripts(ScriptOptions::new().enabled(true))
}

fn with_script(name: &str, url: &str, code: &str) -> String {
    let body: String = code.lines().map(|l| format!("        {l}\n")).collect();
    get(
        name,
        5,
        url,
        &format!("runtime:\n  scripts:\n    - type: before-request\n      code: |-\n{body}"),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_nested_run_request_through_a_shadowed_proxy_asks() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    fs::write(
        dir.path().join("r.yml"),
        with_script("r", "http://example.test/r", "await bru.runRequest('f/b');"),
    )
    .unwrap();
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    let ctx = ctx_of(dir.path());
    send_in(dir.path(), &ctx, "r.yml", &exec_with(c), &scripts_on()).await;
    assert_eq!(changed(&seen).len(), 1, "{:?}", seen.lock().unwrap());
    assert!(!paths(&p).await.contains(&"/b".to_owned()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scripted_send_through_a_shadowed_proxy_asks_and_an_unshadowed_one_goes() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let code = "await bru.sendRequest({ method: 'get', url: 'http://example.test/s' });";
    fs::write(
        dir.path().join("f/s.yml"),
        with_script("s", "http://example.test/top", code),
    )
    .unwrap();
    fs::write(
        dir.path().join("u.yml"),
        with_script("u", "http://example.test/top", code),
    )
    .unwrap();
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    let ctx = ctx_of(dir.path());
    send_in(
        dir.path(),
        &ctx,
        "u.yml",
        &exec_with(c.clone()),
        &scripts_on(),
    )
    .await;
    assert!(changed(&seen).is_empty(), "{:?}", seen.lock().unwrap());
    assert_eq!(paths(&p).await, ["/a", "/s", "/top"]);
    send_in(dir.path(), &ctx, "f/s.yml", &exec_with(c), &scripts_on()).await;
    assert!(!changed(&seen).is_empty());
    assert_eq!(paths(&p).await, ["/a", "/s", "/top"]);
}

fn oauth2_request(name: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 4\nhttp:\n  method: GET\n  url: \"http://example.test/{name}\"\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"http://example.test/token\"\n    credentials:\n      clientId: app\n      clientSecret: cs\n      placement: body\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n    settings:\n      autoFetchToken: true\n      autoRefreshToken: false\n"
    )
}

#[tokio::test]
async fn an_oauth2_token_request_through_a_shadowed_proxy_asks() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    fs::write(dir.path().join("f/o.yml"), oauth2_request("o")).unwrap();
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    let o = send(dir.path(), "f/o.yml", c).await;
    assert!(refused(&o), "{:?}", o.error);
    assert_eq!(changed(&seen).len(), 1);
    assert_eq!(paths(&p).await, ["/a"]);
}

#[tokio::test]
async fn a_token_panel_fetch_through_a_shadowed_proxy_asks() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    fs::write(dir.path().join("f/o.yml"), oauth2_request("o")).unwrap();
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    let ctx = ctx_of(dir.path());
    let request = RequestFile::load(dir.path().join("f/o.yml")).unwrap();
    let fetched = exec_with(c)
        .fetch_token(
            OAuth2Target::Request(&request),
            &ctx,
            &AuthorizeOptions::new(),
            &CancelHandle::new(),
        )
        .await;
    assert!(fetched.is_err());
    assert_eq!(changed(&seen).len(), 1);
    assert_eq!(paths(&p).await, ["/a"]);
}

#[tokio::test]
async fn a_schema_load_through_a_shadowed_proxy_asks() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let g = "info:\n  name: g\n  type: graphql\n  seq: 6\ngraphql:\n  url: \"http://example.test/graphql\"\n  method: POST\n";
    fs::write(dir.path().join("f/g.yml"), g).unwrap();
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    let ctx = ctx_of(dir.path());
    let request = RequestFile::load(dir.path().join("f/g.yml")).unwrap();
    let resolved = introspection_request(&request, &ctx).unwrap();
    let loaded = fetch_schema(&exec_with(c), &resolved, &CancelHandle::new()).await;
    assert!(loaded.is_err());
    assert_eq!(changed(&seen).len(), 1);
    assert_eq!(paths(&p).await, ["/a"]);
}

#[tokio::test]
async fn a_secret_through_a_folder_variable_reaches_no_record_question_or_log() {
    const SECRET: &str = "localhost";
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n{}",
        vars("ph", "{{sec}}")
    );
    fs::write(dir.path().join("f/folder.yml"), folder).unwrap();
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Send);
    let log = SessionLog::new();
    let exec = Arc::new(
        Executor::new(ExecOptions::new().network_check(c).session_log(log.clone())).unwrap(),
    );
    let col = Collection::open(dir.path()).unwrap();
    let ctx = VarContext::new(
        &col,
        ContextOptions::new()
            .env("dev")
            .sensitive_env_var("sec", SECRET),
    )
    .unwrap();
    send_in(dir.path(), &ctx, "a.yml", &exec, &RunOptions::new()).await;
    let b = send_in(dir.path(), &ctx, "f/b.yml", &exec, &RunOptions::new()).await;
    assert!(b.error.is_none(), "{:?}", b.error);
    assert_eq!(paths(&p).await, ["/a", "/b"]);
    let asked = changed(&seen);
    assert_eq!(asked.len(), 1);
    assert!(!asked.join("|").contains(SECRET), "{asked:?}");
    let bytes = fs::read_to_string(&store).unwrap();
    assert!(!bytes.contains(SECRET), "{bytes}");
    assert_eq!(log.network_since(0).entries.len(), 2);
    let entries = format!(
        "{:?}{:?}",
        log.network_since(0).entries,
        log.console_since(0).entries
    );
    assert!(!entries.contains(SECRET), "{entries}");
}

fn root_with_bypass(port: u16, bypass: &str) -> String {
    format!(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: 127.0.0.1\n      port: \"{port}\"\n      bypassProxy: \"{bypass}\"\n"
    )
}

fn folder_with(name: &str, value: &str) -> String {
    format!(
        "info:\n  name: f\n  type: folder\nrequest:\n{}",
        vars(name, value)
    )
}

#[tokio::test]
async fn a_dynamic_token_does_not_hide_the_other_variables_of_its_field() {
    let p = proxy().await;
    let dir = write_tree(&[
        (
            "opencollection.yml",
            &root_with_bypass(p.address().port(), "{{bp}} {{$randomBoolean}}"),
        ),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: bp\n    value: nothing.invalid\n",
        ),
        ("f/folder.yml", &folder_with("bp", "127.0.0.1")),
        ("a.yml", &get("a", 1, "http://example.test/a", "")),
        ("f/b.yml", &get("b", 2, "http://127.0.0.1:9/b", "")),
    ]);
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    send(dir.path(), "a.yml", c.clone()).await;
    assert!(changed(&seen).is_empty(), "{:?}", seen.lock().unwrap());
    let b = send(dir.path(), "f/b.yml", c).await;
    assert!(refused(&b), "{:?}", b.error);
    assert_eq!(changed(&seen).len(), 1);
}

#[tokio::test]
async fn an_approved_send_never_covers_a_later_collection_change() {
    let p = proxy().await;
    let dir = shadowed(p.address().port());
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Send);
    send(dir.path(), "a.yml", c.clone()).await;
    send(dir.path(), "f/b.yml", c.clone()).await;
    assert_eq!(changed(&seen).len(), 1);
    // An outside edit moves the environment's proxy to the one b's approval named.
    fs::write(
        dir.path().join("environments/dev.yml"),
        "name: dev\nvariables:\n  - name: ph\n    value: localhost\n",
    )
    .unwrap();
    send(dir.path(), "a.yml", c.clone()).await;
    assert_eq!(changed(&seen).len(), 2, "{:?}", seen.lock().unwrap());
    // The new state was acknowledged, so approvals made under the old one are gone.
    fs::write(
        dir.path().join("f/folder.yml"),
        folder_with("ph", "127.0.0.1"),
    )
    .unwrap();
    send(dir.path(), "f/b.yml", c).await;
    assert_eq!(changed(&seen).len(), 3, "{:?}", seen.lock().unwrap());
}

#[tokio::test]
async fn a_field_of_dynamic_tokens_only_stays_the_same_across_sends() {
    let p = proxy().await;
    let dir = write_tree(&[
        (
            "opencollection.yml",
            &root_with_bypass(p.address().port(), "{{$randomBoolean}}"),
        ),
        ("environments/dev.yml", ENV),
        ("a.yml", &get("a", 1, "http://example.test/a", "")),
    ]);
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    for _ in 0..3 {
        let a = send(dir.path(), "a.yml", c.clone()).await;
        assert!(a.error.is_none(), "{:?}", a.error);
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
}

#[tokio::test]
async fn a_dollar_name_that_is_not_generated_is_a_variable_and_its_shadow_asks() {
    let p = proxy().await;
    let dir = write_tree(&[
        (
            "opencollection.yml",
            &root_with_bypass(p.address().port(), "{{$bp}}"),
        ),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: \"$bp\"\n    value: nothing.invalid\n",
        ),
        ("f/folder.yml", &folder_with("$bp", "127.0.0.1")),
        ("a.yml", &get("a", 1, "http://example.test/a", "")),
        ("f/b.yml", &get("b", 2, "http://127.0.0.1:9/b", "")),
    ]);
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    send(dir.path(), "a.yml", c.clone()).await;
    let b = send(dir.path(), "f/b.yml", c).await;
    assert!(refused(&b), "{:?}", b.error);
    assert_eq!(changed(&seen).len(), 1, "{:?}", seen.lock().unwrap());
}

#[tokio::test]
async fn a_generated_value_that_forms_a_placeholder_is_asked_about_every_time() {
    let p = proxy().await;
    let folder = format!(
        "info:\n  name: f\n  type: folder\nrequest:\n{}    - name: \"false\"\n      value: \"127.0.0.1\"\n",
        vars("true", "127.0.0.1")
    );
    let dir = write_tree(&[
        (
            "opencollection.yml",
            &root_with_bypass(p.address().port(), "{{{{$randomBoolean}}}}"),
        ),
        ("environments/dev.yml", ENV),
        ("f/folder.yml", &folder),
        ("f/b.yml", &get("b", 2, "http://example.test/b", "")),
    ]);
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    for _ in 0..2 {
        let b = send(dir.path(), "f/b.yml", c.clone()).await;
        assert!(refused(&b), "{:?}", b.error);
    }
    let asked = changed(&seen);
    assert_eq!(asked.len(), 2, "{:?}", seen.lock().unwrap());
    assert!(asked[0].contains("could not be verified"), "{asked:?}");
    assert!(paths(&p).await.is_empty());
}

#[tokio::test]
async fn a_random_int_field_passes_silently_once_acknowledged() {
    let p = proxy().await;
    let dir = write_tree(&[
        (
            "opencollection.yml",
            &root_with_bypass(p.address().port(), "{{$randomInt}}"),
        ),
        ("environments/dev.yml", ENV),
        ("a.yml", &get("a", 1, "http://example.test/a", "")),
    ]);
    let store = dir.path().join("ack.json");
    let (c, seen) = check(&store, Send, Cancel);
    for _ in 0..3 {
        let a = send(dir.path(), "a.yml", c.clone()).await;
        assert!(a.error.is_none(), "{:?}", a.error);
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
}
