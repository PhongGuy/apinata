//! The NTLM handshake client: one http1 connection per handshake, a second connect is an error.

use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use hyper_util::client::legacy::connect::HttpInfo;
use reqwest::header::{CONNECTION, HeaderValue, WWW_AUTHENTICATE};
use reqwest::{Client, StatusCode};
use tower::{Layer, Service};
use url::Url;

use super::deadline::Deadline;
use super::{
    ExecError, Executor, HOP_BODY_LIMIT, Hop, HopBuffer, HttpVersion, NtlmCause, classify, lossy,
    redact_url, sent_headers,
};
use crate::cookies::SetCookie;
use crate::network::{ClientKey, ResolvedProxy};
use crate::ntlm;
use crate::resolve::Warning;

type BoxError = Box<dyn StdError + Send + Sync>;

/// The error a second connect attempt gets, so the driver can tell a lost reuse race from a failure.
#[derive(Debug)]
pub(super) struct SecondConnect;

impl fmt::Display for SecondConnect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the server did not keep the connection")
    }
}

impl StdError for SecondConnect {}

/// True when `e` or anything in its source chain is a refused second connect.
pub(super) fn is_second_connect(e: &(dyn StdError + 'static)) -> bool {
    let mut link = Some(e);
    while let Some(err) = link {
        if err.is::<SecondConnect>() {
            return true;
        }
        link = err.source();
    }
    false
}

/// Lets exactly one connect through; the flag is shared by the clones reqwest makes.
#[derive(Clone)]
struct OneConnect(Arc<AtomicBool>);

#[derive(Clone)]
struct OneConnectService<S> {
    inner: S,
    used: Arc<AtomicBool>,
}

impl<S> Layer<S> for OneConnect {
    type Service = OneConnectService<S>;
    fn layer(&self, inner: S) -> OneConnectService<S> {
        OneConnectService {
            inner,
            used: Arc::clone(&self.0),
        }
    }
}

// Generic over request and response because reqwest's connector types cannot be named.
impl<S, R> Service<R> for OneConnectService<S>
where
    S: Service<R>,
    S::Error: Into<BoxError>,
    S::Future: Send + 'static,
    S::Response: 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<S::Response, BoxError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: R) -> Self::Future {
        if self.used.swap(true, Ordering::SeqCst) {
            return Box::pin(async { Err(Box::new(SecondConnect) as BoxError) });
        }
        let fut = self.inner.call(req);
        Box::pin(async move { fut.await.map_err(Into::into) })
    }
}

impl Executor {
    /// A client for one handshake: the normal rules for `key`, but http1 only, one pooled
    /// connection even under a host policy, and a second connect is refused.
    pub(super) fn build_handshake_client(&self, key: &ClientKey) -> Result<Client, ExecError> {
        self.base_builder(key)?
            .http1_only()
            .pool_max_idle_per_host(1)
            .connector_layer(OneConnect(Arc::new(AtomicBool::new(false))))
            .build()
            .map_err(|e| ExecError::Client {
                message: super::chain(e),
            })
    }
}

/// Why a handshake to an http target through a proxy lost its connection, in plain words.
const PROXY_NOT_KEPT: &str = "the proxy did not keep one connection to the server for the \
     handshake; use https, or a proxy that keeps connections";

/// Why a direct handshake failed when its legs arrived on different connections.
const NOT_ONE_CONNECTION: &str = "the handshake did not stay on one connection";

/// The most a challenge response's body may weigh before the handshake gives up on it.
const CHALLENGE_BODY_LIMIT: usize = 1 << 20;

/// What the driver hands the redirect loop: the final leg and the header it sent, if it signed.
pub(super) struct Negotiated {
    pub response: reqwest::Response,
    pub authorization: Option<String>,
    /// The last leg's deadline, so the caller's read of its body shares it.
    pub deadline: Deadline,
    /// When the last leg was sent, so the timeline times it alone.
    pub started: Instant,
    /// The time the earlier legs took, already on the timeline.
    pub earlier: Duration,
    /// Set when a full handshake through an http(s) proxy ended in 401.
    pub warning: Option<Warning>,
}

/// What a leg sent, taken before the request is consumed.
struct Sent {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    size: u64,
}

impl Sent {
    fn of(req: &reqwest::Request) -> Sent {
        let body = req
            .body()
            .and_then(|b| b.as_bytes())
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        Sent {
            method: req.method().to_string(),
            url: req.url().to_string(),
            headers: sent_headers(req),
            size: body.len() as u64,
            body: body[..body.len().min(HOP_BODY_LIMIT)].to_vec(),
        }
    }

    /// The timeline entry for this leg once its answer is drained.
    fn hop(
        self,
        res: &reqwest::Response,
        started: Instant,
        pac: Option<&str>,
        via_proxy: bool,
    ) -> Hop {
        Hop {
            method: self.method,
            url: self.url,
            request_headers: self.headers,
            status: res.status().as_u16(),
            response_headers: lossy(res.headers()),
            elapsed: started.elapsed(),
            request_body: self.body,
            request_body_size: self.size,
            pac: pac.map(str::to_owned),
            timing: None,
            cookies: Vec::new(),
            version: HttpVersion::from_wire(res.version()),
            remote_addr: res.remote_addr(),
            via_proxy,
            override_from: None,
        }
    }
}

/// The time the legs already pushed took.
fn gone(buf: &HopBuffer, from: usize) -> Duration {
    buf.hops[from..].iter().map(|h| h.elapsed).sum()
}

fn ntlm_err(url: &Url, cause: NtlmCause, message: impl Into<String>) -> ExecError {
    ExecError::Ntlm {
        url: redact_url(url.as_str()),
        cause,
        message: message.into(),
    }
}

/// True when the response asks for the connection to be closed after it.
fn closes(res: &reqwest::Response) -> bool {
    res.headers()
        .get_all(CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|t| t.trim().eq_ignore_ascii_case("close"))
}

fn www_authenticate(res: &reqwest::Response) -> Vec<String> {
    res.headers()
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(str::to_owned)
        .collect()
}

fn local_addr(res: &reqwest::Response) -> Option<std::net::SocketAddr> {
    res.extensions().get::<HttpInfo>().map(HttpInfo::local_addr)
}

impl Executor {
    /// Runs probe, Type 1 and Type 3 for one hop on a single pinned connection and returns the
    /// last response. A probe that is not an NTLM challenge comes back unchanged.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn negotiate(
        &self,
        probe: reqwest::Request,
        key: &ClientKey,
        creds: &ntlm::Credentials<'_>,
        url: &Url,
        proxy: Option<&ResolvedProxy>,
        followed: usize,
        store_cookies: bool,
        timeout: Option<Duration>,
        hop_deadline: Deadline,
        pac: Option<&str>,
        buf: &mut HopBuffer,
    ) -> Result<Negotiated, ExecError> {
        // To an http target through an http(s) proxy the legs go absolute-form, so a lost
        // connection is most likely the proxy's doing; through SOCKS it is a raw tunnel.
        let absolute =
            url.scheme() == "http" && proxy.is_some_and(|p| matches!(p.scheme(), "http" | "https"));
        let lost = |server: &'static str| if absolute { PROXY_NOT_KEPT } else { server };
        let base = buf.hops.len();
        let client = self.build_handshake_client(key)?;
        let transport = |e: reqwest::Error| classify(url, proxy, followed, e);
        // A later leg that needed a new connection means the pinned one was lost.
        let later = |e: reqwest::Error| {
            if is_second_connect(&e) {
                ntlm_err(
                    url,
                    NtlmCause::ConnectionNotKept,
                    lost("the server did not keep the connection"),
                )
            } else {
                classify(url, proxy, followed, e)
            }
        };
        let unclonable = || {
            ntlm_err(
                url,
                NtlmCause::ChallengeUnusable,
                "the request cannot be sent again",
            )
        };
        let leg2 = probe.try_clone().ok_or_else(unclonable)?;
        let leg3 = probe.try_clone().ok_or_else(unclonable)?;

        // The probe leg runs on the hop's deadline, which started before the PAC lookup; the
        // later legs each get a fresh one. Every deadline also covers draining its body.
        let timed_out = || ExecError::timeout(redact_url(url.as_str()));
        let mut leg = hop_deadline;
        let probe_sent = Sent::of(&probe);
        let mut started = Instant::now();
        let first = leg
            .run(client.execute(probe))
            .await
            .ok_or_else(timed_out)?
            .map_err(transport)?;
        let offered = www_authenticate(&first);
        let offered: Vec<&str> = offered.iter().map(String::as_str).collect();
        if first.status() != StatusCode::UNAUTHORIZED || !ntlm::offers_ntlm(&offered) {
            return Ok(Negotiated {
                response: first,
                warning: None,
                authorization: None,
                deadline: leg,
                started,
                earlier: gone(buf, base),
            });
        }
        if closes(&first) {
            return Err(ntlm_err(
                url,
                NtlmCause::ConnectionNotKept,
                lost("the server closed the connection after the first answer"),
            ));
        }
        let pinned = local_addr(&first);
        let hop = probe_sent.hop(&first, started, pac, proxy.is_some());
        let cookies = leg
            .run(self.drain(first, url, store_cookies))
            .await
            .ok_or_else(timed_out)??;
        buf.hops.push(Hop {
            elapsed: started.elapsed(),
            cookies,
            ..hop
        });

        let value = ntlm::authorization(&ntlm::type1());
        buf.place(&value);
        leg = Deadline::after(timeout);
        let leg2 = with_authorization(leg2, &value);
        let type1_sent = Sent::of(&leg2);
        started = Instant::now();
        let second = leg
            .run(client.execute(leg2))
            .await
            .ok_or_else(timed_out)?
            .map_err(later)?;
        check_pinned(url, pinned, &second, lost(NOT_ONE_CONNECTION))?;
        if second.status() != StatusCode::UNAUTHORIZED {
            return Ok(Negotiated {
                response: second,
                warning: None,
                authorization: Some(value),
                deadline: leg,
                started,
                earlier: gone(buf, base),
            });
        }
        let challenge = www_authenticate(&second);
        // The Type 2 blob is server-made, but it sits in a saved exchange as raw as the others.
        for header in &challenge {
            buf.place(header);
        }
        let challenge: Vec<&str> = challenge.iter().map(String::as_str).collect();
        let unusable =
            |e: ntlm::NtlmError| ntlm_err(url, NtlmCause::ChallengeUnusable, e.to_string());
        let parsed = ntlm::challenge_blob(&challenge)
            .and_then(|blob| ntlm::parse_type2(&blob))
            .map_err(unusable)?;
        let (client_challenge, filetime) = ntlm::fresh()
            .ok_or_else(|| ntlm_err(url, NtlmCause::Random, "no random bytes were available"))?;
        let message = ntlm::type3(&parsed, creds, client_challenge, filetime).map_err(unusable)?;
        let hop = type1_sent.hop(&second, started, pac, proxy.is_some());
        let cookies = leg
            .run(self.drain(second, url, store_cookies))
            .await
            .ok_or_else(timed_out)??;
        buf.hops.push(Hop {
            elapsed: started.elapsed(),
            cookies,
            ..hop
        });

        let value = ntlm::authorization(&message);
        buf.place(&value);
        leg = Deadline::after(timeout);
        started = Instant::now();
        let third = leg
            .run(client.execute(with_authorization(leg3, &value)))
            .await
            .ok_or_else(timed_out)?
            .map_err(later)?;
        check_pinned(url, pinned, &third, lost(NOT_ONE_CONNECTION))?;
        // A proxy that quietly switched upstream connections ends the handshake on the same 401
        // as a wrong password, so that case is told apart from a plain rejection.
        let warning = (absolute && third.status() == StatusCode::UNAUTHORIZED)
            .then_some(Warning::NtlmProxyUnauthorized);
        Ok(Negotiated {
            response: third,
            warning,
            authorization: Some(value),
            deadline: leg,
            started,
            earlier: gone(buf, base),
        })
    }

    /// Reads an intermediate response to its end, storing its cookies, so the connection is
    /// free for the next leg. The body cap of the send does not apply; a fixed one does.
    async fn drain(
        &self,
        mut res: reqwest::Response,
        url: &Url,
        store_cookies: bool,
    ) -> Result<Vec<SetCookie>, ExecError> {
        let set_cookies =
            (res.headers().get_all(reqwest::header::SET_COOKIE).iter()).map(|v| v.as_bytes());
        let cookies = if store_cookies {
            self.jar.store(set_cookies, url)
        } else {
            crate::cookies::not_stored(set_cookies)
        };
        let mut total = 0usize;
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|e| ntlm_err(url, NtlmCause::ConnectionNotKept, super::chain(e)))?
        {
            total += chunk.len();
            if total > CHALLENGE_BODY_LIMIT {
                return Err(ntlm_err(
                    url,
                    NtlmCause::ChallengeTooLarge,
                    "the server's challenge response is too large",
                ));
            }
        }
        Ok(cookies)
    }
}

/// The request with its `Authorization` replaced by `value`.
fn with_authorization(mut req: reqwest::Request, value: &str) -> reqwest::Request {
    if let Ok(v) = HeaderValue::from_str(value) {
        req.headers_mut().insert(reqwest::header::AUTHORIZATION, v);
    }
    req
}

/// A leg that arrived from another local address ran on another connection.
fn check_pinned(
    url: &Url,
    pinned: Option<std::net::SocketAddr>,
    res: &reqwest::Response,
    message: &str,
) -> Result<(), ExecError> {
    match (pinned, local_addr(res)) {
        (Some(a), Some(b)) if a == b => Ok(()),
        _ => Err(ntlm_err(url, NtlmCause::ConnectionNotReused, message)),
    }
}

#[cfg(test)]
mod tests;
