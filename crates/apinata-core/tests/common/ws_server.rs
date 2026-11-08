//! A scripted graphql-transport-ws (or legacy graphql-ws) peer on 127.0.0.1, plain or behind the test PKI's TLS, a raw
//! responder for broken handshakes, and the request and executor the subscription tests use.
#![allow(dead_code)] // each test file uses a different subset

use super::{ROOT, at, setup};
use crate::network_support::pki;
use apinata_core::sse::SseEvent;
use apinata_core::{
    ExecOptions, Executor, GlobalProxy, HttpVersionPolicy, NetworkSettings, ResolvedRequest,
    StreamEvent, StreamLimits, StreamOptions,
};
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

pub const QUERY: &str = "subscription Ticks($n: Int) { tick(n: $n) }";
pub const VARS: &str = "{\"n\": 2}";

/// One scripted action, played once the client's `subscribe` arrives.
#[derive(Clone)]
pub enum Step {
    /// `{"id":"1","type":"next","payload":<value>}`.
    Next(serde_json::Value),
    /// `{"id":"1","type":"error","payload":<value>}`.
    Error(serde_json::Value),
    /// `{"id":"1","type":"complete"}`.
    Complete,
    /// The protocol's `{"type":"ping"}`.
    Ping,
    /// The legacy keep-alive `{"type":"ka"}`.
    Ka,
    /// A text frame sent as is.
    Text(String),
    /// A close frame with this code and reason.
    Close(u16, &'static str),
    /// A close frame with no status code.
    CloseBare,
    /// Drops the connection without a close frame.
    Drop,
}

#[derive(Clone)]
pub struct WsScript {
    /// The subprotocol the 101 reply names; None names none.
    pub protocol: Option<&'static str>,
    /// Whether `connection_init` gets a `connection_ack`.
    pub ack: bool,
    pub steps: Vec<Step>,
    /// Played right after `connection_init` (after the ack, if any).
    pub greeting: Vec<Step>,
}

impl WsScript {
    /// A graphql-transport-ws peer that acks, then plays `steps` and keeps reading.
    pub fn new(steps: Vec<Step>) -> WsScript {
        WsScript {
            protocol: Some("graphql-transport-ws"),
            ack: true,
            steps,
            greeting: Vec::new(),
        }
    }
}

/// What the server saw, in order: `alpn: ..`, `path: ..`, `sec-websocket-protocol: ..`,
/// `cookie: ..`, `connection: ..`, `recv: <text>` per client text frame and `close: <code>`.
#[derive(Clone, Default)]
pub struct WsSeen(Arc<Mutex<Vec<String>>>);

impl WsSeen {
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().expect("seen").clone()
    }

    fn push(&self, line: String) {
        self.0.lock().expect("seen").push(line);
    }

    /// The lines once one satisfies `done`, waiting at most two seconds.
    pub async fn until(&self, done: impl Fn(&str) -> bool) -> Vec<String> {
        for _ in 0..200 {
            let lines = self.lines();
            if lines.iter().any(|l| done(l)) {
                return lines;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.lines()
    }
}

pub struct WsServer {
    pub port: u16,
    pub seen: WsSeen,
}

impl WsServer {
    pub fn http(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn https(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{path}", self.port)
    }
}

pub async fn ws_server(script: WsScript) -> WsServer {
    serve(script, None).await
}

/// `ws_server` behind TLS with `server.pem`, offering ALPN `h2, http/1.1`.
pub async fn wss_server(script: WsScript) -> WsServer {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(pki("server.pem"))
        .expect("server.pem")
        .collect::<Result<_, _>>()
        .expect("certs");
    let key = PrivateKeyDer::from_pem_file(pki("server.key")).expect("server.key");
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("tls config");
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    serve(script, Some(TlsAcceptor::from(Arc::new(config)))).await
}

async fn serve(script: WsScript, tls: Option<TlsAcceptor>) -> WsServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen = WsSeen::default();
    let record = seen.clone();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (script, seen, tls) = (script.clone(), record.clone(), tls.clone());
            tokio::spawn(async move {
                match tls {
                    None => session(tcp, script, seen).await,
                    Some(acceptor) => {
                        let Ok(stream) = acceptor.accept(tcp).await else {
                            return;
                        };
                        let alpn = stream.get_ref().1.alpn_protocol();
                        let alpn = alpn.map(|p| String::from_utf8_lossy(p).into_owned());
                        seen.push(format!("alpn: {}", alpn.unwrap_or_default()));
                        session(stream, script, seen).await;
                    }
                }
            });
        }
    });
    WsServer { port, seen }
}

async fn session<S: AsyncRead + AsyncWrite + Unpin>(stream: S, script: WsScript, seen: WsSeen) {
    let record = seen.clone();
    let protocol = script.protocol;
    let legacy = protocol == Some("graphql-ws");
    #[allow(clippy::result_large_err)] // tungstenite's `Callback` fixes this signature
    let callback = move |req: &Request, mut res: Response| -> Result<Response, ErrorResponse> {
        record.push(format!("path: {}", req.uri().path()));
        // One line per value, so a header sent twice shows twice.
        for name in ["sec-websocket-protocol", "cookie", "connection"] {
            for value in req.headers().get_all(name) {
                record.push(format!("{name}: {}", value.to_str().unwrap_or("?")));
            }
        }
        if let Some(chosen) = protocol {
            let value = chosen.parse().expect("header value");
            res.headers_mut().insert("sec-websocket-protocol", value);
        }
        Ok(res)
    };
    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    while let Some(Ok(message)) = ws.next().await {
        match message {
            Message::Text(text) => {
                seen.push(format!("recv: {}", text.as_str()));
                let kind = serde_json::from_str::<serde_json::Value>(text.as_str())
                    .ok()
                    .and_then(|v| v.get("type").and_then(|t| t.as_str().map(str::to_owned)));
                match kind.as_deref() {
                    Some("connection_init") => {
                        let ack = Message::text(r#"{"type":"connection_ack"}"#);
                        if script.ack && ws.send(ack).await.is_err() {
                            return;
                        }
                        if !play(&mut ws, &script.greeting, legacy).await {
                            return;
                        }
                    }
                    Some("subscribe" | "start") if !play(&mut ws, &script.steps, legacy).await => {
                        return;
                    }
                    _ => {}
                }
            }
            Message::Close(frame) => seen.push(match frame {
                Some(f) => format!("close: {}", u16::from(f.code)),
                None => "close: none".to_owned(),
            }),
            _ => {}
        }
    }
}

/// Plays `steps`, naming `Next` as legacy `data` when `legacy`; false when the connection is
/// gone or a step dropped it.
async fn play<S: AsyncRead + AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<S>,
    steps: &[Step],
    legacy: bool,
) -> bool {
    for step in steps {
        let text = match step {
            Step::Next(payload) => {
                let kind = if legacy { "data" } else { "next" };
                json!({"id": "1", "type": kind, "payload": payload}).to_string()
            }
            Step::Error(payload) => {
                json!({"id": "1", "type": "error", "payload": payload}).to_string()
            }
            Step::Complete => r#"{"id":"1","type":"complete"}"#.to_owned(),
            Step::Ping => r#"{"type":"ping"}"#.to_owned(),
            Step::Ka => r#"{"type":"ka"}"#.to_owned(),
            Step::Text(text) => text.clone(),
            Step::Close(code, reason) => {
                let frame = CloseFrame {
                    code: CloseCode::from(*code),
                    reason: (*reason).into(),
                };
                if ws.close(Some(frame)).await.is_err() {
                    return false;
                }
                continue;
            }
            Step::CloseBare => {
                if ws.close(None).await.is_err() {
                    return false;
                }
                continue;
            }
            Step::Drop => return false,
        };
        if ws.send(Message::text(text)).await.is_err() {
            return false;
        }
    }
    true
}

/// Answers the first request head on each connection with `reply`, then holds the connection
/// until the client leaves.
pub async fn raw_server(reply: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tcp.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                if tcp.write_all(reply.as_bytes()).await.is_err() {
                    return;
                }
                while matches!(tcp.read(&mut buf).await, Ok(n) if n > 0) {}
            });
        }
    });
    port
}

/// A GraphQL item POSTing `query` and `variables` to `url`, resolved in a collection whose
/// root file is `root`; `top` holds top-level blocks such as `settings:`.
pub fn graphql_request(
    root: &str,
    url: &str,
    query: &str,
    variables: &str,
    top: &str,
) -> ResolvedRequest {
    // JSON strings are valid YAML double-quoted scalars.
    let text = format!(
        "info:\n  name: s\n  type: graphql\ngraphql:\n  method: POST\n  url: \"{url}\"\n  body:\n    query: {}\n    variables: {}\n{top}",
        serde_json::to_string(query).expect("query"),
        serde_json::to_string(variables).expect("variables"),
    );
    let (_dir, c, ctx) = setup(&[("opencollection.yml", root), ("s.yml", &text)]);
    at(&c, &ctx, "s.yml").expect("resolves")
}

/// The standard subscription (`QUERY`, `VARS`) to `url`.
pub fn sub(url: &str) -> ResolvedRequest {
    graphql_request(ROOT, url, QUERY, VARS, "")
}

/// An executor under `policy` that trusts the test CA and uses no global proxy.
pub fn ws_exec(policy: HttpVersionPolicy) -> Executor {
    let network = NetworkSettings::new()
        .proxy(GlobalProxy::Off)
        .ca_file(Some(pki("ca.pem")));
    Executor::new(ExecOptions::new().http_version(policy).network(network)).expect("executor")
}

pub fn tick(n: u64) -> Step {
    Step::Next(json!({"data": {"tick": n}}))
}

/// The SSE event a transcript record parses to.
pub fn record(kind: &str, data: &str) -> SseEvent {
    SseEvent::new(data).with_event(kind)
}

/// The JSON of each client text frame in `lines`.
pub fn sent(lines: &[String]) -> Vec<serde_json::Value> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix("recv: "))
        .map(|t| serde_json::from_str(t).expect("client sent JSON"))
        .collect()
}

pub type Events = Arc<Mutex<Vec<StreamEvent>>>;

pub fn recording(limits: StreamLimits) -> (StreamOptions, Events) {
    let seen: Events = Arc::default();
    let sink = Arc::clone(&seen);
    let opts = StreamOptions::new()
        .limits(limits)
        .sink(move |e| sink.lock().expect("events").push(e));
    (opts, seen)
}

pub fn chunks(seen: &Events) -> Vec<u8> {
    seen.lock()
        .expect("events")
        .iter()
        .flat_map(|e| match e {
            StreamEvent::Chunk(b) => b.clone(),
            _ => Vec::new(),
        })
        .collect()
}
