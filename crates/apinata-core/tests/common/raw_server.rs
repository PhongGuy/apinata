//! A raw 127.0.0.1 server that answers a request head with a fixed reply, for refused handshakes.
#![allow(dead_code)]

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

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
