//! The public assertion entry point over a collection on disk.

use crate::{common, script_support};

use apinata_core::{
    Collection, ContextOptions, ExecOptions, ItemPath, Outcome, RecheckRefused, RequestFile,
    RequestResult, Response, RunOptions, VarContext, Verdict, evaluate, recheck_assertions,
};
use common::{request, write_tree};
use script_support::{on, script, send_full};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const OPENING: &str = "opencollection.yml";
const ROOT_PLAIN: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: code\n      value: \"201\"\n";
const FOLDER: &str = "info:\n  name: a\n  type: folder\nrequest:\n  variables:\n    - name: kind\n      value: user\n";
const REQ: &str = concat!(
    "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test\n",
    "runtime:\n  variables:\n    - name: who\n      value: ann\n  assertions:\n",
    "    - expression: res.status\n      operator: eq\n      value: \"{{code}}\"\n",
    "    - expression: res.body.kind\n      operator: eq\n      value: \"{{kind}}\"\n",
    "    - expression: res.body.name\n      operator: in\n      value: \"[bob, {{who}}]\"\n",
    "    - expression: res.headers['x-id']\n      operator: eq\n      value: \"'7'\"\n",
    "    - expression: res.body.name\n      operator: eq\n      value: \"{{nope}}\"\n",
    "    - expression: res.url\n      operator: isDefined\n",
    "    - expression: res.status\n      operator: eq\n      value: \"500\"\n      disabled: true\n",
);

#[test]
fn evaluate_interpolates_collection_folder_and_request_variables() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a/folder.yml", FOLDER),
        ("a/r.yml", REQ),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let res = Response::new(201)
        .with_header("X-Id", "7")
        .with_text_body("{\"kind\":\"user\",\"name\":\"ann\"}");
    let outcomes = evaluate(request(&c, "a/r.yml"), &res, &ctx);
    // Verdict variants are non-exhaustive, so each outcome is compared through a plain shape.
    let shapes: Vec<_> = outcomes
        .iter()
        .map(|o| match o.verdict() {
            Verdict::Pass => (o.assertion().expression(), "pass", None, None),
            Verdict::Fail { actual, detail, .. } => (
                o.assertion().expression(),
                "fail",
                Some(actual.clone()),
                detail.clone(),
            ),
            Verdict::Skipped { reason, .. } => (
                o.assertion().expression(),
                "skipped",
                Some(reason.clone()),
                None,
            ),
            other => panic!("unexpected verdict {other:?}"),
        })
        .collect();
    let undefined = Some("variable `{{nope}}` is undefined".to_owned());
    let skip = Some("left side `res.url` needs a JavaScript runtime".to_owned());
    assert_eq!(
        shapes,
        [
            ("res.status", "pass", None, None),
            ("res.body.kind", "pass", None, None),
            ("res.body.name", "pass", None, None),
            ("res.headers['x-id']", "pass", None, None),
            (
                "res.body.name",
                "fail",
                Some("\"ann\"".to_owned()),
                undefined
            ),
            ("res.url", "skipped", skip, None),
        ]
    );
}

#[test]
fn a_truncated_body_skips_body_assertions_but_not_status_or_headers() {
    let req = concat!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test\n",
        "runtime:\n  assertions:\n",
        "    - expression: res.status\n      operator: eq\n      value: \"200\"\n",
        "    - expression: res.headers['x-id']\n      operator: eq\n      value: \"'7'\"\n",
        "    - expression: res.body.kind\n      operator: eq\n      value: user\n",
        "    - expression: res.body\n      operator: isDefined\n",
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", req)]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let res = Response::new(200)
        .with_header("x-id", "7")
        .with_text_body("{\"kind\":\"us")
        .with_truncated(4096);
    assert!(res.truncated());
    assert_eq!(res.size(), 4096);
    let verdicts: Vec<_> = evaluate(request(&c, "r.yml"), &res, &ctx)
        .iter()
        .map(|o| match o.verdict() {
            Verdict::Pass => "pass".to_owned(),
            Verdict::Skipped { reason, .. } => reason.clone(),
            other => panic!("unexpected verdict {other:?}"),
        })
        .collect();
    assert_eq!(
        verdicts,
        [
            "pass",
            "pass",
            "the response body was truncated at the size limit, so `res.body.kind` was not evaluated",
            "the response body was truncated at the size limit, so `res.body` was not evaluated",
        ]
    );
}

#[test]
fn a_decoded_response_hides_its_coding_from_assertions_but_keeps_the_length() {
    let req = concat!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test\n",
        "runtime:\n  assertions:\n",
        "    - expression: res.headers['content-encoding']\n      operator: isUndefined\n",
        "    - expression: res.headers['content-length']\n      operator: eq\n      value: \"'25'\"\n",
    );
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        ),
        ("r.yml", req),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let decoded = Response::new(200)
        .with_header("content-encoding", "gzip")
        .with_header("content-length", "25")
        .with_text_body("hello")
        .with_encoded_size(25);
    let passed = |res: &Response| {
        evaluate(request(&c, "r.yml"), res, &ctx)
            .iter()
            .map(|o| matches!(o.verdict(), Verdict::Pass))
            .collect::<Vec<_>>()
    };
    assert_eq!(passed(&decoded), [true, true]);
    // The response itself still carries the header, as the response pane shows it.
    assert!(
        decoded
            .headers()
            .iter()
            .any(|(n, _)| n == "content-encoding")
    );
    // A body kept as sent is still coded, so its coding stays in view.
    let raw = Response::new(200)
        .with_header("content-encoding", "gzip")
        .with_header("content-length", "25")
        .with_body(vec![0x1f, 0x8b], "");
    assert_eq!(passed(&raw), [false, true]);
}

/// A GET to `url` whose assertions are `assertions` and whose scripts are `scripts`.
fn rechecked(url: &str, assertions: &str, scripts: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  scripts:\n{scripts}  assertions:\n{assertions}"
    )
}

const CHECKS: &str = concat!(
    "    - expression: res.status\n      operator: eq\n      value: \"200\"\n",
    "    - expression: res.body.n\n      operator: eq\n      value: \"2\"\n",
    "    - expression: res.headers['x-id']\n      operator: eq\n      value: \"'7'\"\n",
    "    - expression: res.url\n      operator: isDefined\n",
);

async fn json_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-id", "7")
                .set_body_string("{\"n\":1}"),
        )
        .expect(1)
        .mount(&s)
        .await;
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recheck_reaches_the_verdicts_the_send_did() {
    let s = json_server().await;
    let text = rechecked(&format!("{}/r", s.uri()), CHECKS, "    []\n");
    let dir = script_support::write_tree(&[(OPENING, ROOT_PLAIN), ("r.yml", &text)]);
    let (result, ctx) = send_full(
        dir.path(),
        "r.yml",
        RunOptions::new(),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert_eq!(result.assertions.len(), 4, "{:?}", result.error);
    let request = RequestFile::load(dir.path().join("r.yml")).expect("loads");
    let again = recheck_assertions(&request, &result, &ctx).expect("rechecks");
    assert_eq!(again, result.assertions);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_edited_assertion_gets_a_new_verdict_on_recheck() {
    let s = json_server().await;
    let url = format!("{}/r", s.uri());
    let dir = script_support::write_tree(&[
        (OPENING, ROOT_PLAIN),
        ("r.yml", &rechecked(&url, CHECKS, "    []\n")),
    ]);
    let (result, ctx) = send_full(
        dir.path(),
        "r.yml",
        RunOptions::new(),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert!(matches!(
        result.assertions[1].verdict(),
        Verdict::Fail { .. }
    ));
    let edited = CHECKS.replace("value: \"2\"", "value: \"1\"");
    std::fs::write(
        dir.path().join("r.yml"),
        rechecked(&url, &edited, "    []\n"),
    )
    .expect("edit");
    let request = RequestFile::load(dir.path().join("r.yml")).expect("loads");
    let again = recheck_assertions(&request, &result, &ctx).expect("rechecks");
    assert_eq!(again[1].assertion().value(), Some("1"));
    assert!(matches!(again[1].verdict(), Verdict::Pass), "{again:?}");
    assert!(matches!(again[0].verdict(), Verdict::Pass));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_a_post_response_script_changed_refuses_a_recheck() {
    let s = json_server().await;
    let scripts = script("after-response", "res.setBody({n: 2});");
    let text = rechecked(&format!("{}/r", s.uri()), CHECKS, &scripts);
    let dir = script_support::write_tree(&[(OPENING, ROOT_PLAIN), ("r.yml", &text)]);
    let (result, ctx) = send_full(
        dir.path(),
        "r.yml",
        on(),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert!(result.body_edited, "{:?}", result.error);
    assert!(matches!(result.assertions[1].verdict(), Verdict::Pass));
    let request = RequestFile::load(dir.path().join("r.yml")).expect("loads");
    let refused = recheck_assertions(&request, &result, &ctx).expect_err("refused");
    assert_eq!(refused, RecheckRefused::BodyEdited);
    assert!(
        refused.to_string().contains("post-response script"),
        "{refused}"
    );
}

#[test]
fn a_kept_body_with_a_secret_scrubbed_refuses_a_recheck() {
    let dir = write_tree(&[
        (OPENING, ROOT_PLAIN),
        ("r.yml", &rechecked("https://x.test/r", CHECKS, "    []\n")),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let response = Response::new(200).with_text_body("{\"n\":\"[redacted]\"}");
    let mut kept = RequestResult::from_parts(
        ItemPath::new("r.yml").expect("path"),
        "r",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    );
    let request = request(&c, "r.yml");
    assert_eq!(
        recheck_assertions(request, &kept, &ctx).map(|v| v.len()),
        Ok(4)
    );
    kept.response_scrubbed = true;
    let refused = recheck_assertions(request, &kept, &ctx).expect_err("refused");
    assert_eq!(refused, RecheckRefused::ResponseScrubbed);
    assert!(refused.to_string().contains("secret"), "{refused}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_then_a_recheck_hits_the_server_once() {
    let s = json_server().await;
    let text = rechecked(&format!("{}/r", s.uri()), CHECKS, "    []\n");
    let dir = script_support::write_tree(&[(OPENING, ROOT_PLAIN), ("r.yml", &text)]);
    let (result, ctx) = send_full(
        dir.path(),
        "r.yml",
        on(),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    let request = RequestFile::load(dir.path().join("r.yml")).expect("loads");
    let again = recheck_assertions(&request, &result, &ctx).expect("rechecks");
    assert!(matches!(again[0].verdict(), Verdict::Pass), "{again:?}");
    let hits = s.received_requests().await.expect("recorded");
    assert_eq!(hits.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recheck_skips_left_sides_that_need_the_send_s_scripts() {
    let s = json_server().await;
    let checks =
        "    - expression: res.getHeader('x-id')\n      operator: eq\n      value: \"'7'\"\n";
    let text = rechecked(&format!("{}/r", s.uri()), checks, "    []\n");
    let dir = script_support::write_tree(&[(OPENING, ROOT_PLAIN), ("r.yml", &text)]);
    let (result, ctx) = send_full(
        dir.path(),
        "r.yml",
        on(),
        ContextOptions::new(),
        ExecOptions::new(),
    )
    .await;
    assert!(
        matches!(result.assertions[0].verdict(), Verdict::Pass),
        "{:?}",
        result.assertions
    );
    let request = RequestFile::load(dir.path().join("r.yml")).expect("loads");
    let again = recheck_assertions(&request, &result, &ctx).expect("rechecks");
    assert!(
        matches!(again[0].verdict(), Verdict::Skipped { .. }),
        "{again:?}"
    );
}

#[test]
fn a_result_without_a_response_refuses_a_recheck() {
    let dir = write_tree(&[
        (OPENING, ROOT_PLAIN),
        ("r.yml", &rechecked("https://x.test/r", CHECKS, "    []\n")),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let kept = RequestResult::from_parts(
        ItemPath::new("r.yml").expect("path"),
        "r",
        Outcome::Errored,
        None,
        Vec::new(),
    );
    let refused = recheck_assertions(request(&c, "r.yml"), &kept, &ctx).expect_err("refused");
    assert_eq!(refused, RecheckRefused::NoResponse);
}

const DROPPED_CHECKS: &str = concat!(
    "    - expression: res.status\n      operator: eq\n      value: \"200\"\n",
    "    - expression: res.body.n\n      operator: eq\n      value: \"2\"\n",
);

// A result rebuilt from a save of `response`, as a front end or the MCP server would hold it.
fn kept(response: Response) -> RequestResult {
    RequestResult::from_parts(
        ItemPath::new("r.yml").expect("path"),
        "r",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    )
}

#[test]
fn a_dropped_body_skips_body_assertions_on_recheck() {
    let dir = write_tree(&[
        (OPENING, ROOT_PLAIN),
        (
            "r.yml",
            &rechecked("https://x.test/r", DROPPED_CHECKS, "    []\n"),
        ),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let mut result = kept(Response::new(200).with_text_body("").with_size(9_000_000));
    result.body_dropped = true;
    let again = recheck_assertions(request(&c, "r.yml"), &result, &ctx).expect("rechecks");
    assert!(matches!(again[0].verdict(), Verdict::Pass), "{again:?}");
    // Skipped as truncated, the reason that holds for any body that was never kept.
    match again[1].verdict() {
        Verdict::Skipped { reason, .. } => assert!(reason.contains("truncated"), "{reason}"),
        other => panic!("expected a skip, got {other:?}"),
    }
}

#[test]
fn a_response_saved_without_marks_refuses_a_recheck() {
    let dir = write_tree(&[
        (OPENING, ROOT_PLAIN),
        (
            "r.yml",
            &rechecked("https://x.test/r", DROPPED_CHECKS, "    []\n"),
        ),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let mut result = kept(Response::new(200).with_text_body("{\"n\":2}"));
    result.saved_unmarked = true;
    let refused = recheck_assertions(request(&c, "r.yml"), &result, &ctx).expect_err("refused");
    assert_eq!(refused, RecheckRefused::SavedUnmarked);
    assert!(
        refused
            .to_string()
            .contains("Send again to re-check this response"),
        "{refused}"
    );
}
