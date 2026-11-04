//! `send_request` never shows a password a redirect url carries in a parser-only shape.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::{Value, json};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Location values a url parser reads `joe:hop-pa55` out of, aimed at the mock's `PORT`.
const SHAPES: [&str; 6] = [
    "\\\\joe:hop-pa55@127.0.0.1:PORT/next",
    "/\\joe:hop-pa55@127.0.0.1:PORT/next",
    "https:joe:hop-pa55@127.0.0.1:PORT/next",
    "https:/joe:hop-pa55@127.0.0.1:PORT/next",
    "https:\\\\joe:hop-pa55@127.0.0.1:PORT/next",
    "http:\\\\joe:hop-pa55@127.0.0.1:PORT/next",
];

/// The whole serialized `send_request` result for `/start` answering 302 to `shape`, and
/// its structured part.
async fn sent(shape: &str, follow: bool) -> (String, Value) {
    let mock = MockServer::start().await;
    let location = shape.replace("PORT", &common::port_of(&mock.uri()).to_string());
    let location = location.as_str();
    Mock::given(path("/start"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", location)
                .insert_header("content-location", location)
                .insert_header("refresh", format!("0; url={location}").as_str()),
        )
        .mount(&mock)
        .await;
    Mock::given(path("/next"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let mut file = request("r", 1, "GET", &format!("{}/start", mock.uri()));
    if !follow {
        file += "settings:\n  followRedirects: false\n";
    }
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    let whole = server.call("send_request", json!({"path": "r.yml"})).await;
    let shown = serde_json::to_string(&whole).expect("serialises");
    (shown, whole.structured_content.unwrap_or_default())
}

#[tokio::test]
async fn an_unfollowed_redirect_in_any_url_shape_never_shows_its_password() {
    for shape in SHAPES {
        let (shown, _) = sent(shape, false).await;
        assert!(shown.contains("302"), "{shape}: {shown}");
        assert!(!shown.contains("hop-pa55"), "{shape}: {shown}");
    }
}

#[tokio::test]
async fn a_followed_redirect_in_any_url_shape_never_shows_its_password() {
    // Shapes that resolve to http reach the mock's `/next` and become a second hop.
    for shape in SHAPES {
        let (shown, result) = sent(shape, true).await;
        if shape.starts_with("https") {
            assert!(result.get("timeline").is_none(), "{shape}: {shown}");
        } else {
            assert_eq!(
                result["timeline"].as_array().map(Vec::len),
                Some(2),
                "{shape}: {shown}"
            );
        }
        assert!(!shown.contains("hop-pa55"), "{shape}: {shown}");
        // The hop sends the userinfo as Basic credentials; their encoded form stays hidden too.
        assert!(!shown.contains("am9lOmhvcC1wYTU1"), "{shape}: {shown}");
    }
}

#[tokio::test]
async fn a_url_at_sign_in_the_path_is_shown_as_sent() {
    let mock = MockServer::start().await;
    let port = common::port_of(&mock.uri());
    let next = format!("http://127.0.0.1:{port}/a//b@c");
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", next.as_str()))
        .with_priority(1)
        .mount(&mock)
        .await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let start = format!("http://127.0.0.1:{port}/start");
    let file = request("r", 1, "GET", &start);
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", &file)]).await;
    let result = server
        .call("send_request", json!({"path": "r.yml"}))
        .await
        .structured_content
        .unwrap_or_default();
    assert_eq!(result["url"], next.as_str(), "{result}");
    assert_eq!(result["timeline"][1]["url"], next.as_str(), "{result}");

    let path_url = format!("http://127.0.0.1:{port}/r/joe:pw@127.0.0.1:{port}/next");
    let file = request("p", 1, "GET", &path_url);
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("p.yml", &file)]).await;
    let result = server
        .call("send_request", json!({"path": "p.yml"}))
        .await
        .structured_content
        .unwrap_or_default();
    assert_eq!(result["url"], path_url.as_str(), "{result}");
}
