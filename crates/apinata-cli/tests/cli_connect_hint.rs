//! Connect failures read as a hint in the run report: the same words as the app, with the
//! transport detail after them, and a `hint` field in the JSON that leaves the rest unchanged.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

use crate::common::{self, ROOT, dead_port, write_tree};

fn collection(url: &str) -> tempfile::TempDir {
    let request = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &request)])
}

/// A plain-http server that answers any bytes with a 400, on its own thread.
fn plain_http_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            );
        }
    });
    port
}

fn run(dir: &Path, extra: &[&str]) -> (i32, String) {
    let home = tempfile::TempDir::new().expect("home");
    let out = common::apinata()
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env_remove("XDG_RUNTIME_DIR")
        .timeout(Duration::from_secs(30))
        .arg("run")
        .arg(dir)
        .args(extra)
        .output()
        .expect("runs");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().expect("code"), all)
}

#[test]
fn a_refused_connection_reads_as_a_hint_line() {
    let url = format!("http://127.0.0.1:{}/x", dead_port());
    let dir = collection(&url);
    let (code, text) = run(dir.path(), &[]);
    assert_eq!(code, 2, "{text}");
    let want = format!(
        "{url}: Connection refused: Nothing is listening at 127.0.0.1:{}. Is the server running? (",
        url.rsplit_once(':').unwrap().1.split('/').next().unwrap()
    );
    assert!(text.contains(&want), "{text}");
}

#[test]
fn https_to_a_plain_server_says_the_url_may_need_http() {
    let url = format!("https://127.0.0.1:{}/x", plain_http_port());
    let dir = collection(&url);
    let (code, text) = run(dir.path(), &[]);
    assert_eq!(code, 2, "{text}");
    assert!(
        text.contains(&format!(
            "{url}: TLS error: 127.0.0.1 did not answer TLS. The URL may need http:// instead of https://. ("
        )),
        "{text}"
    );
}

#[test]
fn the_json_report_gains_a_hint_and_keeps_the_error_text() {
    let url = format!("http://127.0.0.1:{}/x", dead_port());
    let dir = collection(&url);
    let (code, report) = common::run_json(dir.path(), &[]);
    assert_eq!(code, 2);
    let row = &report["results"][0];
    assert_eq!(
        row["hint"],
        format!(
            "Connection refused: Nothing is listening at {}. Is the server running?",
            url.trim_start_matches("http://").trim_end_matches("/x")
        )
    );
    // The error text is what it was before the hint existed.
    let error = row["error"].as_str().expect("error");
    assert!(
        error.starts_with(&format!("{url}: could not connect: ")),
        "{error}"
    );
    for key in [
        "path",
        "name",
        "outcome",
        "status",
        "duration_ms",
        "warnings",
        "assertions",
        "post_response_vars",
        "tests",
        "console",
    ] {
        assert!(row.get(key).is_some(), "{key} missing in {row}");
    }
    assert_eq!(row["outcome"], "errored");
}

#[test]
fn a_failure_that_is_not_a_connect_error_has_no_hint_field() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://{{nope}}/\"\n",
        ),
    ]);
    let (_, report) = common::run_json(dir.path(), &[]);
    assert!(report["results"][0].get("hint").is_none(), "{report}");
}

/// A TLS server whose certificate (the core fixtures' server.pem) no default trust store knows.
fn untrusted_tls_port() -> u16 {
    tls_port("server")
}

/// A TLS server on the core fixture `stem` that answers every request with a 200.
fn tls_port(stem: &str) -> u16 {
    use std::sync::Arc;
    use tokio_rustls::rustls::pki_types::pem::PemObject;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use tokio_rustls::rustls::{ServerConfig, ServerConnection, StreamOwned};

    const NET: &str = "../apinata-core/tests/fixtures/network";
    let cert =
        CertificateDer::from_pem_file(format!("{}/{NET}/{stem}.pem", env!("CARGO_MANIFEST_DIR")))
            .expect("cert");
    let key =
        PrivateKeyDer::from_pem_file(format!("{}/{NET}/{stem}.key", env!("CARGO_MANIFEST_DIR")))
            .expect("key");
    let config = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .expect("tls config"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for tcp in listener.incoming() {
            let Ok(tcp) = tcp else { return };
            let Ok(conn) = ServerConnection::new(config.clone()) else {
                return;
            };
            let mut tls = StreamOwned::new(conn, tcp);
            // A verifying client hangs up mid-handshake and the read fails; a client that
            // skips verification gets a plain answer.
            let mut buf = [0u8; 4096];
            if tls.read(&mut buf).is_ok() {
                let _ = tls.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                );
                tls.conn.send_close_notify();
                let _ = tls.flush();
            }
        }
    });
    port
}

#[test]
fn an_untrusted_certificate_line_names_the_cli_flags_before_the_detail() {
    let url = format!("https://127.0.0.1:{}/x", untrusted_tls_port());
    let dir = collection(&url);
    let (code, text) = run(dir.path(), &[]);
    assert_eq!(code, 2, "{text}");
    let want = format!(
        "{url}: TLS error: The certificate of 127.0.0.1 is not trusted (unknown issuer or \
         self-signed). Trust its CA or turn certificate verification off. \
         Try --cacert <file> or --insecure. ("
    );
    assert!(text.contains(&want), "{text}");
}

#[test]
fn the_json_hint_stays_neutral_about_flags() {
    let url = format!("https://127.0.0.1:{}/x", untrusted_tls_port());
    let dir = collection(&url);
    let (_, report) = common::run_json(dir.path(), &[]);
    let hint = report["results"][0]["hint"].as_str().expect("hint");
    assert!(!hint.contains("--"), "{hint}");
}

const CA_LEAF: &str = "selfsigned-ca-leaf";

#[test]
fn a_self_signed_ca_certificate_line_names_the_cli_flags() {
    let url = format!("https://127.0.0.1:{}/x", tls_port(CA_LEAF));
    let dir = collection(&url);
    let (code, text) = run(dir.path(), &[]);
    assert_eq!(code, 2, "{text}");
    let want = format!(
        "{url}: TLS error: The certificate of 127.0.0.1 is not trusted (unknown issuer or \
         self-signed). Trust its CA or turn certificate verification off. \
         Try --cacert <file> or --insecure. ("
    );
    assert!(text.contains(&want), "{text}");
}

#[test]
fn a_self_signed_ca_certificate_json_hint_is_neutral() {
    let url = format!("https://127.0.0.1:{}/x", tls_port(CA_LEAF));
    let dir = collection(&url);
    let (_, report) = common::run_json(dir.path(), &[]);
    let hint = report["results"][0]["hint"].as_str().expect("hint");
    assert!(hint.contains("not trusted"), "{hint}");
    assert!(!hint.contains("--"), "{hint}");
}

#[test]
fn insecure_runs_against_a_self_signed_ca_certificate() {
    let url = format!("https://127.0.0.1:{}/x", tls_port(CA_LEAF));
    let dir = collection(&url);
    let (code, text) = run(dir.path(), &["--insecure"]);
    assert_eq!(code, 0, "{text}");
}
