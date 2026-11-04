//! The NTLM server double answers the way a real NTLM origin does.

use crate::ntlm_support;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ntlm_support::{Options, Stage, ntlm_server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Sends one request on `s` and returns its head and body text.
async fn exchange(s: &mut TcpStream, extra: &str) -> String {
    let req = format!("GET /ok HTTP/1.1\r\nhost: x\r\n{extra}\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut b).await.unwrap();
        buf.push(b[0]);
    }
    let head = String::from_utf8(buf).unwrap();
    let len: usize = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse().unwrap())
        })
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).await.unwrap();
    format!("{head}{}", String::from_utf8_lossy(&body))
}

fn type1() -> String {
    let mut m = b"NTLMSSP\0".to_vec();
    m.extend_from_slice(&1u32.to_le_bytes());
    m.extend_from_slice(&0xa008_8205u32.to_le_bytes());
    m.extend_from_slice(&[0u8; 16]);
    format!("authorization: NTLM {}\r\n", STANDARD.encode(m))
}

#[tokio::test]
async fn it_challenges_then_issues_a_usable_type2() {
    let server = ntlm_server(Options::default()).await;
    let mut s = TcpStream::connect(("127.0.0.1", server.port))
        .await
        .unwrap();
    let probe = exchange(&mut s, "").await;
    assert!(probe.starts_with("HTTP/1.1 401"), "{probe}");
    assert!(
        probe
            .to_ascii_lowercase()
            .contains("www-authenticate: ntlm\r\n"),
        "{probe}"
    );
    let challenge = exchange(&mut s, &type1()).await;
    let blob = challenge
        .lines()
        .find_map(|l| l.strip_prefix("www-authenticate: NTLM "))
        .expect("a Type 2");
    let t2 = STANDARD.decode(blob.trim()).unwrap();
    assert_eq!(&t2[..8], b"NTLMSSP\0");
    assert_eq!(u32::from_le_bytes(t2[8..12].try_into().unwrap()), 2);
    // Unicode is set and the target info is not empty, so a strict client accepts it.
    assert_ne!(u32::from_le_bytes(t2[20..24].try_into().unwrap()) & 1, 0);
    assert_ne!(u16::from_le_bytes(t2[40..42].try_into().unwrap()), 0);
    let stages: Vec<Stage> = server.seen().iter().map(|r| r.stage()).collect();
    assert_eq!(stages, [Stage::Probe, Stage::Type1]);
}

#[tokio::test]
async fn it_rejects_a_forged_type3_and_stays_unauthenticated() {
    let server = ntlm_server(Options::default()).await;
    let mut s = TcpStream::connect(("127.0.0.1", server.port))
        .await
        .unwrap();
    exchange(&mut s, &type1()).await;
    let mut forged = b"NTLMSSP\0".to_vec();
    forged.extend_from_slice(&3u32.to_le_bytes());
    forged.extend_from_slice(&[0u8; 60]);
    let reply = exchange(
        &mut s,
        &format!("authorization: NTLM {}\r\n", STANDARD.encode(forged)),
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 401"), "{reply}");
    let again = exchange(&mut s, "").await;
    assert!(again.starts_with("HTTP/1.1 401"), "{again}");
}

#[tokio::test]
async fn basic_only_routes_never_offer_ntlm() {
    let server = ntlm_server(Options::default()).await;
    let mut s = TcpStream::connect(("127.0.0.1", server.port))
        .await
        .unwrap();
    s.write_all(b"GET /basic HTTP/1.1\r\nhost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 512];
    let n = s.read(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
    assert!(text.contains("www-authenticate: basic"), "{text}");
    assert!(!text.contains("ntlm"), "{text}");
}
