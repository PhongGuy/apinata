//! Sends a resolved request over HTTP/1.1, following redirects by hand the way bruno-cli's
//! axios instance does, and returns the last hop as a [`Response`].

use std::error::Error as _;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hyper_util::client::proxy::matcher::Matcher;
use reqwest::dns::Resolve;
use reqwest::header::{
    AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, SET_COOKIE,
    WWW_AUTHENTICATE,
};
use reqwest::redirect::Policy;
use reqwest::{Client, Identity, Method, StatusCode};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::Role;
use url::Url;

use crate::cancel::CancelHandle;
use crate::cookies::CookieJar;
use crate::default_headers::{defaults_without, omits};
use crate::digest;
use crate::edgegrid;
use crate::host_guard::{
    Guard, HostPolicy, Lookup, SystemLookup, judge_with_overrides, refusal_in,
};
use crate::http_version::{HttpVersion, HttpVersionPolicy};
use crate::import::SourceFetch;
use crate::multipart;
use crate::network::{
    ClientKey, Clients, FetchOptions, GlobalProxy, NetworkCheck, NetworkSettings, OverrideResolve,
    PAC_CACHE, PacCache, PacSend, PacWarning, Preloaded, ProxyRule, RequestProxy,
    ResolvedHostOverride, ResolvedProxy, SystemProxy, apply_rule, label_for, load_identity,
    overridden, read_ca, resolve_hop, route, rule_for,
};
use crate::ntlm;
use crate::oauth1;
use crate::resolve::{Body, FileRef, ResolvedRequest, SendAuth, redact_url};
use crate::response::{HOP_BODY_LIMIT, Hop, Response};
use crate::session_log::{
    Exchange, Logged, NetworkEntry, Redactor, SendHold, SendSource, SessionLog,
};
use crate::stream::{StreamBound, StreamEnd, StreamEvent, StreamOptions};
use crate::subscription::{ack_wait, detect, handshake, socket, unstreamed_limits};
use crate::timing::{Recorder, SystemResolve, TimedConnect, TimedResolve};
use crate::trust::normalize_host;
use crate::wsse;

mod deadline;
mod ntlm_flow;

use deadline::Deadline;

/// How the executor's HTTP client is built.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct ExecOptions {
    insecure: bool,
    max_body_bytes: Option<u64>,
    host_policy: Option<HostPolicy>,
    confine_files: Option<PathBuf>,
    network: Option<NetworkSettings>,
    cookie_jar: Option<Arc<CookieJar>>,
    // Stored inverted so the derived `Default` leaves both switches on.
    no_store_cookies: bool,
    no_send_cookies: bool,
    session_log: Option<SessionLog>,
    http_version: HttpVersionPolicy,
    timeout_cap: Option<Duration>,
    network_check: Option<NetworkCheck>,
}

/// Shows whether a host policy and a jar are set, plus the effective cookie switches: the
/// closure has no `Debug`, and the jar holds cookie values.
impl fmt::Debug for ExecOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecOptions")
            .field("insecure", &self.insecure)
            .field("max_body_bytes", &self.max_body_bytes)
            .field("host_policy", &self.host_policy.is_some())
            .field("confine_files", &self.confine_files)
            .field("network", &self.network)
            .field("cookie_jar", &self.cookie_jar.is_some())
            .field("store_cookies", &!self.no_store_cookies)
            .field("send_cookies", &!self.no_send_cookies)
            .field("session_log", &self.session_log.is_some())
            .field("http_version", &self.http_version)
            .field("timeout_cap", &self.timeout_cap)
            .field("network_check", &self.network_check)
            .finish()
    }
}

impl ExecOptions {
    /// The defaults: certificates are verified.
    pub fn new() -> ExecOptions {
        ExecOptions::default()
    }

    /// Accept invalid certificates (self-signed, expired, wrong host) when `insecure` is true.
    pub fn insecure(mut self, insecure: bool) -> ExecOptions {
        self.insecure = insecure;
        self
    }

    /// Stop reading a response body past `limit` decoded bytes, keeping exactly `limit` of them
    /// and marking the response truncated. `None`, the default, reads a plain `send` whole but caps
    /// a streamed body at 10 MiB; pass `Some(u64::MAX)` to lift the cap there too.
    pub fn max_body_bytes(mut self, limit: Option<u64>) -> ExecOptions {
        self.max_body_bytes = limit;
        self
    }

    /// Refuse, before any I/O for it, every hop whose host is neither a loopback literal nor
    /// `localhost` (or a name under it) and that `trusted` rejects. `trusted` receives the host
    /// normalised as [`crate::normalize_host`] does. Without this, any host is sent to.
    pub fn host_policy(mut self, trusted: Arc<dyn Fn(&str) -> bool + Send + Sync>) -> ExecOptions {
        self.host_policy = Some(trusted);
        self
    }

    /// Refuse, before reading any, a body file that is absolute or resolves outside `root`
    /// through `..` or symlinks. Without this, any path is read.
    pub fn confine_files(mut self, root: impl Into<PathBuf>) -> ExecOptions {
        self.confine_files = Some(root.into());
        self
    }

    /// The proxy layers and CA roots. Without this, collections that inherit connect directly.
    pub fn network(mut self, settings: NetworkSettings) -> ExecOptions {
        self.network = Some(settings);
        self
    }

    /// Which HTTP versions the clients may speak; HTTP/1.1 only by default.
    /// A request in NTLM mode always speaks HTTP/1.1, whatever the policy says.
    pub fn http_version(mut self, policy: HttpVersionPolicy) -> ExecOptions {
        self.http_version = policy;
        self
    }

    /// Caps every request's timeout, token fetches and redirect hops included: the lower of the
    /// request's own and `cap` applies, and a request with none gets `cap`. A timeout caused
    /// by the cap, not the request's own value, names it in `ExecError::Timeout`.
    pub fn timeout_cap(mut self, cap: Option<Duration>) -> ExecOptions {
        self.timeout_cap = cap;
        self
    }

    /// Check each send's network settings against the acknowledgement record before it goes
    /// out, asking through `check` when they changed.
    pub fn network_check(mut self, check: NetworkCheck) -> ExecOptions {
        self.network_check = Some(check);
        self
    }

    /// The jar every send reads and writes; a fresh in-memory jar when not set.
    pub fn cookie_jar(mut self, jar: Arc<CookieJar>) -> ExecOptions {
        self.cookie_jar = Some(jar);
        self
    }

    /// Store each hop's `Set-Cookie` in the jar when `on`; on by default.
    pub fn store_cookies(mut self, on: bool) -> ExecOptions {
        self.no_store_cookies = !on;
        self
    }

    /// Send the jar's cookies for each hop when `on`; on by default. Off, a request's own
    /// `Cookie` header goes as written.
    pub fn send_cookies(mut self, on: bool) -> ExecOptions {
        self.no_send_cookies = !on;
        self
    }

    /// Records every send into `log`; without it nothing is recorded.
    pub fn session_log(mut self, log: SessionLog) -> ExecOptions {
        self.session_log = Some(log);
        self
    }
}

/// What a send is recorded as: its source, the values redacted beyond the request's own and
/// the hold of the top-level send it belongs to, which stages its entry instead of the log.
#[derive(Clone)]
pub(crate) struct SendTag {
    pub(crate) source: SendSource,
    pub(crate) extra: Vec<String>,
    pub(crate) hold: Option<SendHold>,
    /// Set for a token request: the tokens a JSON object answer issues join the redactor.
    pub(crate) response_tokens: bool,
    /// Set to read the final response's body as a stream.
    pub(crate) stream: Option<crate::stream::StreamOptions>,
    /// This send's number and how many sends a retrying request may make; `None` without retries.
    pub(crate) attempt: Option<(u32, u32)>,
}

impl Default for SendTag {
    fn default() -> SendTag {
        SendTag {
            source: SendSource::Request,
            extra: Vec::new(),
            hold: None,
            response_tokens: false,
            stream: None,
            attempt: None,
        }
    }
}

/// What one send's exchange leaves behind, owned by the caller so a cancel keeps it.
#[derive(Default)]
pub(crate) struct HopBuffer {
    /// The first hop's sent bytes (at most `HOP_BODY_LIMIT`) and their full length.
    pub(crate) first_body: Option<(Vec<u8>, u64)>,
    /// The hops completed so far; on success they move into the response's timeline.
    pub(crate) hops: Vec<Hop>,
    /// Credentials the exchange computed per hop, to be redacted.
    pub(crate) placed: Vec<String>,
    /// Set when a log is attached, so the exchange keeps what only the log needs.
    record: bool,
}

impl HopBuffer {
    /// Keeps a credential this send placed on the wire, for its entry's redactor.
    fn place(&mut self, value: &str) {
        if !self.placed.iter().any(|v| v == value) {
            self.placed.push(value.to_owned());
        }
    }
}

/// The `access_token`, `refresh_token` and `id_token` strings of a token answer: a JSON object,
/// or failing that a form-encoded body, which the token request rejects but the log still shows.
fn issued_tokens(text: &str) -> Vec<String> {
    const FIELDS: [&str; 3] = ["access_token", "refresh_token", "id_token"];
    let found: Vec<String> = match serde_json::from_str(text) {
        Ok(serde_json::Value::Object(object)) => FIELDS
            .iter()
            .filter_map(|k| object.get(*k).and_then(serde_json::Value::as_str))
            .map(str::to_owned)
            .collect(),
        Ok(_) => Vec::new(),
        Err(_) => url::form_urlencoded::parse(text.as_bytes())
            .filter(|(k, _)| FIELDS.contains(&k.as_ref()))
            .map(|(_, v)| v.into_owned())
            .collect(),
    };
    found.into_iter().filter(|t| !t.is_empty()).collect()
}

/// Why a connection to a hop's host failed, as far as the error chain shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectCause {
    /// Nothing listens at the address (`io::ErrorKind::ConnectionRefused`).
    Refused,
    /// The host name could not be resolved.
    Dns,
    /// The TLS handshake failed, certificate checks included, for a reason not told apart below.
    Tls,
    /// The server's certificate chains to no trusted issuer, or is self-signed.
    TlsUnknownIssuer,
    /// The server did not answer TLS at all, as a plain-http server on an `https://` URL.
    TlsWrongProtocol,
    /// The server's certificate has expired.
    TlsExpired,
    /// Anything else.
    Other,
}

/// Why an NTLM handshake failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NtlmCause {
    /// The server closed the connection the handshake needs to keep.
    ConnectionNotKept,
    /// The handshake would have needed a second connection.
    ConnectionNotReused,
    /// The server's challenge could not be read.
    ChallengeUnusable,
    /// The server's challenge was larger than allowed.
    ChallengeTooLarge,
    /// NTLM over a proxy to an http origin is not supported.
    ProxyUnsupported,
    /// No random bytes were available for the response.
    Random,
}

/// Why a request could not be sent or its response not read.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum ExecError {
    /// The HTTP client could not be built.
    #[error("could not build the HTTP client: {message}")]
    #[non_exhaustive]
    Client {
        /// The underlying error.
        message: String,
    },
    /// The NTLM handshake could not be completed.
    #[error("{url}: NTLM: {message}")]
    #[non_exhaustive]
    Ntlm {
        /// The URL being negotiated, without credentials.
        url: String,
        /// What went wrong.
        cause: NtlmCause,
        /// A description that names the cause only, never a credential or a header.
        message: String,
    },
    /// The request's method or URL cannot be sent.
    #[error("invalid request: {message}")]
    #[non_exhaustive]
    InvalidRequest {
        /// What is wrong with it.
        message: String,
    },
    /// A hop did not finish within the request's timeout.
    #[error("{url}: timed out{}", run_wide_note(*run_wide))]
    #[non_exhaustive]
    Timeout {
        /// The URL of the hop that timed out.
        url: String,
        /// The run-wide cap, when it rather than the request's own timeout was the limit.
        run_wide: Option<Duration>,
    },
    /// The connection to a hop's host failed, including its TLS handshake.
    #[error("{url}: could not connect: {message}")]
    #[non_exhaustive]
    Connect {
        /// The URL of the hop that failed.
        url: String,
        /// The underlying error.
        message: String,
        /// Why it failed, as far as the error chain shows it.
        cause: ConnectCause,
    },
    /// Connecting through the hop's proxy failed: unreachable, refused, a CONNECT answered with
    /// an error such as 407, or a SOCKS rejection.
    #[error("{url}: proxy {proxy}: {message}")]
    #[non_exhaustive]
    Proxy {
        /// The URL of the hop that failed.
        url: String,
        /// `scheme://host:port`, never credentials.
        proxy: String,
        /// The underlying error, with the proxy's credentials, raw and percent-encoded, removed.
        message: String,
    },
    /// Under a host policy, a sensitive value reached the proxy's host or port, so nothing was sent.
    #[error("a secret cannot be used in a proxy host or port")]
    SecretInProxy,
    /// Sending a hop or reading its response failed.
    #[error("{url}: {message}")]
    #[non_exhaustive]
    Transport {
        /// The URL of the hop that failed.
        url: String,
        /// The underlying error.
        message: String,
    },
    /// A file the body names could not be read, so nothing was sent.
    #[error("{}: {message}", path.display())]
    #[non_exhaustive]
    File {
        /// The path as the request file wrote it, never interpolated.
        path: PathBuf,
        /// The underlying error.
        message: String,
    },
    /// The send was cancelled through its `CancelHandle` before it finished.
    #[error("cancelled")]
    Cancelled,
    /// The host policy refused a hop's host, so nothing was sent to it.
    #[error("host `{host}` is not allowed (hop {hop})")]
    #[non_exhaustive]
    HostNotAllowed {
        /// The redirects followed before the refused hop; 0 is the request itself.
        hop: usize,
        /// The refused host, normalised; for an overridden name, the mapped address.
        host: String,
        /// The overridden name whose mapped address `host` is, when there is one.
        mapped_from: Option<String>,
    },
    /// A hop's host has a host override but goes through a proxy, which resolves it instead.
    #[error(
        "host `{host}` has a host override but hop {hop} goes through a proxy, which ignores it"
    )]
    #[non_exhaustive]
    HostOverrideViaProxy {
        /// The redirects followed before the hop; 0 is the request itself.
        hop: usize,
        /// The overridden host, normalised.
        host: String,
    },
    /// The host policy refused the proxy a hop goes through or, through a local-DNS SOCKS
    /// proxy, the target it resolves on the dial, so nothing was sent.
    #[error("host `{host}` is not allowed (hop {hop}, via proxy {proxy})")]
    #[non_exhaustive]
    ProxyNotAllowed {
        /// The redirects followed before the refused hop; 0 is the request itself.
        hop: usize,
        /// The refused host (the proxy's, or the target's on a local-DNS SOCKS dial), normalised.
        host: String,
        /// The proxy as `scheme://host:port`, without credentials.
        proxy: String,
    },
    /// Under a host policy, a sensitive value appeared in the host, so nothing was sent.
    #[error("a secret cannot be used in a host: `{host}`")]
    #[non_exhaustive]
    SecretInHost {
        /// The host as the request file templated it, never interpolated.
        host: String,
    },
    /// Under file confinement, a body file lay outside the collection, so nothing was read or sent.
    #[error("{}: outside the collection", path.display())]
    #[non_exhaustive]
    FileOutsideCollection {
        /// The path as the request file wrote it, never interpolated.
        path: PathBuf,
    },
    /// The network settings changed and the person did not confirm the send, so nothing was sent.
    #[error("the network settings changed and the send was not confirmed")]
    NetworkDeclined,
    /// A client certificate, its key or the CA file could not be used, so nothing was sent.
    #[error("{}: {message}", path.display())]
    #[non_exhaustive]
    Certificate {
        /// The path as written in the collection or given on the command line.
        path: PathBuf,
        /// What is wrong and, where there is one, the fix.
        message: String,
    },
}

impl ExecError {
    /// Builds a `Connect` error for the hop at `url`.
    pub fn connect(
        url: impl Into<String>,
        message: impl Into<String>,
        cause: ConnectCause,
    ) -> Self {
        Self::Connect {
            url: url.into(),
            message: message.into(),
            cause,
        }
    }

    /// Builds a `Timeout` error for the hop at `url`.
    pub fn timeout(url: impl Into<String>) -> Self {
        Self::Timeout {
            url: url.into(),
            run_wide: None,
        }
    }

    /// Builds a `Proxy` error for the hop at `url` going through `proxy`.
    pub fn proxy(
        url: impl Into<String>,
        proxy: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::Proxy {
            url: url.into(),
            proxy: proxy.into(),
            message: message.into(),
        }
    }

    /// Builds a `Transport` error for the hop at `url`.
    pub fn transport(url: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Transport {
            url: url.into(),
            message: message.into(),
        }
    }

    /// Builds a `File` error for a body file at `path`.
    pub fn file(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::File {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Builds a `HostNotAllowed` error for `host` at redirect `hop`.
    pub fn host_not_allowed(hop: usize, host: impl Into<String>) -> Self {
        Self::HostNotAllowed {
            hop,
            host: host.into(),
            mapped_from: None,
        }
    }

    /// Builds a `ProxyNotAllowed` error for `host` at redirect `hop`, via `proxy`.
    pub fn proxy_not_allowed(
        hop: usize,
        host: impl Into<String>,
        proxy: impl Into<String>,
    ) -> Self {
        Self::ProxyNotAllowed {
            hop,
            host: host.into(),
            proxy: proxy.into(),
        }
    }

    /// Builds a `Certificate` error for the certificate, key or CA file at `path`.
    pub fn certificate(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::Certificate {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// Sends resolved requests, sharing one cookie jar and one connection pool per proxy rule.
#[non_exhaustive]
pub struct Executor {
    clients: Clients,
    network: NetworkSettings,
    system: Arc<SystemProxy>,
    insecure: bool,
    lookup: Arc<dyn Lookup>,
    jar: Arc<CookieJar>,
    max_body_bytes: Option<u64>,
    host_policy: Option<HostPolicy>,
    confine_files: Option<PathBuf>,
    pac_cache: &'static PacCache,
    // The flags guard no other memory, so `Relaxed` loads and stores suffice.
    store_cookies: AtomicBool,
    send_cookies: AtomicBool,
    session_log: Option<SessionLog>,
    http_version: HttpVersionPolicy,
    timeout_cap: Option<Duration>,
    network_check: Option<NetworkCheck>,
}

/// Leaves the jar out, since it holds cookie values.
impl fmt::Debug for Executor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Executor")
            .field("network", &self.network)
            .field("insecure", &self.insecure)
            .finish_non_exhaustive()
    }
}

/// Whether `given` is absolute or has a `..` that climbs above the root it is joined to.
/// Purely lexical: it never touches the disk.
pub(crate) fn escapes_root(given: &Path) -> bool {
    let mut depth = 0usize;
    for part in given.components() {
        match part {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return true,
        }
    }
    false
}

/// Refuses the first of `files` outside `root`. The lexical check runs before any I/O; only
/// paths that pass it are canonicalised, which reads metadata, never contents.
fn confine(files: &[FileRef], root: &Path) -> Result<(), ExecError> {
    let outside = |file: &FileRef| ExecError::FileOutsideCollection {
        path: file.written.clone(),
    };
    let canonical_root = std::fs::canonicalize(root).ok();
    for file in files {
        if escapes_root(&file.given) {
            return Err(outside(file));
        }
        // Symlinks need the filesystem: compare canonical paths, reading metadata only.
        let canonical = std::fs::canonicalize(&file.path).map_err(|e| ExecError::File {
            path: file.written.clone(),
            message: e.to_string(),
        })?;
        if !canonical_root
            .as_ref()
            .is_some_and(|root| canonical.starts_with(root))
        {
            return Err(outside(file));
        }
    }
    Ok(())
}

/// The status codes bruno-cli's redirect loop follows.
const REDIRECTS: [u16; 5] = [301, 302, 303, 307, 308];

/// Methods whose repeat cannot change the outcome (RFC 9110 section 9.2.2).
fn is_idempotent(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::PUT | Method::DELETE | Method::TRACE
    )
}

/// Whether a failed send shows the peer closed a connection before answering: hyper saw the
/// stream end mid-message, or the socket was reset, aborted, broken or hit end of file.
fn is_stale_connection(e: &reqwest::Error) -> bool {
    use std::io::ErrorKind::{BrokenPipe, ConnectionAborted, ConnectionReset, UnexpectedEof};
    let mut source = e.source();
    while let Some(err) = source {
        if err
            .downcast_ref::<hyper::Error>()
            .is_some_and(hyper::Error::is_incomplete_message)
            || err.downcast_ref::<std::io::Error>().is_some_and(|io| {
                matches!(
                    io.kind(),
                    ConnectionReset | BrokenPipe | ConnectionAborted | UnexpectedEof
                )
            })
        {
            return true;
        }
        source = err.source();
    }
    false
}

/// The methods Node's http client sends without a `content-length` when the body is empty.
const NO_EMPTY_LENGTH: [Method; 6] = [
    Method::GET,
    Method::HEAD,
    Method::DELETE,
    Method::OPTIONS,
    Method::TRACE,
    Method::CONNECT,
];

impl Executor {
    /// An executor whose clients follow no redirects themselves, so `send` can apply Bruno's
    /// rules. Clients are built on first use, so a client error surfaces from `send`.
    pub fn new(opts: ExecOptions) -> Result<Executor, ExecError> {
        Executor::with_lookup(opts, Arc::new(SystemLookup))
    }

    /// Like `new`, with `lookup` answering the names a host policy trusts.
    pub(crate) fn with_lookup(
        opts: ExecOptions,
        lookup: Arc<dyn Lookup>,
    ) -> Result<Executor, ExecError> {
        let network = opts
            .network
            .unwrap_or_else(|| NetworkSettings::new().proxy(GlobalProxy::Off));
        // The system proxy is read once, and only when some request could inherit it.
        let system = match network.proxy {
            GlobalProxy::System => SystemProxy::from_system(),
            _ => SystemProxy::new(Matcher::builder().build()),
        };
        Ok(Executor {
            clients: Clients::default(),
            network,
            system: Arc::new(system),
            insecure: opts.insecure,
            lookup,
            jar: opts.cookie_jar.unwrap_or_default(),
            max_body_bytes: opts.max_body_bytes,
            host_policy: opts.host_policy,
            confine_files: opts.confine_files,
            pac_cache: &PAC_CACHE,
            store_cookies: AtomicBool::new(!opts.no_store_cookies),
            send_cookies: AtomicBool::new(!opts.no_send_cookies),
            session_log: opts.session_log,
            http_version: opts.http_version,
            timeout_cap: opts.timeout_cap,
            network_check: opts.network_check,
        })
    }

    /// A client that routes every connection by `key`'s rule. Under a host policy the guarding
    /// resolver is installed and pooling is off, so every connection is judged anew.
    fn build_client(&self, key: &ClientKey) -> Result<Client, ExecError> {
        let mut builder = self.base_builder(key)?;
        // The policy sets the TLS ALPN offer too; Auto leaves reqwest's `h2, http/1.1` default.
        builder = if key.http1_only {
            builder.http1_only()
        } else {
            match self.http_version {
                HttpVersionPolicy::Http1Only => builder.http1_only(),
                HttpVersionPolicy::Auto => builder,
                HttpVersionPolicy::Http2PriorKnowledge => builder.http2_prior_knowledge(),
            }
        };
        // A host policy re-resolves every hop, so no idle connection may outlive its check.
        if self.host_policy.is_some() {
            builder = builder.pool_max_idle_per_host(0);
        }
        builder
            .build()
            .map_err(|e| ExecError::Client { message: chain(e) })
    }

    /// The rules every client shares: TLS, CA, identity, proxy closure and the guard resolver.
    /// The HTTP version and pooling are left to the caller.
    fn base_builder(&self, key: &ClientKey) -> Result<reqwest::ClientBuilder, ExecError> {
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .tls_danger_accept_invalid_certs(self.insecure);
        if !key.compress {
            builder = builder.no_gzip().no_deflate().no_brotli().no_zstd();
        }
        let mut resolver: Arc<dyn Resolve> = match &self.host_policy {
            Some(policy) => Arc::new(Guard {
                policy: Arc::clone(policy),
                lookup: Arc::clone(&self.lookup),
            }),
            None => Arc::new(SystemResolve),
        };
        // Overridden names are answered from their rows, and judged again under a policy.
        if !key.overrides.is_empty() {
            resolver = Arc::new(OverrideResolve {
                overrides: Arc::clone(&key.overrides),
                policy: self.host_policy.clone(),
                inner: resolver,
            });
        }
        builder = builder.dns_resolver(TimedResolve(resolver));
        builder = builder.connector_layer(TimedConnect);
        // The CA file is read per build, so a broken one fails the send rather than `new`.
        if let Some(path) = &self.network.ca_file {
            let roots = read_ca(path)?;
            builder = if self.network.keep_default_roots {
                builder.tls_certs_merge(roots)
            } else {
                builder.tls_certs_only(roots)
            };
        }
        if let Some(pem) = &key.identity {
            let identity =
                Identity::from_pem(pem).map_err(|e| ExecError::Client { message: chain(e) })?;
            builder = builder.identity(identity);
        }
        let builder = apply_rule(builder, &key.rule, &self.system);
        Ok(builder)
    }

    /// Refuses the hop to `url` when a host policy is set and rejects its host; no-op otherwise.
    fn check_hop(
        &self,
        url: &Url,
        hop: usize,
        overrides: &[ResolvedHostOverride],
    ) -> Result<(), ExecError> {
        match &self.host_policy {
            Some(policy) => judge_with_overrides(url, policy, overrides).map_err(|r| {
                ExecError::HostNotAllowed {
                    hop,
                    host: r.host,
                    mapped_from: r.mapped_from,
                }
            }),
            None => Ok(()),
        }
    }

    /// Judges the hop to `url`, then the proxy `rule` sends it through, which it returns. The
    /// proxy is judged by its own address, so a bypassed hop never asks about it.
    fn check_route(
        &self,
        rule: &ProxyRule,
        url: &Url,
        hop: usize,
        overrides: &[ResolvedHostOverride],
    ) -> Result<Option<ResolvedProxy>, ExecError> {
        // Accepted limit: through an HTTP(S) proxy or remote-DNS SOCKS (socks5h, socks4a) only
        // the proxy host is resolved here; the target is judged by name and the proxy resolves it.
        self.check_hop(url, hop, overrides)?;
        let proxy = route(rule, &self.system, url);
        if let Some(proxy) = &proxy {
            self.check_hop(&proxy.address(), hop, overrides)
                .map_err(|e| match e {
                    ExecError::HostNotAllowed { hop, host, .. } => ExecError::ProxyNotAllowed {
                        hop,
                        host,
                        proxy: proxy.to_string(),
                    },
                    other => other,
                })?;
            // The proxy resolves the target, so a mapping for it would be skipped without a word.
            let name = url.host_str().and_then(normalize_host).unwrap_or_default();
            if overridden(overrides, &name).is_some() {
                return Err(ExecError::HostOverrideViaProxy { hop, host: name });
            }
        }
        Ok(proxy)
    }

    /// The concrete rule for the hop to `url` and its PAC line: the request rule unchanged outside
    /// PAC; under PAC the target is judged first, then the script's answer becomes Direct or Custom.
    async fn hop_rule(
        &self,
        rule: &ProxyRule,
        url: &Url,
        hop: usize,
        pac: &mut PacSend,
        deadline: &Deadline,
        overrides: &[ResolvedHostOverride],
    ) -> Result<(ProxyRule, Option<String>), ExecError> {
        let ProxyRule::Pac(source) = rule else {
            return Ok((rule.clone(), None));
        };
        // A refused target never reaches the script, its fetch or its DNS helpers.
        self.check_hop(url, hop, overrides)?;
        let fetch_opts = FetchOptions::new(
            self.insecure,
            self.network.ca_file.clone(),
            self.network.keep_default_roots,
        );
        // The PAC fetch and evaluation spend the hop's own timeout, like its send does.
        let (proxy, line) = deadline
            .run(resolve_hop(self.pac_cache, source, &fetch_opts, url, pac))
            .await
            .ok_or_else(|| ExecError::timeout(redact_url(url.as_str())))?;
        let rule = match proxy {
            Some(proxy) => ProxyRule::Custom {
                proxy,
                bypass: String::new(),
            },
            None => ProxyRule::Direct,
        };
        Ok((rule, Some(line)))
    }

    /// The fetch settings that match this executor's network settings and insecure flag, for
    /// fetching an import source the way requests are sent. It has no host policy; the caller
    /// adds one.
    #[must_use]
    pub fn source_fetch(&self) -> SourceFetch {
        let mut fetch = SourceFetch::default().network(self.network.clone());
        fetch.insecure = self.insecure;
        fetch
    }

    /// The cookies every send has stored, for listing and deletion.
    pub fn cookies(&self) -> &CookieJar {
        &self.jar
    }

    /// Turn storing `Set-Cookie` on or off from the next send; a send in flight keeps its policy.
    pub fn set_store_cookies(&self, on: bool) {
        self.store_cookies.store(on, Ordering::Relaxed);
    }

    /// Turn sending the jar's cookies on or off from the next send; a send in flight keeps its
    /// policy.
    pub fn set_send_cookies(&self, on: bool) {
        self.send_cookies.store(on, Ordering::Relaxed);
    }

    /// Whether the next send stores the `Set-Cookie` headers it receives.
    pub fn store_cookies_enabled(&self) -> bool {
        self.store_cookies.load(Ordering::Relaxed)
    }

    /// Whether the next send sends the jar's cookies.
    pub fn send_cookies_enabled(&self) -> bool {
        self.send_cookies.load(Ordering::Relaxed)
    }

    /// Sends `req`, following redirects as its settings allow, and returns the last response.
    pub async fn send(&self, req: &ResolvedRequest) -> Result<Response, ExecError> {
        self.send_with(req, &CancelHandle::new()).await
    }

    /// Like `send`, but fails with `ExecError::Cancelled` once `cancel` is cancelled.
    /// Cookies stored by hops that finished before the cancel stay in the jar.
    pub async fn send_with(
        &self,
        req: &ResolvedRequest,
        cancel: &CancelHandle,
    ) -> Result<Response, ExecError> {
        self.send_as(req, cancel, &SendTag::default()).await
    }

    /// Like `send_with`, in streaming mode: the final hop's body is read incrementally.
    pub async fn send_streaming(
        &self,
        req: &ResolvedRequest,
        cancel: &CancelHandle,
        stream: StreamOptions,
    ) -> Result<Response, ExecError> {
        let tag = SendTag {
            stream: Some(stream),
            ..SendTag::default()
        };
        self.send_as_reporting(req, cancel, &tag).await.0
    }

    /// `send_with`, plus the PAC warnings of this send, kept even when it fails or is cancelled.
    pub async fn send_reporting(
        &self,
        req: &ResolvedRequest,
        cancel: &CancelHandle,
    ) -> (Result<Response, ExecError>, Vec<PacWarning>) {
        self.send_as_reporting(req, cancel, &SendTag::default())
            .await
    }

    /// The log sends are recorded into, when one is attached.
    pub(crate) fn session_log(&self) -> Option<&SessionLog> {
        self.session_log.as_ref()
    }

    /// Like `send_with`, recording the send as `tag` says into the attached log, if any.
    /// Recording never changes the send's result.
    pub(crate) async fn send_as(
        &self,
        req: &ResolvedRequest,
        cancel: &CancelHandle,
        tag: &SendTag,
    ) -> Result<Response, ExecError> {
        self.send_as_reporting(req, cancel, tag).await.0
    }

    /// `send_as`, plus the PAC warnings of this send, kept even when it fails or is cancelled.
    pub(crate) async fn send_as_reporting(
        &self,
        req: &ResolvedRequest,
        cancel: &CancelHandle,
        tag: &SendTag,
    ) -> (Result<Response, ExecError>, Vec<PacWarning>) {
        // The network check runs before anything is read or logged, on every attempt, so a
        // declined send leaves no trace and each attempt hashes the certificate it will load.
        let preloaded = match &self.network_check {
            Some(check) => match check.run(req, cancel).await {
                Ok(loaded) => Some(loaded),
                Err(e) => return (Err(e), Vec::new()),
            },
            None => None,
        };
        let log = self.session_log();
        let mut buf = HopBuffer {
            record: log.is_some(),
            ..HopBuffer::default()
        };
        let mut pac = PacSend::default();
        let at = SystemTime::now();
        let started = Instant::now();
        // Dropping the exchange aborts the hop in flight and ends its borrows of `buf` and `pac`,
        // so the hops and PAC warnings before a cancel stay readable; the jar holds their cookies.
        let result = cancel
            .run(self.exchange(
                req,
                &mut buf,
                &mut pac,
                tag.stream.as_ref().map(|options| (options, cancel)),
                preloaded.as_ref(),
            ))
            .await
            .unwrap_or(Err(ExecError::Cancelled));
        let waited = started.elapsed();
        if let Some(log) = log {
            let mut values: Vec<&str> = req.sensitive_values().iter().map(String::as_str).collect();
            values.extend(req.auth_secrets().iter().map(String::as_str));
            values.extend(tag.extra.iter().map(String::as_str));
            // An answered send moved its placed values onto the response.
            let placed = match &result {
                Ok(response) => response.derived_credentials(),
                Err(_) => &buf.placed,
            };
            values.extend(placed.iter().map(String::as_str));
            let issued = match &result {
                Ok(response) if tag.response_tokens => issued_tokens(response.text()),
                _ => Vec::new(),
            };
            values.extend(issued.iter().map(String::as_str));
            let redactor = Redactor::new(&values);
            let exchange = Exchange {
                source: tag.source,
                at,
                // The timing users see: an answer reports its own, taken before the text decode,
                // the file commit and this logging work; a failure was over at `waited`.
                duration: result.as_ref().map_or(waited, Response::response_time),
                method: req.method(),
                url: req.url(),
                headers: req.headers(),
                first_body: buf.first_body.as_ref().map(|(b, n)| (b.as_slice(), *n)),
                hops: match &result {
                    Ok(response) => response.timeline(),
                    Err(_) => &buf.hops,
                },
                outcome: result.as_ref(),
            };
            let mut entry = NetworkEntry::capture(&redactor, &exchange, log.limits().body_bytes);
            entry.attempt = tag.attempt;
            match &tag.hold {
                // The hold learns this exchange's own secrets, so a console line printing an
                // echo of one is redacted at publish like the entry is now. Placed credentials
                // change per send, so only the entry's own redactor holds them.
                Some(hold) => {
                    let own: Vec<String> = (req.sensitive_values().iter())
                        .chain(req.auth_secrets())
                        .chain(&issued)
                        .cloned()
                        .collect();
                    hold.learn(&own);
                    hold.stage(Logged::Network(Arc::new(entry)));
                }
                None => {
                    log.push_network(entry);
                }
            }
        }
        (result, pac.into_warnings())
    }

    /// The request's own timeout, lowered to the run-wide cap when that is shorter or the
    /// request has none.
    fn effective_timeout(&self, req: &ResolvedRequest) -> Option<Duration> {
        let own = req.timeout().filter(|t| !t.is_zero());
        self.timeout_cap
            .filter(|cap| own.is_none_or(|t| t > *cap))
            .or(own)
    }

    /// [`Self::exchange_hops`], with a timeout the run-wide cap decided marked as such.
    async fn exchange(
        &self,
        req: &ResolvedRequest,
        buf: &mut HopBuffer,
        pac: &mut PacSend,
        stream: Option<(&StreamOptions, &CancelHandle)>,
        preloaded: Option<&Preloaded>,
    ) -> Result<Response, ExecError> {
        let result = self.exchange_hops(req, buf, pac, stream, preloaded).await;
        let own = req.timeout().filter(|t| !t.is_zero());
        let cap_bound = self.timeout_cap.filter(|cap| own.is_none_or(|t| t > *cap));
        match (result, cap_bound) {
            (Err(ExecError::Timeout { url, .. }), Some(cap)) => Err(ExecError::Timeout {
                url,
                run_wide: Some(cap),
            }),
            (result, _) => result,
        }
    }

    /// The whole send: read the body's files, then run Bruno's redirect loop, keeping each
    /// completed hop in `buf`.
    async fn exchange_hops(
        &self,
        req: &ResolvedRequest,
        buf: &mut HopBuffer,
        pac: &mut PacSend,
        stream: Option<(&StreamOptions, &CancelHandle)>,
        preloaded: Option<&Preloaded>,
    ) -> Result<Response, ExecError> {
        // Read the cookie switches once, so every hop of this send follows one policy.
        let store_cookies = self.store_cookies_enabled();
        let send_cookies = self.send_cookies_enabled();
        // A secret in the host is refused before the policy is asked, so it never reaches a lookup.
        if self.host_policy.is_some()
            && let Some(host) = req.secret_in_host()
        {
            return Err(ExecError::SecretInHost {
                host: host.to_owned(),
            });
        }
        // Likewise a secret in the collection's proxy address, before the policy sees that host.
        if self.host_policy.is_some()
            && matches!(req.network().proxy(), RequestProxy::Custom { .. })
            && req.network().secret_in_proxy()
        {
            return Err(ExecError::SecretInProxy);
        }
        let invalid = |message: String| ExecError::InvalidRequest { message };
        if let Some(problem) = req.header_problem() {
            return Err(invalid(problem.to_owned()));
        }
        // Detected on the sent body, so it sees what a pre-request script left.
        let subscription = match req.body() {
            Body::Text(text) if req.graphql() => detect(text).map_err(invalid)?,
            _ => None,
        };
        // NTLM's negotiate legs would each carry the upgrade headers, so it is refused up front.
        if subscription.is_some() && matches!(req.send_auth(), Some(SendAuth::Ntlm { .. })) {
            return Err(invalid(
                "NTLM is not supported for subscriptions".to_owned(),
            ));
        }
        let mut url = Url::parse(req.url())
            .map_err(|e| invalid(format!("{}: {e}", redact_url(req.url()))))?;
        let mut method = Method::from_bytes(req.method().as_bytes())
            .map_err(|_| invalid(format!("bad method {:?}", req.method())))?;
        let rule = rule_for(req.network().proxy(), &self.network);
        // The first hop and its proxy are judged before the body, so a refusal costs no file read.
        let timeout = self.effective_timeout(req);
        let mut hop_deadline = Deadline::after(timeout);
        let overrides = sorted_overrides(req.network().host_overrides());
        let (hop_rule, mut pac_line) = self
            .hop_rule(&rule, &url, 0, pac, &hop_deadline, &overrides)
            .await?;
        let mut proxy = self.check_route(&hop_rule, &url, 0, &overrides)?;
        // A forwarding HTTP proxy drops the upgrade, so plain ws must tunnel or go direct.
        if subscription.is_some()
            && url.scheme() == "http"
            && proxy
                .as_ref()
                .is_some_and(|p| matches!(p.scheme(), "http" | "https"))
        {
            return Err(invalid(
                "a ws:// subscription cannot pass through an HTTP proxy; \
                 use wss:// or bypass the proxy for this host"
                    .to_owned(),
            ));
        }
        if let Some(root) = &self.confine_files {
            confine(req.files(), root)?;
        }
        // Read on every send, so an edited certificate or key takes effect on the next one;
        // under a network check, from the bytes the check hashed.
        let identity = match req.network().certificate() {
            Some(cert) => Some(load_identity(cert, preloaded).await?),
            None => None,
        };
        let mut headers = req.headers().to_vec();
        let mut body = match req.body() {
            Body::Text(text) => text.as_bytes().to_vec(),
            Body::FormUrlEncoded(pairs) => form_payload(pairs).into_bytes(),
            Body::None => Vec::new(),
            // Files are read once here, so a missing one fails before any hop and a 307 or 308
            // resends the same bytes. The reads yield, so a cancel does not wait for them.
            Body::Multipart(parts) => {
                let boundary = multipart_boundary(&mut headers);
                multipart::encode(parts, &boundary)
                    .await
                    .map_err(|(path, err)| ExecError::File {
                        path,
                        message: err.to_string(),
                    })?
            }
            Body::File { path, .. } => {
                tokio::fs::read(path).await.map_err(|err| ExecError::File {
                    path: path.clone(),
                    message: err.to_string(),
                })?
            }
        };
        // One WSSE value per send, so every hop carries the same nonce; it replaces a user X-WSSE.
        if let Some(SendAuth::Wsse { username, password }) = req.send_auth() {
            let nonce = wsse::nonce().ok_or_else(|| ExecError::Transport {
                url: redact_url(url.as_str()),
                message: "the OS random generator failed, so no WSSE nonce".to_owned(),
            })?;
            headers.retain(|(n, _)| !n.eq_ignore_ascii_case("x-wsse"));
            let created = wsse::created_now();
            let value = wsse::header(username, password, &nonce, &created);
            // The digest and nonce are derived from the password, so each is redacted on its own.
            buf.place(&value);
            buf.place(&wsse::password_digest(password, &nonce, &created));
            buf.place(&nonce);
            headers.push(("X-WSSE".to_owned(), value));
        }
        // A subscription's handshake is a bodiless GET carrying the upgrade headers, set before
        // the hop loop so cookies, auth and signing all see it; its 3xx is never followed.
        let mut ws_key = None;
        if subscription.is_some() {
            let key = handshake::new_key().ok_or_else(|| ExecError::Transport {
                url: redact_url(url.as_str()),
                message: "the OS random generator failed, so no WebSocket key".to_owned(),
            })?;
            ws_key = Some(key.clone());
            method = Method::GET;
            body = Vec::new();
            // The handshake's own upgrade headers replace any the item set, so none goes twice.
            let upgrade = handshake::upgrade_headers(&key);
            let names: Vec<&str> = upgrade.iter().map(|(n, _)| n.as_str()).collect();
            drop_headers(&mut headers, &["content-type", "content-length"]);
            drop_headers(&mut headers, &names);
            headers.extend(upgrade);
        }
        let limit = if req.follow_redirects() && subscription.is_none() {
            req.max_redirects()
        } else {
            0
        };
        // One client per concrete rule: outside PAC it re-routes every hop itself; under PAC a hop
        // the script sends elsewhere swaps in that rule's client, so pools never cross proxies.
        let mut key = ClientKey {
            rule: hop_rule,
            identity,
            compress: !omits(req.omitted_headers(), "Accept-Encoding"),
            overrides: Arc::clone(&overrides),
            http1_only: subscription.is_some(),
        };
        let mut client = self.clients.get(&key, |key| self.build_client(key))?;
        let mut followed = 0;
        let mut total = Duration::ZERO;
        // SigV4 signs hops on the first URL's origin until a hop leaves it; digest answers one
        // challenge per send, its header living only on the retried hop.
        let first_origin = url.origin();
        let mut left_origin = false;
        let mut digest_tried = false;
        let mut digest_header: Option<String> = None;

        // Each pass sends one hop; a followable 3xx rewrites the loop state for the next one,
        // anything else becomes the response.
        loop {
            // Hops pushed from here on are this pass's, NTLM legs included.
            let first_new_hop = buf.hops.len();
            left_origin |= url.origin() != first_origin;
            // OAuth 1.0 signs before the hop's headers are built, so the length rule sees the
            // signed body; a later hop first sheds the previous hop's oauth_* parameters.
            let mut oauth1_authorization = None;
            if let Some(SendAuth::OAuth1(signer)) = req.send_auth() {
                let content_type = headers
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))
                    .map(|(_, v)| v.clone());
                let length = body.len();
                if !buf.hops.is_empty() {
                    oauth1::strip(&mut url, content_type.as_deref(), &mut body);
                }
                if !left_origin {
                    let signed = oauth1::nonce()
                        .and_then(|nonce| {
                            let hop = oauth1::Hop {
                                method: method.as_str(),
                                url: &url,
                                content_type: content_type.as_deref(),
                                body: &body,
                            };
                            signer.sign(&hop, unix_now(), &nonce)
                        })
                        .map_err(|e| invalid(format!("could not sign the request: {e}")))?;
                    buf.place(&signed.signature);
                    url = signed.url;
                    body = signed.body;
                    if let Some(ct) = signed.content_type {
                        headers.retain(|(n, _)| !n.eq_ignore_ascii_case("content-type"));
                        headers.push((CONTENT_TYPE.to_string(), ct.to_owned()));
                    }
                    oauth1_authorization = signed.authorization;
                }
                // A body stripped or rewritten here no longer fits a length the user set by hand.
                if body.len() != length {
                    headers.retain(|(n, _)| !n.eq_ignore_ascii_case("content-length"));
                }
            }
            let transport =
                |e: reqwest::Error| classify(&url, proxy.as_ref(), followed as usize, e);
            // When the jar has cookies for this hop, the first hop merges them into the own Cookie
            // headers as one header in the first one's place, the jar winning; a redirect hop sends
            // the jar's alone. A hop the jar has nothing for keeps the own headers intact.
            let jar_cookie = send_cookies.then(|| self.jar.header_for(&url)).flatten();
            let is_cookie = |(name, _): &&(String, String)| name.eq_ignore_ascii_case("cookie");
            let mut sending: Vec<(String, String)> = match jar_cookie {
                None => headers.clone(),
                Some(jar) => {
                    let mut kept: Vec<_> =
                        headers.iter().filter(|h| !is_cookie(h)).cloned().collect();
                    match headers.iter().position(|h| is_cookie(&h)) {
                        Some(first) if followed == 0 => {
                            let own = headers.iter().filter(is_cookie).map(|(_, v)| v.as_str());
                            let value = merge_cookies(own.chain([jar.as_str()]));
                            kept.insert(first, (headers[first].0.clone(), value));
                        }
                        _ => kept.push((COOKIE.to_string(), jar)),
                    }
                    kept
                }
            };
            // hyper sends an empty body with no length at all, where Node writes `content-length: 0`
            // for every method outside its bodiless list.
            if body.is_empty()
                && !NO_EMPTY_LENGTH.contains(&method)
                && !headers
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case("content-length"))
            {
                sending.push((CONTENT_LENGTH.to_string(), "0".to_owned()));
            }
            if let Some(value) = oauth1_authorization {
                sending.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
                sending.push(("authorization".to_owned(), value));
            }
            // Signed last, over the headers exactly as this hop sends them.
            if let Some(SendAuth::AwsV4(sigv4)) = req.send_auth()
                && !left_origin
            {
                let signed = sigv4
                    .sign(method.as_str(), &url, &sending, &body, SystemTime::now())
                    .map_err(|e| invalid(format!("could not sign the request: {e}")))?;
                sending
                    .retain(|(name, _)| !signed.iter().any(|(s, _)| s.eq_ignore_ascii_case(name)));
                // The whole value and the bare signature, since a server may echo either.
                for (name, value) in &signed {
                    if name.eq_ignore_ascii_case("authorization") {
                        buf.place(value);
                        if let Some((_, sig)) = value.rsplit_once("Signature=") {
                            buf.place(sig);
                        }
                    }
                }
                sending.extend(signed);
            }
            // EdgeGrid signs this hop over its own headers plus the client's defaults and the
            // Cookie it sends (merged with the jar's or the jar's alone), never what the HTTP
            // stack adds (Host, Content-Length). Its tokens go in clear, so a hop off the first
            // origin is signed only when forwarding is on.
            if let Some(SendAuth::EdgeGrid(signer)) = req.send_auth()
                && (!left_origin || req.forward_authorization_header())
            {
                let mut signable: Vec<_> =
                    headers.iter().filter(|h| !is_cookie(h)).cloned().collect();
                signable.extend(sending.iter().filter(is_cookie).cloned());
                for (name, value) in &defaults_without(req.omitted_headers()) {
                    if !signable
                        .iter()
                        .any(|(n, _)| n.eq_ignore_ascii_case(name.as_str()))
                    {
                        signable.push((name.to_string(), lossy_value(value)));
                    }
                }
                let text;
                let payload = if matches!(req.body(), Body::Text(_) | Body::FormUrlEncoded(_)) {
                    text = String::from_utf8_lossy(&body);
                    edgegrid::Payload::Text(&text)
                } else {
                    edgegrid::Payload::Bytes(&body)
                };
                let value = signer
                    .sign(&edgegrid::Outgoing {
                        method: method.as_str(),
                        url: &url,
                        headers: &signable,
                        payload,
                    })
                    .map_err(|e| invalid(format!("could not sign the request: {e}")))?;
                buf.place(&value);
                sending.retain(|(n, _)| !n.eq_ignore_ascii_case("authorization"));
                sending.push(("authorization".to_owned(), value));
            }
            if let Some(value) = digest_header.take() {
                buf.place(&value);
                sending.push(("authorization".to_owned(), value));
            }
            // A URL carrying `user:pass@` sends the Basic reqwest builds from it as its only
            // Authorization, so whatever this hop would otherwise send gives way to it.
            let userinfo = url_credentials(&url);
            if userinfo.is_some() {
                sending.retain(|(n, _)| !n.eq_ignore_ascii_case("authorization"));
            }
            let mut builder = client.request(method.clone(), url.clone());
            for (name, value) in &sending {
                builder = builder.header(name, value);
            }
            // The defaults are added here, not on the shared client, so each request can omit
            // its own; one the request sets itself is never added twice.
            for (name, value) in &defaults_without(req.omitted_headers()) {
                if !sending
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case(name.as_str()))
                {
                    builder = builder.header(name, value);
                }
            }
            if !body.is_empty() {
                builder = builder.body(body.clone());
            }
            let request = builder.build().map_err(transport)?;
            // The whole value and the bare credential, since a server may echo either.
            if let Some(password) = userinfo.as_ref().and_then(|(_, p)| p.as_deref())
                && !password.is_empty()
            {
                buf.place(password);
            }
            if userinfo.is_some()
                && let Some(basic) = request.headers().get(AUTHORIZATION)
                && let Ok(basic) = basic.to_str()
            {
                buf.place(basic);
                if let Some((_, token)) = basic.split_once(' ') {
                    buf.place(token);
                }
            }
            let mut sent = sent_headers(&request);
            // The log shows the first hop's bytes as sent, after any OAuth 1.0 body signing.
            if buf.record && buf.first_body.is_none() {
                let kept = body[..body.len().min(HOP_BODY_LIMIT)].to_vec();
                buf.first_body = Some((kept, body.len() as u64));
            }
            let started = Instant::now();
            // One deadline per hop, over its PAC lookup, headers and body; it started with the
            // lookup. A digest retry is a fresh leg.
            let mut leg = hop_deadline;
            let timed_out = || ExecError::timeout(redact_url(url.as_str()));
            // A hop in NTLM mode runs its three legs on a pinned connection; negotiate pushes the
            // earlier legs onto the timeline and this loop records the last.
            let mut ntlm_sent = None;
            let mut leg_started = started;
            // Set for a plain send only: the recorder and the instant its headers arrived.
            let mut measured = None;
            let res = match req.send_auth() {
                Some(SendAuth::Ntlm {
                    username,
                    password,
                    domain,
                }) if userinfo.is_none()
                    && (!left_origin || req.forward_authorization_header()) =>
                {
                    let creds = ntlm::Credentials {
                        username,
                        password,
                        domain,
                        workstation: "",
                    };
                    let done = self
                        .negotiate(
                            request,
                            &key,
                            &creds,
                            &url,
                            proxy.as_ref(),
                            followed as usize,
                            store_cookies,
                            timeout,
                            leg,
                            pac_line.as_deref(),
                            buf,
                        )
                        .await?;
                    ntlm_sent = done.authorization;
                    leg_started = done.started;
                    total += done.earlier;
                    leg = done.deadline;
                    done.response
                }
                _ => {
                    // An idempotent request whose pooled connection died unanswered is resent once
                    // on a fresh one, as curl and browsers do; the server never saw the first try.
                    let spare = is_idempotent(&method)
                        .then(|| request.try_clone())
                        .flatten();
                    let mut recorder = Recorder::new();
                    let mut sent_once = leg
                        .run(crate::timing::scope(&recorder, client.execute(request)))
                        .await
                        .ok_or_else(timed_out)?;
                    if let (Err(e), Some(again)) = (&sent_once, spare)
                        && !recorder.connected()
                        && is_stale_connection(e)
                    {
                        recorder = Recorder::new();
                        sent_once = leg
                            .run(crate::timing::scope(&recorder, client.execute(again)))
                            .await
                            .ok_or_else(timed_out)?;
                    }
                    let res = sent_once.map_err(transport)?;
                    measured = Some((recorder, Instant::now()));
                    res
                }
            };
            if let Some(value) = ntlm_sent {
                sent.retain(|(n, _)| !n.eq_ignore_ascii_case("authorization"));
                sent.push(("authorization".to_owned(), value));
            }
            let status = res.status();
            let version = HttpVersion::from_wire(res.version());
            let remote_addr = res.remote_addr();
            if version == Some(HttpVersion::Http2) {
                strip_h2_connection_headers(&mut sent);
            }
            let hop_headers = res.headers().clone();
            // Decoded as UTF-8 rather than visible ASCII, as reqwest's jar did, so `sid=café` is kept.
            let set_cookies = hop_headers.get_all(SET_COOKIE).iter().map(|v| v.as_bytes());
            let hop_cookies = if store_cookies {
                self.jar.store(set_cookies, &url)
            } else {
                crate::cookies::not_stored(set_cookies)
            };
            // Whether this hop ends the exchange is known from its status and headers alone: a
            // digest challenge to answer or a redirect to follow makes it a hop to drain.
            let challenge = match req.send_auth() {
                Some(SendAuth::Digest { .. })
                    if status == StatusCode::UNAUTHORIZED
                        && !digest_tried
                        && userinfo.is_none()
                        && !headers
                            .iter()
                            .any(|(n, _)| n.eq_ignore_ascii_case("authorization")) =>
                {
                    digest::parse_challenge(
                        hop_headers
                            .get_all(WWW_AUTHENTICATE)
                            .iter()
                            .filter_map(|v| v.to_str().ok()),
                    )
                }
                _ => None,
            };
            let next =
                (challenge.is_none() && REDIRECTS.contains(&status.as_u16()) && followed < limit)
                    .then(|| hop_headers.get("location")?.to_str().ok())
                    .flatten()
                    .and_then(|location| url.join(location).ok());
            // A refused handshake names the ws URL and why; it is a response, not an ExecError.
            let refused = |why: String| {
                StreamEnd::Error(format!(
                    "WebSocket handshake to {} failed: {why}",
                    handshake::ws_url(&url)
                ))
            };
            let read = match (stream, &subscription) {
                (_, Some(_))
                    if challenge.is_none() && status != StatusCode::SWITCHING_PROTOCOLS =>
                {
                    let mut why = status.to_string();
                    if let Some(location) = hop_headers.get("location") {
                        why.push_str(&format!(
                            " (Location: {})",
                            String::from_utf8_lossy(location.as_bytes())
                        ));
                    }
                    let (bytes, size, truncated) = leg
                        .run(read_body(res, self.max_body_bytes))
                        .await
                        .ok_or_else(timed_out)?
                        .map_err(transport)?;
                    BodyRead {
                        bytes,
                        size,
                        truncated,
                        end: Some(refused(why)),
                        file: None,
                    }
                }
                (_, Some(_))
                    if challenge.is_none()
                        && let Err(why) = handshake::check_reply(
                            ws_key.as_deref().unwrap_or_default(),
                            &hop_headers,
                        ) =>
                {
                    BodyRead {
                        bytes: Vec::new(),
                        size: 0,
                        truncated: false,
                        end: Some(refused(why)),
                        file: None,
                    }
                }
                (_, Some(plan))
                    if challenge.is_none() && status == StatusCode::SWITCHING_PROTOCOLS =>
                {
                    // A send that does not stream is bounded, since a subscription never ends
                    // on its own.
                    let fallback = (
                        StreamOptions::new().limits(unstreamed_limits(timeout)),
                        CancelHandle::new(),
                    );
                    let (options, cancel) = stream.unwrap_or((&fallback.0, &fallback.1));
                    let upgraded = leg
                        .run(res.upgrade())
                        .await
                        .ok_or_else(timed_out)?
                        .map_err(transport)?;
                    let ws = WebSocketStream::from_raw_socket(upgraded, Role::Client, None).await;
                    let _armed = Armed(cancel);
                    cancel.arm();
                    if let Some(sink) = &options.sink {
                        sink(StreamEvent::Started {
                            status: status.as_u16(),
                            headers: lossy(&hop_headers),
                            event_stream: true,
                        });
                    }
                    let session = socket::Session {
                        payload: plan.payload.clone(),
                        ack_wait: ack_wait(timeout),
                        sink: options.sink.as_ref(),
                        cancel,
                        limits: options.limits,
                        cap: self
                            .max_body_bytes
                            .or(options.limits.max_bytes)
                            .unwrap_or(DEFAULT_STREAM_CAP),
                    };
                    let transcript = socket::run(ws, session).await;
                    BodyRead {
                        bytes: transcript.bytes,
                        size: transcript.size,
                        truncated: transcript.truncated,
                        end: Some(transcript.end),
                        file: None,
                    }
                }
                (Some((options, cancel)), _) if challenge.is_none() && next.is_none() => {
                    let wire = StreamHead {
                        status: status.as_u16(),
                        headers: lossy(&hop_headers),
                        event_stream: hop_headers
                            .get(CONTENT_TYPE)
                            .and_then(|v| v.to_str().ok())
                            .is_some_and(crate::sse::is_event_stream),
                    };
                    read_streaming(
                        res,
                        wire,
                        &leg,
                        timeout,
                        options,
                        cancel,
                        self.max_body_bytes,
                    )
                    .await
                    .map_err(|e| match e {
                        ReadFail::Timeout => timed_out(),
                        ReadFail::Transport(e) => transport(e),
                        ReadFail::Stopped => ExecError::Transport {
                            url: redact_url(url.as_str()),
                            message: "stopped before the whole body was saved".to_owned(),
                        },
                        ReadFail::Write(path, e) => ExecError::Transport {
                            url: redact_url(url.as_str()),
                            message: format!("could not write {}: {e}", path.display()),
                        },
                    })?
                }
                _ => {
                    let (bytes, size, truncated) = leg
                        .run(read_body(res, self.max_body_bytes))
                        .await
                        .ok_or_else(timed_out)?
                        .map_err(transport)?;
                    BodyRead {
                        bytes,
                        size,
                        truncated,
                        end: None,
                        file: None,
                    }
                }
            };
            let BodyRead {
                bytes,
                size,
                truncated,
                end: stream_end,
                file,
            } = read;
            let elapsed = leg_started.elapsed();
            total += elapsed;
            let timing = measured.map(|(recorder, headers_at)| {
                let tls = url.scheme() == "https";
                recorder.finish(started, headers_at, Instant::now(), tls, proxy.is_some())
            });
            buf.hops.push(Hop {
                method: method.to_string(),
                url: url.to_string(),
                request_headers: sent,
                status: status.as_u16(),
                response_headers: lossy(&hop_headers),
                elapsed,
                // Capped so a large upload does not live twice for as long as the Response does.
                request_body: body[..body.len().min(HOP_BODY_LIMIT)].to_vec(),
                request_body_size: body.len() as u64,
                pac: pac_line.clone(),
                timing,
                cookies: hop_cookies,
                version,
                remote_addr,
                via_proxy: proxy.is_some(),
                override_from: None,
            });
            for hop in &mut buf.hops[first_new_hop..] {
                hop.override_from = label_for(&overrides, &hop.url);
            }

            // A 401 digest challenge is answered once, on the same URL, outside the redirect count.
            if let Some(challenge) = challenge
                && let Some(SendAuth::Digest { username, password }) = req.send_auth()
            {
                let cnonce = digest::new_cnonce().ok_or_else(|| ExecError::Transport {
                    url: redact_url(url.as_str()),
                    message: "the OS random generator failed, so no digest nonce".to_owned(),
                })?;
                let uri = &url[url::Position::BeforePath..url::Position::AfterQuery];
                digest_tried = true;
                hop_deadline = Deadline::after(timeout);
                digest_header = Some(digest::authorization(
                    &challenge,
                    username,
                    password,
                    method.as_str(),
                    uri,
                    &cnonce,
                ));
                continue;
            }

            let Some(next) = next else {
                // Renaming is the commit: an error above this line dropped the guard and the
                // temp file with it.
                let saved = match file {
                    Some(file) => {
                        Some(
                            file.commit()
                                .await
                                .map_err(|(path, e)| ExecError::Transport {
                                    url: redact_url(url.as_str()),
                                    message: format!("could not write {}: {e}", path.display()),
                                })?,
                        )
                    }
                    None => None,
                };
                let mut response = Response::new(status.as_u16())
                    .with_status_text(status.canonical_reason().unwrap_or(""))
                    .with_url(url.as_str());
                for (name, value) in &hop_headers {
                    response = response.with_header(name, lossy_value(value));
                }
                let text = decode(&hop_headers, &bytes);
                response = response
                    .with_body(bytes, text)
                    .with_response_time(total)
                    .with_timeline(std::mem::take(&mut buf.hops))
                    .with_derived_credentials(derived_list(
                        std::mem::take(&mut buf.placed),
                        req.derived(),
                    ));
                if truncated {
                    response = response.with_truncated(size);
                }
                if let Some(end) = stream_end {
                    response = response.with_stream_end(end);
                }
                if let Some(path) = saved {
                    response = response.with_body_file(path);
                }
                return Ok(response);
            };

            followed += 1;
            hop_deadline = Deadline::after(timeout);
            let (next_rule, next_line) = self
                .hop_rule(
                    &rule,
                    &next,
                    followed as usize,
                    pac,
                    &hop_deadline,
                    &overrides,
                )
                .await?;
            proxy = self.check_route(&next_rule, &next, followed as usize, &overrides)?;
            if next_rule != key.rule {
                key.rule = next_rule;
                client = self.clients.get(&key, |key| self.build_client(key))?;
            }
            pac_line = next_line;
            // 301, 302 and 303 turn anything but HEAD into a bodiless GET.
            if matches!(
                status,
                StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND | StatusCode::SEE_OTHER
            ) && method != Method::HEAD
            {
                method = Method::GET;
                body = Vec::new();
                drop_headers(&mut headers, &["content-type", "content-length"]);
            }
            // A cross-origin hop loses AWS signing headers always, as bruno-cli's does, and the
            // credentials (X-WSSE among them) unless forwardAuthorizationHeader is on.
            if next.origin() != url.origin() {
                headers.retain(|(name, _)| {
                    !name
                        .get(..6)
                        .is_some_and(|p| p.eq_ignore_ascii_case("x-amz-"))
                });
                if !req.forward_authorization_header() {
                    drop_headers(
                        &mut headers,
                        &["authorization", "proxy-authorization", "x-wsse"],
                    );
                }
            }
            url = next;
        }
    }
}

/// Reads a hop's decoded body chunk by chunk, returning the bytes kept, the bytes read and
/// whether `limit` cut it short. Past the limit reading stops and the dropped response closes
/// the connection.
async fn read_body(
    mut res: reqwest::Response,
    limit: Option<u64>,
) -> Result<(Vec<u8>, u64, bool), reqwest::Error> {
    let mut body = Vec::new();
    let mut size = 0u64;
    while let Some(chunk) = res.chunk().await? {
        size += chunk.len() as u64;
        if let Some(limit) = limit {
            let room =
                usize::try_from(limit.saturating_sub(body.len() as u64)).unwrap_or(usize::MAX);
            if chunk.len() > room {
                body.extend_from_slice(&chunk[..room]);
                return Ok((body, size, true));
            }
        }
        body.extend_from_slice(&chunk);
    }
    Ok((body, size, false))
}

/// What a hop's body read produced.
struct BodyRead {
    bytes: Vec<u8>,
    size: u64,
    truncated: bool,
    end: Option<StreamEnd>,
    file: Option<TempBody>,
}

/// A body being written to a hidden sibling of its target, deleted on drop unless committed.
struct TempBody {
    target: PathBuf,
    temp: PathBuf,
    file: Option<tokio::fs::File>,
    committed: bool,
}

impl TempBody {
    /// Creates `.<name>.apinata-part` next to `target`, or a numbered variant if that name is taken.
    async fn create(target: &Path) -> Result<TempBody, std::io::Error> {
        let name = target
            .file_name()
            .ok_or_else(|| std::io::Error::other("the path has no file name"))?;
        let mut part = std::ffi::OsString::from(".");
        part.push(name);
        part.push(".apinata-part");
        // create_new refuses any existing entry, a symlink included, so a planted link is never followed.
        const ATTEMPTS: u32 = 8;
        let mut found = None;
        for n in 0..ATTEMPTS {
            let mut candidate = part.clone();
            if n > 0 {
                candidate.push(format!(".{n}"));
            }
            let temp = target.with_file_name(candidate);
            // Created synchronously: no await between the file existing and TempBody owning it,
            // so a cancel cannot leak it.
            let mut open = std::fs::OpenOptions::new();
            open.write(true).create_new(true);
            match open.open(&temp) {
                Ok(file) => {
                    found = Some((temp, tokio::fs::File::from_std(file)));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        let Some((temp, file)) = found else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("no free temporary name next to the path after {ATTEMPTS} attempts"),
            ));
        };
        Ok(TempBody {
            target: target.to_owned(),
            temp,
            file: Some(file),
            committed: false,
        })
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<(), std::io::Error> {
        use tokio::io::AsyncWriteExt;
        match &mut self.file {
            Some(file) => file.write_all(bytes).await,
            None => Ok(()),
        }
    }

    /// Flushes, closes and renames the temp file over the target, returning the target.
    /// A symlink at the target is replaced by the file, never written through.
    async fn commit(mut self) -> Result<PathBuf, (PathBuf, std::io::Error)> {
        use tokio::io::AsyncWriteExt;
        let done = async {
            if let Some(mut file) = self.file.take() {
                file.flush().await?;
            }
            tokio::fs::rename(&self.temp, &self.target).await
        };
        match done.await {
            Ok(()) => {
                self.committed = true;
                Ok(self.target.clone())
            }
            Err(e) => Err((self.target.clone(), e)),
        }
    }
}

impl Drop for TempBody {
    fn drop(&mut self) {
        if !self.committed {
            self.file = None;
            let _ = std::fs::remove_file(&self.temp);
        }
    }
}

/// The final hop's status line and headers, as the sink's `Started` event reports them.
struct StreamHead {
    status: u16,
    headers: Vec<(String, String)>,
    event_stream: bool,
}

/// Why a streamed read failed rather than ended.
enum ReadFail {
    Timeout,
    Transport(reqwest::Error),
    /// Stop ended a plain body that was being saved, so there is no whole body to commit.
    Stopped,
    /// Writing the body file at this path failed.
    Write(PathBuf, std::io::Error),
}

/// Disarms Stop when the read ends or the exchange is dropped by a cancel.
struct Armed<'a>(&'a CancelHandle);

impl Drop for Armed<'_> {
    fn drop(&mut self) {
        self.0.disarm();
    }
}

/// The transcript cap of a stream with neither an executor cap nor a byte limit.
const DEFAULT_STREAM_CAP: u64 = 10 * 1024 * 1024;

/// Reads the final hop's body chunk by chunk, telling the sink as it goes.
///
/// A body that is not an event-stream keeps today's rules: the leg `deadline` bounds it whole,
/// `max_body_bytes` (default 10 MiB) caps it and a failure is an error. An event-stream is
/// bounded by the idle timer (`idle`), the stream limits, Stop and the byte cap, and every such
/// end returns what arrived together with a `StreamEnd`.
///
/// With a body file a plain body is written whole: the cap bounds memory only, the idle timer
/// replaces the deadline, and Stop is an error that discards the file.
async fn read_streaming(
    mut res: reqwest::Response,
    head: StreamHead,
    deadline: &Deadline,
    idle: Option<Duration>,
    options: &StreamOptions,
    cancel: &CancelHandle,
    max_body_bytes: Option<u64>,
) -> Result<BodyRead, ReadFail> {
    let event_stream = head.event_stream;
    let limits = options.limits;
    // No stream grows unchecked: an event-stream also honours the stream limit, a plain body
    // takes the executor's cap or the default.
    let cap = Some(if event_stream {
        max_body_bytes
            .or(limits.max_bytes)
            .unwrap_or(DEFAULT_STREAM_CAP)
    } else {
        max_body_bytes.unwrap_or(DEFAULT_STREAM_CAP)
    });
    let to_file = options.body_file.is_some();
    // The length is only known when nothing was compressed, since chunks count decoded bytes.
    let total = (!head.headers.iter().any(|(n, _)| n == "content-encoding"))
        .then(|| {
            head.headers
                .iter()
                .find(|(n, _)| n == "content-length")
                .and_then(|(_, v)| v.trim().parse::<u64>().ok())
        })
        .flatten();
    let _armed = Armed(cancel);
    cancel.arm();
    let emit = |event: StreamEvent| {
        if let Some(sink) = &options.sink {
            sink(event);
        }
    };
    emit(StreamEvent::Started {
        status: head.status,
        headers: head.headers,
        event_stream,
    });

    let mut temp = match &options.body_file {
        Some(path) => Some(
            TempBody::create(path)
                .await
                .map_err(|e| ReadFail::Write(path.clone(), e))?,
        ),
        None => None,
    };
    let began = tokio::time::Instant::now();
    let bound_at = limits
        .max_duration
        .filter(|_| event_stream)
        .map(|d| began + d);
    let idle = idle.filter(|_| event_stream || to_file);
    let mut idle_at = idle.map(|t| began + t);
    let mut parser = crate::sse::SseParser::new();
    let mut events = 0u64;
    let mut body = Vec::new();
    let mut size = 0u64;
    let mut truncated = false;
    let end = loop {
        // Listed in priority order: Stop and the bounds win over a chunk that is ready too.
        tokio::select! {
            biased;
            () = cancel.stopped() => {
                if to_file && !event_stream {
                    return Err(ReadFail::Stopped);
                }
                break Some(StreamEnd::Stopped);
            }
            () = async {
                match bound_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => break Some(StreamEnd::Bound(StreamBound::Time)),
            () = async {
                match idle_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                if event_stream {
                    break Some(StreamEnd::IdleTimeout);
                }
                return Err(ReadFail::Timeout);
            }
            () = async {
                if event_stream || to_file {
                    std::future::pending::<()>().await
                } else {
                    deadline.expired().await
                }
            } => return Err(ReadFail::Timeout),
            next = res.chunk() => match next {
                Ok(Some(chunk)) => {
                    size += chunk.len() as u64;
                    if let Some(at) = &mut idle_at {
                        *at = tokio::time::Instant::now() + idle.unwrap_or_default();
                    }
                    let room = cap.map(|c| {
                        usize::try_from(c.saturating_sub(body.len() as u64)).unwrap_or(usize::MAX)
                    });
                    let kept = match room {
                        Some(room) if chunk.len() > room => {
                            truncated = true;
                            &chunk[..room]
                        }
                        _ => &chunk[..],
                    };
                    body.extend_from_slice(kept);
                    if !kept.is_empty() {
                        emit(StreamEvent::Chunk(kept.to_vec()));
                    }
                    if event_stream {
                        events += parser.push(kept).len() as u64;
                    }
                    if let Some(temp) = &mut temp {
                        // A plain body is written whole; an event-stream stops at its cap.
                        let written = if event_stream { kept } else { &chunk[..] };
                        if let Err(e) = temp.write(written).await {
                            return Err(ReadFail::Write(temp.target.clone(), e));
                        }
                        emit(StreamEvent::Progress { received: size, total });
                    }
                    if truncated && (event_stream || !to_file) {
                        // An ordinary capped body is just truncated; a stream says why it ended.
                        break event_stream.then_some(StreamEnd::BodyCap);
                    }
                    if event_stream && limits.max_events.is_some_and(|n| events >= n) {
                        break Some(StreamEnd::Bound(StreamBound::Events));
                    }
                }
                Ok(None) => break event_stream.then_some(StreamEnd::Closed),
                Err(e) if event_stream => break Some(StreamEnd::Error(chain(e))),
                Err(e) => return Err(ReadFail::Transport(e)),
            },
        }
    };
    Ok(BodyRead {
        bytes: body,
        size,
        truncated,
        end,
        file: temp,
    })
}

/// Seconds since the Unix epoch, for an OAuth 1.0 timestamp; a clock before 1970 reads as 0.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The decoded user and password reqwest turns `url`'s userinfo into a Basic header with, by
/// its own rule: `None` when there is none, or when the username is not UTF-8 once decoded.
fn url_credentials(url: &Url) -> Option<(String, Option<String>)> {
    use percent_encoding::percent_decode_str;
    let user = percent_decode_str(url.username()).decode_utf8().ok()?;
    let password = (url.password())
        .and_then(|p| percent_decode_str(p).decode_utf8().ok())
        .map(String::from);
    (!user.is_empty() || password.is_some()).then(|| (user.into_owned(), password))
}

/// The headers a request carries, as the wire shows them.
fn sent_headers(request: &reqwest::Request) -> Vec<(String, String)> {
    lossy(request.headers())
}

/// The headers as name and value strings, values decoded lossily as UTF-8.
fn lossy(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.to_string(), lossy_value(value)))
        .collect()
}

/// A header value decoded lossily as UTF-8.
fn lossy_value(value: &HeaderValue) -> String {
    String::from_utf8_lossy(value.as_bytes()).into_owned()
}

/// The boundary for a multipart body: the one the request's `content-type` names, else a new
/// one, appended to that header as `; boundary=<B>` when its media type is `multipart/*`.
fn multipart_boundary(headers: &mut [(String, String)]) -> String {
    let content_type = headers
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value);
    if let Some(found) = content_type
        .as_deref()
        .and_then(|v| multipart::boundary_from(v))
    {
        return found;
    }
    let boundary = multipart::new_boundary();
    if let Some(value) = content_type {
        let media = value.split(';').next().unwrap_or_default().trim();
        if media
            .get(..10)
            .is_some_and(|p| p.eq_ignore_ascii_case("multipart/"))
        {
            value.push_str("; boundary=");
            value.push_str(&boundary);
        }
    }
    boundary
}

/// Removes what hyper drops from a request before sending it over h2, so an h2 hop records
/// what went over the wire. Mirrors `strip_connection_headers` in hyper 1.12.0
/// (`proto/h2/mod.rs`): only the first `te` and `connection` values count.
fn strip_h2_connection_headers(headers: &mut Vec<(String, String)>) {
    drop_headers(
        headers,
        &[
            "keep-alive",
            "proxy-connection",
            "transfer-encoding",
            "upgrade",
        ],
    );
    let first = |headers: &[(String, String)], name: &str| {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    };
    if first(headers, "te").is_some_and(|v| v != "trailers") {
        drop_headers(headers, &["te"]);
    }
    if let Some(listed) = first(headers, "connection") {
        drop_headers(headers, &["connection"]);
        // hyper reads the list through `HeaderValue::to_str`, which accepts visible ASCII and tab only.
        if listed.bytes().all(|b| b == b'\t' || (32..127).contains(&b)) {
            let names: Vec<&str> = listed.split(',').map(str::trim).collect();
            drop_headers(headers, &names);
        }
    }
}

/// The request's overrides ordered by host, so a reordered file keeps its client.
fn sorted_overrides(rows: &[ResolvedHostOverride]) -> Arc<Vec<ResolvedHostOverride>> {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| a.host().cmp(b.host()));
    Arc::new(rows)
}

/// Removes every header named in `names`, whatever its case.
fn drop_headers(headers: &mut Vec<(String, String)>, names: &[&str]) {
    headers.retain(|(name, _)| !names.iter().any(|n| name.eq_ignore_ascii_case(n)));
}

/// The suffix a timeout error carries when the run-wide cap was its limit.
fn run_wide_note(cap: Option<Duration>) -> String {
    cap.map_or_else(String::new, |cap| {
        format!(" (run-wide timeout of {} s)", cap.as_secs_f64())
    })
}

/// Sorts a reqwest failure on hop `hop` to `url` into a resolver refusal, a timeout, a
/// connect failure (a proxy failure when the hop went through `proxy`) or the rest.
fn classify(
    url: &Url,
    proxy: Option<&ResolvedProxy>,
    hop: usize,
    error: reqwest::Error,
) -> ExecError {
    // Through local-DNS SOCKS the guard resolver refuses the target on the dial, so the
    // refusal names the proxy it went through, as a pre-check refusal of the proxy does.
    if let Some(refusal) = refusal_in(&error) {
        let host = refusal.host;
        return match proxy {
            Some(p) => ExecError::ProxyNotAllowed {
                hop,
                host,
                proxy: p.to_string(),
            },
            None => ExecError::HostNotAllowed {
                hop,
                host,
                mapped_from: refusal.mapped_from,
            },
        };
    }
    let url = redact_url(url.as_str());
    if error.is_timeout() {
        ExecError::timeout(url)
    } else if let (true, Some(proxy)) = (error.is_connect(), proxy) {
        ExecError::Proxy {
            url,
            proxy: proxy.to_string(),
            message: proxy.redact(&chain(error)),
        }
    } else if error.is_connect() {
        let cause = connect_cause(&error);
        ExecError::Connect {
            url,
            message: chain(error),
            cause,
        }
    } else {
        // No known layer echoes proxy credentials here; scrubbing anyway costs nothing.
        let message = chain(error);
        ExecError::Transport {
            url,
            message: match proxy {
                Some(proxy) => proxy.redact(&message),
                None => message,
            },
        }
    }
}

/// Why the connection in `error`'s chain failed: the first link showing a refusal, a TLS
/// failure or hyper-util's `dns error` decides.
fn connect_cause(error: &(dyn std::error::Error + 'static)) -> ConnectCause {
    let mut link = Some(error);
    while let Some(e) = link {
        if let Some(cause) = link_cause(e, 0) {
            return cause;
        }
        link = e.source();
    }
    ConnectCause::Other
}

// A plain-http peer answers the ClientHello with bytes that are no TLS record, which rustls
// reports as an invalid content type; everything else stays the generic Tls.
fn tls_cause(error: &rustls::Error) -> ConnectCause {
    use rustls::{CertificateError, Error, InvalidMessage};
    match error {
        Error::InvalidCertificate(CertificateError::UnknownIssuer) => {
            ConnectCause::TlsUnknownIssuer
        }
        // A self-signed CA:TRUE certificate (openssl's default) is refused this way; to the
        // user it is the same untrusted certificate, so it gets the same advice.
        Error::InvalidCertificate(CertificateError::Other(other))
            if other.to_string() == "CaUsedAsEndEntity" =>
        {
            ConnectCause::TlsUnknownIssuer
        }
        Error::InvalidCertificate(
            CertificateError::Expired | CertificateError::ExpiredContext { .. },
        ) => ConnectCause::TlsExpired,
        Error::InvalidMessage(InvalidMessage::InvalidContentType) => ConnectCause::TlsWrongProtocol,
        _ => ConnectCause::Tls,
    }
}

// An io::Error hides what it wraps from source(), so wrapped errors are opened through
// get_ref() and walked in turn, a few levels deep at most.
fn link_cause(e: &(dyn std::error::Error + 'static), depth: u8) -> Option<ConnectCause> {
    if let Some(tls) = e.downcast_ref::<rustls::Error>() {
        return Some(tls_cause(tls));
    }
    if let Some(io) = e.downcast_ref::<std::io::Error>() {
        if io.kind() == std::io::ErrorKind::ConnectionRefused {
            return Some(ConnectCause::Refused);
        }
        if depth < 4
            && let Some(inner) = io.get_ref()
        {
            let mut next: Option<&(dyn std::error::Error + 'static)> = Some(inner);
            while let Some(n) = next {
                if let Some(cause) = link_cause(n, depth + 1) {
                    return Some(cause);
                }
                next = n.source();
            }
        }
    }
    (e.to_string() == "dns error").then_some(ConnectCause::Dns)
}

/// Decodes `bytes` with the `Content-Type` charset, falling back to UTF-8 for a missing or
/// unknown label, as reqwest's `text_with_charset` does.
fn decode(headers: &HeaderMap, bytes: &[u8]) -> String {
    let encoding = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<mime::Mime>().ok())
        .and_then(|m| {
            encoding_rs::Encoding::for_label(m.get_param(mime::CHARSET)?.as_str().as_bytes())
        })
        .unwrap_or(encoding_rs::UTF_8);
    encoding.decode(bytes).0.into_owned()
}

/// Merges Cookie header values, read in order, into one `name=value; ...` string: a later name
/// overwrites the value but keeps the name's first place, and a piece with an empty name is dropped.
fn merge_cookies<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let mut merged: Vec<(&str, &str)> = Vec::new();
    for piece in values.into_iter().flat_map(|v| v.split(';')) {
        let (name, value) = piece.split_once('=').unwrap_or((piece, ""));
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() {
            continue;
        }
        match merged.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => merged.push((name, value)),
        }
    }
    let pairs: Vec<String> = merged.iter().map(|(n, v)| format!("{n}={v}")).collect();
    pairs.join("; ")
}

/// The error's message followed by each of its sources, joined by `: `; reqwest's own copy of
/// the URL is dropped, since the variants carry a redacted one.
pub(crate) fn chain(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

/// Form fields encoded as `application/x-www-form-urlencoded`, in order.
pub(crate) fn form_payload(pairs: &[(String, String)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// The values placed on the wire followed by the request's own derived values, without repeats.
fn derived_list(mut placed: Vec<String>, extra: &[String]) -> Vec<String> {
    for value in extra {
        if !placed.contains(value) {
            placed.push(value.clone());
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn merge_cookies_reads_every_own_value_and_the_jar_wins_over_both() {
        assert_eq!(merge_cookies(["a=1; b=2", "c=3; a=4"]), "a=4; b=2; c=3");
        assert_eq!(
            merge_cookies(["a=1; b=2", "c=3; a=4", "a=9; c=8"]),
            "a=9; b=2; c=8"
        );
    }

    /// A cancel that lands while the temp file is being created must not leave it behind.
    #[test]
    fn cancelled_temp_creation_leaves_no_file() {
        use std::future::Future;
        let dir = tempfile::tempdir().expect("dir");
        let target = dir.path().join("full.bin");
        let rt = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .build()
            .expect("runtime");
        rt.block_on(async {
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            let mut create = Box::pin(TempBody::create(&target));
            // One poll starts the creation; dropping the future is the cancel.
            let first = create.as_mut().poll(&mut cx);
            drop(first);
            drop(create);
            // With a single blocking thread this runs after any in-flight open.
            tokio::task::spawn_blocking(|| ()).await.expect("flush");
        });
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert!(left.is_empty(), "leftover files: {left:?}");
    }

    /// Feeds `read_streaming` an event-stream whose chunks arrive after the given pauses on the
    /// paused tokio clock, so the idle timer is judged against exact virtual time.
    async fn read_scripted(
        script: &[(u64, &'static str)],
        tail_ms: u64,
        idle: Option<u64>,
    ) -> BodyRead {
        let (mut tx, body) =
            http_body_util::channel::Channel::<bytes::Bytes, std::convert::Infallible>::new(1);
        let script = script.to_vec();
        tokio::spawn(async move {
            for (pause, text) in script {
                tokio::time::sleep(Duration::from_millis(pause)).await;
                let _ = tx
                    .send_data(bytes::Bytes::from_static(text.as_bytes()))
                    .await;
            }
            tokio::time::sleep(Duration::from_millis(tail_ms)).await;
        });
        let res = http::Response::builder()
            .header("content-type", "text/event-stream")
            .body(reqwest::Body::wrap(body))
            .expect("response");
        let head = StreamHead {
            status: 200,
            headers: Vec::new(),
            event_stream: true,
        };
        read_streaming(
            reqwest::Response::from(res),
            head,
            &Deadline::after(None),
            idle.map(Duration::from_millis),
            &StreamOptions::new(),
            &CancelHandle::new(),
            None,
        )
        .await
        .unwrap_or_else(|_| panic!("read failed"))
    }

    #[tokio::test(start_paused = true)]
    async fn comments_reset_the_idle_timer() {
        let pings = vec![(150, ": ping\n\n"); 6];
        let read = read_scripted(&pings, 150, Some(300)).await;
        assert_eq!(read.end, Some(StreamEnd::Closed));
    }

    #[tokio::test(start_paused = true)]
    async fn no_timeout_means_no_idle_timer() {
        let script = [(0, "data: a\n\n"), (700, "data: b\n\n")];
        let read = read_scripted(&script, 0, None).await;
        assert_eq!(read.end, Some(StreamEnd::Closed));
        assert_eq!(
            String::from_utf8_lossy(&read.bytes)
                .matches("data:")
                .count(),
            2
        );
    }

    #[tokio::test(start_paused = true)]
    async fn silence_past_the_idle_timeout_ends_the_stream() {
        let script = [(0, "data: a\n\n"), (500, "data: b\n\n")];
        let read = read_scripted(&script, 0, Some(300)).await;
        assert_eq!(read.end, Some(StreamEnd::IdleTimeout));
        assert_eq!(
            String::from_utf8_lossy(&read.bytes)
                .matches("data:")
                .count(),
            1
        );
    }

    #[derive(Debug)]
    struct Leak;

    impl fmt::Display for Leak {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("upstream said s3cret")
        }
    }

    impl std::error::Error for Leak {}

    // A redirect policy that fails yields a transport error whose source text is ours, so it
    // stands in for any lower layer that echoes the proxy's password.
    #[tokio::test]
    async fn a_transport_error_on_a_proxied_hop_hides_the_proxy_password() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/next"))
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                attempt.error(Leak)
            }))
            .build()
            .expect("client");
        let url = Url::parse(&server.uri()).expect("url");
        let error = client
            .get(url.clone())
            .send()
            .await
            .expect_err("redirect fails");
        let proxy = ResolvedProxy::new("http", "127.0.0.1", "9")
            .expect("proxy")
            .with_credentials("user", "s3cret");
        let ExecError::Transport { message, .. } = classify(&url, Some(&proxy), 0, error) else {
            panic!("not a transport error");
        };
        assert!(!message.contains("s3cret"), "{message}");
        assert!(message.contains("upstream said ***"), "{message}");
    }

    /// A chain link with fixed text and an optional source, standing in for library wrappers.
    #[derive(Debug)]
    struct Link(
        &'static str,
        Option<Box<dyn std::error::Error + Send + Sync>>,
    );

    impl fmt::Display for Link {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for Link {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1
                .as_deref()
                .map(|e| e as &(dyn std::error::Error + 'static))
        }
    }

    fn wrapped(inner: impl std::error::Error + Send + Sync + 'static) -> Link {
        Link("client error (Connect)", Some(Box::new(inner)))
    }

    #[test]
    fn a_ca_certificate_used_as_a_server_certificate_is_an_unknown_issuer() {
        use std::sync::Arc;
        #[derive(Debug)]
        struct Named(&'static str);
        impl std::fmt::Display for Named {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        impl std::error::Error for Named {}
        let other = |name| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
                Arc::new(Named(name)),
            )))
        };
        assert_eq!(
            tls_cause(&other("CaUsedAsEndEntity")),
            ConnectCause::TlsUnknownIssuer
        );
        assert_eq!(tls_cause(&other("SomethingElse")), ConnectCause::Tls);
    }

    #[test]
    fn connect_cause_reads_the_source_chain() {
        use std::io;
        let refused = wrapped(Link(
            "tcp connect error",
            Some(Box::new(io::Error::from(io::ErrorKind::ConnectionRefused))),
        ));
        assert_eq!(connect_cause(&refused), ConnectCause::Refused);

        let tls_in_io = wrapped(io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::General("handshake".into()),
        ));
        assert_eq!(connect_cause(&tls_in_io), ConnectCause::Tls);
        let tls_direct = wrapped(rustls::Error::General("handshake".into()));
        assert_eq!(connect_cause(&tls_direct), ConnectCause::Tls);
        // The real shape: io(Other) wrapping io(InvalidData) wrapping the rustls error.
        let tls_nested = wrapped(io::Error::other(io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
        )));
        assert_eq!(connect_cause(&tls_nested), ConnectCause::TlsUnknownIssuer);
        let wrong_protocol = wrapped(io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::InvalidMessage(rustls::InvalidMessage::InvalidContentType),
        ));
        assert_eq!(
            connect_cause(&wrong_protocol),
            ConnectCause::TlsWrongProtocol
        );
        let expired = wrapped(rustls::Error::InvalidCertificate(
            rustls::CertificateError::Expired,
        ));
        assert_eq!(connect_cause(&expired), ConnectCause::TlsExpired);
        let refused_nested = wrapped(io::Error::other(io::Error::from(
            io::ErrorKind::ConnectionRefused,
        )));
        assert_eq!(connect_cause(&refused_nested), ConnectCause::Refused);

        let dns = wrapped(Link(
            "dns error",
            Some(Box::new(io::Error::other(
                "failed to lookup address information",
            ))),
        ));
        assert_eq!(connect_cause(&dns), ConnectCause::Dns);
        // Only the exact text counts, so a message that merely mentions DNS stays Other.
        let near = wrapped(Link("dns error while retrying", None));
        assert_eq!(connect_cause(&near), ConnectCause::Other);

        let other = wrapped(io::Error::from(io::ErrorKind::TimedOut));
        assert_eq!(connect_cause(&other), ConnectCause::Other);
    }

    /// The handshake's client speaks HTTP/1.1 under any policy, and its key never matches the
    /// policy's own client in the cache.
    #[tokio::test]
    async fn an_http1_only_key_overrides_the_policy() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let exec =
            Executor::new(ExecOptions::new().http_version(HttpVersionPolicy::Http2PriorKnowledge))
                .unwrap();
        let key = |http1_only: bool| ClientKey {
            rule: ProxyRule::Direct,
            identity: None,
            compress: true,
            overrides: Arc::default(),
            http1_only,
        };
        let uri = server.uri();
        let h1 = exec.build_client(&key(true)).unwrap();
        let policy = exec.build_client(&key(false)).unwrap();
        assert_eq!(
            h1.get(&uri).send().await.unwrap().version(),
            reqwest::Version::HTTP_11
        );
        assert_eq!(
            policy.get(&uri).send().await.unwrap().version(),
            reqwest::Version::HTTP_2
        );
        assert!(key(true) != key(false));
    }
}
