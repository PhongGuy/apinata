//! The decoded response size and the body size limit.

use crate::common;

use apinata_core::{ExecError, ExecOptions, Executor};
use common::{executor, resolved_one};
use flate2::Compression;
use flate2::write::GzEncoder;
use std::io::Write;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mount(server: &MockServer, at: &str, template: ResponseTemplate) {
    Mock::given(path(at))
        .respond_with(template)
        .mount(server)
        .await;
}

#[tokio::test]
async fn size_is_the_decoded_body_length() {
    let plain = "x".repeat(1000);
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(plain.as_bytes()).expect("gzip");
    let wire = gz.finish().expect("gzip");
    assert!(wire.len() < 100);
    let server = MockServer::start().await;
    mount(
        &server,
        "/gz",
        ResponseTemplate::new(200)
            .insert_header("content-encoding", "gzip")
            .set_body_raw(wire, "text/plain"),
    )
    .await;
    let res = executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/gz", server.uri()),
            "",
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.size(), 1000);
    assert_eq!(res.body().len(), 1000);
    assert!(!res.truncated());
}

#[tokio::test]
async fn reading_stops_past_the_body_limit() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/big",
        ResponseTemplate::new(200).set_body_string("y".repeat(100)),
    )
    .await;
    let req = resolved_one("GET", &format!("{}/big", server.uri()), "", "");
    let capped = |limit| Executor::new(ExecOptions::new().max_body_bytes(limit)).expect("client");

    let res = capped(Some(10)).send(&req).await.expect("sent");
    assert!(res.truncated());
    assert_eq!(res.body(), b"yyyyyyyyyy");
    assert_eq!(res.text(), "yyyyyyyyyy");
    assert!(res.size() >= 10 && res.size() <= 100, "{}", res.size());

    for limit in [Some(100), None] {
        let res = capped(limit).send(&req).await.expect("sent");
        assert!(!res.truncated(), "{limit:?}");
        assert_eq!((res.body().len(), res.size()), (100, 100), "{limit:?}");
    }
}

#[tokio::test]
async fn a_zero_limit_truncates_any_body_but_not_an_empty_one() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/some",
        ResponseTemplate::new(200).set_body_string("z"),
    )
    .await;
    mount(&server, "/none", ResponseTemplate::new(200)).await;
    let exec = Executor::new(ExecOptions::new().max_body_bytes(Some(0))).expect("client");

    let some = resolved_one("GET", &format!("{}/some", server.uri()), "", "");
    let res = exec.send(&some).await.expect("sent");
    assert!(res.truncated());
    assert!(res.body().is_empty());

    let none = resolved_one("GET", &format!("{}/none", server.uri()), "", "");
    let res = exec.send(&none).await.expect("sent");
    assert!(!res.truncated());
    assert_eq!((res.body().len(), res.size()), (0, 0));
}

/// 1000 `x` bytes compressed by the `zstd` command line tool.
const ZSTD_1000_X: [u8; 21] = [
    0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x68, 0x45, 0x00, 0x00, 0x08, 0x78, 0x01, 0x00, 0xe4, 0x2b, 0x20,
    0x04, 0xe1, 0x00, 0xb7, 0x9c,
];

#[tokio::test]
async fn a_zstd_size_is_the_decoded_body_length() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/zst",
        ResponseTemplate::new(200)
            .insert_header("content-encoding", "zstd")
            .set_body_raw(ZSTD_1000_X.to_vec(), "text/plain"),
    )
    .await;
    let res = executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/zst", server.uri()),
            "",
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.text(), "x".repeat(1000));
    assert_eq!(res.size(), 1000);
}

/// 1000 `x` bytes compressed by the `brotli` command line tool.
const BROTLI_1000_X: [u8; 12] = [
    0xa1, 0x38, 0x1f, 0xc0, 0x2f, 0x81, 0x17, 0x8d, 0x05, 0x02, 0xa1, 0x01,
];

fn gzip(plain: &[u8]) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(plain).expect("gzip");
    gz.finish().expect("gzip")
}

fn header<'a>(res: &'a apinata_core::Response, name: &str) -> Option<&'a str> {
    res.headers()
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

async fn get(server: &MockServer, at: &str) -> apinata_core::Response {
    executor()
        .send(&resolved_one(
            "GET",
            &format!("{}{at}", server.uri()),
            "",
            "",
        ))
        .await
        .expect("sent")
}

#[tokio::test]
async fn a_compressed_body_keeps_the_headers_the_server_sent_and_both_sizes() {
    let wire = gzip("x".repeat(1000).as_bytes());
    let cases: [(&str, &str, Vec<u8>); 3] = [
        ("/gz", "gzip", wire.clone()),
        ("/br", "br", BROTLI_1000_X.to_vec()),
        ("/zst", "zstd", ZSTD_1000_X.to_vec()),
    ];
    let server = MockServer::start().await;
    for (at, coding, bytes) in &cases {
        mount(
            &server,
            at,
            ResponseTemplate::new(200)
                .insert_header("content-encoding", *coding)
                .set_body_raw(bytes.clone(), "text/plain"),
        )
        .await;
    }
    for (at, coding, bytes) in &cases {
        let res = get(&server, at).await;
        assert_eq!(res.text(), "x".repeat(1000), "{coding}");
        assert_eq!(res.size(), 1000, "{coding}");
        assert_eq!(res.encoded_size(), Some(bytes.len() as u64), "{coding}");
        assert_eq!(header(&res, "content-encoding"), Some(*coding));
        let length = bytes.len().to_string();
        assert_eq!(header(&res, "content-length"), Some(length.as_str()));
        let hop = &res.timeline()[0].response_headers;
        assert!(
            hop.iter()
                .any(|(n, v)| n == "content-encoding" && v == coding)
        );
    }
}

#[tokio::test]
async fn a_plain_body_has_no_encoded_size() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/plain",
        ResponseTemplate::new(200).set_body_string("hello"),
    )
    .await;
    let res = get(&server, "/plain").await;
    assert_eq!(res.encoded_size(), None);
    assert_eq!(header(&res, "content-length"), Some("5"));
}

#[tokio::test]
async fn an_empty_body_marked_gzip_reads_as_empty() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/none",
        ResponseTemplate::new(204).insert_header("content-encoding", "gzip"),
    )
    .await;
    let res = get(&server, "/none").await;
    assert_eq!((res.status(), res.size()), (204, 0));
    assert_eq!(header(&res, "content-encoding"), Some("gzip"));
}

#[tokio::test]
async fn a_corrupt_gzip_body_fails_the_send() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/bad",
        ResponseTemplate::new(200)
            .insert_header("content-encoding", "gzip")
            .set_body_raw(b"not gzip at all".to_vec(), "text/plain"),
    )
    .await;
    let req = resolved_one("GET", &format!("{}/bad", server.uri()), "", "");
    let err = executor().send(&req).await.expect_err("undecodable");
    assert!(matches!(err, ExecError::Transport { .. }), "{err:?}");
}

#[tokio::test]
async fn a_compressed_body_stops_decoding_at_the_limit() {
    // Four megabytes of zeros shrink to a few kilobytes, so the cap must hold on decoded bytes.
    let wire = gzip(&vec![0u8; 4 << 20]);
    let server = MockServer::start().await;
    mount(
        &server,
        "/bomb",
        ResponseTemplate::new(200)
            .insert_header("content-encoding", "gzip")
            .set_body_raw(wire, "application/octet-stream"),
    )
    .await;
    let exec = Executor::new(ExecOptions::new().max_body_bytes(Some(1000))).expect("client");
    let req = resolved_one("GET", &format!("{}/bomb", server.uri()), "", "");
    let res = exec.send(&req).await.expect("sent");
    assert!(res.truncated());
    assert_eq!(res.body().len(), 1000);
    assert!(res.size() < 1 << 20, "{}", res.size());
}
