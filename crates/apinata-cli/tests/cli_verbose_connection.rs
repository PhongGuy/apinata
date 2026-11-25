//! `run --verbose` follows each exchange's line with its connection details: the status
//! description, the local address, the response header size and, over https, the TLS details.

use crate::common;

use common::{ROOT, apinata, write_tree};

fn collection(url: &str) -> tempfile::TempDir {
    let request = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &request)])
}

/// The stderr of a verbose run of `url`, with `extra` flags.
fn verbose_stderr(url: &str, extra: &[&str]) -> String {
    let dir = collection(url);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--format", "json", "--verbose"])
        .args(extra)
        .output()
        .expect("runs");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The indented detail lines printed right after the exchange line, without their indent.
fn details(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .skip_while(|l| !l.starts_with('['))
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .map(|l| l.trim_start().to_owned())
        .collect()
}

#[test]
fn an_https_exchange_lists_status_local_address_header_size_and_tls() {
    let port = common::tls_port("server");
    let stderr = verbose_stderr(&format!("https://127.0.0.1:{port}/x"), &["--insecure"]);
    let lines = details(&stderr);
    assert_eq!(lines.len(), 4, "{stderr}");
    assert_eq!(lines[0], "status: The request succeeded.", "{stderr}");
    assert!(lines[1].starts_with("local: 127.0.0.1:"), "{stderr}");
    let head = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    assert_eq!(lines[2], format!("response headers: {} B", head.len()));
    assert_eq!(
        lines[3],
        "tls: TLS 1.3, subject CN=localhost, issuer CN=apinata-test-ca, \
         expires 2126-09-17 07:15:32 UTC",
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_http_exchange_has_no_tls_line() {
    let s = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(404))
        .mount(&s)
        .await;
    let stderr = verbose_stderr(&format!("{}/missing", s.uri()), &[]);
    let lines = details(&stderr);
    assert_eq!(lines.len(), 3, "{stderr}");
    assert_eq!(
        lines[0], "status: The server cannot find the requested resource.",
        "{stderr}"
    );
    assert!(lines[1].starts_with("local: 127.0.0.1:"), "{stderr}");
    assert!(lines[2].starts_with("response headers: "), "{stderr}");
    assert!(!stderr.contains("tls:"), "{stderr}");
}
