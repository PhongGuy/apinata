//! A request to a Unix socket asks for approval like any host, keyed `unix:/path`; a socket
//! is never treated as local, since some (the Docker socket) give root.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::json;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

/// A socket in `dir` that answers every request with a 200 `ok` and closes.
fn serve(dir: &Path) -> PathBuf {
    let path = dir.join("d.sock");
    let listener = UnixListener::bind(&path).expect("bind");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                    )
                    .await;
            });
        }
    });
    path
}

async fn start(socket: &Path) -> TestServer {
    let url = format!("http://unix:{}:/v1.47/info", socket.display());
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &request("r", 1, "GET", &url)),
    ])
    .await
}

#[tokio::test]
async fn an_untrusted_socket_files_an_approval() {
    let dir = TempDir::new().expect("dir");
    let socket = serve(dir.path());
    let server = start(&socket).await;
    let message = server.err("send_request", json!({"path": "r.yml"})).await;
    let key = format!("unix:{}", socket.display());
    assert!(message.contains(&format!("`{key}`")), "{message}");
    assert!(
        message.contains("retry after the user approves"),
        "{message}"
    );
    let pending = server.inbox().pending().expect("inbox");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].host, key);
}

#[tokio::test]
async fn a_trusted_socket_is_sent_to_without_approval() {
    let dir = TempDir::new().expect("dir");
    let socket = serve(dir.path());
    let server = start(&socket).await;
    server.trust(&format!("unix:{}", socket.display()));
    let result = server.ok("send_request", json!({"path": "r.yml"})).await;
    assert_eq!(result["status"], 200, "{result}");
    assert!(server.inbox().pending().expect("inbox").is_empty());
}
