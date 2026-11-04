//! Host overrides at send time: the name stays in the URL and the Host header while the
//! connection goes to the mapped address; the host policy judges that address, not the name.

use crate::network_support::resolve_in;

use apinata_core::{
    ConnectCause, ContextOptions, ExecError, ExecOptions, Executor, ResolvedRequest,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PLAIN: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn root(rows: &[(&str, &str, bool)]) -> String {
    let mut out = format!("{PLAIN}extensions:\n  apinata:\n    hostOverrides:\n");
    for (host, address, disabled) in rows {
        out.push_str(&format!(
            "      - host: \"{host}\"\n        address: \"{address}\"\n"
        ));
        if *disabled {
            out.push_str("        disabled: true\n");
        }
    }
    out
}

fn request(rows: &[(&str, &str, bool)], url: &str) -> ResolvedRequest {
    let r = resolve_in(&root(rows), url, ContextOptions::new());
    r.result.expect("resolves")
}

async fn answering(path_: &str, body: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path(path_))
        .respond_with(ResponseTemplate::new(200).set_body_string(body.to_owned()))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_mapped_name_reaches_the_mock_with_its_own_host_header() {
    let server = MockServer::start().await;
    let port = server.address().port();
    Mock::given(path("/ping"))
        .and(header("host", format!("fake.test:{port}").as_str()))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let req = request(
        &[("fake.test", "127.0.0.1", false)],
        &format!("http://fake.test:{port}/ping"),
    );
    let res = Executor::new(ExecOptions::new())
        .expect("client")
        .send(&req)
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    assert_eq!(res.remote_addr(), Some(*server.address()));
}

#[tokio::test]
async fn the_timeline_hop_shows_the_override() {
    let server = answering("/ping", "ok").await;
    let port = server.address().port();
    let req = request(
        &[("fake.test", "127.0.0.1", false)],
        &format!("http://fake.test:{port}/ping"),
    );
    let res = Executor::new(ExecOptions::new())
        .expect("client")
        .send(&req)
        .await
        .expect("sent");
    assert_eq!(
        res.timeline()[0].override_from(),
        Some("fake.test -> 127.0.0.1")
    );
    // A hop DNS answered shows nothing.
    let plain = request(&[], &format!("{}/ping", server.uri()));
    let res = Executor::new(ExecOptions::new())
        .expect("client")
        .send(&plain)
        .await
        .expect("sent");
    assert_eq!(res.timeline()[0].override_from(), None);
}

#[tokio::test]
async fn a_disabled_row_has_no_effect() {
    let server = answering("/ping", "ok").await;
    let port = server.address().port();
    let req = request(
        &[("fake.test", "127.0.0.1", true)],
        &format!("http://fake.test:{port}/ping"),
    );
    let err = tokio::time::timeout(
        Duration::from_secs(10),
        Executor::new(ExecOptions::new())
            .expect("client")
            .send(&req),
    )
    .await
    .expect("returns")
    .unwrap_err();
    assert!(
        matches!(
            err,
            ExecError::Connect {
                cause: ConnectCause::Dns,
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_address_port_is_used_when_the_url_names_none() {
    let server = answering("/ping", "ok").await;
    let port = server.address().port();
    let req = request(
        &[("fake.test", &format!("127.0.0.1:{port}"), false)],
        "http://fake.test/ping",
    );
    let res = Executor::new(ExecOptions::new())
        .expect("client")
        .send(&req)
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn a_redirect_to_an_overridden_name_follows_the_mapping() {
    let target = answering("/end", "arrived").await;
    let port = target.address().port();
    let first = MockServer::start().await;
    Mock::given(path("/go"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("http://fake.test:{port}/end")),
        )
        .mount(&first)
        .await;
    let req = request(
        &[("fake.test", "127.0.0.1", false)],
        &format!("{}/go", first.uri()),
    );
    let res = Executor::new(ExecOptions::new())
        .expect("client")
        .send(&req)
        .await
        .expect("sent");
    assert_eq!(res.text(), "arrived");
    assert_eq!(res.timeline()[0].override_from(), None);
    assert_eq!(
        res.timeline()[1].override_from(),
        Some("fake.test -> 127.0.0.1")
    );
}

#[tokio::test]
async fn changing_the_mapping_builds_a_new_client() {
    let one = answering("/ping", "one").await;
    let two = answering("/ping", "two").await;
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let to = |server: &MockServer| {
        request(
            &[(
                "fake.test",
                &format!("127.0.0.1:{}", server.address().port()),
                false,
            )],
            "http://fake.test/ping",
        )
    };
    assert_eq!(exec.send(&to(&one)).await.expect("one").text(), "one");
    assert_eq!(exec.send(&to(&two)).await.expect("two").text(), "two");
    assert_eq!(exec.send(&to(&one)).await.expect("again").text(), "one");
}

#[tokio::test]
async fn a_mapped_name_through_a_proxy_is_an_error_not_a_silent_skip() {
    let root = format!(
        "{PLAIN}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: 127.0.0.1\n      port: \"9\"\n      auth:\n        username: \"\"\n        password: \"\"\n      bypassProxy: \"\"\nextensions:\n  apinata:\n    hostOverrides:\n      - host: fake.test\n        address: 127.0.0.1\n"
    );
    let req = resolve_in(&root, "http://fake.test:1/", ContextOptions::new())
        .result
        .expect("resolves");
    let err = Executor::new(ExecOptions::new())
        .expect("client")
        .send(&req)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ExecError::HostOverrideViaProxy { hop: 0, .. }),
        "{err:?}"
    );
}

/// An executor whose policy trusts exactly `trusted` and records what it is asked.
fn guarded(trusted: &'static [&'static str]) -> (Executor, Arc<Mutex<Vec<String>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let policy = move |host: &str| {
        log.lock().unwrap().push(host.to_owned());
        trusted.contains(&host)
    };
    (
        Executor::new(ExecOptions::new().host_policy(Arc::new(policy))).expect("client"),
        asked,
    )
}

async fn refused(exec: &Executor, req: &ResolvedRequest) -> ExecError {
    tokio::time::timeout(Duration::from_secs(2), exec.send(req))
        .await
        .expect("refused without dialling")
        .unwrap_err()
}

#[tokio::test]
async fn under_a_policy_a_name_mapped_to_loopback_passes_without_asking() {
    let server = answering("/ping", "ok").await;
    let port = server.address().port();
    let (exec, asked) = guarded(&[]);
    let req = request(
        &[("fake.test", "127.0.0.1", false)],
        &format!("http://fake.test:{port}/ping"),
    );
    assert_eq!(exec.send(&req).await.expect("sent").status(), 200);
    assert!(asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn under_a_policy_a_name_mapped_off_loopback_is_judged_by_the_address() {
    let rows = [("fake.test", "203.0.113.5", false)];
    let (exec, asked) = guarded(&[]);
    let err = refused(&exec, &request(&rows, "http://fake.test:80/")).await;
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 0, host, mapped_from: Some(name), .. }
            if host == "203.0.113.5" && name == "fake.test"),
        "{err:?}"
    );
    assert_eq!(*asked.lock().unwrap(), ["203.0.113.5"]);
}

#[tokio::test]
async fn a_local_or_trusted_name_is_not_exempt_when_mapped_off_loopback() {
    for name in ["localhost", "api.localhost", "api.test"] {
        let rows = [(name, "203.0.113.5", false)];
        let (exec, _) = guarded(&["api.test"]);
        let err = refused(&exec, &request(&rows, &format!("http://{name}:80/"))).await;
        assert!(
            matches!(&err, ExecError::HostNotAllowed { host, mapped_from: Some(n), .. }
                if host == "203.0.113.5" && n == name),
            "{name}: {err:?}"
        );
    }
}

#[tokio::test]
async fn a_redirect_to_a_name_mapped_off_loopback_is_refused_at_its_hop() {
    let first = MockServer::start().await;
    Mock::given(path("/go"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://fake.test:80/next"),
        )
        .mount(&first)
        .await;
    let (exec, _) = guarded(&[]);
    let req = request(
        &[("fake.test", "203.0.113.5", false)],
        &format!("{}/go", first.uri()),
    );
    let err = refused(&exec, &req).await;
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 1, host, mapped_from: Some(n), .. }
            if host == "203.0.113.5" && n == "fake.test"),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_v4_mapped_loopback_passes_the_policy() {
    let (exec, asked) = guarded(&[]);
    let req = request(
        &[("fake.test", "::ffff:127.0.0.1", false)],
        "http://fake.test:9/",
    );
    let result = tokio::time::timeout(Duration::from_secs(2), exec.send(&req)).await;
    if let Ok(Err(err)) = result {
        assert!(!matches!(err, ExecError::HostNotAllowed { .. }), "{err:?}");
    }
    assert!(asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_resolver_judges_the_mapped_address_again_on_each_connection() {
    // The pre-hop check says yes, the resolver's second question says no, and nothing dials.
    let asked = Arc::new(Mutex::new(0usize));
    let counter = Arc::clone(&asked);
    let policy = move |_: &str| {
        let mut n = counter.lock().unwrap();
        *n += 1;
        *n == 1
    };
    let exec = Executor::new(ExecOptions::new().host_policy(Arc::new(policy))).expect("client");
    let req = request(&[("fake.test", "203.0.113.5", false)], "http://fake.test/");
    let err = refused(&exec, &req).await;
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 0, host, mapped_from: Some(n), .. }
            if host == "203.0.113.5" && n == "fake.test"),
        "{err:?}"
    );
    assert_eq!(*asked.lock().unwrap(), 2);
}
