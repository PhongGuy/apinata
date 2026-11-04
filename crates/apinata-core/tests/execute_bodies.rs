//! Multipart and file bodies on the wire, against bytes recorded from bruno-cli.

use crate::common;

use apinata_core::ExecError;
use common::{at, executor, setup};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const FIXTURE_WIRE: &str = "--<B>\r\nContent-Disposition: form-data; name=\"mp_str\"\r\n\r\nform_string\r\n--<B>\r\nContent-Disposition: form-data; name=\"mp_num\"\r\n\r\n42\r\n--<B>\r\nContent-Disposition: form-data; name=\"mp_bool\"\r\n\r\ntrue\r\n--<B>\r\nContent-Disposition: form-data; name=\"mp_obj\"\r\n\r\n{\"key\":\"value\"}\r\n--<B>--\r\n";

/// A POST of `url` with `under_http` indented under `http:`.
fn post(url: &str, under_http: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"{url}\"\n{under_http}")
}

/// The multipart fixture's fields with its typed variables already spelled out.
const FIXTURE_FIELDS: &str = "  body:\n    type: multipart-form\n    data:\n      - name: mp_str\n        type: text\n        value: form_string\n      - name: mp_num\n        type: text\n        value: \"42\"\n      - name: mp_bool\n        type: text\n        value: \"true\"\n      - name: mp_obj\n        type: text\n        value: '{\"key\":\"value\"}'\n";

async fn server_with(at_path: &str, template: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path(at_path))
        .respond_with(template)
        .mount(&server)
        .await;
    server
}

/// The content type and body of the server's `n`th request.
async fn received(server: &MockServer, n: usize) -> (Option<String>, Vec<u8>) {
    let all = server.received_requests().await.expect("recorded");
    let request = &all[n];
    let content_type = request
        .headers
        .get("content-type")
        .map(|v| v.to_str().expect("ascii").to_owned());
    (content_type, request.body.clone())
}

#[tokio::test]
async fn the_multipart_fixture_matches_bruno_cli_once_the_boundary_is_normalised() {
    let server = server_with("/", ResponseTemplate::new(200)).await;
    let text = post(&format!("{}/", server.uri()), FIXTURE_FIELDS);
    let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    executor()
        .send(&at(&c, &ctx, "r.yml").expect("resolves"))
        .await
        .expect("sent");
    let (content_type, body) = received(&server, 0).await;
    let content_type = content_type.expect("content-type");
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .expect("a generated boundary");
    assert_eq!(boundary.len(), 50, "{boundary}");
    assert_eq!(
        String::from_utf8(body).expect("utf-8"),
        FIXTURE_WIRE.replace("<B>", boundary)
    );
}

#[tokio::test]
async fn a_boundary_in_the_users_content_type_is_reused_and_others_get_one_appended() {
    let server = server_with("/", ResponseTemplate::new(200)).await;
    let url = format!("{}/", server.uri());
    let field = "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: \"1\"\n";
    let with_type = |value: &str| {
        post(
            &url,
            &format!("  headers:\n    - name: Content-Type\n      value: \"{value}\"\n{field}"),
        )
    };
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        (
            "own.yml",
            &with_type("multipart/form-data; boundary=XyZ123"),
        ),
        ("mixed.yml", &with_type("multipart/mixed")),
    ]);
    let exec = executor();
    for rel in ["own.yml", "mixed.yml"] {
        exec.send(&at(&c, &ctx, rel).expect(rel)).await.expect(rel);
    }
    let one_field = |b: &str| {
        format!("--{b}\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n1\r\n--{b}--\r\n")
    };
    let (content_type, body) = received(&server, 0).await;
    assert_eq!(
        content_type.as_deref(),
        Some("multipart/form-data; boundary=XyZ123")
    );
    assert_eq!(body, one_field("XyZ123").into_bytes());
    let (content_type, body) = received(&server, 1).await;
    let content_type = content_type.expect("content-type");
    let boundary = content_type
        .strip_prefix("multipart/mixed; boundary=")
        .expect("appended boundary");
    assert_eq!(body, one_field(boundary).into_bytes());
}

/// Sends a POST whose body (`under_http`) names a file that does not exist, and checks it
/// fails with `shown` as the path, and no secret, before the server sees anything.
async fn expect_unreadable_file(
    under_http: &str,
    shown: impl Fn(&std::path::Path) -> std::path::PathBuf,
) {
    let server = server_with("/", ResponseTemplate::new(200)).await;
    let text = format!(
        "{}runtime:\n  variables:\n    - name: dirvar\n      value: SECRET\n",
        post(&format!("{}/", server.uri()), under_http)
    );
    let (dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    match executor()
        .send(&at(&c, &ctx, "r.yml").expect("resolves"))
        .await
    {
        Err(e @ ExecError::File { .. }) => {
            let ExecError::File { path, .. } = &e else {
                unreachable!()
            };
            assert_eq!(path, &shown(dir.path()));
            assert!(!e.to_string().contains("SECRET"), "{e}");
        }
        other => panic!("expected ExecError::File, got {other:?}"),
    }
    let seen = server.received_requests().await.expect("recorded");
    assert!(seen.is_empty(), "{} requests sent", seen.len());
}

#[tokio::test]
async fn a_missing_multipart_file_fails_before_any_byte_is_sent() {
    expect_unreadable_file(
        "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: \"1\"\n      - name: up\n        type: file\n        value: [nope.bin]\n",
        |_| "nope.bin".into(),
    )
    .await;
}

#[tokio::test]
async fn a_missing_multipart_file_is_reported_as_written_without_its_secret() {
    expect_unreadable_file(
        "  body:\n    type: multipart-form\n    data:\n      - name: up\n        type: file\n        value: [\"{{dirvar}}/nope.bin\"]\n",
        |_| "{{dirvar}}/nope.bin".into(),
    )
    .await;
}

#[tokio::test]
async fn a_307_resends_the_identical_multipart_bytes() {
    let server = MockServer::start().await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(307).insert_header("location", "/b"))
        .mount(&server)
        .await;
    Mock::given(path("/b"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let text = post(
        &format!("{}/a", server.uri()),
        "  body:\n    type: multipart-form\n    data:\n      - name: up\n        type: file\n        value: [a.txt]\n",
    );
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("a.txt", "hello file\n"),
        ("r.yml", &text),
    ]);
    let res = executor()
        .send(&at(&c, &ctx, "r.yml").expect("resolves"))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    let first = received(&server, 0).await;
    assert!(first.1.ends_with(b"--\r\n"), "{first:?}");
    assert_eq!(received(&server, 1).await, first);
}

#[tokio::test]
async fn file_mode_sends_the_file_with_the_entrys_type_even_when_empty() {
    let server = server_with("/", ResponseTemplate::new(200)).await;
    let url = format!("{}/", server.uri());
    let entry = |content_type: &str| {
        post(
            &url,
            &format!(
                "  body:\n    type: file\n    data:\n      - filePath: files/b.json\n        contentType: \"{content_type}\"\n        selected: true\n"
            ),
        )
    };
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", ROOT),
        ("files/b.json", "{\"k\":1}"),
        ("typed.yml", &entry("application/json")),
        ("empty.yml", &entry("")),
    ]);
    let exec = executor();
    for rel in ["typed.yml", "empty.yml"] {
        exec.send(&at(&c, &ctx, rel).expect(rel)).await.expect(rel);
    }
    assert_eq!(
        received(&server, 0).await,
        (Some("application/json".to_owned()), b"{\"k\":1}".to_vec())
    );
    // Measured: axios sends `Content-Type: ` with an empty value for an empty entry type.
    assert_eq!(
        received(&server, 1).await,
        (Some(String::new()), b"{\"k\":1}".to_vec())
    );
}

#[tokio::test]
async fn a_missing_body_file_fails_before_any_byte_is_sent() {
    expect_unreadable_file(
        "  body:\n    type: file\n    data:\n      - filePath: nope.bin\n        contentType: x/y\n        selected: true\n",
        |root| root.join("nope.bin"),
    )
    .await;
}
