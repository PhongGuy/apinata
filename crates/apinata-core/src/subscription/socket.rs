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

use crate::cancel::CancelHandle;
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
    /// Stop ends the run politely.
    pub(crate) cancel: &'a CancelHandle,
}

/// What a run wrote and how it ended.
pub(crate) struct Transcript {
    pub(crate) bytes: Vec<u8>,
    pub(crate) size: u64,
    pub(crate) truncated: bool,
    pub(crate) end: StreamEnd,
}

/// Runs the protocol on `ws` until the operation ends, the socket ends or the caller stops it.
pub(crate) async fn run<S: AsyncRead + AsyncWrite + Unpin>(
    mut ws: WebSocketStream<S>,
    session: Session<'_>,
) -> Transcript {
    let mut transcript = Transcript {
        bytes: Vec::new(),
        size: 0,
        truncated: false,
        end: StreamEnd::Closed,
    };
    let dropped = || StreamEnd::Error("closed by server: connection dropped".to_owned());
    if !send(&mut ws, json!({"type": "connection_init"})).await {
        transcript.end = dropped();
        return transcript;
    }
    let ack_at = tokio::time::Instant::now() + session.ack_wait;
    let mut acked = false;
    // Stop wins over a ready message; the ack deadline only counts until the ack arrives.
    transcript.end = loop {
        tokio::select! {
            biased;
            () = session.cancel.stopped() => {
                polite_close(&mut ws, acked).await;
                break StreamEnd::Stopped;
            }
            () = tokio::time::sleep_until(ack_at), if !acked => {
                close(&mut ws, 4408).await;
                let ms = session.ack_wait.as_millis();
                break StreamEnd::Error(format!("no connection_ack within {ms} ms"));
            }
            next = ws.next() => match next {
                Some(Ok(Message::Text(text))) => {
                    let value = serde_json::from_str::<Value>(text.as_str()).ok();
                    let kind = value.as_ref().and_then(|v| v.get("type")?.as_str());
                    let ours = value.as_ref().and_then(|v| v.get("id")?.as_str()) == Some("1");
                    match kind {
                        Some("connection_ack") if !acked => {
                            acked = true;
                            let subscribe =
                                json!({"id": "1", "type": "subscribe", "payload": session.payload});
                            if !send(&mut ws, subscribe).await {
                                break dropped();
                            }
                        }
                        Some("ping") => {
                            send(&mut ws, json!({"type": "pong"})).await;
                        }
                        // Another operation's messages and keep-alive replies are not ours.
                        Some("pong") => {}
                        Some("next" | "error" | "complete") if !ours => {}
                        Some("next") => {
                            let payload = value.as_ref().and_then(|v| v.get("payload"));
                            write(&mut transcript, &session, "next", payload);
                        }
                        Some("error") => {
                            let payload = value.as_ref().and_then(|v| v.get("payload"));
                            write(&mut transcript, &session, "error", payload);
                            close(&mut ws, 1000).await;
                            break StreamEnd::Closed;
                        }
                        Some("complete") => {
                            write(&mut transcript, &session, "complete", None);
                            close(&mut ws, 1000).await;
                            break StreamEnd::Closed;
                        }
                        _ => {
                            close(&mut ws, 4400).await;
                            let shown: String = text.as_str().chars().take(200).collect();
                            break StreamEnd::Error(format!(
                                "unexpected message from server: {shown}"
                            ));
                        }
                    }
                }
                // tungstenite queues the close reply; draining flushes it.
                Some(Ok(Message::Close(frame))) => {
                    drain(&mut ws).await;
                    break match frame {
                        None => StreamEnd::Error("closed by server: no status".to_owned()),
                        Some(f) if f.code == CloseCode::Normal => StreamEnd::Closed,
                        Some(f) => {
                            let line = format!("closed by server: {} {}", u16::from(f.code), f.reason.as_str());
                            StreamEnd::Error(line.trim_end().to_owned())
                        }
                    };
                }
                Some(Ok(_)) => {}
                // A TCP end without a close frame, reported either way by tungstenite.
                None | Some(Err(_)) => break dropped(),
            },
        }
    };
    transcript
}

/// Ends the run from the client side: completes the operation if it was subscribed, then closes.
async fn polite_close<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>, acked: bool) {
    if acked {
        send(ws, json!({"id": "1", "type": "complete"})).await;
    }
    close(ws, 1000).await;
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

/// Sends a close frame with `code` and waits briefly for the server's reply.
async fn close<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>, code: u16) {
    let frame = CloseFrame {
        code: CloseCode::from(code),
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
