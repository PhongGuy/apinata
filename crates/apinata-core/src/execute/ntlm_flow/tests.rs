use super::*;
use crate::execute::ExecOptions;
use crate::http_version::HttpVersionPolicy;
use crate::network::ProxyRule;
use hyper_util::client::legacy::connect::HttpInfo;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Answers each request head with `ok`; `close` ends every connection after one answer.
async fn server(close: bool) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut b = [0u8; 1];
                loop {
                    while !buf.ends_with(b"\r\n\r\n") {
                        if s.read(&mut b).await.unwrap_or(0) == 0 {
                            return;
                        }
                        buf.push(b[0]);
                    }
                    buf.clear();
                    let extra = if close { "connection: close\r\n" } else { "" };
                    let reply = format!("HTTP/1.1 200 OK\r\n{extra}content-length: 2\r\n\r\nok");
                    if s.write_all(reply.as_bytes()).await.is_err() || close {
                        return;
                    }
                }
            });
        }
    });
    (port, accepted)
}

fn key() -> ClientKey {
    ClientKey {
        rule: ProxyRule::Direct,
        identity: None,
        compress: true,
        overrides: Default::default(),
        http1_only: false,
    }
}

async fn get(c: &Client, port: u16) -> Result<reqwest::Response, reqwest::Error> {
    c.get(format!("http://127.0.0.1:{port}/")).send().await
}

fn local_port(res: &reqwest::Response) -> u16 {
    res.extensions()
        .get::<HttpInfo>()
        .expect("HttpInfo")
        .local_addr()
        .port()
}

#[tokio::test]
async fn two_requests_share_one_connection() {
    let (port, accepted) = server(false).await;
    let exec = Executor::new(ExecOptions::new()).unwrap();
    let c = exec.build_handshake_client(&key()).unwrap();
    let first = get(&c, port).await.unwrap();
    let a = local_port(&first);
    assert_eq!(first.bytes().await.unwrap(), "ok");
    let second = get(&c, port).await.unwrap();
    assert_eq!(local_port(&second), a);
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_second_connect_is_refused_with_a_recognisable_error() {
    let (port, _) = server(true).await;
    let exec = Executor::new(ExecOptions::new()).unwrap();
    let c = exec.build_handshake_client(&key()).unwrap();
    get(&c, port).await.unwrap().bytes().await.unwrap();
    let err = get(&c, port)
        .await
        .expect_err("the server closed, so a new connect is needed");
    assert!(is_second_connect(&err), "{err:?}");
}

#[tokio::test]
async fn every_handshake_client_gets_its_own_connect_allowance() {
    let (port, accepted) = server(true).await;
    let exec = Executor::new(ExecOptions::new()).unwrap();
    for _ in 0..2 {
        let c = exec.build_handshake_client(&key()).unwrap();
        get(&c, port).await.unwrap().bytes().await.unwrap();
    }
    assert_eq!(accepted.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_host_policy_and_prior_knowledge_do_not_change_the_pinning() {
    let (port, accepted) = server(false).await;
    let opts = ExecOptions::new()
        .host_policy(Arc::new(|_: &str| false))
        .http_version(HttpVersionPolicy::Http2PriorKnowledge);
    let exec = Executor::new(opts).unwrap();
    let c = exec.build_handshake_client(&key()).unwrap();
    for _ in 0..2 {
        let res = get(&c, port).await.unwrap();
        assert_eq!(res.version(), reqwest::Version::HTTP_11);
        res.bytes().await.unwrap();
    }
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_handshake_client_keeps_the_host_guard_resolver() {
    use crate::host_guard::{Lookup, refused_host};
    use std::future::Future;
    use std::net::IpAddr;
    use std::pin::Pin;

    // A lookup that must never be asked: the guard refuses the name before any query.
    struct Never(Arc<AtomicUsize>);
    impl Lookup for Never {
        fn lookup(
            &self,
            _: String,
        ) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<IpAddr>>> + Send>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(vec![IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)]) })
        }
    }
    let asked = Arc::new(AtomicUsize::new(0));
    let opts = ExecOptions::new().host_policy(Arc::new(|_: &str| false));
    let exec = Executor::with_lookup(opts, Arc::new(Never(Arc::clone(&asked)))).unwrap();
    let c = exec.build_handshake_client(&key()).unwrap();
    let err = c.get("http://evil.example:1/").send().await.unwrap_err();
    assert_eq!(refused_host(&err).as_deref(), Some("evil.example"));
    assert_eq!(asked.load(Ordering::SeqCst), 0);
}
