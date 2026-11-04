//! Helpers shared by the CLI integration tests.
#![allow(dead_code)] // each test binary uses a different subset

use std::fs;
use std::path::Path;
use tempfile::TempDir;

pub const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// Write `files` (relative path, contents) under a new temporary directory.
pub fn write_tree(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, text).expect("write");
    }
    dir
}

/// Variables a spawned binary must not inherit: the machine's proxy, the CGI-mode variable, and
/// the session bus and runtime dir through which the platform keychain is reached.
const SCRUBBED_ENV: [&str; 10] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "REQUEST_METHOD",
    "XDG_RUNTIME_DIR",
];

/// A session bus address with no socket behind it. Unsetting the variable is not enough: zbus
/// then tries `/run/user/<uid>/bus`, the real session bus.
pub const UNREACHABLE_BUS: &str = "unix:path=/nonexistent/apinata-test-bus";

/// The network acknowledgement file every spawned binary shares unless a test names its own:
/// one per test process, so no test writes the real data directory.
pub fn shared_ack_file() -> &'static Path {
    static DIR: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| TempDir::new().expect("tempdir"));
    Box::leak(Box::new(dir.path().join("network-ack.json")))
}

/// The built `apinata` binary. Every test that starts the binary does so through this or its
/// `apinata_std` and `apinata_tokio` siblings: they make the real keychain unreachable (the
/// binary opens an empty memory store, and has no session bus to find a secret service by).
pub fn apinata() -> assert_cmd::Command {
    let mut cmd = assert_cmd::Command::cargo_bin("apinata").expect("binary");
    cmd.env("APINATA_TEST_NO_KEYCHAIN", "1");
    cmd.env("APINATA_TEST_NETWORK_ACK", shared_ack_file());
    cmd.env("DBUS_SESSION_BUS_ADDRESS", UNREACHABLE_BUS);
    for name in SCRUBBED_ENV {
        cmd.env_remove(name);
    }
    cmd
}

/// [`apinata`] as a `std::process::Command`, for tests that spawn and hold a child.
pub fn apinata_std() -> std::process::Command {
    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("apinata"));
    cmd.env("APINATA_TEST_NO_KEYCHAIN", "1");
    cmd.env("APINATA_TEST_NETWORK_ACK", shared_ack_file());
    cmd.env("DBUS_SESSION_BUS_ADDRESS", UNREACHABLE_BUS);
    for name in SCRUBBED_ENV {
        cmd.env_remove(name);
    }
    cmd
}

/// [`apinata`] as a `tokio::process::Command`.
pub fn apinata_tokio() -> tokio::process::Command {
    tokio::process::Command::from(apinata_std())
}

/// A GET request file; `assertions` holds list items for `runtime.assertions`.
pub fn get(name: &str, seq: u32, url: &str, assertions: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  assertions:\n{assertions}"
    )
}

/// One `runtime.assertions` item: `expression operator value`.
pub fn check(expression: &str, operator: &str, value: &str) -> String {
    format!(
        "    - expression: {expression}\n      operator: {operator}\n      value: \"{value}\"\n"
    )
}

/// Run `apinata run <path> --format json <extra>` and return the exit code and parsed report.
pub fn run_json(path: &Path, extra: &[&str]) -> (i32, serde_json::Value) {
    let out = apinata()
        .arg("run")
        .arg(path)
        .args(["--format", "json"])
        .args(extra)
        .output()
        .expect("runs");
    let report = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().expect("exit code"), report)
}

/// `results[*].<key>` as strings, in report order.
pub fn column(report: &serde_json::Value, key: &str) -> Vec<String> {
    report["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|r| r[key].as_str().unwrap_or("null").to_owned())
        .collect()
}

/// One `runtime.assertions` item for a unary operator, which takes no value.
pub fn unary(expression: &str, operator: &str) -> String {
    format!("    - expression: {expression}\n      operator: {operator}\n")
}

#[path = "../../../apinata-core/tests/common/sse_server.rs"]
pub mod sse_server;

#[path = "../../../apinata-core/tests/common/body_server.rs"]
pub mod body_server;

/// A port that refuses every connection and that no other process can take: the socket is
/// bound for the whole process but never listens.
pub fn dead_port() -> u16 {
    use socket2::{Domain, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("socket");
    socket
        .bind(&std::net::SocketAddr::from(([127, 0, 0, 1], 0)).into())
        .expect("bind");
    let port = socket
        .local_addr()
        .expect("addr")
        .as_socket()
        .expect("ip")
        .port();
    std::mem::forget(socket);
    port
}
