//! The network fixtures themselves: the proxies forward and record, the TLS server speaks with
//! the test PKI. The executor tests rely on these behaving exactly so.

use crate::network_support;

use network_support::{http_proxy, pki, socks5_proxy, test_roots, tls_server};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn origin() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&server)
        .await;
    server
}

async fn read_all(stream: &mut (impl AsyncReadExt + Unpin)) -> String {
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out).await;
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn the_http_proxy_forwards_absolute_form_and_records_it() {
    let origin = origin().await;
    let proxy = http_proxy(None).await;
    let mut s = TcpStream::connect(("127.0.0.1", proxy.port))
        .await
        .expect("connect");
    let target = format!("{}/ping", origin.uri());
    s.write_all(format!("GET {target} HTTP/1.1\r\nhost: x\r\n\r\n").as_bytes())
        .await
        .expect("write");
    let reply = read_all(&mut s).await;
    assert!(
        reply.starts_with("HTTP/1.1 200") && reply.ends_with("pong"),
        "{reply}"
    );
    assert_eq!(proxy.seen.lines(), vec![format!("GET {target} HTTP/1.1")]);
}

#[tokio::test]
async fn the_http_proxy_demands_its_credentials() {
    let proxy = http_proxy(Some("Basic dTpw")).await;
    let mut s = TcpStream::connect(("127.0.0.1", proxy.port))
        .await
        .expect("connect");
    s.write_all(b"GET http://127.0.0.1:9/ HTTP/1.1\r\nproxy-authorization: Basic bad\r\n\r\n")
        .await
        .expect("write");
    assert!(read_all(&mut s).await.starts_with("HTTP/1.1 407"));
    assert_eq!(
        proxy.seen.lines(),
        ["GET http://127.0.0.1:9/ HTTP/1.1", "auth: Basic bad"]
    );
}

#[tokio::test]
async fn the_socks5_proxy_checks_credentials_and_tunnels() {
    let origin = origin().await;
    let port = origin.address().port();
    let proxy = socks5_proxy(Some(("u", "p"))).await;
    let mut s = TcpStream::connect(("127.0.0.1", proxy.port))
        .await
        .expect("connect");
    let mut reply = [0u8; 2];
    s.write_all(&[5, 1, 2]).await.expect("greeting");
    s.read_exact(&mut reply).await.expect("method");
    assert_eq!(reply, [5, 2]);
    s.write_all(&[1, 1, b'u', 1, b'p'])
        .await
        .expect("credentials");
    s.read_exact(&mut reply).await.expect("auth reply");
    assert_eq!(reply, [1, 0]);
    let [hi, lo] = port.to_be_bytes();
    s.write_all(&[5, 1, 0, 1, 127, 0, 0, 1, hi, lo])
        .await
        .expect("connect request");
    let mut connected = [0u8; 10];
    s.read_exact(&mut connected).await.expect("connect reply");
    assert_eq!(connected[..2], [5, 0]);
    s.write_all(b"GET /ping HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n")
        .await
        .expect("request");
    assert!(read_all(&mut s).await.ends_with("pong"));
    assert_eq!(
        proxy.seen.lines(),
        vec!["auth: u:p".to_owned(), format!("connect 127.0.0.1:{port}")]
    );
}

async fn tls_get(port: u16, config: ClientConfig) -> std::io::Result<String> {
    let tcp = TcpStream::connect(("127.0.0.1", port)).await?;
    let name = ServerName::try_from("localhost").expect("name");
    let mut tls = TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await?;
    tls.write_all(b"GET / HTTP/1.1\r\nhost: localhost\r\n\r\n")
        .await?;
    let mut buf = vec![0u8; 256];
    let n = tls.read(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}

#[tokio::test]
async fn the_tls_server_speaks_with_the_test_pki() {
    let server = tls_server(false).await;
    let config = ClientConfig::builder()
        .with_root_certificates(test_roots())
        .with_no_client_auth();
    let reply = tls_get(server.port, config).await.expect("handshake");
    assert!(reply.ends_with("ok"), "{reply}");
    assert_eq!(server.connections(), 1);

    let mtls = tls_server(true).await;
    let chain = vec![CertificateDer::from_pem_file(pki("client.pem")).expect("client.pem")];
    let key = PrivateKeyDer::from_pem_file(pki("client.key")).expect("client.key");
    let config = ClientConfig::builder()
        .with_root_certificates(test_roots())
        .with_client_auth_cert(chain, key)
        .expect("client auth");
    let reply = tls_get(mtls.port, config).await.expect("mutual handshake");
    assert!(reply.ends_with("ok"), "{reply}");
}

#[test]
fn the_pki_files_have_the_expected_shapes() {
    for cert in ["ca.pem", "server.pem", "client.pem", "expired-client.pem"] {
        CertificateDer::from_pem_file(pki(cert)).expect(cert);
    }
    PrivateKeyDer::from_pem_file(pki("client.key")).expect("plain key");
    let encrypted = std::fs::read_to_string(pki("client-enc.key")).expect("client-enc.key");
    assert!(encrypted.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----"));
    let traditional = std::fs::read_to_string(pki("client-trad.key")).expect("client-trad.key");
    assert!(traditional.contains("Proc-Type: 4,ENCRYPTED"));
}
