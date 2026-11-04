//! The static list of texts a send needs approved, and the runtime gate that runs only those.

use crate::script_support;

use apinata_core::{
    Collection, ContextOptions, ExecOptions, Outcome, RequestFile, RunError, RunOptions,
    RuntimeVars, ScriptOptions, ScriptText, ScriptTextKind, VarContext, VarResult, Verdict,
    plan_script_texts, plan_script_texts_across, script_hash, script_texts,
};
use script_support::*;
use std::collections::BTreeSet;
use std::path::Path;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const PRE: &str = "bru.setVar('a', 1)";
const TESTS: &str = "test('ok', () => expect(res.status).to.equal(200))";

/// A request with a pre-request and a tests script, a `res` chain and a JavaScript variable,
/// and assertions with a `res` chain, a JavaScript left side and a template right side.
fn request_text(url: &str) -> String {
    format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}/ok\"\nruntime:\n  scripts:\n{}{}  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"res.body.id\"\n        method: jsonq\n      variable:\n        name: id\n        scope: runtime\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"res.status * 2\"\n        method: jsonq\n      variable:\n        name: twice\n        scope: runtime\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n    - expression: \"res.status + 1\"\n      operator: eq\n      value: \"201\"\n    - expression: res.status\n      operator: eq\n      value: \"`${{200}}`\"\n",
        script("before-request", PRE),
        script("tests", TESTS),
    )
}

fn texts_with(dir: &Path, rel: &str, options: ContextOptions) -> Vec<ScriptText> {
    let c = Collection::open(dir).unwrap();
    let ctx = VarContext::new(&c, options).unwrap();
    let req = RequestFile::load(dir.join(rel)).unwrap();
    script_texts(&req, &ctx)
}

fn kind(t: &ScriptText) -> String {
    match &t.kind {
        ScriptTextKind::Script { phase, level } => format!("{} {level}", phase.as_str()),
        ScriptTextKind::Variable { name } => format!("variable {name}"),
        ScriptTextKind::AssertionLeft => "assertion left".to_owned(),
        ScriptTextKind::AssertionRight => "assertion right".to_owned(),
        _ => "other".to_owned(),
    }
}

fn approved(hashes: impl IntoIterator<Item = String>) -> RunOptions {
    let set: BTreeSet<String> = hashes.into_iter().collect();
    RunOptions::new().scripts(ScriptOptions::new().enabled(true).only_approved(set))
}

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 7})))
        .mount(&s)
        .await;
    s
}

#[test]
fn texts_list_segments_and_javascript_expressions_with_their_hashes() {
    let dir = write_tree(&[
        (
            "opencollection.yml",
            &root(&script("before-request", "bru.setVar('c', 1)"), None),
        ),
        ("r.yml", &request_text("http://127.0.0.1:9")),
    ]);
    let found = texts_with(dir.path(), "r.yml", ContextOptions::new());
    let summary: Vec<(String, String)> = found.iter().map(|t| (kind(t), t.text.clone())).collect();
    let want: Vec<(String, String)> = [
        ("pre-request collection", "bru.setVar('c', 1)"),
        ("pre-request request", PRE),
        ("tests request", TESTS),
        ("variable twice", "res.status * 2"),
        ("assertion left", "res.status + 1"),
        ("assertion right", "`${200}`"),
    ]
    .into_iter()
    .map(|(k, t)| (k.to_owned(), t.to_owned()))
    .collect();
    assert_eq!(summary, want);
    for t in &found {
        assert_eq!(t.hash, script_hash(&t.text));
    }
}

#[test]
fn a_secret_in_a_text_is_shown_masked() {
    let code = "const k = 'S3cret-value-9'";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                "http://127.0.0.1:9",
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let options = ContextOptions::new().sensitive_env_var("tok", "S3cret-value-9");
    let found = texts_with(dir.path(), "r.yml", options);
    assert_eq!(found[0].text, code);
    assert_eq!(found[0].shown, "const k = '<secret>'");
}

#[test]
fn inherited_shell_values_are_shown_but_dotenv_values_are_masked() {
    let code = "bru.setVar('n', 1); const s = 'shell-val-7'; const d = 'dotenv-val-8'";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (".env", "TOKEN=dotenv-val-8\n"),
        (
            "r.yml",
            &scripted(
                "R",
                "http://127.0.0.1:9",
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let options = ContextOptions::new().process_env([("SHLVL", "1"), ("SHELL_ISH", "shell-val-7")]);
    let found = texts_with(dir.path(), "r.yml", options);
    assert_eq!(
        found[0].shown,
        "bru.setVar('n', 1); const s = 'shell-val-7'; const d = '<secret>'"
    );
}

#[test]
fn a_plan_lists_a_shared_collection_script_once() {
    let dir = write_tree(&[
        (
            "opencollection.yml",
            &root(&script("tests", "test('c', () => {})"), None),
        ),
        (
            "a.yml",
            &scripted(
                "A",
                "http://127.0.0.1:9",
                &script("tests", "test('a', () => {})"),
                "",
            ),
        ),
        (
            "b.yml",
            &scripted(
                "B",
                "http://127.0.0.1:9",
                &script("tests", "test('a', () => {})"),
                "",
            ),
        ),
    ]);
    let c = Collection::open(dir.path()).unwrap();
    let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
    let plan = c.plan(dir.path()).unwrap();
    let listed: Vec<(String, String)> = plan_script_texts(&plan, &ctx)
        .into_iter()
        .map(|(item, t)| (item.as_str().to_owned(), t.text))
        .collect();
    assert_eq!(
        listed,
        [
            ("a.yml".to_owned(), "test('a', () => {})".to_owned()),
            ("a.yml".to_owned(), "test('c', () => {})".to_owned()),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unapproved_pre_request_script_sends_nothing() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request_text(&s.uri())),
    ]);
    let result = send(dir.path(), "r.yml", approved([])).await;
    assert_eq!(result.outcome, Outcome::Errored);
    match &result.error {
        Some(RunError::Script { message, .. }) => {
            assert!(message.contains("not approved"), "{message}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approving_every_listed_text_runs_everything() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request_text(&s.uri())),
    ]);
    let hashes = texts_with(dir.path(), "r.yml", ContextOptions::new())
        .into_iter()
        .map(|t| t.hash);
    let result = send(dir.path(), "r.yml", approved(hashes)).await;
    assert_eq!(tests(&result), ["tests ok: pass"]);
    let twice = result
        .post_response_vars
        .iter()
        .find(|v| v.name() == "twice")
        .unwrap();
    assert_eq!(twice.result(), &VarResult::Set);
    for a in &result.assertions {
        if let Verdict::Skipped { reason, .. } = a.verdict() {
            assert!(!reason.contains("not approved"), "{reason}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unapproved_later_scripts_and_expressions_are_skipped_not_run() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request_text(&s.uri())),
    ]);
    let result = send(dir.path(), "r.yml", approved([script_hash(PRE)])).await;
    assert_eq!(result.response.as_ref().map(|r| r.status()), Some(200));
    assert_eq!(
        tests(&result),
        ["tests Test script: skipped the script is not approved"]
    );
    let twice = result
        .post_response_vars
        .iter()
        .find(|v| v.name() == "twice")
        .unwrap();
    assert!(
        matches!(twice.result(), VarResult::Skipped { reason, .. } if reason.contains("not approved"))
    );
    let id = result
        .post_response_vars
        .iter()
        .find(|v| v.name() == "id")
        .unwrap();
    assert_eq!(id.result(), &VarResult::Set);
    let skipped: Vec<&str> = result
        .assertions
        .iter()
        .filter_map(|a| match a.verdict() {
            Verdict::Skipped { reason, .. } => Some(reason.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(skipped.len(), 2, "{skipped:?}");
    assert!(
        skipped.iter().all(|r| r.contains("not approved")),
        "{skipped:?}"
    );
    assert_eq!(result.outcome, Outcome::Passed);
}

/// Sends an approved assertion `value` whose `{{x}}` holds `x`, and returns its verdict and
/// whether the injected `bru.setVar('h', ...)` ran.
async fn inject(value: &str, x: &str) -> (Verdict, bool) {
    let s = server().await;
    let extra = format!(
        "  assertions:\n    - expression: res.status\n      operator: eq\n      value: {}\n",
        serde_json::to_string(value).unwrap()
    );
    let req = format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/ok\"\nruntime:\n{extra}",
        s.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
    let texts = texts_with(dir.path(), "r.yml", ContextOptions::new());
    assert_eq!(texts.len(), 1, "{texts:?}");
    let vars = RuntimeVars::new();
    vars.set_text("x", x);
    let options = ContextOptions::new().runtime_vars(vars);
    let opts = approved(texts.into_iter().map(|t| t.hash));
    let (result, ctx) = send_full(dir.path(), "r.yml", opts, options, ExecOptions::new()).await;
    let ran = ctx.runtime_vars().entries().iter().any(|v| v.name() == "h");
    (result.assertions[0].verdict().clone(), ran)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_variable_cannot_turn_an_approved_list_item_into_javascript() {
    let (verdict, ran) = inject("in [${200}, {{x}}]", "${bru.setVar('h', 'evil')}").await;
    assert!(!ran, "the injected item ran");
    assert!(matches!(verdict, Verdict::Skipped { .. }), "{verdict:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_variable_cannot_close_an_approved_template() {
    let (verdict, ran) = inject("eq ${200}{{x}}", "` + bru.setVar('h', 'evil') + `").await;
    assert!(!ran, "the injected code ran");
    assert!(matches!(verdict, Verdict::Skipped { .. }), "{verdict:?}");
}

#[test]
fn texts_across_contexts_mask_the_sensitive_values_of_each() {
    let code = "const t = 's-one' + 's-two'";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &scripted("A", "http://127.0.0.1:9", &script("tests", code), ""),
        ),
    ]);
    let c = Collection::open(dir.path()).unwrap();
    let one = ContextOptions::new().sensitive_env_var("k", "s-one");
    let two = ContextOptions::new().sensitive_env_var("k", "s-two");
    let contexts = [
        VarContext::new(&c, one).unwrap(),
        VarContext::new(&c, two).unwrap(),
    ];
    let plan = c.plan(dir.path()).unwrap();
    let texts = plan_script_texts_across(&plan, &contexts);
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].1.text, code);
    assert_eq!(texts[0].1.shown, "const t = '<secret>' + '<secret>'");
    assert!(plan_script_texts_across(&plan, &[]).is_empty());
}
