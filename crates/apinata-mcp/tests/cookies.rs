//! The cookie tools: the jar a send fills, listed without values, and cookies set and deleted
//! one at a time.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn get(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

async fn server_with_cookies() -> (MockServer, TestServer) {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "sid=cookie-Value-1; Path=/; HttpOnly")
                .append_header("set-cookie", "theme=dark; Path=/; Max-Age=3600"),
        )
        .mount(&mock)
        .await;
    let server =
        TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &get(&mock.uri()))]).await;
    server.ok("send_request", json!({"path": "r.yml"})).await;
    (mock, server)
}

#[tokio::test]
async fn lists_names_domains_and_flags_but_never_values() {
    let (_mock, server) = server_with_cookies().await;
    let listed = server.call("list_cookies", json!({})).await;
    let shown = serde_json::to_string(&listed).expect("serialises");
    assert!(!shown.contains("cookie-Value-1"), "{shown}");
    assert!(!shown.contains("dark"), "{shown}");
    let cookies = listed.structured_content.expect("structured")["cookies"].clone();
    let mut names: Vec<&str> = cookies
        .as_array()
        .expect("cookies")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    names.sort();
    assert_eq!(names, ["sid", "theme"]);
    let sid = cookies
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "sid")
        .unwrap();
    assert_eq!(sid["domain"], "127.0.0.1");
    assert_eq!(sid["http_only"], true);
    assert_eq!(sid["expires"], serde_json::Value::Null);
    let theme = cookies
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "theme")
        .unwrap();
    assert!(theme["expires"].as_u64().is_some());
}

#[tokio::test]
async fn clears_one_domain_or_everything() {
    let (_mock, server) = server_with_cookies().await;
    let other = server
        .ok("clear_cookies", json!({"domain": "example.com"}))
        .await;
    assert_eq!(other, json!({"removed": 0}));
    let all = server.ok("clear_cookies", json!({})).await;
    assert_eq!(all, json!({"removed": 2}));
    assert_eq!(
        server.ok("list_cookies", json!({})).await,
        json!({"cookies": []})
    );
}

#[tokio::test]
async fn clearing_a_domain_removes_only_its_cookies() {
    let (_mock, server) = server_with_cookies().await;
    let removed = server
        .ok("clear_cookies", json!({"domain": "127.0.0.1"}))
        .await;
    assert_eq!(removed, json!({"removed": 2}));
}

const SECRET: &str = "sEcReT-cookie-VALUE";

/// Every text a tool result carries, structured or not, for a leak search.
async fn reply(server: &TestServer, tool: &str, args: serde_json::Value) -> String {
    let result = server.call(tool, args).await;
    serde_json::to_string(&result).expect("serialises")
}

#[tokio::test]
async fn a_set_cookie_is_listed_without_its_value() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let set = server
        .ok(
            "set_cookie",
            json!({"url": "http://127.0.0.1:9/api/login", "name": "sid", "value": SECRET,
                   "http_only": true, "expires": 4_102_444_800u64}),
        )
        .await;
    assert_eq!(set["replaced"], false, "{set}");
    assert_eq!(set["cookie"]["domain"], "127.0.0.1");
    assert_eq!(set["cookie"]["path"], "/api");
    let listed = server.ok("list_cookies", json!({})).await;
    assert_eq!(
        listed,
        json!({"cookies": [{"name": "sid", "domain": "127.0.0.1", "path": "/api",
                            "expires": 4_102_444_800u64, "secure": false, "http_only": true}]})
    );
    let again = server
        .ok(
            "set_cookie",
            json!({"url": "http://127.0.0.1:9/api/login", "name": "sid", "value": "other"}),
        )
        .await;
    assert_eq!(again["replaced"], true, "{again}");
}

#[tokio::test]
async fn a_set_cookie_goes_to_its_path_and_below_but_not_elsewhere() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let base = mock.uri();
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("inside.yml", &get(&format!("{base}/a/b"))),
        ("outside.yml", &get(&format!("{base}/c"))),
    ])
    .await;
    server
        .ok(
            "set_cookie",
            json!({"url": format!("{base}/a/login"), "name": "sid", "value": SECRET}),
        )
        .await;
    server
        .ok("send_request", json!({"path": "inside.yml"}))
        .await;
    server
        .ok("send_request", json!({"path": "outside.yml"}))
        .await;
    let sent: Vec<(String, Option<String>)> = mock
        .received_requests()
        .await
        .expect("recorded")
        .iter()
        .map(|r| {
            let cookie = r
                .headers
                .get("cookie")
                .map(|v| v.to_str().unwrap().to_owned());
            (r.url.path().to_owned(), cookie)
        })
        .collect();
    assert_eq!(
        sent,
        [
            ("/a/b".to_owned(), Some(format!("sid={SECRET}"))),
            ("/c".to_owned(), None)
        ]
    );
}

#[tokio::test]
async fn a_domain_the_url_cannot_set_is_refused() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let error = server
        .err(
            "set_cookie",
            json!({"url": "http://api.example.com/", "domain": "other.com", "name": "sid",
                   "value": SECRET}),
        )
        .await;
    assert!(error.contains("does not match api.example.com"), "{error}");
    let suffix = server
        .err(
            "set_cookie",
            json!({"url": "http://api.example.co.uk/", "domain": "co.uk", "name": "sid",
                   "value": SECRET}),
        )
        .await;
    assert!(suffix.contains("public suffix"), "{suffix}");
    assert_eq!(
        server.ok("list_cookies", json!({})).await,
        json!({"cookies": []})
    );
}

#[tokio::test]
async fn a_domain_cookie_covers_the_parent_domain() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    server
        .ok(
            "set_cookie",
            json!({"url": "https://api.example.com/", "domain": ".example.com", "name": "sid",
                   "value": SECRET, "secure": true}),
        )
        .await;
    let listed = server.ok("list_cookies", json!({})).await;
    assert_eq!(listed["cookies"][0]["domain"], "example.com", "{listed}");
    assert_eq!(listed["cookies"][0]["secure"], true, "{listed}");
}

#[tokio::test]
async fn delete_removes_one_cookie_and_keeps_the_rest() {
    let (_mock, server) = server_with_cookies().await;
    let deleted = server
        .ok(
            "delete_cookie",
            json!({"domain": "127.0.0.1", "path": "/", "name": "sid"}),
        )
        .await;
    assert_eq!(deleted, json!({"removed": true}));
    let listed = server.ok("list_cookies", json!({})).await;
    let names: Vec<&str> = listed["cookies"]
        .as_array()
        .expect("cookies")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, ["theme"]);
}

#[tokio::test]
async fn deleting_a_missing_cookie_reports_no_match() {
    let (_mock, server) = server_with_cookies().await;
    let deleted = server
        .ok(
            "delete_cookie",
            json!({"domain": "127.0.0.1", "path": "/other", "name": "sid"}),
        )
        .await;
    assert_eq!(deleted, json!({"removed": false}));
    let listed = server.ok("list_cookies", json!({})).await;
    assert_eq!(listed["cookies"].as_array().expect("cookies").len(), 2);
}

#[tokio::test]
async fn no_reply_repeats_a_set_cookie_value() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let url = "http://127.0.0.1:9/";
    let replies = [
        reply(
            &server,
            "set_cookie",
            json!({"url": url, "name": "sid", "value": SECRET}),
        )
        .await,
        reply(
            &server,
            "set_cookie",
            json!({"url": url, "name": "sid", "value": SECRET}),
        )
        .await,
        reply(
            &server,
            "set_cookie",
            json!({"url": url, "name": "bad name", "value": SECRET}),
        )
        .await,
        reply(
            &server,
            "set_cookie",
            json!({"url": url, "name": "sid", "value": format!("{SECRET};x")}),
        )
        .await,
        reply(&server, "list_cookies", json!({})).await,
        reply(
            &server,
            "delete_cookie",
            json!({"domain": "127.0.0.1", "path": "/", "name": "sid"}),
        )
        .await,
    ];
    for r in replies {
        assert!(!r.contains(SECRET), "{r}");
    }
}
