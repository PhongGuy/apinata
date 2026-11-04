//! The snippet table: every entry is well formed and runs unedited against a local response.

use crate::script_support;

use apinata_core::{SnippetPhase, script_snippets};
use script_support::*;
use serde_json::json;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const IDS: [&str; 9] = [
    "status-200",
    "time-under",
    "content-type-json",
    "body-contains",
    "json-field",
    "header-exists",
    "var-from-body",
    "set-header",
    "set-var",
];

#[test]
fn ids_unique_and_select_in_code() {
    let table = script_snippets();
    let mut seen = std::collections::HashSet::new();
    for s in table {
        assert!(!s.id.is_empty() && seen.insert(s.id), "id {:?}", s.id);
        assert!(s.code.contains(s.select), "{}: select not in code", s.id);
        assert!(!s.phases.is_empty(), "{}: no phase", s.id);
        for text in [s.code, s.label] {
            assert!(!text.to_lowercase().contains("bruno"), "{}", s.id);
        }
    }
    for id in IDS {
        assert!(table.iter().any(|s| s.id == id), "missing {id}");
    }
}

#[test]
fn set_header_is_pre_request_only() {
    let s = script_snippets().iter().find(|s| s.id == "set-header");
    assert_eq!(s.expect("present").phases, [SnippetPhase::PreRequest]);
}

fn kind(phase: SnippetPhase) -> &'static str {
    match phase {
        SnippetPhase::PreRequest => "before-request",
        SnippetPhase::PostResponse => "after-response",
        SnippetPhase::Tests => "tests",
        _ => unreachable!("unknown phase"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_snippet_runs_and_passes() {
    let s = MockServer::start().await;
    Mock::given(path("/j"))
        .and(header("x-trace", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-trace", "1")
                .set_body_json(json!({"id": 7, "name": "apinata"})),
        )
        .mount(&s)
        .await;
    // Every snippet except set-header gets the x-trace header from the request itself below.
    let url = format!("{}/j", s.uri());
    for snippet in script_snippets() {
        for &phase in snippet.phases {
            let probe = match snippet.id {
                "var-from-body" => r#"console.log("probe=" + bru.getVar("id"))"#,
                "set-var" => r#"console.log("probe=" + bru.getVar("name"))"#,
                "set-header" => {
                    r#"test("sent", function () { expect(req.getHeader("x-trace")).to.equal("1"); });"#
                }
                _ => "",
            };
            let code = format!("{}\n{probe}", snippet.code);
            let request = if snippet.id == "set-header" {
                scripted("R", &url, &script(kind(phase), &code), "")
            } else {
                let headers = "  headers:\n    - name: x-trace\n      value: \"1\"\n";
                let text = scripted("R", &url, &script(kind(phase), &code), "");
                text.replace(
                    "http:\n  method: GET\n",
                    &format!("http:\n  method: GET\n{headers}"),
                )
            };
            let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &request)]);
            let result = send(dir.path(), "r.yml", on()).await;
            let label = format!("{} in {phase:?}", snippet.id);
            assert!(result.error.is_none(), "{label}: {:?}", result.error);
            assert!(
                tests(&result).iter().all(|t| t.ends_with(": pass")),
                "{label}: {:?}",
                tests(&result)
            );
            if phase == SnippetPhase::Tests && snippet.code.contains("test(") {
                assert!(!result.tests.is_empty(), "{label}: no test");
            }
            let expected = match snippet.id {
                "var-from-body" => Some("log:probe=7"),
                "set-var" => Some("log:probe=value"),
                _ => None,
            };
            if let Some(line) = expected {
                assert!(
                    console(&result).iter().any(|c| c == line),
                    "{label}: {:?}",
                    console(&result)
                );
            }
        }
    }
}

/// Runs one snippet against a mock answering with `content_type` (none sends no header).
async fn run_snippet_with_type(id: &str, content_type: Option<&str>) -> Vec<String> {
    let s = MockServer::start().await;
    let mut response = ResponseTemplate::new(200);
    if let Some(ct) = content_type {
        response = response.insert_header("content-type", ct);
    }
    Mock::given(path("/b"))
        .respond_with(response.set_body_bytes(b"{}".to_vec()))
        .mount(&s)
        .await;
    let snippet = script_snippets()
        .iter()
        .find(|x| x.id == id)
        .expect("present");
    let url = format!("{}/b", s.uri());
    let request = scripted("R", &url, &script("tests", snippet.code), "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &request)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert!(
        result.error.is_none(),
        "{content_type:?}: {:?}",
        result.error
    );
    tests(&result)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_type_json_passes_for_json_types() {
    for ct in [
        "application/json",
        "application/problem+json",
        "Application/JSON; charset=utf-8",
    ] {
        let got = run_snippet_with_type("content-type-json", Some(ct)).await;
        assert!(
            got.len() == 1 && got[0].ends_with(": pass"),
            "{ct}: {got:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_type_json_fails_for_other_or_missing_types() {
    for ct in [Some("text/plain"), Some("text/html"), None] {
        let got = run_snippet_with_type("content-type-json", ct).await;
        assert!(
            got.len() == 1 && got[0].contains(": fail"),
            "{ct:?}: {got:?}"
        );
    }
}

#[test]
fn titles_follow_the_edited_placeholder() {
    for (id, select) in [
        ("status-200", "200"),
        ("time-under", "500"),
        ("json-field", "\"name\""),
        ("body-contains", "\"apinata\""),
        ("header-exists", "\"x-trace\""),
        ("content-type-json", "\"json\""),
    ] {
        let s = script_snippets()
            .iter()
            .find(|s| s.id == id)
            .expect("present");
        assert_eq!(s.select, select, "{id}");
        assert_eq!(
            s.code.matches(select).count(),
            1,
            "{id}: placeholder repeated"
        );
        let title = s.code.lines().find(|l| l.contains("test(")).expect("line");
        assert!(!title.contains(select), "{id}: {title}");
    }
}
