//! The decoded response size and the body size limit.

use crate::common;

use apinata_core::{ExecOptions, Executor};
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
