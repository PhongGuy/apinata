//! `import_request`: the same file core writes for the same command, notes, and refusals.

use crate::common;

use apinata_core::import::{add_request, parse_request};
use apinata_core::{Collection, ItemPath};
use common::{ROOT, TestServer, files_under};
use serde_json::json;
use std::fs;

const COMMAND: &str = r#"curl -X POST 'https://{{host}}/users?x=1' -H 'Authorization: Bearer {{token}}' -k --json '{"a":1}'"#;

#[tokio::test]
async fn a_curl_command_becomes_the_request_core_writes() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let out = server
        .ok(
            "import_request",
            json!({"text": COMMAND, "name": "Create user"}),
        )
        .await;
    assert_eq!(out["path"], "Create user.yml");
    let notes = out["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n["source"] == "curl flag -k" && n["kind"] == "dropped"),
        "{notes:?}"
    );
    let core = tempfile::tempdir().unwrap();
    fs::write(core.path().join("opencollection.yml"), ROOT).unwrap();
    let mut collection = Collection::open(core.path()).unwrap();
    add_request(
        &mut collection,
        &ItemPath::root(),
        "Create user",
        &parse_request(None, COMMAND).unwrap().request,
    )
    .unwrap();
    let read = |dir: &std::path::Path| fs::read_to_string(dir.join("Create user.yml")).unwrap();
    assert_eq!(read(server.root()), read(core.path()));
    // The URL holds a variable, so encoding stays off: its value may already be encoded.
    assert!(
        read(server.root()).contains("encodeUrl: false"),
        "{}",
        read(server.root())
    );
}

#[tokio::test]
async fn parent_and_format_are_honoured() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "api/folder.yml",
            "info:\n  name: api\n  type: folder\n  seq: 1\n",
        ),
    ])
    .await;
    let out = server
        .ok("import_request", json!({"text": "curl https://x.test/ping", "format": "curl", "parent": "api", "name": "ping"}))
        .await;
    assert_eq!(out["path"], "api/ping.yml");
    assert_eq!(out["notes"], json!([]));
}

#[tokio::test]
async fn refusals_name_the_problem_and_write_nothing() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let e = server
        .err(
            "import_request",
            json!({"text": "wget https://x.test", "name": "w"}),
        )
        .await;
    assert!(e.contains("not a recognized request format"), "{e}");
    let e = server
        .err(
            "import_request",
            json!({"text": "curl https://x.test", "format": "wget", "name": "w"}),
        )
        .await;
    assert!(e.contains("expected auto, curl"), "{e}");
    let e = server
        .err(
            "import_request",
            json!({"text": "curl 'https://x.test", "name": "w"}),
        )
        .await;
    assert!(e.contains("unclosed quote"), "{e}");
    server
        .err(
            "import_request",
            json!({"text": "curl https://x.test", "parent": "../out", "name": "w"}),
        )
        .await;
    server
        .err(
            "import_request",
            json!({"text": "curl https://x.test", "parent": "nope", "name": "w"}),
        )
        .await;
    assert_eq!(files_under(server.root()).len(), 1);
}

#[tokio::test]
async fn importing_sends_and_reads_nothing() {
    let mock = wiremock::MockServer::start().await;
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let text = format!("curl {}/x -d @/nonexistent/apinata-secret", mock.uri());
    server
        .ok("import_request", json!({"text": text, "name": "x"}))
        .await;
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn import_request_is_a_closed_world_write_naming_no_other_tool() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "import_request")
        .expect("listed");
    let hints = tool.annotations.as_ref().expect("annotations");
    assert_eq!(
        (hints.destructive_hint, hints.open_world_hint),
        (Some(false), Some(false))
    );
    let description = tool
        .description
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    assert!(
        description.contains("nothing is sent") && !description.contains("bruno"),
        "{description}"
    );
}

#[tokio::test]
async fn an_aws_secret_goes_to_the_only_environments_secret_variable() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
    ])
    .await;
    let out = server
        .ok(
            "import_request",
            json!({
                "text": "curl --aws-sigv4 aws:amz:us-east-1:s --user AKID:TOPSECRET https://x.test/d",
                "name": "d"
            }),
        )
        .await;
    assert_eq!(
        server.secret("dev", "awsSecretAccessKey").as_deref(),
        Some("TOPSECRET")
    );
    let request = fs::read_to_string(server.root().join("d.yml")).unwrap();
    let env = fs::read_to_string(server.root().join("environments/dev.yml")).unwrap();
    assert!(request.contains("{{awsSecretAccessKey}}"), "{request}");
    assert!(env.contains("secret: true"), "{env}");
    for text in [&request, &env, &out.to_string()] {
        assert!(!text.contains("TOPSECRET"), "{text}");
    }
}

const AWS_CURL: &str =
    "curl --aws-sigv4 aws:amz:us-east-1:service --user AKID:TOPSECRET https://x.test/a";

async fn two_environments() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/prod.yml", "name: prod\n"),
    ])
    .await
}

fn notes_of(out: &serde_json::Value) -> Vec<serde_json::Value> {
    out["notes"].as_array().expect("notes").clone()
}

fn assert_nothing_stored(server: &TestServer) {
    for env in ["dev", "prod"] {
        assert_eq!(server.secret(env, "awsSecretAccessKey"), None, "{env}");
    }
    for (path, text) in files_under(server.root()) {
        assert!(!text.contains("TOPSECRET"), "{path}");
    }
}

#[tokio::test]
async fn a_named_environment_receives_the_secret_among_several() {
    let server = two_environments().await;
    let out = server
        .ok(
            "import_request",
            json!({"text": AWS_CURL, "name": "a", "environment": "prod"}),
        )
        .await;
    assert_eq!(
        server.secret("prod", "awsSecretAccessKey").as_deref(),
        Some("TOPSECRET")
    );
    assert_eq!(server.secret("dev", "awsSecretAccessKey"), None);
    let prod = fs::read_to_string(server.root().join("environments/prod.yml")).unwrap();
    assert!(prod.contains("secret: true"), "{prod}");
    for (path, text) in files_under(server.root()) {
        assert!(!text.contains("TOPSECRET"), "{path}");
    }
    let notes = notes_of(&out);
    assert!(
        notes
            .iter()
            .any(|n| n["kind"] == "converted" && n["detail"].as_str().unwrap().contains("prod")),
        "{notes:?}"
    );
    assert!(!out.to_string().contains("TOPSECRET"));
}

#[tokio::test]
async fn several_environments_and_none_named_stores_nothing() {
    let server = two_environments().await;
    let out = server
        .ok("import_request", json!({"text": AWS_CURL, "name": "a"}))
        .await;
    assert_nothing_stored(&server);
    let notes = notes_of(&out);
    assert!(
        notes.iter().any(|n| n["kind"] == "needs-input"
            && n["detail"]
                .as_str()
                .unwrap()
                .contains("several environments")),
        "{notes:?}"
    );
    let request = fs::read_to_string(server.root().join("a.yml")).unwrap();
    assert!(request.contains("{{awsSecretAccessKey}}"), "{request}");
}

fn assert_names_missing_environment(notes: &[serde_json::Value], available: &str) {
    assert!(
        notes.iter().any(|n| n["kind"] == "needs-input" && {
            let d = n["detail"].as_str().unwrap();
            d == format!(
                "the secret access key is not written to the file; define a secret variable named `awsSecretAccessKey` (environment \"staging\" not found; available: {available})"
            )
        }),
        "{notes:?}"
    );
}

#[tokio::test]
async fn an_unknown_environment_among_several_stores_nothing() {
    let server = two_environments().await;
    let out = server
        .ok(
            "import_request",
            json!({"text": AWS_CURL, "name": "a", "environment": "staging"}),
        )
        .await;
    assert_nothing_stored(&server);
    assert_names_missing_environment(&notes_of(&out), "dev, prod");
}

#[tokio::test]
async fn an_unknown_environment_with_one_existing_stores_nothing() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
    ])
    .await;
    let out = server
        .ok(
            "import_request",
            json!({"text": AWS_CURL, "name": "a", "environment": "staging"}),
        )
        .await;
    assert_nothing_stored(&server);
    assert_names_missing_environment(&notes_of(&out), "dev");
}

#[tokio::test]
async fn the_input_schema_lists_the_environment_argument() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "import_request")
        .expect("listed");
    let schema = serde_json::Value::Object((*tool.input_schema).clone());
    assert!(schema["properties"]["environment"].is_object(), "{schema}");
}
