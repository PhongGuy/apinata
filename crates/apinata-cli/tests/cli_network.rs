//! `apinata run` network flags: the system proxy from the environment, --noproxy, the
//! collection proxy, --cacert, --ignore-truststore and --client-cert-config.

use crate::common;

use crate::common::dead_port;
use common::{ROOT, apinata, check, column, get, write_tree};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A stand-in HTTP proxy on 127.0.0.1 answering every request itself with 200 `proxied`,
/// recording each request line and `Proxy-Authorization` value.
fn fake_proxy() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            let mut lines = head.split("\r\n");
            let mut log = log.lock().expect("log");
            log.push(lines.next().unwrap_or_default().to_owned());
            for line in lines {
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("proxy-authorization")
                {
                    log.push(format!("auth: {}", value.trim()));
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-length: 7\r\nconnection: close\r\n\r\nproxied",
            );
        }
    });
    (port, seen)
}

const TARGET: &str = "http://127.0.0.1:9/x";

fn collection(root: &str, url: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", root),
        (
            "r.yml",
            &get("r", 1, url, &check("res.status", "eq", "200")),
        ),
    ])
}

/// `apinata run <dir> --format json <args>` with `env` set; returns the exit code, the report and stderr.
fn run(
    dir: &std::path::Path,
    args: &[&str],
    env: &[(&str, String)],
) -> (i32, serde_json::Value, String) {
    let mut cmd = apinata();
    cmd.arg("run")
        .arg(dir)
        .args(["--format", "json"])
        .args(args);
    for (name, value) in env {
        cmd.env(name, value);
    }
    let out = cmd.output().expect("runs");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let report = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out.status.code().expect("code"), report, stderr)
}

#[test]
fn the_system_proxy_from_the_environment_carries_the_request() {
    let (port, seen) = fake_proxy();
    let dir = collection(ROOT, TARGET);
    let (code, report, _) = run(
        dir.path(),
        &[],
        &[("HTTP_PROXY", format!("http://u:p@127.0.0.1:{port}"))],
    );
    assert_eq!(
        (code, column(&report, "outcome")),
        (0, vec!["passed".to_owned()])
    );
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            format!("GET {TARGET} HTTP/1.1"),
            "auth: Basic dTpw".to_owned()
        ]
    );
}

#[test]
fn noproxy_ignores_the_environment() {
    let (port, seen) = fake_proxy();
    let dir = collection(ROOT, TARGET);
    let (_, report, _) = run(
        dir.path(),
        &["--noproxy"],
        &[("HTTP_PROXY", format!("http://127.0.0.1:{port}"))],
    );
    assert_eq!(column(&report, "outcome"), ["errored"]);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn the_collection_proxy_applies_without_any_environment() {
    let (port, seen) = fake_proxy();
    let root = format!(
        "{ROOT}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: 127.0.0.1\n      port: \"{port}\"\n      auth:\n        username: \"\"\n        password: pw-SEKRIT\n      bypassProxy: \"\"\n"
    );
    let dir = collection(&root, TARGET);
    let (code, _, _) = run(dir.path(), &[], &[]);
    assert_eq!(code, 0);
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "request line and credentials"
    );
}

#[test]
fn a_proxy_failure_never_prints_the_password() {
    let closed = dead_port();
    let root = format!(
        "{ROOT}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: 127.0.0.1\n      port: \"{closed}\"\n      auth:\n        username: u\n        password: pw-SEKRIT\n      bypassProxy: \"\"\n"
    );
    let dir = collection(&root, TARGET);
    for format in ["json", "text"] {
        let out = apinata()
            .arg("run")
            .arg(dir.path())
            .args(["--format", format])
            .output()
            .expect("runs");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(all.contains(&format!("127.0.0.1:{closed}")), "{all}");
        assert!(!all.contains("SEKRIT"), "{all}");
    }
}

#[test]
fn ignore_truststore_requires_cacert() {
    let dir = collection(ROOT, TARGET);
    let (code, _, stderr) = run(dir.path(), &["--ignore-truststore"], &[]);
    assert_eq!(code, 3);
    assert!(stderr.contains("--cacert"), "{stderr}");
}

#[test]
fn a_missing_cacert_fails_the_https_request_naming_the_file() {
    let dir = collection(ROOT, "https://127.0.0.1:9/x");
    let missing = dir.path().join("missing-ca.pem");
    let (_, report, _) = run(
        dir.path(),
        &["--cacert", missing.to_str().expect("utf-8")],
        &[],
    );
    assert_eq!(column(&report, "outcome"), ["errored"]);
    assert!(
        column(&report, "error")[0].contains("missing-ca.pem"),
        "{report}"
    );
}

#[test]
fn client_cert_config_adds_certificates_after_the_collections() {
    let dir = collection(ROOT, "https://127.0.0.1:9/x");
    let config = dir.path().join("certs.json");
    std::fs::write(&config, r#"{"enabled": true, "certs": [{"domain": "127.0.0.1", "type": "pfx", "pfxFilePath": "c.p12", "passphrase": ""}]}"#).expect("write");
    let (_, report, _) = run(
        dir.path(),
        &["--client-cert-config", config.to_str().expect("utf-8")],
        &[],
    );
    assert!(
        column(&report, "error")[0].contains("openssl pkcs12"),
        "{report}"
    );
    std::fs::write(&config, r#"{"enabled": false, "certs": [{"domain": "127.0.0.1", "type": "pfx", "pfxFilePath": "c.p12"}]}"#).expect("write");
    let (_, report, _) = run(
        dir.path(),
        &["--client-cert-config", config.to_str().expect("utf-8")],
        &[],
    );
    assert!(
        !column(&report, "error")[0].contains("openssl pkcs12"),
        "{report}"
    );
}

#[test]
fn a_bad_client_cert_config_stops_before_any_request() {
    let dir = collection(ROOT, TARGET);
    let missing = dir.path().join("nope.json");
    let (code, _, stderr) = run(
        dir.path(),
        &["--client-cert-config", missing.to_str().expect("utf-8")],
        &[],
    );
    assert_eq!(code, 3);
    assert!(
        stderr.contains("nope.json") && stderr.contains("does not exist"),
        "{stderr}"
    );
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, "{not json").expect("write");
    let (code, _, stderr) = run(
        dir.path(),
        &["--client-cert-config", bad.to_str().expect("utf-8")],
        &[],
    );
    assert_eq!(code, 3);
    assert!(
        stderr.contains("cannot parse client certificate config"),
        "{stderr}"
    );
}

#[test]
fn a_client_cert_config_parse_error_never_echoes_a_value() {
    let dir = collection(ROOT, TARGET);
    let config = dir.path().join("certs.json");
    std::fs::write(
        &config,
        r#"{"enabled": true, "certs": [{"domain": "127.0.0.1", "type": "pfx", "passphrase": 482915}]}"#,
    )
    .expect("write");
    let (code, _, stderr) = run(
        dir.path(),
        &["--client-cert-config", config.to_str().expect("utf-8")],
        &[],
    );
    assert_eq!(code, 3);
    assert!(
        stderr.contains("cannot parse client certificate config") && stderr.contains("line 1"),
        "{stderr}"
    );
    assert!(!stderr.contains("482915"), "{stderr}");
}

/// Serves `body` as a PAC file to every request on 127.0.0.1; returns the port.
fn pac_server(body: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/x-ns-proxy-autoconfig\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    port
}

/// A PAC script sending everything through the HTTP proxy on 127.0.0.1:`port`.
fn via(port: u16) -> String {
    format!("function FindProxyForURL(url, host) {{ return \"PROXY 127.0.0.1:{port}\"; }}")
}

/// `run` started in `cwd`, so a relative `--proxy-pac` resolves against it.
fn run_in(cwd: &Path, dir: &Path, args: &[&str]) -> (i32, serde_json::Value, String) {
    let out = apinata()
        .current_dir(cwd)
        .arg("run")
        .arg(dir)
        .args(["--format", "json"])
        .args(args)
        .output()
        .expect("runs");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let report = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out.status.code().expect("code"), report, stderr)
}

fn closed_port() -> u16 {
    dead_port()
}

#[test]
fn proxy_pac_takes_a_path_relative_to_the_working_directory() {
    let (port, seen) = fake_proxy();
    let dir = collection(ROOT, TARGET);
    std::fs::write(dir.path().join("proxy.pac"), via(port)).expect("write");
    let (code, report, stderr) = run_in(dir.path(), dir.path(), &["--proxy-pac", "proxy.pac"]);
    assert_eq!(
        (code, column(&report, "outcome")),
        (0, vec!["passed".to_owned()]),
        "{stderr}"
    );
    assert_eq!(
        *seen.lock().unwrap(),
        vec![format!("GET {TARGET} HTTP/1.1")]
    );
    assert_eq!(report["results"][0]["warnings"], serde_json::json!([]));
}

#[test]
fn proxy_pac_takes_an_http_url() {
    let (port, seen) = fake_proxy();
    let pac = pac_server(via(port));
    let dir = collection(ROOT, TARGET);
    let url = format!("http://127.0.0.1:{pac}/proxy.pac");
    let (code, _, stderr) = run(dir.path(), &["--proxy-pac", &url], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        *seen.lock().unwrap(),
        vec![format!("GET {TARGET} HTTP/1.1")]
    );
}

#[test]
fn an_unreachable_pac_file_connects_directly_and_warns_in_both_reports() {
    // The fake proxy answers every request itself, so here it stands in for the origin.
    let (origin, seen) = fake_proxy();
    let dir = collection(ROOT, &format!("http://127.0.0.1:{origin}/x"));
    let source = format!("http://u:pw-SEKRIT@127.0.0.1:{}/proxy.pac", closed_port());
    let (code, report, stderr) = run(dir.path(), &["--proxy-pac", &source], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(*seen.lock().unwrap(), vec!["GET /x HTTP/1.1".to_owned()]);
    let warnings = report["results"][0]["warnings"]
        .as_array()
        .expect("warnings")
        .clone();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let warning = warnings[0].as_str().expect("text");
    assert!(
        warning.starts_with("proxy auto-config could not be fetched: http://***@127.0.0.1:")
            && warning.ends_with("; connected directly"),
        "{warning}"
    );
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--proxy-pac", &source])
        .output()
        .expect("runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("   ! proxy auto-config could not be fetched: "),
        "{text}"
    );
    for shown in [
        text.into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        stderr,
        warning.to_owned(),
    ] {
        assert!(!shown.contains("pw-SEKRIT"), "{shown}");
    }
}

#[test]
fn a_credentialed_bare_path_is_refused_without_echo() {
    let dir = collection(ROOT, TARGET);
    for (format, bare) in [
        ("json", "u:pw-SEKRIT@127.0.0.1/p.pac"),
        ("text", "u:pw-SEKRIT@127.0.0.1/p.pac"),
        ("json", "u:pw-SEKRIT@host/p?x=http://y"),
    ] {
        let out = apinata()
            .current_dir(dir.path())
            .arg("run")
            .arg(dir.path())
            .args(["--format", format])
            .args(["--proxy-pac", bare])
            .output()
            .expect("runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(3), "{stderr}");
        assert!(
            stderr.contains(
                "the PAC source looks like a URL with credentials; add http:// or https://"
            ),
            "{stderr}"
        );
        assert!(
            !stdout.contains("SEKRIT") && !stderr.contains("SEKRIT"),
            "{stdout}{stderr}"
        );
    }
}

#[test]
fn a_bad_proxy_pac_is_a_usage_error_that_never_echoes_it() {
    let dir = collection(ROOT, TARGET);
    let (code, _, stderr) = run(
        dir.path(),
        &["--proxy-pac", "ftp://u:pw-SEKRIT@127.0.0.1/p.pac"],
        &[],
    );
    assert_eq!(code, 3);
    assert!(
        stderr.contains("--proxy-pac")
            && stderr.contains("must be an http, https or file URL, not ftp"),
        "{stderr}"
    );
    assert!(!stderr.contains("pw-SEKRIT"), "{stderr}");
    let (code, _, stderr) = run(dir.path(), &["--proxy-pac", "proxy.corp.example:8080"], &[]);
    assert_eq!(code, 3);
    assert!(stderr.contains("needs http:// or https://"), "{stderr}");
    let (code, _, stderr) = run(dir.path(), &["--proxy-pac", "p.pac", "--noproxy"], &[]);
    assert_eq!(code, 3);
    assert!(stderr.contains("--noproxy"), "{stderr}");
    let help = apinata().args(["run", "--help"]).output().expect("runs");
    // Joined on single spaces, so the check holds however clap wraps the help.
    let help = String::from_utf8_lossy(&help.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        help.contains("--proxy-pac <URL|PATH>")
            && help.contains(
                "Route requests that inherit the global proxy through this proxy auto-config \
                 file (URL or path)",
            ),
        "{help}"
    );
}
