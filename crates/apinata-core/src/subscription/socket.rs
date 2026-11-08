//! The GraphQL-over-WebSocket client loop: init, ack, one subscription, then the server's
//! messages written as a GraphQL-over-SSE transcript. The codec the handshake chose names the
//! messages (graphql-transport-ws or legacy graphql-ws); the loop itself is shared.

use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use super::handshake::Codec;
use crate::cancel::CancelHandle;
use crate::stream::{Sink, StreamBound, StreamEnd, StreamEvent, StreamLimits};

/// How long a closing client waits for the server's close reply.
const CLOSE_WAIT: Duration = Duration::from_secs(1);

/// One subscription run over an upgraded socket.
pub(crate) struct Session<'a> {
    /// The protocol the server chose.
    pub(crate) codec: Codec,
    /// The `subscribe` message's payload.
    pub(crate) payload: Value,
    /// How long the server has to send `connection_ack`.
    pub(crate) ack_wait: Duration,
    /// Told `Chunk` per record as it is written.
    pub(crate) sink: Option<&'a Sink>,
    /// Stop ends the run politely.
    pub(crate) cancel: &'a CancelHandle,
    /// The duration and event bounds; the duration counts from once `connection_init` is sent.
    pub(crate) limits: StreamLimits,
    /// Transcript bytes kept at most; the record crossing it is cut there.
    pub(crate) cap: u64,
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
    let began = tokio::time::Instant::now();
    let ack_at = began + session.ack_wait;
    let bound_at = session.limits.max_duration.map(|d| began + d);
    let mut acked = false;
    let mut events = 0u64;
    // Stop, then the ack deadline, then the time bound win over a ready message, so a missing
    // ack is reported even when the bound falls due at the same instant.
    transcript.end = loop {
        tokio::select! {
            biased;
            () = session.cancel.stopped() => {
                polite_close(&mut ws, session.codec, acked).await;
                break StreamEnd::Stopped;
            }
            () = tokio::time::sleep_until(ack_at), if !acked => {
                close(&mut ws, 4408).await;
                let ms = session.ack_wait.as_millis();
                break StreamEnd::Error(format!("no connection_ack within {ms} ms"));
            }
            () = async {
                match bound_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                polite_close(&mut ws, session.codec, acked).await;
                break StreamEnd::Bound(StreamBound::Time);
            }
            next = ws.next() => match next {
                Some(Ok(Message::Text(text))) => {
                    let value = serde_json::from_str::<Value>(text.as_str()).ok();
                    let kind = value.as_ref().and_then(|v| v.get("type")?.as_str());
                    let ours = value.as_ref().and_then(|v| v.get("id")?.as_str()) == Some("1");
                    match classify(session.codec, kind) {
                        Incoming::Ack if !acked => {
                            acked = true;
                            let subscribe = subscribe_message(session.codec, &session.payload);
                            if !bounded_send(&mut ws, subscribe).await {
                                break dropped();
                            }
                        }
                        Incoming::Ping => {
                            bounded_send(&mut ws, json!({"type": "pong"})).await;
                        }
                        // Another operation's messages and keep-alives are not ours.
                        Incoming::Ignore => {}
                        Incoming::Next | Incoming::Error | Incoming::Complete if !ours => {}
                        Incoming::Next => {
                            let payload = value.as_ref().and_then(|v| v.get("payload"));
                            // Every record counts, but only a `next` leaves the run open to end here.
                            if !write(&mut transcript, &session, "next", payload) {
                                polite_close(&mut ws, session.codec, acked).await;
                                break StreamEnd::BodyCap;
                            }
                            events += 1;
                            if session.limits.max_events.is_some_and(|n| events >= n) {
                                polite_close(&mut ws, session.codec, acked).await;
                                break StreamEnd::Bound(StreamBound::Events);
                            }
                        }
                        Incoming::Error => {
                            let payload = value.as_ref().and_then(|v| v.get("payload"));
                            let kept = write(&mut transcript, &session, "error", payload);
                            close(&mut ws, 1000).await;
                            break if kept { StreamEnd::Closed } else { StreamEnd::BodyCap };
                        }
                        Incoming::Complete => {
                            let kept = write(&mut transcript, &session, "complete", None);
                            close(&mut ws, 1000).await;
                            break if kept { StreamEnd::Closed } else { StreamEnd::BodyCap };
                        }
                        // The legacy server refused the connection; nothing was subscribed.
                        Incoming::Refused => {
                            close(&mut ws, 1000).await;
                            let payload = value.as_ref().and_then(|v| v.get("payload"));
                            let payload = payload.map(Value::to_string).unwrap_or_default();
                            let shown: String = payload.chars().take(200).collect();
                            break StreamEnd::Error(format!("connection_error from server: {shown}"));
                        }
                        // A second ack and any message the codec does not know.
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
                // Binary frames are deliberately ignored: both protocols speak text only.
                Some(Ok(_)) => {}
                // A TCP end without a close frame, reported either way by tungstenite.
                None | Some(Err(_)) => break dropped(),
            },
        }
    };
    transcript
}

/// A server message as the loop acts on it.
enum Incoming {
    Ack,
    Ping,
    Ignore,
    Next,
    Error,
    Complete,
    Refused,
    Unknown,
}

/// Names a server message's `type` under `codec`; legacy `data` is the transcript's `next`.
fn classify(codec: Codec, kind: Option<&str>) -> Incoming {
    match (codec, kind) {
        (_, Some("connection_ack")) => Incoming::Ack,
        (_, Some("error")) => Incoming::Error,
        (_, Some("complete")) => Incoming::Complete,
        (Codec::TransportWs, Some("next")) | (Codec::Legacy, Some("data")) => Incoming::Next,
        (Codec::TransportWs, Some("ping")) => Incoming::Ping,
        (Codec::TransportWs, Some("pong")) | (Codec::Legacy, Some("ka")) => Incoming::Ignore,
        (Codec::Legacy, Some("connection_error")) => Incoming::Refused,
        _ => Incoming::Unknown,
    }
}

/// The one operation's opening message: `subscribe`, or legacy `start`.
fn subscribe_message(codec: Codec, payload: &Value) -> Value {
    let kind = match codec {
        Codec::TransportWs => "subscribe",
        Codec::Legacy => "start",
    };
    json!({"id": "1", "type": kind, "payload": payload})
}

/// Ends the run from the client side: ends the operation if it was subscribed (legacy also ends
/// the connection), then closes.
async fn polite_close<S: AsyncRead + AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<S>,
    codec: Codec,
    acked: bool,
) {
    // Bounded, so a server that stops reading cannot hold Stop or a limit.
    if acked {
        let goodbye = match codec {
            Codec::TransportWs => vec![json!({"id": "1", "type": "complete"})],
            Codec::Legacy => vec![
                json!({"id": "1", "type": "stop"}),
                json!({"type": "connection_terminate"}),
            ],
        };
        for message in goodbye {
            bounded_send(ws, message).await;
        }
    }
    close(ws, 1000).await;
}

async fn send<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>, v: Value) -> bool {
    ws.send(Message::text(v.to_string())).await.is_ok()
}

/// `send` given at most `CLOSE_WAIT`, so a server that stops reading cannot stall the loop;
/// a send that times out counts as failed.
async fn bounded_send<S: AsyncRead + AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<S>,
    v: Value,
) -> bool {
    tokio::time::timeout(CLOSE_WAIT, send(ws, v))
        .await
        .unwrap_or(false)
}

/// Appends one `event: <kind>` record with the compact payload as its data, cut at the cap,
/// and tells the sink; false when the cap cut it.
fn write(
    transcript: &mut Transcript,
    session: &Session<'_>,
    kind: &str,
    payload: Option<&Value>,
) -> bool {
    // No payload (the complete record) writes a bare `data:` line.
    let record = match payload {
        Some(payload) => format!("event: {kind}\ndata: {payload}\n\n"),
        None => format!("event: {kind}\ndata:\n\n"),
    };
    transcript.size += record.len() as u64;
    let room = session.cap.saturating_sub(transcript.bytes.len() as u64);
    let room = usize::try_from(room).unwrap_or(usize::MAX);
    let kept = &record.as_bytes()[..record.len().min(room)];
    transcript.truncated = kept.len() < record.len();
    transcript.bytes.extend_from_slice(kept);
    if let Some(sink) = session.sink
        && !kept.is_empty()
    {
        sink(StreamEvent::Chunk(kept.to_vec()));
    }
    !transcript.truncated
}

/// Sends a close frame with `code` and waits briefly for the server's reply.
async fn close<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>, code: u16) {
    let frame = CloseFrame {
        code: CloseCode::from(code),
        reason: "".into(),
    };
    if let Ok(Ok(())) = tokio::time::timeout(CLOSE_WAIT, ws.close(Some(frame))).await {
        drain(ws).await;
    }
}

/// Reads what is left so a queued close reply is flushed, for at most `CLOSE_WAIT`.
async fn drain<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>) {
    let rest = async { while let Some(Ok(_)) = ws.next().await {} };
    let _ = tokio::time::timeout(CLOSE_WAIT, rest).await;
}
