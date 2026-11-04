//! The one-shot loopback listener that receives the authorization redirect.

use super::OAuth2Error;
use crate::cancel::CancelHandle;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use url::{Host, Url};

/// How long the listener waits for the redirect (R§5).
pub(crate) const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

/// The longest request head a connection may send, and how long it may take to send it.
const MAX_HEAD: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

const DONE_PAGE: &str = "Apinata received the sign-in response. You can close this tab.";
const BAD_PAGE: &str = "This sign-in response has no code or error.";
const MISSING_PAGE: &str = "Not found.";

/// How the redirect from the authorization server reaches Apinata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallbackPlan {
    /// Listen on `bind` (port 0 = ephemeral); `redirect` is the callback URL, its port replaced after binding.
    Listen { bind: SocketAddr, redirect: Url },
    /// The user pastes the URL the browser landed on; `redirect_uri` is the callback verbatim.
    Paste { redirect_uri: String },
}

impl CallbackPlan {
    /// Empty means `http://127.0.0.1/callback` on an ephemeral port.
    pub(crate) fn for_callback(callback_url: &str) -> CallbackPlan {
        let paste = || CallbackPlan::Paste {
            redirect_uri: callback_url.to_owned(),
        };
        let text = if callback_url.is_empty() {
            "http://127.0.0.1/callback"
        } else {
            callback_url
        };
        let Ok(redirect) = Url::parse(text) else {
            return paste();
        };
        if redirect.scheme() != "http" {
            return paste();
        }
        // `localhost` binds IPv4 loopback but keeps its name in the redirect URI.
        let ip = match redirect.host() {
            Some(Host::Ipv4(ip)) if ip == Ipv4Addr::LOCALHOST => IpAddr::V4(ip),
            Some(Host::Ipv6(ip)) if ip == Ipv6Addr::LOCALHOST => IpAddr::V6(ip),
            Some(Host::Domain("localhost")) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            _ => return paste(),
        };
        let bind = SocketAddr::new(ip, redirect.port().unwrap_or(0));
        CallbackPlan::Listen { bind, redirect }
    }
}

/// Where connections come from; tests wrap the real socket to inject accept errors.
pub(crate) trait Accept: Send + Sync {
    fn accept(&self) -> impl Future<Output = std::io::Result<(TcpStream, SocketAddr)>> + Send;
}

impl Accept for TcpListener {
    async fn accept(&self) -> std::io::Result<(TcpStream, SocketAddr)> {
        TcpListener::accept(self).await
    }
}

/// A bound loopback socket waiting for one authorization redirect.
pub(crate) struct Listener<A = TcpListener> {
    listener: A,
    path: String,
    redirect_uri: String,
}

impl Listener {
    /// Fails with `CallbackBind { address: bind.to_string(), message }`.
    pub(crate) async fn bind(bind: SocketAddr, redirect: &Url) -> Result<Listener, OAuth2Error> {
        let failed = |e: std::io::Error| OAuth2Error::CallbackBind {
            address: bind.to_string(),
            message: e.to_string(),
        };
        let listener = TcpListener::bind(bind).await.map_err(failed)?;
        let port = listener.local_addr().map_err(failed)?.port();
        let mut redirect = redirect.clone();
        // An http URL with a host always accepts a port.
        let _ = redirect.set_port(Some(port));
        Ok(Listener {
            listener,
            path: redirect.path().to_owned(),
            redirect_uri: redirect.to_string(),
        })
    }

    #[cfg(test)]
    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.listener
            .local_addr()
            .expect("a bound listener has an address")
    }

    /// Swaps the socket for a wrapper around it, keeping the path and redirect URI.
    #[cfg(test)]
    fn with_accept<B: Accept>(self, wrap: impl FnOnce(TcpListener) -> B) -> Listener<B> {
        Listener {
            listener: wrap(self.listener),
            path: self.path,
            redirect_uri: self.redirect_uri,
        }
    }
}

impl<A: Accept> Listener<A> {
    /// The redirect URL with the bound port, e.g. `http://127.0.0.1:54321/callback`.
    pub(crate) fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Answers requests until a GET to the callback path carries `code` or `error`, then returns
    /// its decoded query pairs. `CallbackTimeout` after `timeout`, `Cancelled` on `cancel`.
    /// The socket and every connection are closed when this returns.
    pub(crate) async fn wait(
        self,
        timeout: Duration,
        cancel: &CancelHandle,
    ) -> Result<Vec<(String, String)>, OAuth2Error> {
        // Each connection is served on its own task so a browser's silent preconnect cannot
        // block the callback; dropping the set on return aborts whatever is still open.
        let (tx, mut rx) = mpsc::channel(1);
        let mut connections = tokio::task::JoinSet::new();
        let accept = async {
            loop {
                tokio::select! {
                    accepted = self.listener.accept() => {
                        match accepted {
                            Ok((stream, _)) => {
                                connections.spawn(serve(stream, self.path.clone(), tx.clone()));
                            }
                            // A persistent error such as EMFILE would otherwise spin the CPU
                            // until the timeout; pause so finished connections can free slots.
                            Err(_) => tokio::time::sleep(ACCEPT_BACKOFF).await,
                        }
                    }
                    Some(pairs) = rx.recv() => return pairs,
                }
            }
        };
        match cancel.run(tokio::time::timeout(timeout, accept)).await {
            None => Err(OAuth2Error::Cancelled),
            Some(Err(_)) => Err(OAuth2Error::CallbackTimeout),
            Some(Ok(pairs)) => Ok(pairs),
        }
    }
}

/// Reads one request head, answers it with a fixed page, and passes on a finished callback's
/// query only after the page is written, so the browser always sees it.
async fn serve(mut stream: TcpStream, path: String, done: mpsc::Sender<Vec<(String, String)>>) {
    let Ok(Some(head)) = tokio::time::timeout(READ_TIMEOUT, read_head(&mut stream)).await else {
        return;
    };
    let (status, page, pairs) = match callback_query(&head, &path) {
        Answer::NotFound => ("404 Not Found", MISSING_PAGE, None),
        Answer::Incomplete => ("400 Bad Request", BAD_PAGE, None),
        Answer::Done(pairs) => ("200 OK", DONE_PAGE, Some(pairs)),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    if stream.write_all(response.as_bytes()).await.is_err() {
        return;
    }
    let _ = stream.shutdown().await;
    if let Some(pairs) = pairs {
        let _ = done.send(pairs).await;
    }
}

/// The request head up to its blank line, or `None` if the peer closes or sends too much first.
async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 || head.len() + n > MAX_HEAD {
            return None;
        }
        head.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8(head).ok()
}

enum Answer {
    NotFound,
    Incomplete,
    Done(Vec<(String, String)>),
}

/// Classifies `GET <target> HTTP/1.x`: only the callback path with `code` or `error` finishes.
fn callback_query(head: &str, path: &str) -> Answer {
    let line = head.lines().next().unwrap_or_default();
    let mut parts = line.split(' ');
    let (Some("GET"), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return Answer::NotFound;
    };
    if !version.starts_with("HTTP/1.") {
        return Answer::NotFound;
    }
    let (target_path, query) = target.split_once('?').unwrap_or((target, ""));
    if target_path != path {
        return Answer::NotFound;
    }
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    if pairs.iter().any(|(k, _)| k == "code" || k == "error") {
        Answer::Done(pairs)
    } else {
        Answer::Incomplete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    const LONG: Duration = Duration::from_secs(10);

    // Fails the first `fails` accepts (or all of them), counting every call.
    struct Failing {
        inner: TcpListener,
        fails: usize,
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Accept for Failing {
        async fn accept(&self) -> std::io::Result<(TcpStream, SocketAddr)> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.fails {
                // Yield like a real accept would, so a missing backoff spins visibly instead
                // of starving the test's timers inside one poll.
                tokio::task::yield_now().await;
                return Err(std::io::Error::other("EMFILE"));
            }
            self.inner.accept().await
        }
    }

    fn get(addr: SocketAddr, target: &str) -> String {
        let mut s = std::net::TcpStream::connect(addr).expect("connect");
        write!(
            s,
            "GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).expect("response");
        out
    }
    async fn visit(addr: SocketAddr, target: &'static str) -> String {
        tokio::task::spawn_blocking(move || get(addr, target))
            .await
            .expect("client thread")
    }
    // Connects until one is refused, for up to 2 s. A fork elsewhere in the test process can
    // hold a copy of the closed listening socket until its exec, so one connect may still land.
    // Only a refusal counts: a listener left open with a full backlog times out instead.
    fn port_closes(addr: SocketAddr) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match std::net::TcpStream::connect_timeout(&addr, left.max(Duration::from_millis(1))) {
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => return true,
                _ => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        false
    }
    fn listen(ip: IpAddr, port: u16, redirect: &str) -> CallbackPlan {
        CallbackPlan::Listen {
            bind: SocketAddr::new(ip, port),
            redirect: Url::parse(redirect).unwrap(),
        }
    }
    async fn bound(callback: &str) -> Listener {
        match CallbackPlan::for_callback(callback) {
            CallbackPlan::Listen { bind, redirect } => {
                Listener::bind(bind, &redirect).await.unwrap()
            }
            other => panic!("expected a listener, got {other:?}"),
        }
    }

    #[test]
    fn callback_urls_choose_listen_or_paste() {
        let v4 = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let v6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
        assert_eq!(
            CallbackPlan::for_callback(""),
            listen(v4, 0, "http://127.0.0.1/callback")
        );
        assert_eq!(
            CallbackPlan::for_callback("http://127.0.0.1/cb"),
            listen(v4, 0, "http://127.0.0.1/cb")
        );
        assert_eq!(
            CallbackPlan::for_callback("http://127.0.0.1:8765/cb"),
            listen(v4, 8765, "http://127.0.0.1:8765/cb")
        );
        assert_eq!(
            CallbackPlan::for_callback("http://[::1]/cb"),
            listen(v6, 0, "http://[::1]/cb")
        );
        assert_eq!(
            CallbackPlan::for_callback("http://localhost:9001/cb"),
            listen(v4, 9001, "http://localhost:9001/cb")
        );
        for paste in [
            "https://127.0.0.1/cb",
            "https://app.example.test/cb",
            "http://127.0.0.2/cb",
            "not a url",
        ] {
            assert_eq!(
                CallbackPlan::for_callback(paste),
                CallbackPlan::Paste {
                    redirect_uri: paste.to_owned()
                },
                "{paste}"
            );
        }
    }

    #[tokio::test]
    async fn an_ephemeral_listener_binds_loopback_and_returns_the_callback_query() {
        let l = bound("").await;
        let addr = l.local_addr();
        assert!(addr.ip().is_loopback());
        assert_ne!(addr.port(), 0);
        assert_eq!(
            l.redirect_uri(),
            format!("http://127.0.0.1:{}/callback", addr.port())
        );
        let cancel = CancelHandle::new();
        let (got, page) = tokio::join!(
            l.wait(LONG, &cancel),
            visit(addr, "/callback?code=c%2F0de&state=xyz")
        );
        assert_eq!(
            got.unwrap(),
            vec![
                ("code".to_owned(), "c/0de".to_owned()),
                ("state".to_owned(), "xyz".to_owned())
            ]
        );
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("You can close this tab"), "{page}");
        assert!(
            !page.contains("c/0de") && !page.contains("xyz"),
            "the page echoes the query"
        );
        assert!(port_closes(addr), "the listener is still open");
    }

    #[tokio::test]
    async fn other_paths_and_bare_callbacks_do_not_finish() {
        let l = bound("http://127.0.0.1/cb").await;
        let addr = l.local_addr();
        let cancel = CancelHandle::new();
        let (got, (favicon, bare, denied)) = tokio::join!(l.wait(LONG, &cancel), async {
            let favicon = visit(addr, "/favicon.ico").await;
            let bare = visit(addr, "/cb?state=s").await;
            let denied = visit(addr, "/cb?error=access_denied&state=s").await;
            (favicon, bare, denied)
        });
        assert!(favicon.starts_with("HTTP/1.1 404"), "{favicon}");
        assert!(bare.starts_with("HTTP/1.1 400"), "{bare}");
        assert!(denied.starts_with("HTTP/1.1 200"), "{denied}");
        assert_eq!(
            got.unwrap(),
            vec![
                ("error".to_owned(), "access_denied".to_owned()),
                ("state".to_owned(), "s".to_owned())
            ]
        );
    }

    #[tokio::test]
    async fn a_silent_connection_does_not_block_the_callback() {
        let l = bound("").await;
        let addr = l.local_addr();
        let _preconnect = std::net::TcpStream::connect(addr).unwrap();
        let cancel = CancelHandle::new();
        let (got, _) = tokio::join!(
            l.wait(LONG, &cancel),
            visit(addr, "/callback?code=a&state=b")
        );
        assert_eq!(got.unwrap()[0], ("code".to_owned(), "a".to_owned()));
    }

    #[tokio::test]
    async fn no_callback_in_time_is_a_timeout_and_closes_the_port() {
        let l = bound("").await;
        let addr = l.local_addr();
        let err = l
            .wait(Duration::from_millis(200), &CancelHandle::new())
            .await
            .unwrap_err();
        assert!(matches!(err, OAuth2Error::CallbackTimeout), "{err:?}");
        assert!(port_closes(addr), "the listener is still open");
    }

    #[tokio::test]
    async fn cancelling_stops_the_wait() {
        let l = bound("").await;
        let cancel = CancelHandle::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            stop.cancel();
        });
        let err = l.wait(LONG, &cancel).await.unwrap_err();
        assert!(matches!(err, OAuth2Error::Cancelled), "{err:?}");
    }

    #[tokio::test]
    async fn a_busy_fixed_port_is_callback_bind() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let callback = format!("http://127.0.0.1:{port}/cb");
        let CallbackPlan::Listen { bind, redirect } = CallbackPlan::for_callback(&callback) else {
            panic!("expected a listener");
        };
        let err = Listener::bind(bind, &redirect)
            .await
            .err()
            .expect("an error");
        match err {
            OAuth2Error::CallbackBind { address, message } => {
                assert_eq!(address, format!("127.0.0.1:{port}"));
                assert!(!message.is_empty());
            }
            other => panic!("expected CallbackBind, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn localhost_binds_ipv4_loopback_and_keeps_its_name() {
        let l = bound("http://localhost/cb").await;
        assert_eq!(l.local_addr().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(
            l.redirect_uri(),
            format!("http://localhost:{}/cb", l.local_addr().port())
        );
    }

    #[tokio::test]
    async fn a_persistent_accept_error_backs_off_instead_of_spinning() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        let listener = bound("http://127.0.0.1/callback")
            .await
            .with_accept(move |inner| Failing {
                inner,
                fails: usize::MAX,
                calls: counted,
            });
        let out = listener
            .wait(Duration::from_millis(450), &CancelHandle::new())
            .await;
        assert!(matches!(out, Err(OAuth2Error::CallbackTimeout)), "{out:?}");
        let n = calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!((3..=6).contains(&n), "{n} accept calls in 450 ms");
    }

    #[tokio::test]
    async fn the_callback_still_arrives_after_accept_errors() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        let listener = bound("http://127.0.0.1/callback").await;
        let addr = listener.local_addr();
        let listener = listener.with_accept(move |inner| Failing {
            inner,
            fails: 2,
            calls: counted,
        });
        let browser = visit(addr, "/callback?code=c0de&state=s");
        let cancel = CancelHandle::new();
        let (out, _) = tokio::join!(listener.wait(LONG, &cancel), browser);
        let pairs = out.unwrap();
        assert!(
            pairs.contains(&("code".to_owned(), "c0de".to_owned())),
            "{pairs:?}"
        );
        assert!(calls.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    }
}
