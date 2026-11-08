//! NTLM across redirects and through proxies.

use crate::common;
use crate::network_support;
use crate::ntlm_support;

use apinata_core::{
    ExecError, ExecOptions, Executor, GlobalProxy, NetworkSettings, NtlmCause, ResolvedProxy,
    ResolvedRequest,
};
use common::{executor, resolved_one};
use network_support::{closing_proxy, http_proxy, keeping_proxy};
use ntlm_support::{Options, PASSWORD, Stage, USER, ntlm_server};

const JSON: &str = "  body:\n    type: json\n    data: '{\"a\":1}'\n";

fn auth() -> String {
    format!(
        "  auth:\n    type: ntlm\n    username: {USER}\n    password: {PASSWORD}\n    domain: CORP\n"
    )
}

fn req(method: &str, url: &str, under: &str, settings: &str) -> ResolvedRequest {
    resolved_one(method, url, under, settings)
}

fn stages(s: &ntlm_support::NtlmServer) -> Vec<Stage> {
    s.seen().iter().map(|r| r.stage()).collect()
}

#[tokio::test]
async fn a_same_origin_302_renegotiates_with_a_bodiless_get() {
    let s = ntlm_server(Options::default()).await;
    let under = format!("{}{JSON}", auth());
    let res = executor()
        .send(&req(
            "POST",
            &format!("{}/redirect302", s.origin()),
            &under,
            "",
        ))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.text()), (200, "GET 0"));
    let seen = s.seen();
    let all = [Stage::Probe, Stage::Type1, Stage::Type3];
    assert_eq!(stages(&s), [all, all].concat());
    assert!(
        seen[..3]
            .iter()
            .all(|r| r.method == "POST" && r.body.len() == 7)
    );
    assert!(
        seen[3..]
            .iter()
            .all(|r| r.method == "GET" && r.body.is_empty() && r.target == "/ok")
    );
    assert_ne!(
        seen[0].conn, seen[3].conn,
        "a redirect handshakes on a new connection"
    );
    assert_eq!(res.timeline().len(), 6);
}

#[tokio::test]
async fn a_307_keeps_the_method_and_resends_the_body_on_every_leg() {
    let s = ntlm_server(Options::default()).await;
    let under = format!("{}{JSON}", auth());
    let res = executor()
        .send(&req(
            "POST",
            &format!("{}/redirect307", s.origin()),
            &under,
            "",
        ))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.text()), (200, "POST 7"));
    assert_eq!(s.seen().len(), 6);
    assert!(
        s.seen()
            .iter()
            .all(|r| r.method == "POST" && r.body.len() == 7)
    );
}

#[tokio::test]
async fn handshake_legs_do_not_spend_the_redirect_budget() {
    let s = ntlm_server(Options::default()).await;
    let res = executor()
        .send(&req(
            "GET",
            &format!("{}/redirect302", s.origin()),
            &auth(),
            "settings:\n  maxRedirects: 1\n",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn a_cross_origin_redirect_without_forwarding_ends_on_the_unauthenticated_401() {
    let b = ntlm_server(Options::default()).await;
    let a = ntlm_server(Options {
        redirect_to: Some(format!("{}/ok", b.origin())),
        ..Options::default()
    })
    .await;
    let settings = "settings:\n  forwardAuthorizationHeader: false\n";
    let res = executor()
        .send(&req(
            "GET",
            &format!("{}/redirect302", a.origin()),
            &auth(),
            settings,
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 401);
    let seen = b.seen();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].auth.is_none());
}

/// A to B to a third origin without forwarding: once the credential is stripped at B, later hops
/// stay plain. A third server stands for "back on A" because the fixture cannot name an origin
/// before it is bound; Bruno compares the previous hop (B) with the target, so the outcome is
/// the same for a return to A.
#[tokio::test]
async fn a_hop_after_leaving_the_origin_stays_plain_without_forwarding() {
    let c = ntlm_server(Options::default()).await;
    let b = ntlm_server(Options {
        redirect_to: Some(format!("{}/ok", c.origin())),
        open_redirects: true,
        ..Options::default()
    })
    .await;
    let a = ntlm_server(Options {
        redirect_to: Some(format!("{}/redirect302", b.origin())),
        ..Options::default()
    })
    .await;
    let settings = "settings:\n  forwardAuthorizationHeader: false\n";
    let res = executor()
        .send(&req(
            "GET",
            &format!("{}/redirect302", a.origin()),
            &auth(),
            settings,
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 401);
    assert!(b.seen().iter().all(|r| r.auth.is_none()));
    let at_c = c.seen();
    assert_eq!(at_c.len(), 1);
    assert!(at_c[0].auth.is_none());
}

#[tokio::test]
async fn a_cross_origin_redirect_with_forwarding_renegotiates() {
    let b = ntlm_server(Options::default()).await;
    let a = ntlm_server(Options {
        redirect_to: Some(format!("{}/ok", b.origin())),
        ..Options::default()
    })
    .await;
    let settings = "settings:\n  forwardAuthorizationHeader: true\n";
    let res = executor()
        .send(&req(
            "GET",
            &format!("{}/redirect302", a.origin()),
            &auth(),
            settings,
        ))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.text()), (200, "GET 0"));
    assert_eq!(stages(&b), [Stage::Probe, Stage::Type1, Stage::Type3]);
}

fn via(port: u16) -> Executor {
    let proxy = ResolvedProxy::new("http", "127.0.0.1", &port.to_string()).expect("proxy");
    let manual = GlobalProxy::Manual {
        proxy,
        bypass: String::new(),
    };
    Executor::new(
        ExecOptions::new()
            .insecure(true)
            .network(NetworkSettings::new().proxy(manual)),
    )
    .expect("executor")
}

/// Plain http through a proxy that keeps one upstream connection runs the whole handshake
/// absolute-form on that connection.
#[tokio::test]
async fn plain_http_through_a_keeping_proxy_completes_on_one_upstream_connection() {
    let s = ntlm_server(Options::default()).await;
    let proxy = keeping_proxy().await;
    let res = via(proxy.port)
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(), ""))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.text()), (200, "GET 0"));
    assert_eq!(stages(&s), [Stage::Probe, Stage::Type1, Stage::Type3]);
    assert_eq!(s.connections(), 1, "one upstream connection");
    assert!(
        s.seen()
            .iter()
            .all(|r| r.target == format!("{}/ok", s.origin())),
        "absolute-form on every leg"
    );
    assert_eq!(proxy.seen.lines().len(), 1, "one client connection");
}

/// A proxy that does not keep the connection fails the handshake and says why in plain words.
#[tokio::test]
async fn plain_http_through_a_closing_proxy_names_the_proxy() {
    let s = ntlm_server(Options::default()).await;
    let proxy = closing_proxy().await;
    let err = via(proxy.port)
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(), ""))
        .await
        .expect_err("not kept");
    assert!(
        matches!(
            err,
            ExecError::Ntlm {
                cause: NtlmCause::ConnectionNotKept,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(
        err.to_string().ends_with(
            "NTLM: the proxy did not keep one connection to the server for the handshake; \
             use https, or a proxy that keeps connections"
        ),
        "{err}"
    );
}

/// https through a CONNECT tunnel completes the handshake.
#[tokio::test]
async fn https_through_connect_completes() {
    let s = ntlm_server(Options {
        tls: true,
        ..Options::default()
    })
    .await;
    let proxy = http_proxy(None).await;
    let res = via(proxy.port)
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(), ""))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.text()), (200, "GET 0"));
    assert_eq!(stages(&s), [Stage::Probe, Stage::Type1, Stage::Type3]);
    assert!(proxy.seen.lines()[0].starts_with("CONNECT "));
}
