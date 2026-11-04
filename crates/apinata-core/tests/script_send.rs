//! Script sends go through the run's executor, its guard and the host policy.

use crate::script_support;

use apinata_core::{ContextOptions, ExecError, ExecOptions, Outcome, ResolvedRequest};
use script_support::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wiremock::matchers::{body_string, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tree(url: &str, kind: &str, code: &str) -> tempfile::TempDir {
    let text = scripted("R", url, &script(kind, code), "");
    write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pre_request_send_feeds_a_variable_into_the_request() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(header("content-type", "application/json"))
        .and(header("x-raw", "{{tok}}"))
        .and(body_string(r#"{"user":"u"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "t1"})))
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(path("/api/t1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let code = format!(
        r#"const r = await bru.sendRequest({{ method: "post", url: "{}/token", headers: {{ "x-raw": "{{{{tok}}}}" }}, data: {{ user: "u" }} }});
console.log(r.status, r.statusText, typeof r.headers, r.data.token);
bru.setVar("tok", r.data.token);"#,
        s.uri()
    );
    let dir = tree(
        &format!("{}/api/{{{{tok}}}}", s.uri()),
        "before-request",
        &code,
    );
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:200 OK object t1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_host_policy_refuses_script_sends() {
    let s = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&s)
        .await;
    // Loopback hosts pass any policy, so the script aims at a TEST-NET address, refused before I/O.
    let code = r#"try { await bru.sendRequest("http://192.0.2.1/x"); } catch (e) { console.log(e.message); }"#;
    let dir = tree(&format!("{}/r", s.uri()), "before-request", code);
    let exec = ExecOptions::new().host_policy(Arc::new(|_: &str| false));
    let (result, _) = send_full(dir.path(), "r.yml", on(), ContextOptions::new(), exec).await;
    let lines = console(&result);
    assert!(
        lines[0].starts_with("log:host `") && lines[0].contains("is not allowed"),
        "{lines:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_in_a_script_send_host_is_refused() {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let code = format!(
        r#"const h = bru.getEnvVar("h");
try {{ await bru.sendRequest("http://" + h + ":{}/x"); }} catch (e) {{ console.log(e.message); }}"#,
        s.address().port()
    );
    let dir = tree(&format!("{}/r", s.uri()), "before-request", &code);
    let ctx = ContextOptions::new().sensitive_env_var("h", "127.0.0.1");
    let exec = ExecOptions::new().host_policy(Arc::new(|_: &str| true));
    let (result, _) = send_full(dir.path(), "r.yml", on(), ctx, exec).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let lines = console(&result);
    assert!(
        lines[0].contains("a secret cannot be used in a host"),
        "{lines:?}"
    );
    assert!(!lines[0].contains("127.0.0.1"), "{lines:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_run_guard_refuses_script_sends() {
    let s = MockServer::start().await;
    Mock::given(path("/blocked"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&s)
        .await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let code = format!(
        r#"try {{ await bru.sendRequest("{}/blocked"); }} catch (e) {{ console.log(e.message); }}"#,
        s.uri()
    );
    let dir = tree(&format!("{}/ok", s.uri()), "before-request", &code);
    let guard = Arc::new(|r: &ResolvedRequest| {
        r.url()
            .contains("/blocked")
            .then(|| "refused by test".to_owned())
    });
    let result = send(dir.path(), "r.yml", on().guard(guard)).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:refused by test"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unawaited_send_finishes_before_the_phase_ends() {
    let s = MockServer::start().await;
    Mock::given(path("/late"))
        .respond_with(ResponseTemplate::new(201).set_delay(Duration::from_millis(50)))
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(path("/after/201"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let code = format!(
        r#"bru.sendRequest("{}/late").then((r) => bru.setVar("late", String(r.status)));"#,
        s.uri()
    );
    let dir = tree(
        &format!("{}/after/{{{{late}}}}", s.uri()),
        "before-request",
        &code,
    );
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_send_stops_at_the_phase_time_limit() {
    let s = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&s)
        .await;
    let code = format!(r#"await bru.sendRequest("{}/slow");"#, s.uri());
    let dir = tree(&format!("{}/r", s.uri()), "before-request", &code);
    let start = Instant::now();
    let result = send(dir.path(), "r.yml", limited(300)).await;
    assert!(
        start.elapsed() < Duration::from_millis(1_500),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(result.outcome, Outcome::Errored);
    assert!(script_error(&result).contains("script exceeded the 0.3 s limit"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn script_sends_share_the_executor_cookie_jar() {
    let s = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", "sid=abc; Path=/"))
        .mount(&s)
        .await;
    Mock::given(path("/me"))
        .and(header("cookie", "sid=abc"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let code = format!(r#"await bru.sendRequest("{}/login");"#, s.uri());
    let dir = tree(&format!("{}/me", s.uri()), "before-request", &code);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_send_attaches_no_oauth2_token() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "tok", "token_type": "Bearer", "expires_in": 3600
        })))
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(path("/api"))
        .and(header("authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(path("/side"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    // The send runs after the response, when the request's token is fetched and cached.
    let p = s.uri();
    let code = format!(r#"await bru.sendRequest("{p}/side");"#);
    let text = format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{p}/api\"\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{p}/token\"\n    credentials:\n      clientId: app\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\nruntime:\n  scripts:\n{}",
        script("after-response", &code)
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let received = s.received_requests().await.expect("recording");
    let side: Vec<_> = received
        .iter()
        .filter(|r| r.url.path() == "/side")
        .collect();
    assert_eq!(side.len(), 1);
    assert!(
        !side[0].headers.contains_key("authorization"),
        "{:?}",
        side[0].headers
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn axios_and_callbacks_follow_axios_rejections() {
    let s = MockServer::start().await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
        .mount(&s)
        .await;
    Mock::given(path("/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_string("nope"))
        .mount(&s)
        .await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let code = format!(
        r#"const u = "{}";
const a = await axios.get(u + "/a");
let e1; try {{ await axios.post(u + "/missing", "x"); }} catch (e) {{ e1 = e; }}
const viaCb = await bru.sendRequest({{ url: u + "/missing" }}, (err, res) => {{ console.log("cb", err.message, err.status, err.response.status, res); }});
console.log(a.status, a.data, e1.message, e1.code, e1.response.status, e1.response.data, viaCb, require("axios") === axios);"#,
        s.uri()
    );
    let dir = tree(&format!("{}/r", s.uri()), "after-response", &code);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            "log:cb Request failed with status code 404 404 404 null",
            "log:200 hello Request failed with status code 404 ERR_BAD_REQUEST 404 nope undefined true",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_huge_timeout_still_sends() {
    let s = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let code = format!(
        r#"const r = await bru.sendRequest({{ url: "{}/x", timeout: 1e300 }});
console.log(r.status);"#,
        s.uri()
    );
    let dir = tree(&format!("{}/r", s.uri()), "before-request", &code);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:200"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_host_policy_refuses_a_script_send_redirect_hop() {
    let s = MockServer::start().await;
    Mock::given(path("/hop"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "http://192.0.2.1/x"))
        .expect(1)
        .mount(&s)
        .await;
    let code = format!(
        r#"try {{ await bru.sendRequest("{}/hop"); console.log("sent"); }} catch (e) {{ console.log(e.message); }}"#,
        s.uri()
    );
    let dir = tree(&format!("{}/r", s.uri()), "before-request", &code);
    let exec = ExecOptions::new().host_policy(Arc::new(|_: &str| false));
    let start = Instant::now();
    let (result, _) = send_full(dir.path(), "r.yml", on(), ContextOptions::new(), exec).await;
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(
        console(&result),
        ["log:host `192.0.2.1` is not allowed (hop 1)"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_script_send_is_reported_on_the_result() {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    // Two refused sends to one host are reported once; the main request is local and sent.
    let code = r#"for (const _ of [1, 2]) {
  try { await bru.sendRequest({url: 'http://api.example.com/x'}); } catch (e) {}
}"#;
    let dir = tree(&format!("{}/r", s.uri()), "before-request", code);
    let policy = |host: &str| host == "127.0.0.1";
    let exec = ExecOptions::new().host_policy(Arc::new(policy));
    let (result, _) = send_full(dir.path(), "r.yml", on(), ContextOptions::new(), exec).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    let refusals = result.script_refusals();
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert!(
        matches!(&refusals[0], ExecError::HostNotAllowed { host, .. } if host == "api.example.com"),
        "{refusals:?}"
    );
}

/// A `pm.sendRequest` from an imported Postman script runs on the app's `bru.sendRequest`:
/// the header array arrives as headers, the raw body as data, and `res.json()` as `res.data`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_imported_pm_send_request_runs_and_feeds_a_variable() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(header("x-a", "1"))
        .and(body_string(r#"{"user":"u"}"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "t2"})))
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(path("/api/t2"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let exec = format!(
        "pm.sendRequest({{url: '{}/token', method: 'POST', header: [{{key: 'X-A', value: '1'}}], body: {{mode: 'raw', raw: JSON.stringify({{user: 'u'}})}}}}, function (err, res) {{\n  console.log(res.code);\n  pm.variables.set('tok', res.json().token);\n}});",
        s.uri()
    );
    let collection = serde_json::json!({
        "info": {"name": "c", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
        "item": [{"name": "r",
            "event": [{"listen": "prerequest", "script": {"exec": [exec]}}],
            "request": {"method": "GET", "url": "x"}}]
    });
    let imported = apinata_core::import::parse_postman(collection.to_string().as_bytes()).unwrap();
    let apinata_core::import::ImportedItem::Request(r) = &imported.items[0] else {
        panic!("request")
    };
    let code = r.request.scripts.pre_request.clone();
    assert!(code.starts_with("await bru.sendRequest("), "{code}");
    let dir = tree(
        &format!("{}/api/{{{{tok}}}}", s.uri()),
        "before-request",
        &code,
    );
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:200"]);
}
