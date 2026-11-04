//! `save_example`: the last response `send_request` got for a request, saved scrubbed into
//! its file, then listed by `read_request` and served by `mock_start`.

use crate::common;

use common::{ROOT, TestServer, http_get, port_of};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
const TOKEN: &str = "echo-Secret-591";

/// A GET of `url` that sends the `token` secret as a header.
fn request(url: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}/ping\"\n  headers:\n    - name: X-Token\n      value: \"{{{{token}}}}\"\n"
    )
}

/// A mock answering 201 with a session cookie and a body echoing the secret.
async fn echo_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("set-cookie", "sid=cookie-Value-77; Path=/")
                .set_body_raw(format!("{{\"echo\":\"{TOKEN}\"}}"), "application/json"),
        )
        .mount(&server)
        .await;
    server
}

async fn start(url: &str) -> TestServer {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &request(url)),
    ])
    .await;
    server.set_secret("dev", "token", TOKEN);
    server
}

#[tokio::test]
async fn a_sent_response_is_saved_scrubbed_listed_and_mocked() {
    let mock = echo_server().await;
    let server = start(&mock.uri()).await;
    server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    let saved = server
        .ok(
            "save_example",
            json!({"path": "r.yml", "name": "created", "description": "first"}),
        )
        .await;
    assert_eq!(saved["name"], "created");
    let disk = std::fs::read_to_string(server.root().join("r.yml")).expect("file");
    assert!(disk.contains("examples:"), "{disk}");
    assert!(!disk.contains(TOKEN), "{disk}");
    assert!(!disk.contains("cookie-Value-77"), "{disk}");
    assert!(disk.contains("{{token}}"), "{disk}");

    let read = server.ok("read_request", json!({"path": "r.yml"})).await;
    assert_eq!(
        read["examples"],
        json!([{"name": "created", "description": "first", "status": 201}])
    );

    let started = server.ok("mock_start", json!({"env": "dev"})).await;
    let port = port_of(started["url"].as_str().expect("url"));
    assert_eq!(http_get(port, "/ping").await.0, 201);
}

#[tokio::test]
async fn saving_without_a_prior_send_is_an_error_and_writes_nothing() {
    let mock = echo_server().await;
    let server = start(&mock.uri()).await;
    let before = std::fs::read_to_string(server.root().join("r.yml")).expect("file");
    let message = server
        .err("save_example", json!({"path": "r.yml", "name": "x"}))
        .await;
    assert!(message.contains("send the request first"), "{message}");
    let after = std::fs::read_to_string(server.root().join("r.yml")).expect("file");
    assert_eq!(before, after);
}

#[tokio::test]
async fn a_taken_name_is_refused() {
    let mock = echo_server().await;
    let server = start(&mock.uri()).await;
    server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    server
        .ok("save_example", json!({"path": "r.yml", "name": "one"}))
        .await;
    let message = server
        .err("save_example", json!({"path": "r.yml", "name": "one"}))
        .await;
    assert!(message.contains("already"), "{message}");
}

#[tokio::test]
async fn a_request_without_examples_reads_without_the_key() {
    let mock = echo_server().await;
    let server = start(&mock.uri()).await;
    let read = server.ok("read_request", json!({"path": "r.yml"})).await;
    assert!(read.get("examples").is_none(), "{read}");
}

#[tokio::test]
async fn an_echoed_examples_list_is_accepted_by_update_request_and_never_written() {
    let mock = echo_server().await;
    let server = start(&mock.uri()).await;
    server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    server
        .ok("save_example", json!({"path": "r.yml", "name": "kept"}))
        .await;
    let updated = server
        .ok(
            "update_request",
            json!({"path": "r.yml", "docs": "hi", "examples": [{"name": "invented", "status": 1}]}),
        )
        .await;
    assert_eq!(updated["examples"][0]["name"], "kept");
    assert_eq!(updated["examples"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn a_name_differing_only_by_javascript_whitespace_is_taken() {
    let mock = echo_server().await;
    let server = start(&mock.uri()).await;
    server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    server
        .ok("save_example", json!({"path": "r.yml", "name": "one"}))
        .await;
    let message = server
        .err(
            "save_example",
            json!({"path": "r.yml", "name": "\u{feff}one"}),
        )
        .await;
    assert!(message.contains("already"), "{message}");
}

#[tokio::test]
async fn a_later_send_without_a_response_leaves_nothing_to_save() {
    let mock = echo_server().await;
    let server = start(&mock.uri()).await;
    server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    // Nothing listens on port 1, so the second send gets no response.
    server.write("r.yml", &request("http://127.0.0.1:1"));
    server
        .err(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    let message = server
        .err("save_example", json!({"path": "r.yml", "name": "stale"}))
        .await;
    assert!(message.contains("send the request first"), "{message}");
    let disk = std::fs::read_to_string(server.root().join("r.yml")).expect("file");
    assert!(!disk.contains("examples:"), "{disk}");
}

#[tokio::test]
async fn a_compact_json_body_is_saved_re_indented() {
    let mock = echo_server().await;
    let server = start(&mock.uri()).await;
    server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    server
        .ok("save_example", json!({"path": "r.yml", "name": "pretty"}))
        .await;
    let c = apinata_core::Collection::open(server.root()).expect("collection");
    let item = apinata_core::ItemPath::new("r.yml").expect("item");
    let example = (c.request(&item).expect("request").examples())
        .into_iter()
        .find(|e| e.name == "pretty")
        .expect("the example");
    assert_eq!(
        example.response.and_then(|r| r.body).expect("a body").data,
        "{\n  \"echo\": \"{{token}}\"\n}"
    );
}

#[tokio::test]
async fn a_name_blank_to_javascript_is_refused_and_writes_nothing() {
    let mock = echo_server().await;
    let server = start(&mock.uri()).await;
    server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    let before = std::fs::read_to_string(server.root().join("r.yml")).expect("file");
    let message = server
        .err("save_example", json!({"path": "r.yml", "name": "\u{feff}"}))
        .await;
    assert!(message.contains("blank"), "{message}");
    let after = std::fs::read_to_string(server.root().join("r.yml")).expect("file");
    assert_eq!(before, after);
}

#[tokio::test]
async fn a_padded_taken_name_is_refused_and_named_trimmed() {
    let mock = echo_server().await;
    let server = start(&mock.uri()).await;
    server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    server
        .ok("save_example", json!({"path": "r.yml", "name": "dup"}))
        .await;
    let message = server
        .err("save_example", json!({"path": "r.yml", "name": "  dup  "}))
        .await;
    assert!(
        message.contains("already has an example named `dup`"),
        "{message}"
    );
}

#[tokio::test]
async fn a_saved_example_url_has_no_userinfo() {
    let mock = echo_server().await;
    let typed = mock.uri().replacen("http://", "http://acct-7781:pw-55@", 1);
    let server = start(&typed).await;
    server
        .ok(
            "send_request",
            json!({"path": "r.yml", "environment": "dev"}),
        )
        .await;
    server
        .ok("save_example", json!({"path": "r.yml", "name": "created"}))
        .await;
    let disk = std::fs::read_to_string(server.root().join("r.yml")).expect("file");
    assert_eq!(
        disk.matches("acct-7781").count(),
        1,
        "only the request keeps it: {disk}"
    );
    assert_eq!(disk.matches("pw-55").count(), 1, "{disk}");
    assert!(disk.contains(&format!("{}/ping", mock.uri())), "{disk}");
}
