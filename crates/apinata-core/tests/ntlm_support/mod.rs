//! An NTLM origin on 127.0.0.1 for the executor tests. NTLMv2 only, written without
//! `crate::ntlm` so it also cross-checks the client's Type 3. State is per connection, like IIS.
#![allow(dead_code)] // each test binary uses a different subset

use crate::network_support::pki;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use md4::{Digest as _, Md4};
use md5::Md5;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub const USER: &str = "ntlmtest";
pub const PASSWORD: &str = "Pw-1";

/// Fixed server challenge; the client's response is checked against it.
const CHALLENGE: [u8; 8] = *b"\x01\x23\x45\x67\x89\xab\xcd\xef";

#[derive(Clone, Default)]
pub struct Options {
    pub tls: bool,
    pub close_on_401: bool,
    pub challenge_body: usize,
    pub redirect_to: Option<String>,
    pub cookie: bool,
    pub junk_challenge: bool,
    pub open_redirects: bool,
    /// Echo the request's `Authorization` value into the 200 body.
    pub echo_auth: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Probe,
    Type1,
    Type3,
}

#[derive(Clone, Debug)]
pub struct Seen {
    pub conn: u16,
    pub method: String,
    pub target: String,
    pub auth: Option<String>,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

/// The NTLM message carried by an `Authorization: NTLM <base64>` value, if it is one.
fn ntlm_message(auth: &str) -> Option<Vec<u8>> {
    let (scheme, blob) = auth.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("ntlm") {
        return None;
    }
    STANDARD.decode(blob.trim()).ok()
}

impl Seen {
    /// The leg this request belongs to, from the NTLM message type byte.
    pub fn stage(&self) -> Stage {
        let message = self.auth.as_deref().and_then(ntlm_message);
        match message.as_deref() {
            Some(m) if m.len() > 8 && m.starts_with(b"NTLMSSP\0") => match m[8] {
                1 => Stage::Type1,
                3 => Stage::Type3,
                _ => Stage::Probe,
            },
            _ => Stage::Probe,
        }
    }
}

pub struct NtlmServer {
    pub port: u16,
    tls: bool,
    seen: Arc<Mutex<Vec<Seen>>>,
    accepted: Arc<AtomicUsize>,
}

impl NtlmServer {
    pub fn origin(&self) -> String {
        let scheme = if self.tls { "https" } else { "http" };
        format!("{scheme}://127.0.0.1:{}", self.port)
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    pub fn connections(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn hmac_md5(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut mac = <Hmac<Md5> as KeyInit>::new_from_slice(key).expect("any key length");
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().to_vec()
}

/// Type 2 with Unicode set and a one-pair target info, so a strict client accepts it.
fn type2() -> Vec<u8> {
    let mut info = Vec::new();
    info.extend_from_slice(&2u16.to_le_bytes());
    info.extend_from_slice(&6u16.to_le_bytes());
    info.extend_from_slice(&utf16("DOM"));
    info.extend_from_slice(&[0u8; 4]);
    let mut m = b"NTLMSSP\0".to_vec();
    m.extend_from_slice(&2u32.to_le_bytes());
    m.extend_from_slice(&[0, 0, 0, 0]);
    m.extend_from_slice(&56u32.to_le_bytes());
    m.extend_from_slice(&0xA289_8205u32.to_le_bytes());
    m.extend_from_slice(&CHALLENGE);
    m.extend_from_slice(&[0u8; 8]);
    m.extend_from_slice(&(info.len() as u16).to_le_bytes());
    m.extend_from_slice(&(info.len() as u16).to_le_bytes());
    m.extend_from_slice(&56u32.to_le_bytes());
    m.extend_from_slice(&[0u8; 8]);
    m.extend_from_slice(&info);
    m
}

/// A security buffer (length at `at`, offset at `at + 4`), bounds-checked.
fn field(m: &[u8], at: usize) -> Option<&[u8]> {
    let len = u16::from_le_bytes(m.get(at..at + 2)?.try_into().ok()?) as usize;
    let off = u32::from_le_bytes(m.get(at + 4..at + 8)?.try_into().ok()?) as usize;
    m.get(off..off.checked_add(len)?)
}

/// True when the Type 3 proves knowledge of `PASSWORD` for `USER`, whatever the domain.
fn verify_type3(m: &[u8]) -> bool {
    let check = || -> Option<bool> {
        let nt = field(m, 20)?;
        let domain = field(m, 28)?;
        let user = field(m, 36)?;
        if nt.len() < 16 {
            return Some(false);
        }
        let units: Vec<u16> = user
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let name = String::from_utf16_lossy(&units);
        if !name.eq_ignore_ascii_case(USER) {
            return Some(false);
        }
        let hash = Md4::digest(utf16(PASSWORD));
        let key = hmac_md5(&hash, &[&utf16(&name.to_uppercase()), domain]);
        let proof = hmac_md5(&key, &[&CHALLENGE, &nt[16..]]);
        Some(nt[..16] == proof[..])
    };
    check().unwrap_or(false)
}

struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

async fn read_request<S: AsyncRead + Unpin>(s: &mut S) -> Option<Request> {
    let mut buf = Vec::new();
    while !buf.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        if s.read(&mut b).await.ok()? == 0 {
            return None;
        }
        buf.push(b[0]);
    }
    let text = String::from_utf8(buf).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).await.ok()?;
    Some(Request {
        method,
        target,
        headers,
        body,
    })
}

struct Reply {
    status: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    end: bool,
}

impl Reply {
    fn bytes(&self) -> Vec<u8> {
        let mut out = format!("HTTP/1.1 {}\r\n", self.status);
        for (k, v) in &self.headers {
            out.push_str(&format!("{k}: {v}\r\n"));
        }
        out.push_str(&format!("content-length: {}\r\n\r\n", self.body.len()));
        let mut out = out.into_bytes();
        out.extend_from_slice(&self.body);
        out
    }
}

/// A 401 offering `offer` (`NTLM`, `NTLM <blob>` or `Basic`), shaped by the options.
fn unauthorized(opts: &Options, offer: String) -> Reply {
    let mut headers = vec![("www-authenticate", offer)];
    if opts.cookie {
        headers.push(("set-cookie", "sid=1; Path=/".to_owned()));
    }
    if opts.close_on_401 {
        headers.push(("connection", "close".to_owned()));
    }
    Reply {
        status: "401 Unauthorized",
        headers,
        body: vec![b'x'; opts.challenge_body],
        end: opts.close_on_401,
    }
}

fn redirect(opts: &Options, status: &'static str) -> Reply {
    let to = opts.redirect_to.clone().unwrap_or_else(|| "/ok".to_owned());
    Reply {
        status,
        headers: vec![("location", to)],
        body: Vec::new(),
        end: false,
    }
}

fn served(req: &Request, path: &str, opts: &Options) -> Reply {
    match path {
        "/ok" => Reply {
            status: "200 OK",
            headers: Vec::new(),
            body: if opts.echo_auth {
                let auth = req
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                    .map_or("", |(_, v)| v.as_str());
                format!("{} {} {auth}", req.method, req.body.len()).into_bytes()
            } else {
                format!("{} {}", req.method, req.body.len()).into_bytes()
            },
            end: false,
        },
        "/redirect302" => redirect(opts, "302 Found"),
        "/redirect307" => redirect(opts, "307 Temporary Redirect"),
        _ => Reply {
            status: "404 Not Found",
            headers: Vec::new(),
            body: Vec::new(),
            end: false,
        },
    }
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    conn: u16,
    opts: Options,
    log: Arc<Mutex<Vec<Seen>>>,
) {
    let mut authenticated = false;
    while let Some(req) = read_request(&mut s).await {
        let auth = req
            .headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .map(|(_, v)| v.clone());
        log.lock().unwrap().push(Seen {
            conn,
            method: req.method.clone(),
            target: req.target.clone(),
            auth: auth.clone(),
            body: req.body.clone(),
            headers: req.headers.clone(),
        });
        // An absolute-form target, as a plain-http proxy forwards it, routes by its path.
        let origin_form = req
            .target
            .strip_prefix("http://")
            .map_or(req.target.as_str(), |rest| {
                rest.find('/').map_or("/", |i| &rest[i..])
            });
        let path = origin_form.split('?').next().unwrap_or_default().to_owned();
        let reply = if path == "/basic" {
            unauthorized(&opts, "Basic realm=\"x\"".to_owned())
        } else if opts.open_redirects && path.starts_with("/redirect") {
            served(&req, &path, &opts)
        } else {
            match auth.as_deref().and_then(ntlm_message) {
                Some(m) if m.starts_with(b"NTLMSSP\0") && m.get(8) == Some(&1) => {
                    authenticated = false;
                    let blob = if opts.junk_challenge {
                        "AAAA".to_owned()
                    } else {
                        STANDARD.encode(type2())
                    };
                    unauthorized(&opts, format!("NTLM {blob}"))
                }
                Some(m) if m.starts_with(b"NTLMSSP\0") && m.get(8) == Some(&3) => {
                    authenticated = verify_type3(&m);
                    if authenticated {
                        served(&req, &path, &opts)
                    } else {
                        unauthorized(&opts, "NTLM".to_owned())
                    }
                }
                _ if authenticated => served(&req, &path, &opts),
                _ => unauthorized(&opts, "NTLM".to_owned()),
            }
        };
        if s.write_all(&reply.bytes()).await.is_err() || reply.end {
            return;
        }
    }
}

pub async fn ntlm_server(opts: Options) -> NtlmServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let accepted = Arc::new(AtomicUsize::new(0));
    let acceptor = opts.tls.then(|| {
        let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(pki("server.pem"))
            .expect("server.pem")
            .collect::<Result<_, _>>()
            .expect("certs");
        let key = PrivateKeyDer::from_pem_file(pki("server.key")).expect("server.key");
        TlsAcceptor::from(Arc::new(
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .expect("tls config"),
        ))
    });
    let (log, count, tls) = (seen.clone(), accepted.clone(), opts.tls);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let conn = (count.fetch_add(1, Ordering::SeqCst) + 1) as u16;
            let (log, opts, acceptor) = (log.clone(), opts.clone(), acceptor.clone());
            tokio::spawn(async move {
                match acceptor {
                    Some(a) => {
                        if let Ok(t) = a.accept(tcp).await {
                            serve(t, conn, opts, log).await;
                        }
                    }
                    None => serve(tcp, conn, opts, log).await,
                }
            });
        }
    });
    NtlmServer {
        port,
        tls,
        seen,
        accepted,
    }
}
