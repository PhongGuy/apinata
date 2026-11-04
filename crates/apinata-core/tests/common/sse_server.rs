//! A raw TCP server that answers with a chunked body written in timed steps.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub enum Step {
    /// One HTTP chunk.
    Chunk(String),
    Pause(Duration),
    /// The terminating zero chunk: a clean end of the body.
    Finish,
    /// Drop the connection mid-body.
    Abort,
}

/// One `data: <text>` event as a chunk.
pub fn event(data: &str) -> Step {
    Step::Chunk(format!("data: {data}\n\n"))
}

pub struct Script {
    head: String,
    head_delay: Duration,
    steps: Vec<Step>,
}

impl Script {
    pub fn with_type(content_type: &str, steps: Vec<Step>) -> Script {
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ntransfer-encoding: chunked\r\n\r\n"
        );
        Script {
            head,
            head_delay: Duration::ZERO,
            steps,
        }
    }
    pub fn sse(steps: Vec<Step>) -> Script {
        Script::with_type("text/event-stream", steps)
    }
    /// A script with a hand-written response head (include the final blank line).
    pub fn raw_head(head: String, steps: Vec<Step>) -> Script {
        Script {
            head,
            head_delay: Duration::ZERO,
            steps,
        }
    }
    pub fn head_delay(mut self, d: Duration) -> Script {
        self.head_delay = d;
        self
    }
}

/// Events of about 1 KiB each, enough to carry `total_bytes` of body, then a clean end.
pub fn large_events(total_bytes: usize) -> Vec<Step> {
    let data = "x".repeat(1000);
    let mut steps: Vec<Step> = (0..total_bytes.div_ceil(1000))
        .map(|_| event(&data))
        .collect();
    steps.push(Step::Finish);
    steps
}

/// `count` small numbered events written back to back with no pause. The body is left open
/// afterwards, so a client still has a live stream to stop.
pub fn fast_events(count: usize) -> Vec<Step> {
    (0..count).map(|i| event(&i.to_string())).collect()
}

/// The body bytes `fast_events(count)` puts on the wire.
pub fn fast_bytes(count: usize) -> u64 {
    (0..count)
        .map(|i| format!("data: {i}\n\n").len() as u64)
        .sum()
}

/// Bytes the `/large` route streams: over the 1 MiB stream history cap.
pub const LARGE_BYTES: usize = 3 * 1024 * 1024;
/// Events the `/fast` route streams.
pub const FAST_COUNT: usize = 20_000;

/// Prefix routes and the fallback script.
type Routes = (Vec<(String, Arc<Script>)>, Arc<Script>);

pub struct SlowServer {
    port: u16,
    requests: Arc<AtomicUsize>,
}

impl SlowServer {
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// Serves `script` to every connection on a free 127.0.0.1 port. When the steps run out the
/// connection stays open until the client closes it.
pub async fn slow_sse_server(script: Script) -> SlowServer {
    routed_sse_server(Vec::new(), script).await
}

/// Like [`slow_sse_server`], but a request whose path starts with a route's prefix gets that
/// route's script and any other path gets `fallback`.
pub async fn routed_sse_server(routes: Vec<(&str, Script)>, fallback: Script) -> SlowServer {
    let routes: Vec<(String, Arc<Script>)> = routes
        .into_iter()
        .map(|(prefix, script)| (prefix.to_owned(), Arc::new(script)))
        .collect();
    let routes = Arc::new((routes, Arc::new(fallback)));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let requests = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&requests);
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(serve(sock, Arc::clone(&routes), Arc::clone(&count)));
        }
    });
    SlowServer { port, requests }
}

/// The QA server: `/large` streams [`LARGE_BYTES`] of events, `/fast` streams [`FAST_COUNT`]
/// events with no pause, and any other path holds a stream open with one event.
pub async fn qa_sse_server() -> SlowServer {
    routed_sse_server(
        vec![
            ("/large", Script::sse(large_events(LARGE_BYTES))),
            ("/fast", Script::sse(fast_events(FAST_COUNT))),
        ],
        Script::sse(vec![event("hello")]),
    )
    .await
}

async fn serve(mut sock: TcpStream, routes: Arc<Routes>, count: Arc<AtomicUsize>) {
    let mut head = [0u8; 4096];
    let read = sock.read(&mut head).await.unwrap_or(0);
    if read == 0 {
        return;
    }
    // The request line is `GET <path> HTTP/1.1`; the first matching prefix picks the script.
    let request = String::from_utf8_lossy(&head[..read]);
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let (table, fallback) = &*routes;
    let script = table
        .iter()
        .find(|(prefix, _)| path.starts_with(prefix.as_str()))
        .map_or(fallback, |(_, script)| script);
    count.fetch_add(1, Ordering::SeqCst);
    tokio::time::sleep(script.head_delay).await;
    if sock.write_all(script.head.as_bytes()).await.is_err() {
        return;
    }
    for step in &script.steps {
        match step {
            Step::Chunk(text) => {
                let frame = format!("{:x}\r\n{text}\r\n", text.len());
                if sock.write_all(frame.as_bytes()).await.is_err() {
                    return;
                }
            }
            Step::Pause(d) => tokio::time::sleep(*d).await,
            Step::Finish => {
                let _ = sock.write_all(b"0\r\n\r\n").await;
                return;
            }
            Step::Abort => return,
        }
    }
    let mut sink = [0u8; 64];
    while sock.read(&mut sink).await.is_ok_and(|n| n > 0) {}
}
