//! `send_request` and `run_folder` append the core hint to an undefined-variable message, except
//! where the variable's name could reveal part of a secret.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::json;

const DEV: &str = "name: dev\nvariables:\n  - name: base\n    value: b\n  - secret: true\n    name: tok\n  - secret: true\n    name: wrapped\n";
const TOK_HINT: &str =
    "tok is a secret in environment dev with no value; set it in the app's environment editor";

async fn server(url: &str) -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &request("r", 1, "GET", url)),
    ])
    .await
}

#[tokio::test]
async fn send_appends_the_hint_after_the_message() {
    let server = server("http://127.0.0.1:1/{{tok}}").await;
    let message = server
        .err(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    assert!(
        message.ends_with(&format!("nothing was sent; {TOK_HINT}")),
        "{message}"
    );
}

#[tokio::test]
async fn run_folder_carries_the_same_hint() {
    let server = server("http://127.0.0.1:1/{{tok}}").await;
    let result = server
        .ok("run_folder", json!({"path": "", "environment": "dev"}))
        .await;
    let error = result["results"][0]["error"].as_str().expect("error");
    assert!(error.ends_with(TOK_HINT), "{error}");
}

#[tokio::test]
async fn a_name_inside_a_secret_value_gets_no_hint() {
    let server = server("http://127.0.0.1:1/{{wrapped}}").await;
    server.set_secret("dev", "wrapped", "pre-{{inner}}-post");
    let message = server
        .err(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    assert_eq!(
        message,
        "a variable written inside a secret value is not defined; nothing was sent"
    );
}

#[tokio::test]
async fn a_name_defined_elsewhere_asks_for_that_environment_without_a_parameter_name() {
    let server = server("http://127.0.0.1:1/{{base}}").await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    assert!(
        message.ends_with(
            "nothing was sent; base is defined in environment dev; select that environment"
        ),
        "{message}"
    );
    assert!(
        !message.contains("--env") && !message.contains("argument"),
        "{message}"
    );
}
