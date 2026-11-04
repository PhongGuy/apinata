//! A local mock server that answers from a collection's saved examples.

mod pick;
mod respond;
mod routes;
mod server;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use crate::{Collection, ContextError, VarContext};

type HitHook = Arc<dyn Fn(&MockHit) + Send + Sync>;

/// Where and how the server listens.
#[derive(Clone)]
#[non_exhaustive]
pub struct MockOptions {
    ip: IpAddr,
    port: u16,
    delay: Duration,
    allow_non_loopback: bool,
    hook: Option<HitHook>,
}

impl Default for MockOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl MockOptions {
    /// Listens on 127.0.0.1:4010 with no delay and no hit hook.
    pub fn new() -> Self {
        MockOptions {
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 4010,
            delay: Duration::ZERO,
            allow_non_loopback: false,
            hook: None,
        }
    }

    /// The address to bind.
    pub fn bind(mut self, ip: IpAddr) -> Self {
        self.ip = ip;
        self
    }

    /// The port to bind; 0 picks a free one.
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// A pause before every answer.
    pub fn delay_ms(mut self, ms: u64) -> Self {
        self.delay = Duration::from_millis(ms);
        self
    }

    /// Permits binding an address that is not loopback.
    pub fn allow_non_loopback(mut self, allow: bool) -> Self {
        self.allow_non_loopback = allow;
        self
    }

    /// Calls `f` for every request answered, as it happens.
    pub fn on_hit(mut self, f: impl Fn(&MockHit) + Send + Sync + 'static) -> Self {
        self.hook = Some(Arc::new(f));
        self
    }
}

/// Why the server could not start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MockError {
    /// The bind address is not loopback and the caller did not allow it.
    #[error("refusing to bind a non-loopback address")]
    NonLoopbackBind,
    /// The listener could not be opened.
    #[error("{0}")]
    Bind(String),
    /// The variable context could not be built.
    #[error(transparent)]
    Context(#[from] ContextError),
}

/// A running server. Dropping it stops the server.
pub struct MockHandle {
    addr: SocketAddr,
    routes: Vec<MockRoute>,
    warnings: Vec<String>,
    log: Arc<respond::HitLog>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl MockHandle {
    /// The address actually bound.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The routes the server answers.
    pub fn routes(&self) -> &[MockRoute] {
        &self.routes
    }

    /// Startup problems that left a request or header out.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The most recent hits, oldest first.
    pub fn hits(&self) -> Vec<MockHit> {
        self.log.snapshot()
    }

    /// Stops the server; the listener is closed when this returns.
    pub async fn stop(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for MockHandle {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Builds the route table from `collection`, binds the listener and starts answering.
///
/// The loopback guard runs before anything else is built. Fails with
/// [`MockError::NonLoopbackBind`] or [`MockError::Bind`].
pub async fn start(
    collection: &Collection,
    ctx: &VarContext,
    options: MockOptions,
) -> Result<MockHandle, MockError> {
    if !options.allow_non_loopback && !crate::trust::is_loopback(options.ip) {
        return Err(MockError::NonLoopbackBind);
    }
    let (table, warnings) = routes::RouteTable::build(collection, ctx);
    let routes = table
        .routes()
        .iter()
        .map(|r| MockRoute {
            method: r.method.clone(),
            path: format!("/{}", r.segments.join("/")),
            examples: r.entries.len(),
        })
        .collect();

    let want = SocketAddr::new(options.ip, options.port);
    let listener = tokio::net::TcpListener::bind(want)
        .await
        .map_err(|e| MockError::Bind(format!("cannot listen on {want}: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| MockError::Bind(format!("cannot listen on {want}: {e}")))?;

    let log = Arc::new(respond::HitLog::new());
    let shared = Arc::new(server::Shared {
        table,
        log: log.clone(),
        delay: options.delay,
        hook: options.hook,
    });
    let task = tokio::spawn(server::accept_loop(listener, shared));
    Ok(MockHandle {
        addr,
        routes,
        warnings,
        log,
        task: Some(task),
    })
}

/// A route the server answers: a method, a path template and how many examples back it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MockRoute {
    /// The upper-case HTTP method.
    pub method: String,
    /// The path template, e.g. `/api/users/:id`.
    pub path: String,
    /// How many saved examples back the route.
    pub examples: usize,
}

/// One request the server answered.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MockHit {
    /// RFC 3339 UTC, to the second.
    pub at: String,
    /// The upper-case HTTP method of the request.
    pub method: String,
    /// The request path as received.
    pub path: String,
    /// The status sent.
    pub status: u16,
    /// The request's item path, or `None` for a miss.
    pub request: Option<String>,
    /// The example's name, or `None` for a miss.
    pub example: Option<String>,
    /// Whether the example was chosen as the fallback rather than matched.
    pub fallback: bool,
}

impl MockHit {
    pub(crate) fn new(
        method: &str,
        path: &str,
        status: u16,
        hit: Option<(&str, &str)>,
        fallback: bool,
    ) -> Self {
        MockHit {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            method: method.to_string(),
            path: path.to_string(),
            status,
            request: hit.map(|(r, _)| r.to_string()),
            example: hit.map(|(_, e)| e.to_string()),
            fallback,
        }
    }

    /// A one-line log entry: `<at> <METHOD> <path> -> <status> <request> / <example>`.
    pub fn line(&self) -> String {
        let head = format!("{} {} {} -> ", self.at, self.method, self.path);
        match (&self.request, &self.example) {
            (Some(r), Some(e)) => {
                let tail = if self.fallback { " (fallback)" } else { "" };
                format!("{head}{} {r} / {e}{tail}", self.status)
            }
            _ if self.status == 413 => format!("{head}413 (rejected)"),
            _ => format!("{head}{} (no match)", self.status),
        }
    }
}

/// Builds throwaway collections for the module's unit tests.
#[cfg(test)]
pub(crate) mod testkit {
    use crate::Collection;

    /// Opens a temporary collection holding `files` (relative path, content).
    pub fn collection(files: &[(&str, &str)]) -> (tempfile::TempDir, Collection) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("opencollection.yml"),
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        )
        .unwrap();
        for (path, content) in files {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, content).unwrap();
        }
        let c = Collection::open(root).unwrap();
        (dir, c)
    }

    /// A request file; the URL is quoted because `{{x}}/b` is a YAML flow mapping when bare.
    pub fn request(name: &str, seq: u32, method: &str, url: &str, examples: &str) -> String {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: {method}\n  url: \"{url}\"\n{examples}"
        )
    }
}
