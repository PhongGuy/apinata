//! Wire defaults, bodies, the Response built from the reply, and transport errors.

use crate::common;

use apinata_core::{ConnectCause, ExecError};
use common::{executor, resolved_one};
use std::io::Write;
use std::time::Duration;
use wiremock::matchers::{any, body_string, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn sends_text_bodies_with_default_user_agent_and_accept() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users"))
        .and(body_string("{\"a\": 1}"))
        .and(header("content-type", "application/json"))
        .and(header(
            "user-agent",
            concat!("apinata/", env!("CARGO_PKG_VERSION")),
        ))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("X-Id", "1")
                .append_header("x-multi", "a")
                .append_header("x-multi", "b")
                .set_body_string("{\"ok\":true}"),
        )
        .mount(&server)
        .await;
    let body = "  body:\n    type: json\n    data: '{\"a\": 1}'\n";
    let url = format!("{}/users", server.uri());
    let res = executor()
        .send(&resolved_one("POST", &url, body, ""))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.status_text()), (201, "Created"));
    assert_eq!(res.text(), "{\"ok\":true}");
    assert_eq!(res.body(), b"{\"ok\":true}");
    assert_eq!(res.url(), url);
    let has = |k: &str, v: &str| res.headers().contains(&(k.to_owned(), v.to_owned()));
    assert!(
        has("x-id", "1") && has("x-multi", "a") && has("x-multi", "b"),
        "{:?}",
        res.headers()
    );
    // wiremock's header matcher splits on commas, so the Accept value is read back raw.
    let received = server.received_requests().await.expect("recorded");
    let accept = received[0]
        .headers
        .get("accept")
        .expect("sent")
        .to_str()
        .expect("ascii");
    assert_eq!(accept, "application/json, text/plain, */*");
}

#[tokio::test]
async fn own_user_agent_and_accept_replace_the_defaults() {
    let server = MockServer::start().await;
    Mock::given(header("user-agent", "mine/1"))
        .and(header("accept", "text/csv"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let own = "  headers:\n    - name: User-Agent\n      value: mine/1\n    - name: accept\n      value: text/csv\n";
    let res = executor()
        .send(&resolved_one("GET", &server.uri(), own, ""))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    let received = server.received_requests().await.expect("recorded");
    assert_eq!(received[0].headers.get_all("user-agent").iter().count(), 1);
}

#[tokio::test]
async fn form_bodies_encode_like_url_search_params() {
    let server = MockServer::start().await;
    // new URLSearchParams([["a","1 2"],["b","x&y"],["c","~*-._!é"]]).toString() in Node 24.
    Mock::given(body_string("a=1+2&b=x%26y&c=%7E*-._%21%C3%A9"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let form = "  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: 1 2\n      - name: b\n        value: x&y\n      - name: c\n        value: ~*-._!é\n";
    let res = executor()
        .send(&resolved_one("POST", &server.uri(), form, ""))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
}

/// `hello zstd` compressed by the `zstd` command line tool.
const ZSTD_HELLO: [u8; 23] = [
    0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x68, 0x51, 0x00, 0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x20, 0x7a,
    0x73, 0x74, 0x64, 0xcf, 0xdb, 0x60, 0x9c,
];

async fn zstd_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/zst"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-encoding", "zstd")
                .set_body_bytes(ZSTD_HELLO.to_vec()),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn zstd_is_decompressed_while_the_default_accept_encoding_stays_unchanged() {
    let server = zstd_server().await;
    let res = executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/zst", server.uri()),
            "",
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.text(), "hello zstd");
    let received = server.received_requests().await.expect("recorded");
    assert_eq!(
        received[0].headers.get("accept-encoding").expect("sent"),
        "gzip,deflate,br"
    );
}

#[tokio::test]
async fn an_omitted_accept_encoding_leaves_a_zstd_body_undecoded() {
    let server = zstd_server().await;
    let res = executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/zst", server.uri()),
            "",
            "settings:\n  omitHeaders:\n    - Accept-Encoding\n",
        ))
        .await
        .expect("sent");
    assert_eq!(res.body(), ZSTD_HELLO);
    let received = server.received_requests().await.expect("recorded");
    assert!(received[0].headers.get("accept-encoding").is_none());
}

#[tokio::test]
async fn gzip_is_decompressed_and_charsets_decode_the_text() {
    let server = MockServer::start().await;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(b"hello").expect("gzip");
    Mock::given(path("/gz"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-encoding", "gzip")
                .set_body_bytes(gz.finish().expect("gzip")),
        )
        .mount(&server)
        .await;
    Mock::given(path("/latin1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            vec![0x63, 0x61, 0x66, 0xe9],
            "text/plain; charset=iso-8859-1",
        ))
        .mount(&server)
        .await;
    let gz = executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/gz", server.uri()),
            "",
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(gz.text(), "hello");
    let received = server.received_requests().await.expect("recorded");
    let encodings = received[0]
        .headers
        .get("accept-encoding")
        .expect("sent")
        .to_str()
        .expect("ascii")
        .to_owned();
    assert!(
        encodings.contains("gzip") && encodings.contains("br"),
        "{encodings}"
    );
    let latin = executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/latin1", server.uri()),
            "",
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(latin.text(), "café");
    assert_eq!(latin.body(), [0x63, 0x61, 0x66, 0xe9]);
}

#[tokio::test]
async fn timeouts_and_refused_connections_are_errors() {
    let server = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .mount(&server)
        .await;
    let slow = resolved_one(
        "GET",
        &format!("{}/slow", server.uri()),
        "",
        "settings:\n  timeout: 100\n",
    );
    let err = executor().send(&slow).await.expect_err("times out");
    assert!(matches!(err, ExecError::Timeout { .. }), "{err:?}");

    let refused = resolved_one("GET", "http://127.0.0.1:1/", "", "");
    let err = executor().send(&refused).await.expect_err("refused");
    assert!(matches!(err, ExecError::Connect { .. }), "{err:?}");
    assert!(err.to_string().starts_with("http://127.0.0.1:1/"), "{err}");
}

#[tokio::test]
async fn status_text_is_the_canonical_reason_and_empty_when_unknown() {
    let server = MockServer::start().await;
    Mock::given(path("/teapot"))
        .respond_with(ResponseTemplate::new(418))
        .mount(&server)
        .await;
    Mock::given(path("/odd"))
        .respond_with(ResponseTemplate::new(599))
        .mount(&server)
        .await;
    let send = |p: &str| resolved_one("GET", &format!("{}{p}", server.uri()), "", "");
    let teapot = executor().send(&send("/teapot")).await.expect("sent");
    assert_eq!(
        (teapot.status(), teapot.status_text()),
        (418, "I'm a teapot")
    );
    let odd = executor().send(&send("/odd")).await.expect("sent");
    assert_eq!((odd.status(), odd.status_text()), (599, ""));
}

#[tokio::test]
async fn empty_bodies_send_content_length_zero_where_node_does() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    // Measured with axios 1.18.0 on Node 24: Node's http client writes `content-length: 0` for
    // an empty or absent body unless the method is GET, HEAD, DELETE, OPTIONS, TRACE or CONNECT.
    let empty_text = "  body:\n    type: text\n";
    let cases = [
        ("POST", empty_text, Some("0")),
        ("POST", "", Some("0")),
        ("PUT", "", Some("0")),
        ("PATCH", "", Some("0")),
        ("GET", empty_text, None),
        ("GET", "", None),
        ("DELETE", "", None),
    ];
    for (verb, under_http, _) in cases {
        let res = executor()
            .send(&resolved_one(verb, &server.uri(), under_http, ""))
            .await
            .expect("sent");
        assert_eq!(res.status(), 200);
    }
    let received = server.received_requests().await.expect("recorded");
    for (request, (verb, under_http, expected)) in received.iter().zip(cases) {
        let length = request
            .headers
            .get("content-length")
            .map(|v| v.to_str().expect("ascii"));
        assert_eq!(length, expected, "{verb} {under_http:?}");
        assert!(request.body.is_empty(), "{verb} {under_http:?}");
    }
}

#[tokio::test]
async fn an_unknown_charset_label_falls_back_to_utf8() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "café".as_bytes().to_vec(),
            "text/plain; charset=x-no-such-charset",
        ))
        .mount(&server)
        .await;
    let res = executor()
        .send(&resolved_one("GET", &server.uri(), "", ""))
        .await
        .expect("sent");
    assert_eq!(res.text(), "café");
}

#[tokio::test]
async fn a_refused_connection_names_its_cause_and_keeps_its_text() {
    let refused = resolved_one("GET", "http://127.0.0.1:1/", "", "");
    let err = executor().send(&refused).await.expect_err("refused");
    assert!(
        matches!(
            err,
            ExecError::Connect {
                cause: ConnectCause::Refused,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(
        err.to_string()
            .starts_with("http://127.0.0.1:1/: could not connect: "),
        "{err}"
    );
}
