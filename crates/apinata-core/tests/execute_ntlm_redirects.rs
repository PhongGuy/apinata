//! NTLM across redirects and through proxies.

use crate::common;
use crate::network_support;
use crate::ntlm_support;
use crate::script_support;

use apinata_core::{
    ContextOptions, ExecError, ExecOptions, Executor, GlobalProxy, NetworkSettings, NtlmCause,
    ResolvedProxy, ResolvedRequest, RunOptions, Warning,
};
use common::{executor, resolved_one};
use network_support::{closing_proxy, http_proxy, keeping_proxy, socks5_proxy};
use ntlm_support::{Options, PASSWORD, Stage, USER, ntlm_server};
use script_support::{send_full, write_tree};

const JSON: &str = "  body:\n    type: json\n    data: '{\"a\":1}'\n";

fn auth() -> String {
    auth_with(PASSWORD)
}

fn auth_with(password: &str) -> String {
    format!(
        "  auth:\n    type: ntlm\n    username: {USER}\n    password: {password}\n    domain: CORP\n"
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
    through(ResolvedProxy::new("http", "127.0.0.1", &port.to_string()).expect("proxy"))
}

fn through(proxy: ResolvedProxy) -> Executor {
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
    let target = format!("GET {}/ok HTTP/1.1", s.origin());
    assert_eq!(proxy.seen.lines(), [target.clone(), target.clone(), target]);
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

/// Through SOCKS an http target is a raw tunnel, as direct, so the handshake runs as usual.
#[tokio::test]
async fn plain_http_through_socks5_completes_on_one_connection() {
    let s = ntlm_server(Options::default()).await;
    let proxy = socks5_proxy(None).await;
    let socks = ResolvedProxy::new("socks5", "127.0.0.1", &proxy.port.to_string()).expect("proxy");
    let res = through(socks)
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(), ""))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.text()), (200, "GET 0"));
    assert_eq!(stages(&s), [Stage::Probe, Stage::Type1, Stage::Type3]);
    assert_eq!(s.connections(), 1, "one connection");
    assert_eq!(
        proxy.seen.lines().len(),
        1,
        "one tunnel: {:?}",
        proxy.seen.lines()
    );
}

/// Through SOCKS a closed connection is the server's doing and is told as such.
#[tokio::test]
async fn a_close_through_socks5_names_the_server() {
    let s = ntlm_server(Options {
        close_on_401: true,
        ..Options::default()
    })
    .await;
    let proxy = socks5_proxy(None).await;
    let socks = ResolvedProxy::new("socks5", "127.0.0.1", &proxy.port.to_string()).expect("proxy");
    let err = through(socks)
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(), ""))
        .await
        .expect_err("not kept");
    assert!(
        err.to_string()
            .ends_with("NTLM: the server closed the connection after the first answer"),
        "{err}"
    );
}

/// A challenge body cut short through a proxy keeps its own reason, not the proxy wording.
#[tokio::test]
async fn a_cut_challenge_body_through_a_proxy_keeps_its_reason() {
    let s = ntlm_server(Options {
        cut_401_body: true,
        ..Options::default()
    })
    .await;
    let proxy = keeping_proxy().await;
    let err = via(proxy.port)
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(), ""))
        .await
        .expect_err("cut");
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
    assert!(!err.to_string().contains("the proxy did not keep"), "{err}");
}

/// Proxy credentials go to the proxy on every leg and stay out of the timeline.
#[tokio::test]
async fn proxy_credentials_reach_the_proxy_on_every_leg_but_not_the_timeline() {
    let s = ntlm_server(Options::default()).await;
    let proxy = keeping_proxy().await;
    let credentialed = ResolvedProxy::new("http", "127.0.0.1", &proxy.port.to_string())
        .expect("proxy")
        .with_credentials("pu", "pp");
    let res = through(credentialed)
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(), ""))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    let auths: Vec<String> = (proxy.seen.lines().into_iter())
        .filter(|l| l.starts_with("auth: "))
        .collect();
    assert_eq!(auths, ["auth: Basic cHU6cHA="; 3]);
    assert_eq!(res.timeline().len(), 3);
    for hop in res.timeline() {
        assert!(
            !(hop.request_headers.iter())
                .any(|(n, _)| n.eq_ignore_ascii_case("proxy-authorization")),
            "{:?}",
            hop.request_headers
        );
    }
    assert!(
        s.seen()
            .iter()
            .all(|r| !r.headers.iter().any(|(n, _)| n == "proxy-authorization"))
    );
}

/// A wrong password through an http proxy may also be a proxy that switched the upstream
/// connection, so the final 401 carries a warning that says both.
#[tokio::test]
async fn a_final_401_through_an_http_proxy_warns_about_the_proxy() {
    let s = ntlm_server(Options::default()).await;
    let proxy = keeping_proxy().await;
    let res = via(proxy.port)
        .send(&req(
            "GET",
            &format!("{}/ok", s.origin()),
            &auth_with("nope"),
            "",
        ))
        .await
        .expect("a 401 is a response");
    assert_eq!(res.status(), 401);
    assert_eq!(res.warnings(), [Warning::NtlmProxyUnauthorized]);
    assert_eq!(
        Warning::NtlmProxyUnauthorized.to_string(),
        "NTLM through an HTTP proxy ended in 401: the credentials may be wrong, or the proxy may \
         not keep one connection to the server for the handshake; use https to rule out the proxy"
    );
}

#[tokio::test]
async fn a_final_401_without_a_proxy_has_no_proxy_warning() {
    let s = ntlm_server(Options::default()).await;
    let res = executor()
        .send(&req(
            "GET",
            &format!("{}/ok", s.origin()),
            &auth_with("nope"),
            "",
        ))
        .await
        .expect("a 401 is a response");
    assert_eq!(res.status(), 401);
    assert!(res.warnings().is_empty(), "{:?}", res.warnings());
}

#[tokio::test]
async fn a_final_401_through_socks5_has_no_proxy_warning() {
    let s = ntlm_server(Options::default()).await;
    let proxy = socks5_proxy(None).await;
    let socks = ResolvedProxy::new("socks5", "127.0.0.1", &proxy.port.to_string()).expect("proxy");
    let res = through(socks)
        .send(&req(
            "GET",
            &format!("{}/ok", s.origin()),
            &auth_with("nope"),
            "",
        ))
        .await
        .expect("a 401 is a response");
    assert_eq!(res.status(), 401);
    assert!(res.warnings().is_empty(), "{:?}", res.warnings());
}

#[tokio::test]
async fn a_signed_in_run_through_an_http_proxy_has_no_proxy_warning() {
    let s = ntlm_server(Options::default()).await;
    let proxy = keeping_proxy().await;
    let res = via(proxy.port)
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(), ""))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    assert!(res.warnings().is_empty(), "{:?}", res.warnings());
}

/// The runner lists the proxy warning with the request's other warnings.
#[tokio::test]
async fn the_runner_lists_the_ntlm_proxy_warning() {
    let s = ntlm_server(Options::default()).await;
    let proxy = keeping_proxy().await;
    let manual = GlobalProxy::Manual {
        proxy: ResolvedProxy::new("http", "127.0.0.1", &proxy.port.to_string()).expect("proxy"),
        bypass: String::new(),
    };
    let request = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{}/ok\"\n{}",
        s.origin(),
        auth_with("nope")
    );
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        ),
        ("r.yml", &request),
    ]);
    let (result, _) = send_full(
        dir.path(),
        "r.yml",
        RunOptions::new(),
        ContextOptions::new(),
        ExecOptions::new().network(NetworkSettings::new().proxy(manual)),
    )
    .await;
    assert_eq!(result.response.as_ref().map(|r| r.status()), Some(401));
    assert_eq!(result.warnings, [Warning::NtlmProxyUnauthorized]);
}
