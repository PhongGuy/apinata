//! A scripted GraphQL-over-WebSocket peer for `apinata run` tests.
#![allow(dead_code)]

use futures_util::{SinkExt as _, StreamExt as _};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

/// A `next` (or legacy `data`) frame for operation "1" with `{"data":{"tick":n}}`.
pub fn tick(legacy: bool, n: u64) -> String {
    let kind = if legacy { "data" } else { "next" };
    format!(r#"{{"id":"1","type":"{kind}","payload":{{"data":{{"tick":{n}}}}}}}"#)
}

pub const COMPLETE: &str = r#"{"id":"1","type":"complete"}"#;

/// Answers with subprotocol `protocol`, acks `connection_init`, and after `subscribe` or `start`
/// sends `frames`, pausing `gap` before each, then keeps reading. Returns the port.
pub async fn ws_server(protocol: &'static str, frames: Vec<String>, gap: Duration) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let frames = frames.clone();
            tokio::spawn(async move {
                #[allow(clippy::result_large_err)] // tungstenite's `Callback` fixes this signature
                let callback =
                    move |_: &Request, mut res: Response| -> Result<Response, ErrorResponse> {
                        let value = protocol.parse().expect("header value");
                        res.headers_mut().insert("sec-websocket-protocol", value);
                        Ok(res)
                    };
                let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(tcp, callback).await else {
                    return;
                };
                while let Some(Ok(message)) = ws.next().await {
                    let Message::Text(text) = message else {
                        continue;
                    };
                    let kind = serde_json::from_str::<serde_json::Value>(text.as_str())
                        .ok()
                        .and_then(|v| v.get("type")?.as_str().map(str::to_owned));
                    match kind.as_deref() {
                        Some("connection_init") => {
                            let ack = Message::text(r#"{"type":"connection_ack"}"#);
                            if ws.send(ack).await.is_err() {
                                return;
                            }
                        }
                        Some("subscribe" | "start") => {
                            for frame in &frames {
                                tokio::time::sleep(gap).await;
                                if ws.send(Message::text(frame.clone())).await.is_err() {
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
    });
    port
}
