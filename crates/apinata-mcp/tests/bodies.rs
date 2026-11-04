//! Response bodies and body files: text cut to the agent's budget on a character boundary,
//! binary reported by type and size, and one answer for a missing or escaping body file.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn get(url: &str, extra: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}")
}

async fn mock(template: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(template)
        .mount(&server)
        .await;
    server
}

async fn start(request: &str) -> TestServer {
    TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", request)]).await
}

#[tokio::test]
async fn text_is_cut_on_a_character_boundary_and_binary_is_type_and_size_only() {
    let text = mock(ResponseTemplate::new(200).set_body_string("ééééé")).await;
    let server = start(&get(&text.uri(), "")).await;
    let result = server
        .ok(
            "send_request",
            json!({"path": "r.yml", "max_body_bytes": 5}),
        )
        .await;
    assert_eq!(result["body"]["text"], "éé");
    assert_eq!(result["body"]["truncated"], true);
    assert_eq!(result["body"]["total_bytes"], 10);

    let binary = mock(
        ResponseTemplate::new(200)
            .set_body_raw(vec![0xff, 0xfe, 0x00, 0x01], "application/octet-stream"),
    )
    .await;
    server.write("r.yml", &get(&binary.uri(), ""));
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["body"]["text"], Value::Null);
    assert_eq!(result["body"]["content_type"], "application/octet-stream");
    assert_eq!(result["body"]["total_bytes"], 4);
}

#[tokio::test]
async fn a_missing_body_file_and_one_outside_the_root_answer_alike() {
    let mock = mock(ResponseTemplate::new(200)).await;
    let body = |file: &str| {
        format!(
            "  body:\n    type: file\n    data:\n      - filePath: \"{file}\"\n        contentType: text/plain\n        selected: true\n"
        )
    };
    let server = start(&get(&mock.uri(), &body("../outside.txt"))).await;
    let outside = server.err("send_request", json!({"path": "r.yml"})).await;
    server.write("r.yml", &get(&mock.uri(), &body("missing.txt")));
    let missing = server.err("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(
        outside.replace("../outside.txt", "F"),
        missing.replace("missing.txt", "F"),
        "{outside} / {missing}"
    );
    assert!(mock.received_requests().await.expect("recorded").is_empty());
}

/// Every two-character window of `secret`.
fn windows(secret: &str) -> Vec<String> {
    let chars: Vec<char> = secret.chars().collect();
    chars.windows(2).map(|w| w.iter().collect()).collect()
}

#[tokio::test]
async fn no_body_cap_returns_a_piece_of_a_secret() {
    for secret in ["Le4k \"me\"/now+9", "Zq7Wk9Px"] {
        let text = format!("body:{secret}:end");
        let mock = mock(ResponseTemplate::new(200).set_body_string(text.clone())).await;
        let uses = "  headers:\n    - name: X-Key\n      value: \"{{token}}\"\n";
        let server = TestServer::start(&[
            ("opencollection.yml", ROOT),
            (
                "environments/dev.yml",
                "name: dev\nvariables:\n  - name: token\n    secret: true\n",
            ),
            ("r.yml", &get(&mock.uri(), uses)),
        ])
        .await;
        server.set_secret("dev", "token", secret);
        // Step the cap through every byte of the body, as an agent probing for the secret would.
        for max in 1..=text.len() + 1 {
            let args = json!({"path": "r.yml", "environment": "dev", "max_body_bytes": max});
            let result = server.ok("send_request", args).await;
            let body = result["body"]["text"].as_str().expect("text");
            for window in windows(secret) {
                assert!(
                    !body.contains(&window),
                    "cap {max}: {body:?} holds {window:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_punycoded_label_embedded_in_a_body_is_scrubbed_at_every_cap() {
    let host = url::Url::parse("http://xGeheimnüs9y.localhost/").expect("url");
    let label = host
        .host_str()
        .expect("host")
        .split('.')
        .next()
        .expect("label")
        .to_owned();
    let text =
        format!("http%3A%2F%2F{label}.localhost%2Fx 9{label} axn--xn--xn--{label}.localhost");
    let mock = mock(ResponseTemplate::new(200).set_body_string(text.clone())).await;
    let uses = "  headers:\n    - name: X-Key\n      value: \"{{key}}\"\n";
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("r.yml", &get(&mock.uri(), uses)),
        ],
        |o| o.env_var("key", "Geheimnüs9"),
    )
    .await;
    for max in 1..=text.len() + 1 {
        let args = json!({"path": "r.yml", "max_body_bytes": max});
        let result = server.ok("send_request", args).await;
        let body = result["body"]["text"]
            .as_str()
            .expect("text")
            .to_lowercase();
        for leak in ["xn--", "geheimn"] {
            assert!(!body.contains(leak), "cap {max}: {body:?} holds {leak:?}");
        }
    }
}

#[tokio::test]
async fn a_sparql_body_reads_updates_and_sends_as_sparql() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let request = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"{}/q\"\n  body:\n    type: sparql\n    data: ASK {{}}\n",
        server.uri()
    );
    let mcp = start(&request).await;
    let doc = mcp.ok("read_request", json!({"path": "r.yml"})).await;
    assert_eq!(doc["body"], json!({"type": "sparql", "text": "ASK {}"}));
    let doc = mcp
        .ok(
            "update_request",
            json!({"path": "r.yml", "body": {"type": "sparql", "text": "SELECT * WHERE { ?s ?p ?o }"}}),
        )
        .await;
    assert_eq!(doc["body"]["type"], "sparql");
    mcp.ok("send_request", json!({"path": "r.yml"})).await;
    let sent = &server.received_requests().await.expect("recorded")[0];
    assert_eq!(
        sent.headers.get("content-type").map(|v| v.as_bytes()),
        Some(&b"application/sparql-query"[..])
    );
    assert_eq!(sent.body, b"SELECT * WHERE { ?s ?p ?o }");
}
