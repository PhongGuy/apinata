//! What the default headers look like on the wire, read from a raw 127.0.0.1 listener, and that
//! the list core shows for each one is what a send carries.

use crate::common;

use apinata_core::default_headers;
use common::{executor, resolved_one};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Sends one request with `top` as its settings and returns the raw header lines the server saw.
async fn raw_head(top: &str) -> Vec<String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let server = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.expect("accept");
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = tcp.read(&mut buf).await.expect("read");
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&buf[..n]);
        }
        tcp.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .expect("reply");
        String::from_utf8_lossy(&seen).into_owned()
    });
    executor()
        .send(&resolved_one("GET", &url, "", top))
        .await
        .expect("sent");
    let head = server.await.expect("server");
    head.lines().skip(1).map(str::to_owned).collect()
}

fn value(lines: &[String], name: &str) -> Option<String> {
    lines.iter().find_map(|l| {
        let (n, v) = l.split_once(':')?;
        n.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
    })
}

fn omit(names: &[&str]) -> String {
    let items: String = names.iter().map(|n| format!("    - {n}\n")).collect();
    format!("settings:\n  omitHeaders:\n{items}")
}

#[tokio::test]
async fn every_listed_default_shows_the_value_the_wire_carries() {
    let lines = raw_head("").await;
    for header in default_headers() {
        assert_eq!(
            value(&lines, &header.name).as_deref(),
            Some(header.value.as_str()),
            "{} in {lines:?}",
            header.name
        );
    }
}

#[tokio::test]
async fn the_list_names_the_four_omittable_defaults_in_order() {
    let names: Vec<_> = default_headers().into_iter().map(|h| h.name).collect();
    assert_eq!(
        names,
        ["User-Agent", "Accept", "Accept-Encoding", "Connection"]
    );
}

#[tokio::test]
async fn an_omitted_accept_encoding_is_absent_from_the_wire() {
    let lines = raw_head(&omit(&["Accept-Encoding"])).await;
    assert_eq!(value(&lines, "accept-encoding"), None, "{lines:?}");
    assert!(value(&lines, "user-agent").is_some(), "{lines:?}");
}

#[tokio::test]
async fn an_omitted_connection_is_absent_from_the_wire() {
    let lines = raw_head(&omit(&["Connection"])).await;
    assert_eq!(value(&lines, "connection"), None, "{lines:?}");
}

#[tokio::test]
async fn an_omitted_accept_still_sends_the_stacks_wildcard() {
    let lines = raw_head(&omit(&["Accept"])).await;
    assert_eq!(value(&lines, "accept").as_deref(), Some("*/*"), "{lines:?}");
}
