//! req edits reach the wire; res reads the response and setBody feeds later steps.

use crate::script_support;

use apinata_core::{ContextOptions, ExecOptions, ExecutionMode, Outcome, Verdict};
use script_support::*;
use wiremock::matchers::{body_string, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pre_request_script_reshapes_the_request() {
    let s = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/signed/abc"))
        .and(header("x-sig", "abc-1"))
        .and(query_param("q", "1"))
        .and(body_string("{\"n\":2}"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let code = r#"bru.setVar("t", "abc");
req.setMethod("PUT");
req.setUrl(req.getUrl().replace("/start", "/signed/{{t}}?q=1"));
req.setHeader("X-Sig", "{{t}}-1");
req.deleteHeader("X-Old");
const b = req.getBody(); b.n = b.n + 1; req.setBody(b);
console.log(req.getMethod(), req.method, req.getHeader("X-Sig"), typeof req.getBody());"#;
    let text = format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"{}/start\"\n  headers:\n    - name: X-Old\n      value: o\n  body:\n    type: json\n    data: '{{\"n\":1}}'\nruntime:\n  scripts:\n{}",
        s.uri(),
        script("before-request", code)
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:PUT POST {{t}}-1 object"]);
    let received = s.received_requests().await.expect("recorded");
    assert!(received[0].headers.get("x-old").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req_getters_report_the_prepared_request() {
    let s = MockServer::start().await;
    Mock::given(path("/p/7"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let code = format!(
        "console.log(JSON.stringify([req.getName(), req.getHost() === '{}', req.getPath(), req.getQueryString(), req.getAuthMode(), req.getPathParams(), req.getTags(), req.getTimeout(), req.getExecutionMode(), req.getUrl() === req.url]))",
        s.address()
    );
    let text = format!(
        "info:\n  name: R\n  type: http\n  seq: 1\n  tags:\n    - a\n    - b\nhttp:\n  method: GET\n  url: \"{}/p/:id?x=1\"\n  params:\n    - name: id\n      value: \"7\"\n      type: path\n  auth:\n    type: bearer\n    token: tk\nruntime:\n  scripts:\n{}settings:\n  timeout: 2500\n",
        s.uri(),
        script("before-request", &code)
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on().execution_mode(ExecutionMode::Cli)).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(
        console(&result),
        [
            r#"log:["R",true,"/p/7","x=1","bearer",[{"name":"id","value":"7","type":"path"}],["a","b"],2500,"cli",true]"#
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn res_exposes_the_response_and_set_body_feeds_later_steps() {
    let s = MockServer::start().await;
    let body = r#"{"items":[1,2,3],"name":"n"}"#;
    Mock::given(path("/r"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("X-Up", "v")
                .set_body_string(body),
        )
        .mount(&s)
        .await;
    let post = r#"console.log(res.status, res.statusText, res.getHeader("X-UP"), res.headers["x-up"], res.body.items.length, typeof res.responseTime, res.getUrl() === res.url, res.getSize().body);
res.setBody({name: "changed"});"#;
    let tests_code = r#"test("sees new body", () => { if (res.getBody().name !== "changed") throw new Error(res.getBody().name) });"#;
    let scripts = script("after-response", post) + &script("tests", tests_code);
    let assertion = "  assertions:\n    - expression: res.body.name\n      operator: eq\n      value: changed\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/r", s.uri()), &scripts, assertion),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(
        result.outcome,
        Outcome::Passed,
        "{:?} {:?}",
        result.error,
        tests(&result)
    );
    assert_eq!(
        console(&result),
        [format!("log:201 Created v v 3 number true {}", body.len())]
    );
    assert_eq!(tests(&result), ["tests sees new body: pass"]);
    assert!(matches!(result.assertions[0].verdict(), Verdict::Pass));
    assert!(
        result
            .response
            .as_ref()
            .expect("response")
            .text()
            .contains("items")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_truncated_body_throws_as_unsupported() {
    let s = MockServer::start().await;
    Mock::given(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(4096)))
        .mount(&s)
        .await;
    let scripts = script(
        "tests",
        "test('status', () => { if (res.status !== 200) throw new Error('s') });\ntest('body', () => res.body)",
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/big", s.uri()), &scripts, ""),
        ),
    ]);
    let exec = ExecOptions::new().max_body_bytes(Some(16));
    let (result, _) = send_full(dir.path(), "r.yml", on(), ContextOptions::new(), exec).await;
    assert_eq!(
        tests(&result),
        [
            "tests status: pass",
            "tests body: skipped the response body was truncated"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_body_with_nothing_sends_an_empty_body() {
    for value in ["undefined", "null"] {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string(""))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&s)
            .await;
        let code = format!("req.setBody({value});");
        let text = format!(
            "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"{}/x\"\n  body:\n    type: text\n    data: hello\nruntime:\n  scripts:\n{}",
            s.uri(),
            script("before-request", &code)
        );
        let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
        let result = send(dir.path(), "r.yml", on()).await;
        assert_eq!(
            result.outcome,
            Outcome::Passed,
            "{value}: {:?}",
            result.error
        );
        s.verify().await;
    }
}
