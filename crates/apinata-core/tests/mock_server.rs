use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use crate::common::dead_port;
use apinata_core::mock::{MockError, MockOptions, start};
use apinata_core::{Collection, ContextOptions, VarContext};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn open(files: &[(&str, &str)]) -> (tempfile::TempDir, Collection, VarContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("opencollection.yml"), ROOT).unwrap();
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let c = Collection::open(dir.path()).unwrap();
    let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
    (dir, c, ctx)
}

fn req(name: &str, seq: u32, method: &str, url: &str, examples: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: {method}\n  url: \"{url}\"\n{examples}"
    )
}

const USERS: &str = "examples:\n  - name: ok\n    response:\n      status: 200\n      statusText: Fine\n      headers:\n        - name: Content-Type\n          value: application/json\n        - name: Content-Length\n          value: \"999\"\n      body:\n        type: json\n        data: '{\"id\": 7}'\n  - name: teapot\n    response:\n      status: 418\n";

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

fn options() -> MockOptions {
    MockOptions::new().port(0)
}

#[tokio::test]
async fn serves_the_example_with_its_status_headers_and_body() {
    let (_d, c, ctx) = open(&[(
        "u.yml",
        &req("u", 1, "GET", "http://api.test/users/:id", USERS),
    )]);
    let h = start(&c, &ctx, options()).await.unwrap();
    assert!(h.addr().ip().is_loopback());
    let r = client()
        .get(format!("http://{}/users/7", h.addr()))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(r.headers()["content-type"], "application/json");
    assert_eq!(
        r.content_length(),
        Some(9),
        "length is recomputed from the stored body"
    );
    assert_eq!(r.text().await.unwrap(), "{\"id\": 7}");
    let hits = h.hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].status, 200);
    assert_eq!(hits[0].request.as_deref(), Some("u.yml"));
    h.stop().await;
}

#[tokio::test]
async fn override_headers_select_an_example() {
    let (_d, c, ctx) = open(&[("u.yml", &req("u", 1, "GET", "/users", USERS))]);
    let h = start(&c, &ctx, options()).await.unwrap();
    let url = format!("http://{}/users", h.addr());
    let r = client()
        .get(&url)
        .header("x-mock-response-name", "teapot")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 418);
    let r = client()
        .get(&url)
        .header("x-mock-response-code", "999")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);
    h.stop().await;
}

#[tokio::test]
async fn a_miss_answers_404_json_with_candidates_and_other_method_routes() {
    let (_d, c, ctx) = open(&[("u.yml", &req("u", 1, "POST", "/users", USERS))]);
    let h = start(&c, &ctx, options()).await.unwrap();
    let r = client()
        .get(format!("http://{}/users", h.addr()))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);
    assert_eq!(r.headers()["content-type"], "application/json");
    let v: serde_json::Value = serde_json::from_slice(&r.bytes().await.unwrap()).unwrap();
    assert_eq!(v["error"], "no matching mock");
    assert_eq!(v["candidates"][0]["request"], "u.yml");
    let hits = h.hits();
    assert_eq!((hits[0].status, hits[0].request.clone()), (404, None));
    h.stop().await;
}

#[tokio::test]
async fn a_request_body_over_one_mib_gets_413_and_small_ones_are_discarded() {
    let (_d, c, ctx) = open(&[("u.yml", &req("u", 1, "POST", "/users", USERS))]);
    let h = start(&c, &ctx, options()).await.unwrap();
    let url = format!("http://{}/users", h.addr());
    let r = client()
        .post(&url)
        .body(vec![b'x'; 1024 * 1024 + 1])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 413);
    let hits = h.hits();
    assert_eq!((hits[0].status, hits[0].request.clone()), (413, None));
    assert!(hits[0].line().ends_with("-> 413 (rejected)"));
    let r = client().post(&url).body("small").send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    h.stop().await;
}

#[tokio::test]
async fn the_delay_applies_and_the_hit_hook_fires() {
    let (_d, c, ctx) = open(&[("u.yml", &req("u", 1, "GET", "/users", USERS))]);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let o = options()
        .delay_ms(150)
        .on_hit(move |hit| sink.lock().unwrap().push(hit.line()));
    let h = start(&c, &ctx, o).await.unwrap();
    let t = std::time::Instant::now();
    client()
        .get(format!("http://{}/users", h.addr()))
        .send()
        .await
        .unwrap();
    assert!(t.elapsed() >= Duration::from_millis(150));
    assert_eq!(seen.lock().unwrap().len(), 1);
    h.stop().await;
}

#[tokio::test]
async fn a_non_loopback_bind_is_refused_unless_allowed_and_nothing_is_listening_then() {
    let (_d, c, ctx) = open(&[("u.yml", &req("u", 1, "GET", "/users", USERS))]);
    let any = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    let err = start(&c, &ctx, options().bind(any))
        .await
        .err()
        .expect("refused");
    assert!(matches!(err, MockError::NonLoopbackBind), "{err}");
}

#[tokio::test]
async fn a_port_in_use_is_a_bind_error() {
    let (_d, c, ctx) = open(&[("u.yml", &req("u", 1, "GET", "/users", USERS))]);
    let taken = dead_port();
    let err = start(&c, &ctx, MockOptions::new().port(taken))
        .await
        .err()
        .expect("bind fails");
    assert!(matches!(err, MockError::Bind(_)), "{err}");
}

#[tokio::test]
async fn stop_and_drop_free_the_port() {
    let (_d, c, ctx) = open(&[("u.yml", &req("u", 1, "GET", "/users", USERS))]);
    let h = start(&c, &ctx, options()).await.unwrap();
    let addr = h.addr();
    h.stop().await;
    assert!(
        client()
            .get(format!("http://{addr}/users"))
            .send()
            .await
            .is_err()
    );
    let h2 = start(&c, &ctx, options()).await.unwrap();
    let addr2 = h2.addr();
    drop(h2);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        client()
            .get(format!("http://{addr2}/users"))
            .send()
            .await
            .is_err()
    );
}

#[tokio::test]
async fn startup_warnings_and_routes_are_reported() {
    let ok = req("ok", 1, "GET", "/a", USERS);
    let bad = req("bad", 2, "GET", "{{nope}}/b", USERS);
    let (_d, c, ctx) = open(&[("ok.yml", &ok), ("bad.yml", &bad)]);
    let h = start(&c, &ctx, options()).await.unwrap();
    assert_eq!(h.routes().len(), 1);
    assert_eq!(h.routes()[0].path, "/a");
    assert_eq!(h.routes()[0].examples, 2);
    assert_eq!(h.warnings().len(), 1);
    h.stop().await;
}

const TEMPLATED: &str = "examples:\n  - name: ok\n    response:\n      status: 200\n      headers:\n        - name: X-Id\n          value: 'id-{{id}}'\n        - name: X-Tok\n          value: '{{token}}'\n      body:\n        type: text\n        data: 'user {{id}} of {{org}} {{token}} {{nope}} {{'\n";

async fn get_templated(
    path: &str,
    files: &[(&str, &str)],
) -> (reqwest::Response, apinata_core::mock::MockHandle) {
    let (_d, c, _) = open(files);
    let mut o = ContextOptions::new();
    o = o.env_var("token", "SECRET");
    let ctx = VarContext::new(&c, o).unwrap();
    let h = start(&c, &ctx, options()).await.unwrap();
    let r = client()
        .get(format!("http://{}{path}", h.addr()))
        .send()
        .await
        .unwrap();
    (r, h)
}

#[tokio::test]
async fn path_params_fill_the_body_and_headers_and_nothing_else() {
    let f = req("u", 1, "GET", "/orgs/:org/users/:id", TEMPLATED);
    let (r, h) = get_templated("/orgs/acme/users/7", &[("u.yml", &f)]).await;
    assert_eq!(r.headers()["x-id"], "id-7");
    assert_eq!(r.headers()["x-tok"], "{{token}}");
    let len = r.content_length();
    let body = r.text().await.unwrap();
    assert_eq!(body, "user 7 of acme {{token}} {{nope}} {{");
    assert_eq!(len, Some(body.len() as u64), "length follows the new body");
    h.stop().await;
}

#[tokio::test]
async fn path_param_values_are_percent_decoded_and_not_expanded_again() {
    let f = req("u", 1, "GET", "/orgs/:org/users/:id", TEMPLATED);
    let (r, h) = get_templated("/orgs/a%20b/users/%7B%7Btoken%7D%7D", &[("u.yml", &f)]).await;
    assert_eq!(
        r.text().await.unwrap(),
        "user {{token}} of a b {{token}} {{nope}} {{"
    );
    h.stop().await;
}

#[tokio::test]
async fn binary_bodies_are_not_templated() {
    let ex = "examples:\n  - name: ok\n    response:\n      status: 200\n      body:\n        type: binary\n        data: e3tpZH19\n";
    let f = req("u", 1, "GET", "/u/:id", ex);
    let (r, h) = get_templated("/u/7", &[("u.yml", &f)]).await;
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"{{id}}");
    h.stop().await;
}
