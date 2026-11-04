//! The executor's own cookie jar: listing and deleting what every send stored.

use crate::common;

use apinata_core::{CookieInfo, CookieJar, ExecOptions, Executor, MemoryStore, SecretStore};
use common::{executor, resolved_one};
use std::sync::Arc;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A server whose `/set` stores three cookies and whose `/echo` answers 200.
async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/set"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "sid=SECRET1; Path=/; HttpOnly")
                .append_header("set-cookie", "theme=dark; Path=/; Max-Age=3600")
                .append_header("set-cookie", "deep=SECRET2; Path=/api"),
        )
        .mount(&server)
        .await;
    Mock::given(path("/echo"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

/// The Cookie header of the server's last request.
async fn last_cookie(server: &MockServer) -> Option<String> {
    let all = server.received_requests().await.expect("recorded");
    let last = all.last().expect("a request");
    last.headers
        .get("cookie")
        .map(|v| v.to_str().expect("ascii").to_owned())
}

/// (domain, path, name, value, host_only, http_only, persistent) for each listed cookie.
fn shapes(list: &[CookieInfo]) -> Vec<(&str, &str, &str, &str, bool, bool, bool)> {
    list.iter()
        .map(|c| {
            (
                c.domain.as_str(),
                c.path.as_str(),
                c.name.as_str(),
                c.value.as_str(),
                c.host_only,
                c.http_only,
                c.expires.is_some(),
            )
        })
        .collect()
}

#[tokio::test]
async fn stored_cookies_are_listed_sorted_with_their_attributes() {
    let server = server().await;
    let exec = executor();
    assert!(exec.cookies().list().is_empty());
    let set = resolved_one("GET", &format!("{}/set", server.uri()), "", "");
    exec.send(&set).await.expect("sent");
    assert_eq!(
        shapes(&exec.cookies().list()),
        [
            ("127.0.0.1", "/", "sid", "SECRET1", true, true, false),
            ("127.0.0.1", "/", "theme", "dark", true, false, true),
            ("127.0.0.1", "/api", "deep", "SECRET2", true, false, false),
        ]
    );
    let shown = format!("{:?} {:?}", exec.cookies(), exec.cookies().list());
    assert!(!shown.contains("SECRET"), "{shown}");
    assert!(shown.contains("sid"), "{shown}");
}

#[tokio::test]
async fn deleted_cookies_are_no_longer_sent() {
    let server = server().await;
    let exec = executor();
    let set = resolved_one("GET", &format!("{}/set", server.uri()), "", "");
    let echo = resolved_one("GET", &format!("{}/echo", server.uri()), "", "");
    exec.send(&set).await.expect("sent");

    assert!(exec.cookies().delete("127.0.0.1", "/", "sid"));
    assert!(!exec.cookies().delete("127.0.0.1", "/", "sid"));
    assert!(!exec.cookies().delete("other.test", "/", "theme"));
    exec.send(&echo).await.expect("sent");
    assert_eq!(last_cookie(&server).await.as_deref(), Some("theme=dark"));

    assert_eq!(exec.cookies().delete_domain("other.test"), 0);
    assert_eq!(exec.cookies().delete_domain("127.0.0.1"), 2);
    assert!(exec.cookies().list().is_empty());
    exec.send(&echo).await.expect("sent");
    assert_eq!(last_cookie(&server).await, None);

    exec.send(&set).await.expect("sent");
    assert_eq!(exec.cookies().list().len(), 3);
    exec.cookies().clear();
    assert!(exec.cookies().list().is_empty());
    exec.send(&echo).await.expect("sent");
    assert_eq!(last_cookie(&server).await, None);
}

#[tokio::test]
async fn a_utf8_cookie_value_round_trips_as_reqwests_jar_kept_it() {
    let server = MockServer::start().await;
    Mock::given(path("/set"))
        .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", "sid=café; Path=/"))
        .mount(&server)
        .await;
    Mock::given(path("/echo"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let exec = executor();
    let set = resolved_one("GET", &format!("{}/set", server.uri()), "", "");
    let echo = resolved_one("GET", &format!("{}/echo", server.uri()), "", "");
    exec.send(&set).await.expect("sent");
    let values: Vec<String> = exec.cookies().list().into_iter().map(|c| c.value).collect();
    assert_eq!(values, ["café"]);
    exec.send(&echo).await.expect("sent");
    let all = server.received_requests().await.expect("recorded");
    let sent = all
        .last()
        .expect("a request")
        .headers
        .get("cookie")
        .expect("cookie");
    assert_eq!(sent.as_bytes(), "sid=café".as_bytes());
}

#[tokio::test]
async fn deleting_a_domain_counts_only_the_cookies_list_showed() {
    let server = MockServer::start().await;
    Mock::given(path("/set"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "brief=1; Path=/; Max-Age=1")
                .append_header("set-cookie", "kept=1; Path=/"),
        )
        .mount(&server)
        .await;
    let exec = executor();
    let set = resolved_one("GET", &format!("{}/set", server.uri()), "", "");
    exec.send(&set).await.expect("sent");
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(exec.cookies().list().len(), 1);
    assert_eq!(exec.cookies().delete_domain("127.0.0.1"), 1);
}

#[tokio::test]
async fn a_jar_given_to_the_executor_collects_set_cookie_and_saves_it() {
    let server = server().await;
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("cookies.json");
    let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let jar = Arc::new(CookieJar::open(path.clone(), keys.clone()));
    jar.load();
    let options = ExecOptions::new().cookie_jar(jar.clone());
    assert!(!format!("{options:?}").contains("SECRET"));
    let exec = Executor::new(options).expect("client");
    assert!(std::ptr::eq(exec.cookies(), &*jar));
    let set = resolved_one("GET", &format!("{}/set", server.uri()), "", "");
    exec.send(&set).await.expect("sent");
    assert_eq!(jar.list().len(), 3);
    assert!(!path.exists(), "the send itself writes nothing");
    jar.save().expect("saved");
    let again = CookieJar::open(path, keys);
    again.load();
    assert_eq!(shapes(&again.list()), shapes(&jar.list()));
}

#[tokio::test]
async fn a_cookie_inserted_into_the_shared_jar_is_sent() {
    let server = server().await;
    let jar = Arc::new(CookieJar::default());
    jar.insert(CookieInfo::new("127.0.0.1", "manual", "m1"))
        .unwrap();
    let exec = Executor::new(ExecOptions::new().cookie_jar(jar)).expect("client");
    let echo = resolved_one("GET", &format!("{}/echo", server.uri()), "", "");
    exec.send(&echo).await.expect("sent");
    assert_eq!(last_cookie(&server).await.as_deref(), Some("manual=m1"));
}
