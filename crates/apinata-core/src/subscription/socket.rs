//! The graphql-transport-ws client loop: init, ack, one `subscribe`, then the server's
//! messages written as a GraphQL-over-SSE transcript.

use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::stream::{Sink, StreamEnd, StreamEvent};

/// How long a closing client waits for the server's close reply.
const CLOSE_WAIT: Duration = Duration::from_secs(1);

/// One subscription run over an upgraded socket.
pub(crate) struct Session<'a> {
    /// The `subscribe` message's payload.
    pub(crate) payload: Value,
    /// How long the server has to send `connection_ack`.
    pub(crate) ack_wait: Duration,
    /// Told `Chunk` per record as it is written.
    pub(crate) sink: Option<&'a Sink>,
}

/// What a run wrote and how it ended.
pub(crate) struct Transcript {
    pub(crate) bytes: Vec<u8>,
    pub(crate) size: u64,
    pub(crate) truncated: bool,
    pub(crate) end: StreamEnd,
}

/// Runs the protocol on `ws` until the server completes the operation or the socket ends.
pub(crate) async fn run<S: AsyncRead + AsyncWrite + Unpin>(
    mut ws: WebSocketStream<S>,
    session: Session<'_>,
) -> Transcript {
    let mut transcript = Transcript {
        bytes: Vec::new(),
        size: 0,
        truncated: false,
        end: StreamEnd::Error(String::new()),
    };
    let subscribe = json!({"id": "1", "type": "subscribe", "payload": session.payload});
    let acked = send(&mut ws, json!({"type": "connection_init"})).await
        && tokio::time::timeout(session.ack_wait, await_ack(&mut ws))
            .await
            .unwrap_or(false)
        && send(&mut ws, subscribe).await;
    if !acked {
        return transcript;
    }
    // Server messages until the operation or the socket ends; a server `complete` gets the
    // client's close 1000, a server close 1000 gets tungstenite's automatic reply.
    transcript.end = loop {
        let Some(Ok(message)) = ws.next().await else {
            break StreamEnd::Error(String::new());
        };
        match message {
            Message::Text(text) => {
                let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                let ours = value.get("id").and_then(Value::as_str) == Some("1");
                match value.get("type").and_then(Value::as_str) {
                    Some("next") if ours => {
                        write(&mut transcript, &session, "next", value.get("payload"));
                    }
                    Some("complete") if ours => {
                        write(&mut transcript, &session, "complete", None);
                        close(&mut ws).await;
                        break StreamEnd::Closed;
                    }
                    Some("ping") => {
                        send(&mut ws, json!({"type": "pong"})).await;
                    }
                    _ => {}
                }
            }
            Message::Close(frame) => {
                let normal = frame.is_some_and(|f| f.code == CloseCode::Normal);
                drain(&mut ws).await;
                break if normal {
                    StreamEnd::Closed
                } else {
                    StreamEnd::Error(String::new())
                };
            }
            _ => {}
        }
    };
    transcript
}

/// Reads until `connection_ack`, answering protocol pings; false when the socket ends first.
async fn await_ack<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>) -> bool {
    while let Some(Ok(message)) = ws.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let value = serde_json::from_str::<Value>(text.as_str()).unwrap_or_default();
        match value.get("type").and_then(Value::as_str) {
            Some("connection_ack") => return true,
            Some("ping") if !send(ws, json!({"type": "pong"})).await => return false,
            _ => {}
        }
    }
    false
}

async fn send<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>, v: Value) -> bool {
    ws.send(Message::text(v.to_string())).await.is_ok()
}

/// Appends one `event: <kind>` record with the compact payload as its data, and tells the sink.
fn write(transcript: &mut Transcript, session: &Session<'_>, kind: &str, payload: Option<&Value>) {
    // No payload (the complete record) writes a bare `data:` line.
    let record = match payload {
        Some(payload) => format!("event: {kind}\ndata: {payload}\n\n"),
        None => format!("event: {kind}\ndata:\n\n"),
    };
    transcript.size += record.len() as u64;
    transcript.bytes.extend_from_slice(record.as_bytes());
    if let Some(sink) = session.sink {
        sink(StreamEvent::Chunk(record.into_bytes()));
    }
}

/// Sends close 1000 and waits briefly for the server's reply.
async fn close<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>) {
    let frame = CloseFrame {
        code: CloseCode::Normal,
        reason: "".into(),
    };
    if ws.close(Some(frame)).await.is_ok() {
        drain(ws).await;
    }
}

/// Reads what is left so a queued close reply is flushed, for at most `CLOSE_WAIT`.
async fn drain<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>) {
    let rest = async { while let Some(Ok(_)) = ws.next().await {} };
    let _ = tokio::time::timeout(CLOSE_WAIT, rest).await;
}
