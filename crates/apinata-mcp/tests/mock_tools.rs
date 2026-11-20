//! `mock_start`, `mock_hits` and `mock_stop`: one loopback mock per session.

use crate::common;

use common::{MOCK_ITEMS, ROOT, TestServer, http_get, port_of};
use serde_json::json;

#[tokio::test]
async fn start_serves_the_saved_examples_on_loopback_and_lists_routes() {
    let server = TestServer::start(MOCK_ITEMS).await;
    let started = server
        .ok("mock_start", json!({"env": "dev", "port": 0}))
        .await;
    let url = started["url"].as_str().expect("url");
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert_eq!(started["warnings"], json!([]));
    let mut routes: Vec<(String, String)> = started["routes"]
        .as_array()
        .expect("routes")
        .iter()
        .map(|r| {
            (
                r["method"].as_str().unwrap().into(),
                r["path"].as_str().unwrap().into(),
            )
        })
        .collect();
    routes.sort();
    assert_eq!(
        routes,
        [
            ("GET".into(), "/ping".into()),
            ("GET".into(), "/v1/items".into())
        ]
    );
    let port = port_of(url);
    assert_eq!(http_get(port, "/ping").await.0, 201);
    assert_eq!(http_get(port, "/v1/items").await.0, 202);
    assert_eq!(http_get(port, "/nope").await.0, 404);
}

#[tokio::test]
async fn a_second_start_errors_and_the_first_keeps_serving() {
    let server = TestServer::start(MOCK_ITEMS).await;
    let first = server.ok("mock_start", json!({"env": "dev"})).await;
    let url = first["url"].as_str().unwrap().to_owned();
    let message = server.err("mock_start", json!({"env": "dev"})).await;
    assert!(message.contains("already running"), "{message}");
    assert!(message.contains(&url), "{message}");
    assert_eq!(http_get(port_of(&url), "/ping").await.0, 201);
}

#[tokio::test]
async fn an_unknown_environment_is_a_tool_error_and_nothing_runs() {
    let server = TestServer::start(MOCK_ITEMS).await;
    let message = server.err("mock_start", json!({"env": "missing"})).await;
    assert!(!message.contains("nothing was sent"), "{message}");
    // A failed start leaves no mock behind, so a valid one can start.
    server.ok("mock_start", json!({"env": "dev"})).await;
}

#[tokio::test]
async fn a_route_url_with_an_undefined_variable_is_a_warning_not_a_literal_route() {
    let server = TestServer::start(MOCK_ITEMS).await;
    // No environment selected: `{{base}}` is undefined, so `items.yml` is skipped.
    let started = server.ok("mock_start", json!({})).await;
    assert_eq!(started["routes"].as_array().unwrap().len(), 1);
    assert_eq!(started["warnings"].as_array().unwrap().len(), 1);
    let port = port_of(started["url"].as_str().unwrap());
    assert_eq!(http_get(port, "/ping").await.0, 201);
}

#[tokio::test]
async fn an_empty_collection_starts_with_no_routes() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let started = server.ok("mock_start", json!({})).await;
    assert_eq!(started["routes"], json!([]));
}

#[tokio::test]
async fn stop_frees_the_port_and_a_new_start_succeeds() {
    // Parallel tests bind port 0 too and may be handed the freed port at once, so the stopped
    // mock is told apart from them by a body only it serves.
    let nonce = format!("stopped-mock-{}-{}", std::process::id(), rand_suffix());
    let ping = format!(
        "info:\n  name: Ping\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"/ping\"\n\
         examples:\n  - name: ok\n    response:\n      status: 201\n      body:\n        \
         type: text\n        data: '{nonce}'\n"
    );
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("ping.yml", &ping)]).await;
    let started = server.ok("mock_start", json!({})).await;
    let url = started["url"].as_str().unwrap().to_owned();
    let port = port_of(&url);
    assert_eq!(http_get(port, "/ping").await, (201, nonce.clone()));
    let stopped = server.ok("mock_stop", json!({})).await;
    assert_eq!(stopped["url"], url);
    assert_eq!(stopped["hits"], 1);
    assert!(
        !serves(port, &nonce).await,
        "the stopped mock still answers on {port}"
    );
    // A neighbour holding the port only lives as long as its test, so wait it out.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let again = loop {
        let result = server.call("mock_start", json!({"port": port})).await;
        if result.is_error != Some(true) || tokio::time::Instant::now() > deadline {
            break result;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_ne!(again.is_error, Some(true), "{:?}", again.content);
    let again = again.structured_content.expect("structured content");
    assert_eq!(port_of(again["url"].as_str().unwrap()), port);
    assert_eq!(http_get(port, "/ping").await, (201, nonce));
}

/// A suffix that differs between runs, so a nonce never matches another process's mock.
fn rand_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// Whether whatever listens on 127.0.0.1:`port` serves `body` for `GET /ping`. A refused
/// connect, a socket the OS connected to itself, or another server all answer false.
async fn serves(port: u16, body: &str) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(mut stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await else {
        return false;
    };
    let req = "GET /ping HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(req.as_bytes()).await.is_err() {
        return false;
    }
    let mut raw = Vec::new();
    let read = stream.read_to_end(&mut raw);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), read).await;
    String::from_utf8_lossy(&raw).contains(body)
}

#[tokio::test]
async fn stop_with_nothing_running_is_an_error() {
    let server = TestServer::start(MOCK_ITEMS).await;
    let message = server.err("mock_stop", json!({})).await;
    assert!(message.contains("no mock server is running"), "{message}");
}

#[tokio::test]
async fn dropping_the_server_closes_the_listener() {
    let server = TestServer::start(MOCK_ITEMS).await;
    let started = server.ok("mock_start", json!({"env": "dev"})).await;
    let port = port_of(started["url"].as_str().unwrap());
    drop(server);
    // The server task ends when the client side of the pipe closes; give it a moment.
    let mut closed = false;
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
        {
            closed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(closed, "the mock still listens after the server dropped");
}

#[tokio::test]
async fn hits_report_each_request_in_order_with_the_matched_example() {
    let server = TestServer::start(MOCK_ITEMS).await;
    let started = server.ok("mock_start", json!({"env": "dev"})).await;
    let port = port_of(started["url"].as_str().unwrap());
    http_get(port, "/ping").await;
    http_get(port, "/nope").await;
    let log = server.ok("mock_hits", json!({})).await;
    assert_eq!(log["next"], 2);
    let hits = log["hits"].as_array().unwrap();
    assert_eq!(hits[0]["method"], "GET");
    assert_eq!(hits[0]["path"], "/ping");
    assert_eq!(hits[0]["status"], 201);
    assert_eq!(hits[0]["request"], "ping.yml");
    assert_eq!(hits[0]["example"], "ok");
    assert_eq!(hits[0]["fallback"], false);
    assert_eq!(hits[1]["path"], "/nope");
    assert_eq!(hits[1]["status"], 404);
    assert_eq!(hits[1]["request"], json!(null));
}

#[tokio::test]
async fn since_skips_what_the_caller_has_seen() {
    let server = TestServer::start(MOCK_ITEMS).await;
    let started = server.ok("mock_start", json!({"env": "dev"})).await;
    let port = port_of(started["url"].as_str().unwrap());
    http_get(port, "/ping").await;
    let first = server.ok("mock_hits", json!({})).await;
    http_get(port, "/v1/items").await;
    let second = server
        .ok("mock_hits", json!({"since": first["next"]}))
        .await;
    let hits = second["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["path"], "/v1/items");
    assert_eq!(second["next"], 2);
}

#[tokio::test]
async fn since_past_the_end_is_empty_and_hits_without_a_mock_is_an_error() {
    let server = TestServer::start(MOCK_ITEMS).await;
    let message = server.err("mock_hits", json!({})).await;
    assert!(message.contains("no mock server is running"), "{message}");
    server.ok("mock_start", json!({"env": "dev"})).await;
    let log = server.ok("mock_hits", json!({"since": 99})).await;
    assert_eq!(log["hits"], json!([]));
    assert_eq!(log["next"], 0);
}

#[tokio::test]
async fn a_secret_in_a_route_path_is_masked_in_the_result_but_still_served() {
    const SECRET: &str = "s3cr3t-segment";
    let env = "name: dev\nvariables:\n  - name: base\n    value: http://api.test\n  - secret: true\n    name: key\n";
    let files = [
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", env),
        (
            "keyed.yml",
            concat!(
                "info:\n  name: Keyed\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{{base}}/v1/{{key}}/data\"\n",
                "examples:\n  - name: ok\n    response:\n      status: 203\n"
            ),
        ),
    ];
    let server = TestServer::start(&files).await;
    server.set_secret("dev", "key", SECRET);
    let started = server.ok("mock_start", json!({"env": "dev"})).await;
    assert!(!started.to_string().contains(SECRET), "{started}");
    let port = port_of(started["url"].as_str().unwrap());
    assert_eq!(http_get(port, &format!("/v1/{SECRET}/data")).await.0, 203);
}
