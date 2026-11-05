//! Per-hop timing of a send: DNS, connect (and TLS), wait and download.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tokio::task::Id;
use tower::{BoxError, Layer, Service};

/// What the connect phase of a [`HopTiming`] covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectKind {
    /// A plain TCP connect (an http target).
    Connect,
    /// TCP connect and the TLS handshake together; no public hook splits them.
    ConnectAndTls,
}

/// Where the time of one hop went. A phase is `None` when it was not measured, never zero.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HopTiming {
    /// Name resolution; `None` for an IP literal host or a reused connection.
    pub dns: Option<Duration>,
    /// Connecting, excluding DNS; `None` on a reused connection.
    pub connect: Option<Duration>,
    /// What `connect` covers.
    pub connect_kind: ConnectKind,
    /// The connect went through a proxy, so it includes the tunnel.
    pub via_proxy: bool,
    /// Time to first byte: from the end of connect (or the start of the send on a reused
    /// connection) to the response headers. It includes sending the request.
    pub wait: Duration,
    /// From the response headers to the end of the body.
    pub download: Duration,
    /// The hop used a pooled connection, so no connect phase was measured.
    pub reused: bool,
}

tokio::task_local! {
    /// The recorder of the send in progress; absent in any task the send spawned.
    static CURRENT: Arc<Recorder>;
}

/// Collects the DNS and connect spans of one hop's send. A span counts only when it completes in
/// the task that created the recorder: a connect the pool moved to a background task belongs to
/// no request and is dropped.
pub(crate) struct Recorder {
    task: Option<Id>,
    spans: Mutex<Spans>,
}

#[derive(Default)]
struct Spans {
    dns: Option<Duration>,
    connect_layer: Option<Duration>,
}

impl Recorder {
    pub(crate) fn new() -> Arc<Recorder> {
        Arc::new(Recorder {
            task: tokio::task::try_id(),
            spans: Mutex::new(Spans::default()),
        })
    }

    fn own_task(&self) -> bool {
        tokio::task::try_id() == self.task
    }

    fn spans(&self) -> std::sync::MutexGuard<'_, Spans> {
        self.spans.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a new connection was made. A failure without one most likely came from a pooled one.
    pub(crate) fn connected(&self) -> bool {
        self.spans().connect_layer.is_some()
    }

    /// Turns the spans into a hop's timing. `started` is when the send began, `headers_at` when
    /// the response headers arrived and `done` when its body was read.
    pub(crate) fn finish(
        &self,
        started: Instant,
        headers_at: Instant,
        done: Instant,
        tls: bool,
        via_proxy: bool,
    ) -> HopTiming {
        let Spans { dns, connect_layer } = std::mem::take(&mut *self.spans());
        // The layer span covers the resolver call, so DNS is taken out of it.
        let connect = connect_layer.map(|span| span.saturating_sub(dns.unwrap_or_default()));
        let before_wait = dns.unwrap_or_default() + connect.unwrap_or_default();
        HopTiming {
            dns,
            connect,
            connect_kind: if tls {
                ConnectKind::ConnectAndTls
            } else {
                ConnectKind::Connect
            },
            via_proxy,
            wait: headers_at
                .duration_since(started)
                .saturating_sub(before_wait),
            download: done.duration_since(headers_at),
            reused: connect_layer.is_none(),
        }
    }
}

/// Runs `fut` with `recorder` visible to the resolver and connector layer it drives.
pub(crate) async fn scope<F: Future>(recorder: &Arc<Recorder>, fut: F) -> F::Output {
    CURRENT.scope(Arc::clone(recorder), fut).await
}

/// The default lookup, used when no host policy resolves. Like hyper-util 0.1.21's `GaiResolver`
/// it runs `(host, 0).to_socket_addrs()` on a blocking thread and keeps the address order; the
/// connector replaces port 0 with the URL's.
pub(crate) struct SystemResolve;

impl Resolve for SystemResolve {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((name.as_str(), 0)).await?;
            let addrs: Addrs = Box::new(addrs.collect::<Vec<SocketAddr>>().into_iter());
            Ok::<Addrs, BoxError>(addrs)
        })
    }
}

/// Times every lookup `inner` answers for the current send, passing its answer through.
pub(crate) struct TimedResolve(pub(crate) Arc<dyn Resolve>);

impl Resolve for TimedResolve {
    fn resolve(&self, name: Name) -> Resolving {
        // A SOCKS connect hands even an IP literal to the resolver; no lookup happens, so no
        // DNS phase is recorded for it.
        let host = name.as_str().trim_start_matches('[').trim_end_matches(']');
        let recorder = match host.parse::<std::net::IpAddr>() {
            Ok(_) => None,
            Err(_) => CURRENT.try_with(Arc::clone).ok(),
        };
        let started = Instant::now();
        let fut = self.0.resolve(name);
        Box::pin(async move {
            let answer = fut.await;
            if let Some(rec) = recorder
                && rec.own_task()
            {
                let span = started.elapsed();
                let mut spans = rec.spans();
                spans.dns = Some(spans.dns.unwrap_or_default() + span);
            }
            answer
        })
    }
}

/// A pass-through connector layer that times the whole connect: DNS, TCP, proxy tunnel, TLS.
#[derive(Clone)]
pub(crate) struct TimedConnect;

#[derive(Clone)]
pub(crate) struct TimedConnectService<S>(S);

impl<S> Layer<S> for TimedConnect {
    type Service = TimedConnectService<S>;
    fn layer(&self, inner: S) -> TimedConnectService<S> {
        TimedConnectService(inner)
    }
}

// Generic over request and response because reqwest's connector types cannot be named.
impl<S, R> Service<R> for TimedConnectService<S>
where
    S: Service<R>,
    S::Error: Into<BoxError>,
    S::Future: Send + 'static,
    S::Response: 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<S::Response, BoxError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
        self.0.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: R) -> Self::Future {
        let recorder = CURRENT.try_with(Arc::clone).ok();
        let started = Instant::now();
        let fut = self.0.call(req);
        Box::pin(async move {
            let out = fut.await.map_err(Into::into);
            if let Some(rec) = recorder
                && out.is_ok()
                && rec.own_task()
            {
                rec.spans().connect_layer = Some(started.elapsed());
            }
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_guard::Lookup;
    use crate::{
        Collection, ContextOptions, ExecOptions, Executor, ResolvedRequest, VarContext, resolve,
    };
    use std::future::Future;
    use std::io;
    use std::net::{IpAddr, Ipv4Addr};
    use std::pin::Pin;
    use std::sync::Arc;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn resolved(url: &str) -> ResolvedRequest {
        let dir = tempfile::TempDir::new().unwrap();
        let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
        let req =
            format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n");
        std::fs::write(dir.path().join("opencollection.yml"), root).unwrap();
        std::fs::write(dir.path().join("r.yml"), req).unwrap();
        let c = Collection::open(dir.path()).unwrap();
        let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
        resolve(&c.tree().requests()[0], &ctx).unwrap()
    }

    fn timing(res: &crate::Response, hop: usize) -> HopTiming {
        res.timeline()[hop].timing.clone().expect("hop has timing")
    }

    const DELAY: Duration = Duration::from_millis(500);

    #[tokio::test]
    async fn a_delayed_response_lands_in_wait_not_download() {
        let server = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(DELAY))
            .mount(&server)
            .await;
        let exec = Executor::new(ExecOptions::new()).unwrap();
        let res = exec
            .send(&resolved(&format!("{}/slow", server.uri())))
            .await
            .unwrap();
        let t = timing(&res, 0);
        assert!(t.wait >= DELAY, "{t:?}");
        assert!(t.download < DELAY / 2, "{t:?}");
    }

    #[tokio::test]
    async fn a_slow_body_lands_in_download_not_wait() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut s, &mut buf).await;
            s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 4\r\nconnection: close\r\n\r\nab")
                .await
                .unwrap();
            s.flush().await.unwrap();
            tokio::time::sleep(DELAY).await;
            s.write_all(b"cd").await.unwrap();
        });
        let exec = Executor::new(ExecOptions::new()).unwrap();
        let res = exec
            .send(&resolved(&format!("http://127.0.0.1:{port}/")))
            .await
            .unwrap();
        let t = timing(&res, 0);
        // The server starts its sleep right after writing the headers, so the client sees them
        // a moment into it; the download is the sleep less that lag.
        assert!(t.download >= DELAY * 4 / 5, "{t:?}");
        assert!(t.wait < DELAY / 2, "{t:?}");
    }

    #[tokio::test]
    async fn an_ip_literal_has_no_dns_phase_but_a_connect() {
        let server = MockServer::start().await;
        let exec = Executor::new(ExecOptions::new()).unwrap();
        let res = exec.send(&resolved(&server.uri())).await.unwrap();
        let t = timing(&res, 0);
        assert_eq!(t.dns, None, "{t:?}");
        assert!(t.connect.is_some(), "{t:?}");
        assert_eq!(t.connect_kind, ConnectKind::Connect);
        assert!(!t.reused && !t.via_proxy, "{t:?}");
    }

    #[tokio::test]
    async fn a_second_send_reuses_the_connection() {
        let server = MockServer::start().await;
        let exec = Executor::new(ExecOptions::new()).unwrap();
        let req = resolved(&server.uri());
        let first = timing(&exec.send(&req).await.unwrap(), 0);
        let second = timing(&exec.send(&req).await.unwrap(), 0);
        assert!(!first.reused && first.connect.is_some(), "{first:?}");
        assert!(second.reused, "{second:?}");
        assert_eq!((second.dns, second.connect), (None, None), "{second:?}");
    }

    /// A lookup that sleeps, then answers loopback.
    struct SlowLookup;

    impl Lookup for SlowLookup {
        fn lookup(
            &self,
            _host: String,
        ) -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>> {
            Box::pin(async {
                tokio::time::sleep(DELAY).await;
                Ok(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)])
            })
        }
    }

    #[tokio::test]
    async fn a_slow_lookup_lands_in_dns_not_connect() {
        let server = MockServer::start().await;
        let opts = ExecOptions::new().host_policy(Arc::new(|_: &str| true));
        let exec = Executor::with_lookup(opts, Arc::new(SlowLookup)).unwrap();
        let url = format!("http://api.test:{}/", server.address().port());
        let t = timing(&exec.send(&resolved(&url)).await.unwrap(), 0);
        assert!(t.dns.expect("dns measured") >= DELAY, "{t:?}");
        assert!(t.connect.expect("connect measured") < DELAY / 2, "{t:?}");
    }

    /// A connector that takes `DELAY` to answer, standing in for a slow TCP connect.
    #[derive(Clone)]
    struct SlowConnector;

    impl Service<()> for SlowConnector {
        type Response = ();
        type Error = BoxError;
        type Future = Pin<Box<dyn Future<Output = Result<(), BoxError>> + Send>>;
        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, _: ()) -> Self::Future {
            Box::pin(async {
                tokio::time::sleep(DELAY).await;
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn a_slow_connect_lands_in_connect_not_dns() {
        let rec = Recorder::new();
        let mut svc = TimedConnect.layer(SlowConnector);
        scope(&rec, async { svc.call(()).await.unwrap() }).await;
        let t = rec.finish(Instant::now(), Instant::now(), Instant::now(), false, false);
        assert!(t.connect.expect("connect measured") >= DELAY, "{t:?}");
        assert_eq!(t.dns, None, "{t:?}");
        assert!(!t.reused, "{t:?}");
    }

    #[tokio::test]
    async fn a_connect_finishing_in_another_task_is_not_attributed() {
        let rec = Recorder::new();
        let mut svc = TimedConnect.layer(SlowConnector);
        // The task-local does not follow a spawn, and the task id differs besides.
        scope(&rec, async {
            tokio::spawn(async move { svc.call(()).await.unwrap() })
                .await
                .unwrap()
        })
        .await;
        let t = rec.finish(Instant::now(), Instant::now(), Instant::now(), false, false);
        assert!(t.reused && t.connect.is_none(), "{t:?}");
    }

    #[tokio::test]
    async fn every_hop_of_a_redirect_chain_has_its_own_timing() {
        let server = MockServer::start().await;
        Mock::given(path("/a"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/b"))
            .mount(&server)
            .await;
        Mock::given(path("/b"))
            .respond_with(ResponseTemplate::new(200).set_delay(DELAY))
            .mount(&server)
            .await;
        let exec = Executor::new(ExecOptions::new()).unwrap();
        let res = exec
            .send(&resolved(&format!("{}/a", server.uri())))
            .await
            .unwrap();
        assert_eq!(res.timeline().len(), 2);
        let (a, b) = (timing(&res, 0), timing(&res, 1));
        assert!(a.wait < DELAY / 2, "{a:?}");
        assert!(b.wait >= DELAY, "{b:?}");
        assert!(a.connect.is_some(), "{a:?}");
        assert!(b.reused, "{b:?}");
    }
}
