#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::{ROOT, apinata, write_tree};
use predicates::str::contains;

const EXAMPLES: &str = "examples:\n  - name: ok\n    response:\n      status: 200\n      headers:\n        - name: X-Mock\n          value: \"yes\"\n      body:\n        type: json\n        data: '{\"id\": 7}'\n";

fn request(url: &str) -> String {
    format!(
        "info:\n  name: u\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{EXAMPLES}"
    )
}

/// A spawned `apinata mock` that is killed through its own child handle when dropped.
struct Mock {
    child: Child,
    addr: SocketAddr,
    out: BufReader<std::process::ChildStdout>,
}

impl Mock {
    fn start(args: &[&str]) -> Mock {
        let mut child = common::apinata_std()
            .arg("mock")
            .args(args)
            .args(["--port", "0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut first = String::new();
        out.read_line(&mut first).expect("first line");
        let addr = first
            .trim()
            .strip_prefix("Listening on http://")
            .unwrap_or_else(|| panic!("first line: {first:?}"))
            .parse()
            .expect("addr");
        Mock { child, addr, out }
    }
    fn get(&self, path: &str) -> String {
        let mut s = TcpStream::connect_timeout(&self.addr, Duration::from_secs(5)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut text = String::new();
        s.read_to_string(&mut text).unwrap();
        text
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn serves_examples_prints_the_first_line_then_one_log_line_per_hit() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("u.yml", &request("{{baseUrl}}/users/:id")),
    ]);
    let mut m = Mock::start(&[
        dir.path().to_str().unwrap(),
        "--env-var",
        "baseUrl=http://api.test",
    ]);
    let reply = m.get("/users/7");
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.to_lowercase().contains("x-mock: yes"), "{reply}");
    assert!(reply.ends_with("{\"id\": 7}"), "{reply}");
    let mut line = String::new();
    m.out.read_line(&mut line).unwrap();
    assert!(line.contains("GET /users/7 -> 200 u.yml / ok"), "{line}");
    assert!(m.get("/nope").starts_with("HTTP/1.1 404"));
}

#[cfg(unix)]
#[test]
fn ctrl_c_stops_it_with_exit_zero() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("u.yml", &request("/users"))]);
    let mut m = Mock::start(&[dir.path().to_str().unwrap()]);
    let pid = m.child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-INT", &pid])
            .status()
            .unwrap()
            .success()
    );
    let status = m.child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn a_port_already_in_use_exits_3() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("u.yml", &request("/users"))]);
    let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = held.local_addr().unwrap().port().to_string();
    apinata()
        .arg("mock")
        .arg(dir.path())
        .args(["--port", &port])
        .timeout(Duration::from_secs(30))
        .assert()
        .code(3)
        .stderr(contains("cannot listen"));
}

#[test]
fn a_non_loopback_bind_is_refused_with_exit_3() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("u.yml", &request("/users"))]);
    apinata()
        .arg("mock")
        .arg(dir.path())
        .args(["--bind", "0.0.0.0", "--port", "0"])
        .timeout(Duration::from_secs(30))
        .assert()
        .code(3)
        .stderr(contains("--allow-non-loopback"));
}

#[test]
fn a_missing_collection_exits_3() {
    let dir = write_tree(&[("note.txt", "x")]);
    apinata()
        .arg("mock")
        .arg(dir.path())
        .timeout(Duration::from_secs(30))
        .assert()
        .code(3);
}

#[test]
fn an_undefined_url_variable_warns_on_stderr_and_still_starts() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("u.yml", &request("{{nope}}/users")),
        ("v.yml", &request("/ok").replace("seq: 1", "seq: 2")),
    ]);
    let out = common::apinata_std()
        .arg("mock")
        .arg(dir.path())
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = out;
    let mut first = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut first)
        .unwrap();
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    assert!(
        first.starts_with("Listening on http://127.0.0.1:"),
        "{first}"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("nope"));
}

#[test]
fn help_lists_the_options_and_exit_codes() {
    apinata()
        .args(["mock", "--help"])
        .assert()
        .success()
        .stdout(contains("--port "))
        .stdout(contains("--bind "))
        .stdout(contains("--delay "))
        .stdout(contains("--allow-non-loopback"))
        .stdout(contains("--global-env-var "))
        .stdout(contains("Exit codes:"));
}

/// Once the reader of stdout is gone, the mock must keep serving. The port comes from the
/// Listening line (`--port 0`), read before the read end of the pipe is dropped.
#[test]
fn keeps_serving_after_stdout_is_dropped() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("u.yml", &request("/users"))]);
    let mut child = common::apinata_std()
        .arg("mock")
        .arg(dir.path())
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    out.read_line(&mut first).expect("first line");
    let addr: SocketAddr = first
        .trim()
        .strip_prefix("Listening on http://")
        .unwrap_or_else(|| panic!("first line: {first:?}"))
        .parse()
        .expect("addr");
    drop(out);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut reply = None;
    while std::time::Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            write!(
                s,
                "GET /users HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut text = String::new();
            s.read_to_string(&mut text).unwrap();
            reply = Some(text);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let exited = child.try_wait().unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(exited, None, "the mock exited instead of serving");
    let reply = reply.expect("no reply");
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
}

/// Fetch `/users` from `addr`, retrying connects until the deadline or the child exits.
fn get_users_while_alive(child: &mut Child, addr: SocketAddr) -> Option<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return None;
        }
        if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let _ = write!(
                s,
                "GET /users HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
            );
            let mut text = String::new();
            let _ = s.read_to_string(&mut text);
            return Some(text);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// With the reader of stdout gone before spawn, the Listening line hits a broken pipe and the
/// mock must keep serving. The port cannot be read back, so pick one and retry on bind failure.
#[test]
fn keeps_serving_when_stdout_is_a_broken_pipe_at_startup() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("u.yml", &request("/users"))]);
    let mut served = None;
    for attempt in 0..20u32 {
        let port = 30000
            + (std::process::id()
                .wrapping_mul(7919)
                .wrapping_add(attempt * 104729))
                % 30000;
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let mut child = common::apinata_std()
            .arg("mock")
            .arg(dir.path())
            .args(["--port", &port.to_string()])
            .stdout(Stdio::from(writer))
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port as u16));
        let reply = get_users_while_alive(&mut child, addr);
        let exited = child.try_wait().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        // Exit 3 is also what a bind failure gives, so only a port conflict is retried.
        if reply.is_none() && exited.and_then(|s| s.code()) == Some(3) {
            continue;
        }
        served = Some((reply, exited));
        break;
    }
    let (reply, exited) =
        served.expect("every try exited 3 (no free port, or the mock died on the broken pipe)");
    assert_eq!(exited, None, "the mock exited instead of serving");
    let reply = reply.expect("no reply");
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
}
