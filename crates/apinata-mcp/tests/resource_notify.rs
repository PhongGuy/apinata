//! Subscriptions and the notification policy: which tool calls tell the client to list
//! resources again.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const PING: &str = "apinata://shop/ping.yml";

async fn shop() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "ping.yml",
            &request("Ping", 1, "GET", "http://127.0.0.1:1/ping"),
        ),
    ])
    .await
}

#[tokio::test]
async fn initialize_advertises_resource_subscriptions() {
    let server = shop().await;
    let resources = server
        .server_capabilities()
        .resources
        .expect("resources capability");
    assert_eq!(resources.subscribe, Some(true));
    assert_eq!(resources.list_changed, Some(true));
}

#[tokio::test]
async fn subscribing_to_a_listed_resource_is_accepted() {
    let server = shop().await;
    for resource in server.resources().await {
        server
            .subscribe(&resource.uri)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", resource.uri));
    }
}

#[tokio::test]
async fn subscribing_to_a_uri_outside_the_collection_is_invalid_params() {
    let server = shop().await;
    for uri in [
        "apinata://other/ping.yml",
        "file:///etc/passwd",
        "apinata://shop/../x.yml",
        "nonsense",
    ] {
        let error = server.subscribe(uri).await.expect_err(uri);
        assert!(error.contains("-32602"), "{uri}: {error}");
    }
}

#[tokio::test]
async fn unsubscribing_is_accepted_and_repeatable() {
    let server = shop().await;
    server.subscribe(PING).await.expect("subscribe");
    server.unsubscribe(PING).await.expect("unsubscribe");
    server.unsubscribe(PING).await.expect("again");
    let error = server.unsubscribe("nonsense").await.expect_err("invalid");
    assert!(error.contains("-32602"), "{error}");
}

#[tokio::test]
async fn sending_a_request_does_not_announce_a_resource_change() {
    let local = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&local)
        .await;
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &request("R", 1, "GET", &format!("{}/x", local.uri())),
        ),
        ("w.yml", &request("W", 2, "GET", "http://127.0.0.1:1/w")),
    ])
    .await;
    server.ok("send_request", json!({"path": "r.yml"})).await;
    // The server.rs unit test of the notification policy is the deterministic
    // guarantee; this one can only miss a stray notification by passing early.
    // The sentinel call notifies, so once it has, anything the send sent has arrived too.
    server
        .ok("update_request", json!({"path": "w.yml", "name": "W2"}))
        .await;
    server.wait_list_changed(1).await;
    assert_eq!(server.settled_list_changed().await, 1);
}

#[tokio::test]
async fn a_tool_that_writes_a_file_announces_a_resource_change() {
    let server = shop().await;
    server
        .ok(
            "update_request",
            json!({"path": "ping.yml", "name": "Pong"}),
        )
        .await;
    server.wait_list_changed(1).await;
}

// External edits: the server watches the collection and pushes notifications.

const READY: &str = "apinata://shop/ready.yml";
const SENTINEL: &str = "apinata://shop/sentinel.yml";

fn uri(name: &str) -> String {
    format!("apinata://shop/{name}")
}

async fn watched() -> TestServer {
    watched_after(std::time::Duration::ZERO).await
}

/// As `watched`, but the subscriptions come `pause` after the handshake, so the watcher's
/// first report of the existing files has already gone by.
async fn watched_after(pause: std::time::Duration) -> TestServer {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "ping.yml",
            &request("Ping", 1, "GET", "http://127.0.0.1:1/ping"),
        ),
        (
            "other.yml",
            &request("Other", 2, "GET", "http://127.0.0.1:1/other"),
        ),
        (
            "ready.yml",
            &request("Ready", 3, "GET", "http://127.0.0.1:1/ready"),
        ),
        (
            "sentinel.yml",
            &request("Sentinel", 4, "GET", "http://127.0.0.1:1/sentinel"),
        ),
    ])
    .await;
    tokio::time::sleep(pause).await;
    server.subscribe(READY).await.expect("subscribe ready");
    server
        .subscribe(SENTINEL)
        .await
        .expect("subscribe sentinel");
    // One write is enough. Before the watch is placed the watcher reports the files already
    // there; after, it reports the edit. Retrying would restart the debounce each time.
    server.write(
        "ready.yml",
        &request("Ready", 3, "GET", "http://127.0.0.1:1/ready0"),
    );
    server.wait_updated(READY, 1).await;
    server
}

/// Edit the sentinel and wait for its notification, which the watcher delivers after
/// everything it saw before.
async fn flush(server: &TestServer, round: usize) {
    let url = format!("http://127.0.0.1:1/sentinel{round}");
    server.write("sentinel.yml", &request("Sentinel", 4, "GET", &url));
    server.wait_updated(SENTINEL, round).await;
}

#[tokio::test]
async fn readiness_holds_when_the_subscriptions_follow_the_watcher_start() {
    // Rewriting the file again before the debouncer has reported the last write restarts its
    // timer, so a probe that retries can starve the watcher it is waiting for.
    let server = watched_after(std::time::Duration::from_secs(1)).await;
    let text = std::fs::read_to_string(server.root().join("ready.yml")).expect("ready.yml");
    assert!(
        text.contains("/ready0"),
        "the probe rewrote ready.yml: {text}"
    );
}

#[tokio::test]
async fn an_external_edit_of_a_subscribed_request_is_announced() {
    let server = watched().await;
    server.subscribe(PING).await.expect("subscribe");
    server.write(
        "ping.yml",
        &request("Ping", 1, "POST", "http://127.0.0.1:1/ping"),
    );
    server.wait_updated(PING, 1).await;
}

#[tokio::test]
async fn an_edit_of_an_unsubscribed_resource_is_not_announced() {
    let server = watched().await;
    server.write(
        "other.yml",
        &request("Other", 2, "POST", "http://127.0.0.1:1/other"),
    );
    flush(&server, 1).await;
    assert!(!server.updated().contains(&uri("other.yml")));
}

#[tokio::test]
async fn an_added_file_announces_a_new_resource_list() {
    let server = watched().await;
    server.write(
        "added.yml",
        &request("Added", 9, "GET", "http://127.0.0.1:1/added"),
    );
    server.wait_list_changed(1).await;
    assert!(
        server
            .resources()
            .await
            .iter()
            .any(|r| r.uri == uri("added.yml"))
    );
}

#[tokio::test]
async fn a_removed_subscribed_file_is_announced_and_changes_the_list() {
    let server = watched().await;
    server.subscribe(PING).await.expect("subscribe");
    std::fs::remove_file(server.root().join("ping.yml")).expect("remove");
    server.wait_updated(PING, 1).await;
    server.wait_list_changed(1).await;
}

#[cfg(unix)]
#[tokio::test]
async fn ignored_files_and_links_out_of_the_root_announce_nothing() {
    let outside = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(
        outside.path().join("t.yml"),
        request("T", 1, "GET", "http://127.0.0.1:1/t"),
    )
    .expect("outside file");
    let server = watched().await;
    server.subscribe(PING).await.expect("subscribe");
    server.write(".env", "TOKEN=1\n");
    server.write(".git/x", "ref\n");
    server.write(".git/x.yml", "a: 1\n");
    std::os::unix::fs::symlink(outside.path().join("t.yml"), server.root().join("link.yml"))
        .expect("link");
    std::os::unix::fs::symlink(outside.path(), server.root().join("linkdir")).expect("dir link");
    flush(&server, 1).await;
    let seen = server.updated();
    assert!(
        seen.iter().all(|u| u == READY || u == SENTINEL),
        "unexpected updates: {seen:?}"
    );
    assert_eq!(server.settled_list_changed().await, 0);
}

#[tokio::test]
async fn a_burst_of_new_files_announces_the_list_a_few_times_at_most() {
    let server = watched().await;
    let last = uri("burst19.yml");
    for n in 0..20 {
        server.write(
            &format!("burst{n}.yml"),
            &request("B", 10 + n, "GET", "http://127.0.0.1:1/b"),
        );
    }
    server.wait_list_changed(1).await;
    // A subscription needs an existing file, so the last one is rewritten once it is listed.
    server.subscribe(&last).await.expect("subscribe");
    server.write(
        "burst19.yml",
        &request("B", 29, "GET", "http://127.0.0.1:1/c"),
    );
    server.wait_updated(&last, 1).await;
    flush(&server, 1).await;
    let announced = server.settled_list_changed().await;
    assert!((1..=2).contains(&announced), "{announced} list_changed");
}

#[tokio::test]
async fn subscribing_to_a_path_that_does_not_exist_is_invalid_params() {
    let server = shop().await;
    for uri in [
        "apinata://shop/nope.yml",
        "apinata://shop/missing/",
        "apinata://shop/@environment/ghost",
    ] {
        let error = server.subscribe(uri).await.expect_err(uri);
        assert!(error.contains("-32602"), "{uri}: {error}");
    }
}

#[tokio::test]
async fn renaming_a_request_through_a_tool_announces_the_list_once() {
    let server = watched().await;
    server
        .ok(
            "update_request",
            json!({"path": "ping.yml", "name": "Pong"}),
        )
        .await;
    server.wait_list_changed(1).await;
    // The watcher sees the tool's own write; it must not announce it a second time.
    flush(&server, 1).await;
    assert_eq!(server.settled_list_changed().await, 1);
}

#[tokio::test]
async fn an_external_rename_announces_the_list_once() {
    let server = watched().await;
    server.write(
        "ping.yml",
        &request("Pong", 1, "GET", "http://127.0.0.1:1/ping"),
    );
    server.wait_list_changed(1).await;
    flush(&server, 1).await;
    assert_eq!(server.settled_list_changed().await, 1);
}

#[tokio::test]
async fn an_external_edit_that_keeps_the_name_does_not_announce_the_list() {
    let server = watched().await;
    server.write(
        "ping.yml",
        &request("Ping", 1, "POST", "http://127.0.0.1:1/ping"),
    );
    flush(&server, 1).await;
    assert_eq!(server.settled_list_changed().await, 0);
}

#[tokio::test]
async fn a_tool_that_changes_nothing_listed_announces_nothing() {
    let server = watched().await;
    server
        .ok(
            "update_request",
            json!({"path": "ping.yml", "method": "POST"}),
        )
        .await;
    flush(&server, 1).await;
    assert_eq!(server.settled_list_changed().await, 0);
}

#[tokio::test]
async fn a_tool_that_creates_a_request_announces_the_list_once() {
    let server = watched().await;
    // Drain the readiness probe's trailing events first: one landing between the tool's write
    // and its recording would let the watcher announce first and hide a second announcement.
    flush(&server, 1).await;
    server
        .ok("create_request", json!({"parent": "", "name": "Fresh"}))
        .await;
    server.wait_list_changed(1).await;
    flush(&server, 2).await;
    assert_eq!(server.settled_list_changed().await, 1);
}

#[tokio::test]
async fn a_tool_that_deletes_a_request_announces_the_list_once() {
    let server = watched().await;
    server.ok("delete_item", json!({"path": "other.yml"})).await;
    server.wait_list_changed(1).await;
    flush(&server, 1).await;
    assert_eq!(server.settled_list_changed().await, 1);
}

#[tokio::test]
async fn an_in_place_save_that_keeps_the_name_does_not_announce_the_list() {
    let server = watched().await;
    server.write_in_place(
        "ping.yml",
        &request("Ping", 1, "POST", "http://127.0.0.1:1/ping"),
    );
    flush(&server, 1).await;
    assert_eq!(server.settled_list_changed().await, 0);
}

#[tokio::test]
async fn an_in_place_save_that_renames_announces_the_list_once() {
    let server = watched().await;
    server.write_in_place(
        "ping.yml",
        &request("Pong", 1, "GET", "http://127.0.0.1:1/ping"),
    );
    server.wait_list_changed(1).await;
    flush(&server, 1).await;
    assert_eq!(server.settled_list_changed().await, 1);
}

#[tokio::test]
async fn renaming_an_item_through_a_tool_announces_the_list_once() {
    let server = watched().await;
    flush(&server, 1).await;
    server
        .ok(
            "rename_item",
            json!({"path": "other.yml", "name": "Renamed"}),
        )
        .await;
    server.wait_list_changed(1).await;
    flush(&server, 2).await;
    assert_eq!(server.settled_list_changed().await, 1);
}
