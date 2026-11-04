//! The public constructors of the data-carrying `ExecError` variants.

use std::path::Path;

use apinata_core::{ConnectCause, ExecError};

#[test]
fn connect_carries_url_message_and_cause() {
    let e = ExecError::connect("http://a.test/", "refused", ConnectCause::Refused);
    assert!(matches!(
        &e,
        ExecError::Connect { url, message, cause: ConnectCause::Refused, .. }
            if url == "http://a.test/" && message == "refused"
    ));
}

#[test]
fn timeout_carries_url() {
    let e = ExecError::timeout("http://a.test/");
    assert!(matches!(&e, ExecError::Timeout { url, .. } if url == "http://a.test/"));
}

#[test]
fn proxy_carries_url_proxy_and_message() {
    let e = ExecError::proxy("http://a.test/", "http://p.test:3128", "407");
    assert!(matches!(
        &e,
        ExecError::Proxy { url, proxy, message, .. }
            if url == "http://a.test/" && proxy == "http://p.test:3128" && message == "407"
    ));
}

#[test]
fn transport_carries_url_and_message() {
    let e = ExecError::transport("http://a.test/", "reset");
    assert!(matches!(
        &e,
        ExecError::Transport { url, message, .. } if url == "http://a.test/" && message == "reset"
    ));
}

#[test]
fn file_carries_path_and_message() {
    let e = ExecError::file("data/x.txt", "missing");
    assert!(matches!(
        &e,
        ExecError::File { path, message, .. }
            if path == Path::new("data/x.txt") && message == "missing"
    ));
}

#[test]
fn host_not_allowed_carries_hop_and_host() {
    let e = ExecError::host_not_allowed(2, "evil.example");
    assert!(matches!(
        &e,
        ExecError::HostNotAllowed { hop: 2, host, .. } if host == "evil.example"
    ));
}

#[test]
fn proxy_not_allowed_carries_hop_host_and_proxy() {
    let e = ExecError::proxy_not_allowed(1, "p.test", "http://p.test:3128");
    assert!(matches!(
        &e,
        ExecError::ProxyNotAllowed { hop: 1, host, proxy, .. }
            if host == "p.test" && proxy == "http://p.test:3128"
    ));
}

#[test]
fn certificate_carries_path_and_message() {
    let e = ExecError::certificate("certs/ca.pem", "unreadable");
    assert!(matches!(
        &e,
        ExecError::Certificate { path, message, .. }
            if path == Path::new("certs/ca.pem") && message == "unreadable"
    ));
}
