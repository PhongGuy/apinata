//! One cookie jar per executor, fed by every hop. On the first hop a request's own Cookie header is
//! merged with the jar's cookies for the URL, the jar winning; on a redirect hop the jar's replace it.

use crate::common;

use apinata_core::{ExecOptions, Executor};
use common::resolved_one;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The Cookie header the server saw on its `n`th request, split into sorted pairs.
async fn cookies_seen(server: &MockServer, n: usize) -> Vec<String> {
    let received = server.received_requests().await.expect("recorded");
    let mut pairs: Vec<String> = received_cookie(&received[n])
        .unwrap_or_default()
        .split("; ")
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    pairs.sort();
    pairs
}

/// The raw Cookie header of one recorded request, after checking it was sent at most once.
fn received_cookie(request: &wiremock::Request) -> Option<String> {
    let all: Vec<_> = request.headers.get_all("cookie").iter().collect();
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

const OWN: &str = "  headers:\n    - name: Cookie\n      value: a=0; u=2\n";

#[tokio::test]
async fn cookies_from_every_hop_and_error_status_are_stored_and_sent_later() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/login",
        set_cookie(302, "a=1; Path=/").insert_header("location", "/home"),
    )
    .await;
    mount(&server, "/home", set_cookie(200, "b=2; Path=/")).await;
    mount(&server, "/denied", set_cookie(401, "c=3; Path=/")).await;
    mount(&server, "/check", ResponseTemplate::new(200)).await;
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let send = |p: &str| resolved_one("GET", &format!("{}{p}", server.uri()), "", "");
    exec.send(&send("/login")).await.expect("sent");
    assert_eq!(
        cookies_seen(&server, 1).await,
        ["a=1"],
        "the redirect hop carries the cookie it just got"
    );
    assert_eq!(
        exec.send(&send("/denied")).await.expect("sent").status(),
        401
    );
    exec.send(&send("/check")).await.expect("sent");
    assert_eq!(cookies_seen(&server, 3).await, ["a=1", "b=2", "c=3"]);

    let fresh = Executor::new(ExecOptions::new()).expect("client");
    fresh.send(&send("/check")).await.expect("sent");
    assert_eq!(
        cookies_seen(&server, 4).await,
        Vec::<String>::new(),
        "jars are per executor"
    );
}

/// The Cookie header `/echo` saw when sent with `own` headers after `/set` stored `stored`.
async fn merged(stored: &str, own: &str) -> Option<String> {
    let server = MockServer::start().await;
    mount(&server, "/set", set_cookie(200, stored)).await;
    mount(&server, "/echo", ResponseTemplate::new(200)).await;
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let set = resolved_one("GET", &format!("{}/set", server.uri()), "", "");
    exec.send(&set).await.expect("sent");
    let echo = resolved_one("GET", &format!("{}/echo", server.uri()), own, "");
    exec.send(&echo).await.expect("sent");
    let received = server.received_requests().await.expect("recorded");
    received_cookie(&received[1])
}

#[tokio::test]
async fn jar_cookies_for_the_url_merge_into_an_own_cookie_header() {
    assert_eq!(
        merged("a=1; Path=/", OWN).await,
        Some("a=1; u=2".to_owned()),
        "the jar's a=1 wins and keeps a's place; the own-only u=2 stays"
    );
    let own = "  headers:\n    - name: Cookie\n      value: a=1; b=2\n";
    assert_eq!(
        merged("b=3; Path=/", own).await,
        Some("a=1; b=3".to_owned())
    );
}

#[tokio::test]
async fn a_jar_only_cookie_is_appended_and_a_valueless_own_piece_is_kept() {
    let own = "  headers:\n    - name: Cookie\n      value: \"flag; =x; t=a=b ;; a = 1 ; a=2\"\n";
    assert_eq!(
        merged("j=9; Path=/", own).await,
        Some("flag=; t=a=b; a=2; j=9".to_owned()),
        "an unnamed piece is dropped, an inner = kept, a repeated name collapses to its last value"
    );
}

#[tokio::test]
async fn an_own_cookie_header_is_sent_untouched_when_the_jar_has_none_for_the_url() {
    let server = MockServer::start().await;
    mount(&server, "/set", set_cookie(200, "a=1; Path=/scoped")).await;
    mount(&server, "/echo", ResponseTemplate::new(200)).await;
    mount(&server, "/scoped/echo", ResponseTemplate::new(200)).await;
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let echo = resolved_one("GET", &format!("{}/echo", server.uri()), OWN, "");
    exec.send(&echo).await.expect("sent");
    let set = resolved_one("GET", &format!("{}/set", server.uri()), "", "");
    exec.send(&set).await.expect("sent");
    exec.send(&echo).await.expect("sent");
    let scoped = resolved_one("GET", &format!("{}/scoped/echo", server.uri()), OWN, "");
    exec.send(&scoped).await.expect("sent");
    let received = server.received_requests().await.expect("recorded");
    let raw = |n: usize| received_cookie(&received[n]);
    assert_eq!(raw(0), Some("a=0; u=2".to_owned()), "an empty jar");
    assert_eq!(
        raw(2),
        Some("a=0; u=2".to_owned()),
        "a jar whose only cookie is scoped to another path"
    );
    assert_eq!(
        raw(3),
        Some("a=1; u=2".to_owned()),
        "inside the cookie's path the jar's a=1 merges in"
    );
}

#[tokio::test]
async fn each_hop_chooses_jar_or_own_cookies_and_the_own_header_crosses_origins() {
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    mount(&a, "/prime", set_cookie(200, "s=1; Path=/scoped")).await;
    mount(
        &a,
        "/start",
        ResponseTemplate::new(302).insert_header("location", "/scoped/in"),
    )
    .await;
    mount(
        &a,
        "/scoped/in",
        ResponseTemplate::new(302).insert_header("location", "/out"),
    )
    .await;
    let to_b = format!("{}/end", b.uri());
    mount(
        &a,
        "/out",
        ResponseTemplate::new(307).insert_header("location", to_b.as_str()),
    )
    .await;
    mount(&b, "/end", ResponseTemplate::new(200)).await;
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let prime = resolved_one("GET", &format!("{}/prime", a.uri()), "", "");
    exec.send(&prime).await.expect("sent");
    let own = "  headers:\n    - name: Cookie\n      value: u=2\n";
    let start = resolved_one("GET", &format!("{}/start", a.uri()), own, "");
    assert_eq!(exec.send(&start).await.expect("sent").status(), 200);
    let on_a = a.received_requests().await.expect("recorded");
    let seen: Vec<_> = on_a[1..].iter().map(received_cookie).collect();
    assert_eq!(
        seen,
        [
            Some("u=2".to_owned()),
            Some("s=1".to_owned()),
            Some("u=2".to_owned()),
        ],
        "/start, /scoped/in, /out"
    );
    let on_b = b.received_requests().await.expect("recorded");
    assert_eq!(
        received_cookie(&on_b[0]),
        Some("u=2".to_owned()),
        "bruno-cli keeps an own Cookie header on a cross-origin hop"
    );
}
