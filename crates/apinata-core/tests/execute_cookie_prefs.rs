//! The store and send cookie switches: on by default, changeable between sends, and fixed for
//! every hop of one send.

use crate::common;

use apinata_core::{CookieInfo, ExecOptions, Executor};
use common::resolved_one;
use std::time::Duration;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The raw Cookie header of the server's `n`th request, after checking it was sent at most once.
async fn cookie_at(server: &MockServer, n: usize) -> Option<String> {
    let received = server.received_requests().await.expect("recorded");
    let all: Vec<_> = received[n].headers.get_all("cookie").iter().collect();
    assert!(all.len() <= 1, "one Cookie header at most: {all:?}");
    all.first().map(|v| v.to_str().expect("ascii").to_owned())
}

async fn mount(server: &MockServer, at: &str, template: ResponseTemplate) {
    Mock::given(path(at))
        .respond_with(template)
        .mount(server)
        .await;
}

fn set_cookie(status: u16, cookie: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).insert_header("set-cookie", cookie)
}

fn names(exec: &Executor) -> Vec<String> {
    exec.cookies().list().into_iter().map(|c| c.name).collect()
}

/// `/login` answers 302 to `/home` setting `a=1`, `/home` sets `b=2`, `/echo` sets nothing.
async fn server() -> MockServer {
    let server = MockServer::start().await;
    mount(
        &server,
        "/login",
        set_cookie(302, "a=1; Path=/").insert_header("location", "/home"),
    )
    .await;
    mount(&server, "/home", set_cookie(200, "b=2; Path=/")).await;
    mount(&server, "/echo", ResponseTemplate::new(200)).await;
    server
}

const OWN: &str = "  headers:\n    - name: Cookie\n      value: u=2\n";

#[test]
fn both_switches_default_on_and_show_in_debug() {
    let exec = Executor::new(ExecOptions::new()).expect("client");
    assert!(exec.store_cookies_enabled() && exec.send_cookies_enabled());
    let on = format!("{:?}", ExecOptions::new());
    assert!(
        on.contains("store_cookies: true") && on.contains("send_cookies: true"),
        "{on}"
    );
    let off = format!(
        "{:?}",
        ExecOptions::new().store_cookies(false).send_cookies(false)
    );
    assert!(
        off.contains("store_cookies: false") && off.contains("send_cookies: false"),
        "{off}"
    );
    let exec = Executor::new(ExecOptions::new().store_cookies(false)).expect("client");
    assert!(!exec.store_cookies_enabled() && exec.send_cookies_enabled());
    exec.set_store_cookies(true);
    exec.set_send_cookies(false);
    assert!(exec.store_cookies_enabled() && !exec.send_cookies_enabled());
}

#[tokio::test]
async fn with_store_off_no_hop_is_stored_and_the_jar_keeps_what_it_had() {
    let server = server().await;
    let exec = Executor::new(ExecOptions::new().store_cookies(false)).expect("client");
    exec.cookies()
        .insert(CookieInfo::new("127.0.0.1", "pre", "0"))
        .expect("inserted");
    let login = resolved_one("GET", &format!("{}/login", server.uri()), "", "");
    exec.send(&login).await.expect("sent");
    assert_eq!(
        names(&exec),
        ["pre"],
        "neither the 302 nor the 200 hop stored"
    );
    assert_eq!(cookie_at(&server, 0).await.as_deref(), Some("pre=0"));
    assert_eq!(
        cookie_at(&server, 1).await.as_deref(),
        Some("pre=0"),
        "the redirect hop sends the jar, which never got a=1"
    );
    assert!(
        exec.cookies().delete("127.0.0.1", "/", "pre"),
        "edits still work"
    );
    assert!(names(&exec).is_empty());
}

#[tokio::test]
async fn with_send_off_the_jar_fills_but_is_never_read_and_an_own_header_goes_as_written() {
    let server = server().await;
    let exec = Executor::new(ExecOptions::new().send_cookies(false)).expect("client");
    let get = |p: &str, own: &str| resolved_one("GET", &format!("{}{p}", server.uri()), own, "");
    exec.send(&get("/login", "")).await.expect("sent");
    assert_eq!(names(&exec), ["a", "b"]);
    assert_eq!(
        cookie_at(&server, 1).await,
        None,
        "the redirect hop carries no jar cookie"
    );
    exec.send(&get("/echo", OWN)).await.expect("sent");
    assert_eq!(
        cookie_at(&server, 2).await.as_deref(),
        Some("u=2"),
        "the jar's a=1; b=2 does not replace the own header"
    );
    exec.send(&get("/echo", "")).await.expect("sent");
    assert_eq!(cookie_at(&server, 3).await, None);
}

#[tokio::test]
async fn the_setters_apply_from_the_next_send() {
    let server = server().await;
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let get = |p: &str| resolved_one("GET", &format!("{}{p}", server.uri()), "", "");
    exec.set_store_cookies(false);
    exec.send(&get("/home")).await.expect("sent");
    assert!(names(&exec).is_empty());
    exec.set_store_cookies(true);
    exec.send(&get("/home")).await.expect("sent");
    assert_eq!(names(&exec), ["b"]);
    exec.set_send_cookies(false);
    exec.send(&get("/echo")).await.expect("sent");
    assert_eq!(cookie_at(&server, 2).await, None);
    exec.set_send_cookies(true);
    exec.send(&get("/echo")).await.expect("sent");
    assert_eq!(cookie_at(&server, 3).await.as_deref(), Some("b=2"));
}

#[tokio::test]
async fn one_send_keeps_the_policy_it_started_with_across_its_hops() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/slow",
        set_cookie(302, "a=1; Path=/")
            .insert_header("location", "/home")
            .set_delay(Duration::from_millis(500)),
    )
    .await;
    mount(&server, "/home", set_cookie(200, "b=2; Path=/")).await;
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let slow = resolved_one("GET", &format!("{}/slow", server.uri()), "", "");
    // Both switches flip while the first hop waits for its delayed response.
    let (sent, ()) = tokio::join!(exec.send(&slow), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        exec.set_send_cookies(false);
        exec.set_store_cookies(false);
    });
    sent.expect("sent");
    assert_eq!(
        cookie_at(&server, 1).await.as_deref(),
        Some("a=1"),
        "the hop after the flip still stores and sends"
    );
    assert_eq!(names(&exec), ["a", "b"]);
    assert!(!exec.store_cookies_enabled() && !exec.send_cookies_enabled());
}
