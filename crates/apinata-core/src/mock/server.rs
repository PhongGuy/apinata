//! The hyper accept loop and request handler behind [`super::start`].

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::ext::ReasonPhrase;
use hyper::{Request, Response};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::pick::{Candidate, Overrides, Pick, parse_query, pick};
use super::respond::{HitLog, fill_params, miss_body};
use super::routes::{RouteTable, path_segments};
use super::{HitHook, MockHit};

/// Largest request body read before answering 413.
const MAX_BODY: usize = 1024 * 1024;

/// Everything a connection needs, shared across tasks; only the route table is ever replaced.
pub(super) struct Shared {
    pub table: std::sync::RwLock<Arc<RouteTable>>,
    pub log: Arc<HitLog>,
    pub delay: Duration,
    pub hook: Option<HitHook>,
}

/// Accepts connections until the task is aborted; each one runs in a `JoinSet` owned here, so
/// aborting the task closes every connection and the listener together.
pub(super) async fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    let mut conns = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else {
                    // A failed accept (e.g. fd exhaustion) is transient; back off briefly.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                };
                let shared = Arc::clone(&shared);
                conns.spawn(async move {
                    let service = hyper::service::service_fn(move |req| {
                        let shared = Arc::clone(&shared);
                        async move { Ok::<_, Infallible>(handle(&shared, req).await) }
                    });
                    let io = hyper_util::rt::TokioIo::new(stream);
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service)
                        .await;
                });
            }
            // Reap finished connections; a panicked one is ignored and the loop goes on.
            Some(_) = conns.join_next(), if !conns.is_empty() => {}
        }
    }
}

fn json_response(status: u16, body: Vec<u8>, close: bool) -> Response<Full<Bytes>> {
    let mut builder = Response::builder()
        .status(status)
        .header("content-type", "application/json");
    if close {
        builder = builder.header("connection", "close");
    }
    builder
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

fn record(shared: &Shared, hit: MockHit) {
    // Logged first so a panicking hook cannot lose the hit.
    shared.log.push(hit.clone());
    if let Some(hook) = &shared.hook {
        hook(&hit);
    }
}

async fn handle(shared: &Shared, req: Request<Incoming>) -> Response<Full<Bytes>> {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let query = parse_query(req.uri().query());
    let overrides = Overrides {
        name: req
            .headers()
            .get("x-mock-response-name")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        code: req
            .headers()
            .get("x-mock-response-code")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok()),
    };

    // The body is never used, only bounded: an oversized one is refused and the connection
    // closed so the client sees the 413 instead of a hang.
    if Limited::new(req.into_body(), MAX_BODY)
        .collect()
        .await
        .is_err()
    {
        record(shared, MockHit::new(&method, &path, 413, None, false));
        let body = br#"{"error":"request body too large"}"#.to_vec();
        return json_response(413, body, true);
    }

    // A reload swaps the table; this request answers from the one current on arrival.
    let table = Arc::clone(&shared.table.read().unwrap_or_else(|e| e.into_inner()));
    let (route, others) = match table.find(&method, &path) {
        Some(route) => (Some(route), Vec::new()),
        None => (None, table.find_other_methods(&method, &path)),
    };
    let picked = match route {
        Some(route) => pick(route, &query, &overrides),
        None => Pick::Miss {
            candidates: others
                .iter()
                .flat_map(|r| {
                    r.entries.iter().map(|e| Candidate {
                        request: e.request.clone(),
                        example: e.example.name.clone(),
                        reason: format!("method is {}", r.method),
                    })
                })
                .take(20)
                .collect(),
        },
    };

    if !shared.delay.is_zero() {
        tokio::time::sleep(shared.delay).await;
    }

    match picked {
        Pick::Hit { entry, fallback } => {
            let p = &entry.prepared;
            // Path params fill `{{name}}` in the example; no other variable is ever resolved.
            let decoded = path_segments(&path);
            let params: Vec<(&str, &str)> = route
                .iter()
                .flat_map(|r| &r.segments)
                .zip(&decoded)
                .filter(|(t, _)| t.len() > 1 && t.starts_with(':'))
                .map(|(t, v)| (&t[1..], v.as_str()))
                .collect();
            let mut builder = Response::builder().status(p.status);
            for (name, raw) in &p.headers {
                // A substituted value that is not a valid header keeps the recorded text.
                let filled = fill_params(raw, &params);
                let value = if http::HeaderValue::from_bytes(filled.as_bytes()).is_ok() {
                    filled
                } else {
                    raw.clone()
                };
                builder = builder.header(name.as_str(), value);
            }
            let body = match std::str::from_utf8(&p.body) {
                Ok(text) if !p.binary && !params.is_empty() => {
                    fill_params(text, &params).into_bytes()
                }
                _ => p.body.clone(),
            };
            let mut response = builder
                .body(Full::new(Bytes::from(body)))
                .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())));
            if let Some(reason) = p
                .reason
                .as_deref()
                .and_then(|r| ReasonPhrase::try_from(r.as_bytes()).ok())
            {
                response.extensions_mut().insert(reason);
            }
            let hit = MockHit::new(
                &method,
                &path,
                p.status,
                Some((&entry.request, &entry.example.name)),
                fallback,
            );
            record(shared, hit);
            response
        }
        Pick::Miss { candidates } => {
            record(shared, MockHit::new(&method, &path, 404, None, false));
            json_response(404, miss_body(&method, &path, &candidates), false)
        }
    }
}
