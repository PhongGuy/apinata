//! Shared by the network tests: collections with a `config` block, resolved with given options.
//! The fixtures task adds local proxies, a TLS server and the test PKI.
#![allow(dead_code)] // each test binary uses a different subset

use apinata_core::{
    Collection, ContextOptions, ResolveError, ResolvedRequest, VarContext, resolve,
};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// A collection root whose `config:` holds `config` (lines indented two spaces).
pub fn root_with(config: &str) -> String {
    format!("opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n{config}")
}

/// A custom-mode `proxy` block for `root_with`; `auth` None writes empty credentials.
pub fn custom_proxy(
    protocol: &str,
    host: &str,
    port: &str,
    auth: Option<(&str, &str)>,
    bypass: &str,
) -> String {
    let (user, pass) = auth.unwrap_or(("", ""));
    format!(
        "  proxy:\n    inherit: false\n    config:\n      protocol: \"{protocol}\"\n      hostname: \"{host}\"\n      port: \"{port}\"\n      auth:\n        username: \"{user}\"\n        password: \"{pass}\"\n      bypassProxy: \"{bypass}\"\n"
    )
}

/// A `clientCertificates` block for `root_with` of PEM entries `(domain, certificate, key,
/// passphrase)`; paths are single-quoted so absolute Windows paths survive.
pub fn pem_certificates(entries: &[(&str, &str, &str, &str)]) -> String {
    let mut out = String::from("  clientCertificates:\n");
    for (domain, cert, key, passphrase) in entries {
        out.push_str(&format!(
            "    - domain: \"{domain}\"\n      type: pem\n      certificateFilePath: '{cert}'\n      privateKeyFilePath: '{key}'\n"
        ));
        if !passphrase.is_empty() {
            out.push_str(&format!("      passphrase: \"{passphrase}\"\n"));
        }
    }
    out
}

/// A collection on disk and one request resolved in it; `dir` keeps the files alive.
pub struct Resolved {
    pub dir: TempDir,
    /// The collection root as `Collection` spells it, which relative paths join to.
    pub root: PathBuf,
    pub result: Result<ResolvedRequest, ResolveError>,
}

impl Resolved {
    pub fn request(&self) -> &ResolvedRequest {
        self.result.as_ref().expect("resolves")
    }
}

/// Write `root` as `opencollection.yml` and a GET of `url` as `r.yml`, then resolve it.
pub fn resolve_in(root: &str, url: &str, options: ContextOptions) -> Resolved {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("opencollection.yml"), root).expect("root");
    let req = format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n");
    fs::write(dir.path().join("r.yml"), req).expect("request");
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, options).expect("context");
    let file = c
        .tree()
        .requests()
        .iter()
        .find(|r| r.path().ends_with("r.yml"))
        .expect("r.yml");
    let result = resolve(file, &ctx);
    Resolved {
        root: c.root().to_path_buf(),
        dir,
        result,
    }
}

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::server::WebPkiClientVerifier;
use tokio_rustls::rustls::{RootCertStore, ServerConfig};

/// A file of the committed test PKI.
pub fn pki(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/network")
        .join(name)
}

/// The passphrase of `client-enc.key` and `client-trad.key`.
pub const PASSPHRASE: &str = "secret";

/// The CA as a root store, for the TLS server and for test clients.
pub fn test_roots() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    for ca in CertificateDer::pem_file_iter(pki("ca.pem")).expect("ca.pem") {
        roots.add(ca.expect("ca")).expect("root");
    }
    roots
}

/// What a local proxy saw, in order.
#[derive(Clone, Default)]
pub struct Seen(Arc<Mutex<Vec<String>>>);

impl Seen {
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().expect("seen").clone()
    }
    fn push(&self, line: String) {
        self.0.lock().expect("seen").push(line);
    }
}

/// A proxy listening on 127.0.0.1.
pub struct LocalProxy {
    pub port: u16,
    pub seen: Seen,
}

/// Reads an HTTP head through its blank line, a byte at a time so nothing after it is consumed.
async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        if stream.read(&mut byte).await.ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    String::from_utf8(head).ok()
}

/// Whether the header line `header` is named `name`, in any case.
fn named(header: &str, name: &str) -> bool {
    header
        .split(':')
        .next()
        .is_some_and(|k| k.trim().eq_ignore_ascii_case(name))
}

/// An HTTP proxy. CONNECT opens a tunnel; any other request is forwarded as written (origin
/// servers accept absolute-form) with `connection: close`, so each request arrives on its own
/// connection. With `auth`, a different `Proxy-Authorization` gets 407. Records each request
/// line and, when present, `auth: <Proxy-Authorization>`.
pub async fn http_proxy(auth: Option<&'static str>) -> LocalProxy {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen = Seen::default();
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let Some(head) = read_head(&mut client).await else {
                    return;
                };
                let mut lines = head.split("\r\n").filter(|l| !l.is_empty());
                let request_line = lines.next().unwrap_or_default().to_owned();
                let headers: Vec<&str> = lines.collect();
                let given = headers
                    .iter()
                    .find(|h| named(h, "proxy-authorization"))
                    .and_then(|h| h.split_once(':'))
                    .map(|(_, v)| v.trim().to_owned());
                log.push(request_line.clone());
                if let Some(value) = &given {
                    log.push(format!("auth: {value}"));
                }
                if auth.is_some() && given.as_deref() != auth {
                    let _ = client
                        .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nproxy-authenticate: Basic realm=\"t\"\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                        .await;
                    return;
                }
                let mut parts = request_line.split(' ');
                let method = parts.next().unwrap_or_default();
                let target = parts.next().unwrap_or_default();
                if method == "CONNECT" {
                    let Ok(mut upstream) = TcpStream::connect(target).await else {
                        return;
                    };
                    let _ = client
                        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                        .await;
                    let _ = copy_bidirectional(&mut client, &mut upstream).await;
                    return;
                }
                let authority = target
                    .strip_prefix("http://")
                    .and_then(|rest| rest.split('/').next())
                    .unwrap_or_default();
                let Ok(mut upstream) = TcpStream::connect(authority).await else {
                    return;
                };
                let mut forwarded = format!("{request_line}\r\n");
                for h in &headers {
                    if ["connection", "proxy-connection", "proxy-authorization"]
                        .iter()
                        .any(|n| named(h, n))
                    {
                        continue;
                    }
                    forwarded.push_str(h);
                    forwarded.push_str("\r\n");
                }
                forwarded.push_str("connection: close\r\n\r\n");
                if upstream.write_all(forwarded.as_bytes()).await.is_ok() {
                    let _ = copy_bidirectional(&mut client, &mut upstream).await;
                }
            });
        }
    });
    LocalProxy { port, seen }
}

/// A plain-http proxy that opens one upstream connection per client connection and keeps it,
/// relaying each request without its `proxy-*` headers. Records each request line and, when
/// present, `auth: <Proxy-Authorization>`.
pub async fn keeping_proxy() -> LocalProxy {
    one_upstream_proxy(false).await
}

/// A plain-http proxy that serves exactly one request per client connection and tells the
/// client so with `connection: close` on the response, as a proxy that does not keep upstream
/// connections does. Records each request line.
pub async fn closing_proxy() -> LocalProxy {
    one_upstream_proxy(true).await
}

/// The `content-length` of an HTTP head, 0 when it has none.
fn content_length(head: &str) -> usize {
    head.split("\r\n")
        .find(|h| named(h, "content-length"))
        .and_then(|h| h.split_once(':'))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Copies exactly `len` bytes from `from` to `to`.
async fn relay_body(from: &mut TcpStream, to: &mut TcpStream, len: usize) -> Option<()> {
    let mut body = vec![0u8; len];
    from.read_exact(&mut body).await.ok()?;
    to.write_all(&body).await.ok()
}

async fn one_upstream_proxy(close: bool) -> LocalProxy {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen = Seen::default();
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            tokio::spawn(relay_requests(client, close, log.clone()));
        }
    });
    LocalProxy { port, seen }
}

/// Relays request after request from `client` over one upstream connection, opened to the
/// first request's authority. With `close`, the first answer says so and ends the session.
async fn relay_requests(mut client: TcpStream, close: bool, log: Seen) -> Option<()> {
    let mut upstream: Option<TcpStream> = None;
    loop {
        let head = read_head(&mut client).await?;
        let request_line = head.split("\r\n").next().unwrap_or_default().to_owned();
        log.push(request_line.clone());
        if let Some(h) = head.split("\r\n").find(|h| named(h, "proxy-authorization")) {
            log.push(format!("auth: {}", h.split_once(':')?.1.trim()));
        }
        if upstream.is_none() {
            let authority = request_line
                .split(' ')
                .nth(1)?
                .strip_prefix("http://")?
                .split('/')
                .next()?
                .to_owned();
            upstream = Some(TcpStream::connect(authority).await.ok()?);
        }
        let up = upstream.as_mut()?;
        let forwarded: String = head
            .split_inclusive("\r\n")
            .filter(|h| !named(h, "proxy-connection") && !named(h, "proxy-authorization"))
            .collect();
        up.write_all(forwarded.as_bytes()).await.ok()?;
        relay_body(&mut client, up, content_length(&head)).await?;
        let answer = read_head(up).await?;
        let mut sent: String = answer
            .strip_suffix("\r\n")
            .unwrap_or(&answer)
            .split_inclusive("\r\n")
            .filter(|h| !close || !named(h, "connection"))
            .collect();
        sent.push_str(if close {
            "connection: close\r\n\r\n"
        } else {
            "\r\n"
        });
        client.write_all(sent.as_bytes()).await.ok()?;
        relay_body(up, &mut client, content_length(&answer)).await?;
        if close {
            return Some(());
        }
    }
}

/// A SOCKS5 proxy (RFC 1928; RFC 1929 username and password when `credentials` is set).
/// Records `auth: <user>:<pass>` when credentials arrive and `connect <host>:<port>` per tunnel.
pub async fn socks5_proxy(credentials: Option<(&'static str, &'static str)>) -> LocalProxy {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen = Seen::default();
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            tokio::spawn(socks5_session(client, credentials, log.clone()));
        }
    });
    LocalProxy { port, seen }
}

async fn socks5_session(
    mut client: TcpStream,
    credentials: Option<(&'static str, &'static str)>,
    log: Seen,
) -> Option<()> {
    let mut greeting = [0u8; 2];
    client.read_exact(&mut greeting).await.ok()?;
    let mut methods = vec![0u8; usize::from(greeting[1])];
    client.read_exact(&mut methods).await.ok()?;
    let wanted = if credentials.is_some() { 2 } else { 0 };
    if greeting[0] != 5 || !methods.contains(&wanted) {
        client.write_all(&[5, 0xff]).await.ok()?;
        return None;
    }
    client.write_all(&[5, wanted]).await.ok()?;
    if let Some((user, pass)) = credentials {
        let mut head = [0u8; 2];
        client.read_exact(&mut head).await.ok()?;
        let mut name = vec![0u8; usize::from(head[1])];
        client.read_exact(&mut name).await.ok()?;
        let mut len = [0u8; 1];
        client.read_exact(&mut len).await.ok()?;
        let mut password = vec![0u8; usize::from(len[0])];
        client.read_exact(&mut password).await.ok()?;
        let (name, password) = (
            String::from_utf8_lossy(&name).into_owned(),
            String::from_utf8_lossy(&password).into_owned(),
        );
        log.push(format!("auth: {name}:{password}"));
        let ok = name == user && password == pass;
        client.write_all(&[1, if ok { 0 } else { 1 }]).await.ok()?;
        if !ok {
            return None;
        }
    }
    let mut request = [0u8; 4];
    client.read_exact(&mut request).await.ok()?;
    let host = match request[3] {
        1 => {
            let mut ip = [0u8; 4];
            client.read_exact(&mut ip).await.ok()?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await.ok()?;
            let mut name = vec![0u8; usize::from(len[0])];
            client.read_exact(&mut name).await.ok()?;
            String::from_utf8_lossy(&name).into_owned()
        }
        4 => {
            let mut ip = [0u8; 16];
            client.read_exact(&mut ip).await.ok()?;
            format!("[{}]", std::net::Ipv6Addr::from(ip))
        }
        _ => return None,
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await.ok()?;
    let target = format!("{host}:{}", u16::from_be_bytes(port));
    log.push(format!("connect {target}"));
    let mut upstream = TcpStream::connect(&target).await.ok()?;
    client
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .ok()?;
    copy_bidirectional(&mut client, &mut upstream).await.ok()?;
    Some(())
}

/// An HTTPS server on 127.0.0.1 with `server.pem` (SAN localhost and 127.0.0.1), answering
/// every request on a kept-alive connection with 200 `ok`. With `client_auth` the handshake
/// demands a client certificate issued by `ca.pem`.
pub struct TlsServer {
    pub port: u16,
    accepted: Arc<AtomicUsize>,
}

impl TlsServer {
    /// TCP connections accepted so far, whether or not their handshake succeeded.
    pub fn connections(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

pub async fn tls_server(client_auth: bool) -> TlsServer {
    tls_server_using("server.pem", "server.key", client_auth).await
}

/// `tls_server` presenting the fixture certificate `cert` with its key `key`.
pub async fn tls_server_using(cert: &str, key: &str, client_auth: bool) -> TlsServer {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(pki(cert))
        .expect("certificate fixture")
        .collect::<Result<_, _>>()
        .expect("certs");
    let key = PrivateKeyDer::from_pem_file(pki(key)).expect("key fixture");
    let builder = ServerConfig::builder();
    let config = if client_auth {
        let verifier = WebPkiClientVerifier::builder(Arc::new(test_roots()))
            .build()
            .expect("verifier");
        builder
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs, key)
    } else {
        builder.with_no_client_auth().with_single_cert(certs, key)
    }
    .expect("tls config");
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                // A failed handshake already sent its alert; there is nothing to answer.
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut pending = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let Ok(n) = tls.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    pending.extend_from_slice(&buf[..n]);
                    // Requests here are bodiless GETs: answer each complete head.
                    while let Some(end) = pending.windows(4).position(|w| w == b"\r\n\r\n") {
                        pending.drain(..end + 4);
                        let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok";
                        if tls.write_all(reply).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    TlsServer { port, accepted }
}

/// A port on 127.0.0.1 that refuses every connection and that no other process can take: the
/// socket is bound for the whole process but never listens.
pub fn closed_port() -> u16 {
    use socket2::{Domain, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("socket");
    socket
        .bind(&std::net::SocketAddr::from(([127, 0, 0, 1], 0)).into())
        .expect("bind");
    let port = socket
        .local_addr()
        .expect("addr")
        .as_socket()
        .expect("ip")
        .port();
    std::mem::forget(socket);
    port
}
