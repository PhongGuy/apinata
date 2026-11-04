//! A raw 127.0.0.1 server that answers with a body written in paced pieces, for large downloads.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// What the connection does once the body bytes are written.
#[derive(Clone, Copy)]
pub enum Tail {
    /// Close the connection.
    Close,
    /// Keep it open and silent until the client leaves.
    Hold,
}

/// One canned answer: a hand-written head, the body bytes, how they are paced and the ending.
#[derive(Clone)]
pub struct Reply {
    pub head: String,
    pub body: Vec<u8>,
    pub piece: usize,
    pub gap: Duration,
    pub tail: Tail,
}

impl Reply {
    /// A 200 with a matching `Content-Length`, written in 64 KiB pieces and then closed.
    pub fn plain(body: Vec<u8>) -> Reply {
        let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
        Reply {
            head,
            body,
            piece: 64 * 1024,
            gap: Duration::ZERO,
            tail: Tail::Close,
        }
    }

    /// A 200 that promises `promised` bytes but sends only `body`, then ends as `tail` says.
    pub fn short(body: Vec<u8>, promised: usize, tail: Tail) -> Reply {
        Reply {
            head: format!("HTTP/1.1 200 OK\r\ncontent-length: {promised}\r\n\r\n"),
            tail,
            ..Reply::plain(body)
        }
    }

    /// A 200 with extra header lines (each ends in CRLF) and the given `Content-Length`.
    pub fn with_headers(body: Vec<u8>, extra: &str) -> Reply {
        let head = format!(
            "HTTP/1.1 200 OK\r\n{extra}content-length: {}\r\n\r\n",
            body.len()
        );
        Reply {
            head,
            ..Reply::plain(body)
        }
    }

    /// A chunked 200 with no `Content-Length`.
    pub fn chunked(body: Vec<u8>) -> Reply {
        Reply {
            head: "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n".to_owned(),
            ..Reply::plain(body)
        }
    }

    pub fn redirect(to: &str) -> Reply {
        Reply {
            head: format!("HTTP/1.1 302 Found\r\nlocation: {to}\r\ncontent-length: 0\r\n\r\n"),
            ..Reply::plain(Vec::new())
        }
    }

    pub fn paced(mut self, piece: usize, gap: Duration) -> Reply {
        self.piece = piece;
        self.gap = gap;
        self
    }
}

/// A body of `len` bytes that differs from place to place, so a shifted copy cannot match.
pub fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i % 251) as u8 ^ (i / 4093) as u8)
        .collect()
}

pub struct BodyServer {
    port: u16,
}

impl BodyServer {
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

/// Serves each reply to requests whose path starts with its prefix; the first match wins.
pub async fn body_server(routes: Vec<(&str, Reply)>) -> BodyServer {
    let routes: Arc<Vec<(String, Reply)>> = Arc::new(
        routes
            .into_iter()
            .map(|(prefix, reply)| (prefix.to_owned(), reply))
            .collect(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(serve(sock, Arc::clone(&routes)));
        }
    });
    BodyServer { port }
}

async fn serve(mut sock: TcpStream, routes: Arc<Vec<(String, Reply)>>) {
    let mut head = [0u8; 4096];
    let read = sock.read(&mut head).await.unwrap_or(0);
    let request = String::from_utf8_lossy(&head[..read]);
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let Some((_, reply)) = routes.iter().find(|(p, _)| path.starts_with(p.as_str())) else {
        return;
    };
    if sock.write_all(reply.head.as_bytes()).await.is_err() {
        return;
    }
    let chunked = reply.head.contains("transfer-encoding: chunked");
    for piece in reply.body.chunks(reply.piece.max(1)) {
        let ok = if chunked {
            let mut frame = format!("{:x}\r\n", piece.len()).into_bytes();
            frame.extend_from_slice(piece);
            frame.extend_from_slice(b"\r\n");
            sock.write_all(&frame).await
        } else {
            sock.write_all(piece).await
        };
        if ok.is_err() {
            return;
        }
        if !reply.gap.is_zero() {
            tokio::time::sleep(reply.gap).await;
        }
    }
    if chunked {
        let _ = sock.write_all(b"0\r\n\r\n").await;
    }
    if matches!(reply.tail, Tail::Hold) {
        let mut sink = [0u8; 64];
        while sock.read(&mut sink).await.is_ok_and(|n| n > 0) {}
    }
}
