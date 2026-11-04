//! Fetching an import source from an http(s) URL. The fetch returns bytes only; the format is
//! found later by `parse_collections`. It sends no cookies and no credentials. It uses no proxy unless
//! [`SourceFetch::network`] hands it the settings requests are sent with.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use percent_encoding::percent_decode_str;
use reqwest::Client;
use reqwest::header::{ACCEPT, LOCATION};
use reqwest::redirect::Policy;
use url::Url;

use crate::execute::chain;
use crate::host_guard::{Guard, HostPolicy, SystemLookup, judge, refused_host};
use crate::network::{
    FetchOptions, GlobalProxy, NetworkSettings, PAC_CACHE, PacSend, ProxyRule, RequestProxy,
    SystemProxy, apply_rule, read_ca, resolve_hop, route, rule_for,
};
use hyper_util::client::proxy::matcher::Matcher;

/// How a source is fetched.
#[derive(Clone)]
#[non_exhaustive]
pub struct SourceFetch {
    /// Total time for the whole fetch, redirects included; 30 s by default.
    pub timeout: Duration,
    /// Largest body accepted, counted as it streams; 64 MiB by default, the import size limit.
    pub max_bytes: u64,
    /// Redirects followed before giving up; 5 by default.
    pub max_redirects: usize,
    /// Accept invalid TLS certificates.
    pub insecure: bool,
    /// A PEM file of root certificates to trust.
    pub ca_file: Option<PathBuf>,
    /// Keep the system roots beside `ca_file`.
    pub keep_default_roots: bool,
    host_policy: Option<HostPolicy>,
    proxy: GlobalProxy,
    force_no_proxy: bool,
}

impl Default for SourceFetch {
    fn default() -> Self {
        SourceFetch {
            timeout: Duration::from_secs(30),
            max_bytes: 64 << 20,
            max_redirects: 5,
            insecure: false,
            ca_file: None,
            keep_default_roots: true,
            host_policy: None,
            proxy: GlobalProxy::Off,
            force_no_proxy: false,
        }
    }
}

impl std::fmt::Debug for SourceFetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceFetch")
            .field("timeout", &self.timeout)
            .field("max_bytes", &self.max_bytes)
            .field("max_redirects", &self.max_redirects)
            .field("insecure", &self.insecure)
            .field("ca_file", &self.ca_file)
            .field("keep_default_roots", &self.keep_default_roots)
            .field("host_policy", &self.host_policy.is_some())
            .field("proxy", &self.proxy)
            .field("force_no_proxy", &self.force_no_proxy)
            .finish()
    }
}

impl SourceFetch {
    /// Fetch the way requests are sent under `settings`: its proxy (system, manual with a bypass
    /// list, or PAC), `force_no_proxy`, extra CA file and default roots. Without this call the
    /// fetch connects directly.
    #[must_use]
    pub fn network(mut self, settings: NetworkSettings) -> Self {
        self.ca_file = settings.ca_file;
        self.keep_default_roots = settings.keep_default_roots;
        self.proxy = settings.proxy;
        self.force_no_proxy = settings.force_no_proxy;
        self
    }

    /// The settings the fields describe, in the form the shared routing code reads.
    fn settings(&self) -> NetworkSettings {
        NetworkSettings::new()
            .proxy(self.proxy.clone())
            .force_no_proxy(self.force_no_proxy)
            .ca_file(self.ca_file.clone())
            .keep_default_roots(self.keep_default_roots)
    }

    /// Restrict the fetch to hosts `policy` accepts, given a normalised host name. Loopback
    /// addresses and `localhost` always pass; the given URL and every redirect hop are judged
    /// before any request for it is made.
    #[must_use]
    pub fn host_policy(mut self, policy: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        self.host_policy = Some(Arc::new(policy));
        self
    }
}

/// Why a fetch failed. Messages never contain the URL, so credentials cannot leak.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// The URL (or a redirect target) is not a usable http(s) URL without credentials.
    #[error("{0}")]
    Scheme(String),
    /// The connection, TLS or protocol failed.
    #[error("{0}")]
    Transport(String),
    /// The fetch did not finish within the timeout.
    #[error("the fetch timed out")]
    Timeout,
    /// The server answered with a status other than 2xx (or a redirect without a target).
    #[error("the server answered HTTP {0}")]
    Status(u16),
    /// The body is larger than the cap.
    #[error("the body is larger than {max_bytes} bytes")]
    TooLarge {
        /// The cap that was exceeded.
        max_bytes: u64,
    },
    /// The redirect chain is longer than allowed.
    #[error("too many redirects")]
    TooManyRedirects,
    /// The host policy refused a host.
    #[error("host `{host}` is not allowed (request {hop} of the redirect chain)")]
    HostNotAllowed {
        /// The normalised host.
        host: String,
        /// 0 for the given URL, then the number of redirects followed before this request.
        hop: usize,
    },
}

/// Fetches `url` with GET and returns the body. Only http and https URLs without a user name
/// or password are accepted. Redirects (301, 302, 303, 307, 308) are followed by hand up to
/// `opts.max_redirects`, each target judged by the host policy when one is set.
///
/// # Errors
/// A [`FetchError`] naming what stopped the fetch.
pub async fn fetch_source(url: &str, opts: &SourceFetch) -> Result<Vec<u8>, FetchError> {
    let first =
        checked(Url::parse(url).map_err(|_| FetchError::Scheme("not a valid URL".into()))?)?;
    // The system proxy is read once, and only when this fetch would inherit it.
    let rule = rule_for(&RequestProxy::Inherit, &opts.settings());
    let system = match rule {
        ProxyRule::System => SystemProxy::from_system(),
        _ => SystemProxy::new(Matcher::builder().build()),
    };
    tokio::time::timeout(opts.timeout, follow(first, opts, &rule, Arc::new(system)))
        .await
        .unwrap_or(Err(FetchError::Timeout))
}

/// Refuses anything but an http(s) URL with a host and no credentials.
fn checked(url: Url) -> Result<Url, FetchError> {
    if !matches!(url.scheme(), "http" | "https") {
        let scheme = url.scheme();
        return Err(FetchError::Scheme(format!(
            "only http and https URLs can be fetched, not `{scheme}`"
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FetchError::Scheme(
            "a URL with a user name or password is not accepted".into(),
        ));
    }
    if url.host_str().is_none() {
        return Err(FetchError::Scheme("the URL has no host".into()));
    }
    Ok(url)
}

/// The request loop: judge, send, and either return the body or move to the redirect target.
async fn follow(
    mut url: Url,
    opts: &SourceFetch,
    rule: &ProxyRule,
    system: Arc<SystemProxy>,
) -> Result<Vec<u8>, FetchError> {
    let mut hop = 0;
    loop {
        let refused = |host| FetchError::HostNotAllowed { host, hop };
        if let Some(policy) = &opts.host_policy {
            judge(&url, policy).map_err(refused)?;
        }
        // Under PAC the script picks this hop's proxy, after the target passed the policy.
        let rule = match rule {
            ProxyRule::Pac(source) => {
                let fetch_opts =
                    FetchOptions::new(opts.insecure, opts.ca_file.clone(), opts.keep_default_roots);
                let (proxy, _line) = resolve_hop(
                    &PAC_CACHE,
                    source,
                    &fetch_opts,
                    &url,
                    &mut PacSend::default(),
                )
                .await;
                match proxy {
                    Some(proxy) => ProxyRule::Custom {
                        proxy,
                        bypass: String::new(),
                    },
                    None => ProxyRule::Direct,
                }
            }
            other => other.clone(),
        };
        // A proxy is judged by its own address, so a bypassed hop never asks about it.
        if let (Some(policy), Some(proxy)) = (&opts.host_policy, route(&rule, &system, &url)) {
            judge(&proxy.address(), policy).map_err(refused)?;
        }
        let client = client(opts, &rule, &system)?;
        let response = client
            .get(url.clone())
            .header(
                ACCEPT,
                "application/json, application/yaml, text/yaml, */*;q=0.5",
            )
            .send()
            .await
            .map_err(|e| transport(e, hop))?;
        let status = response.status();
        if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
            if hop >= opts.max_redirects {
                return Err(FetchError::TooManyRedirects);
            }
            let target = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or(FetchError::Status(status.as_u16()))?;
            // A relative target resolves against the URL that answered.
            let next = url
                .join(target)
                .map_err(|_| FetchError::Scheme("a redirect target is not a valid URL".into()))?;
            url = checked(next)?;
            hop += 1;
            continue;
        }
        if !status.is_success() {
            return Err(FetchError::Status(status.as_u16()));
        }
        return read_body(response, opts.max_bytes, hop).await;
    }
}

/// Reads the body, refusing early on an announced length and counting what actually arrives,
/// since a server can announce nothing or lie.
async fn read_body(
    mut response: reqwest::Response,
    max_bytes: u64,
    hop: usize,
) -> Result<Vec<u8>, FetchError> {
    let too_large = FetchError::TooLarge { max_bytes };
    if response.content_length().is_some_and(|n| n > max_bytes) {
        return Err(too_large);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| transport(e, hop))? {
        if (body.len() + chunk.len()) as u64 > max_bytes {
            return Err(too_large);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn transport(error: reqwest::Error, hop: usize) -> FetchError {
    if error.is_timeout() {
        return FetchError::Timeout;
    }
    if let Some(host) = refused_host(&error) {
        return FetchError::HostNotAllowed { host, hop };
    }
    FetchError::Transport(chain(error))
}

/// A one-off client for one hop: routed by `rule`, no cookie store, no automatic redirects, the
/// system roots plus any CA file, and the guard resolver only when a host policy is set.
fn client(
    opts: &SourceFetch,
    rule: &ProxyRule,
    system: &Arc<SystemProxy>,
) -> Result<Client, FetchError> {
    let mut builder = Client::builder()
        .redirect(Policy::none())
        .tls_danger_accept_invalid_certs(opts.insecure);
    if let Some(policy) = &opts.host_policy {
        builder = builder.dns_resolver(Arc::new(Guard {
            policy: Arc::clone(policy),
            lookup: Arc::new(SystemLookup),
        }));
    }
    if let Some(path) = &opts.ca_file {
        let roots = read_ca(path).map_err(|e| FetchError::Transport(e.to_string()))?;
        builder = if opts.keep_default_roots {
            builder.tls_certs_merge(roots)
        } else {
            builder.tls_certs_only(roots)
        };
    }
    apply_rule(builder, rule, system)
        .build()
        .map_err(|e| FetchError::Transport(chain(e)))
}

/// The collection name to use for an imported URL: the last non-empty path segment,
/// percent-decoded and without its extension, else the host. `None` when `url` is not an
/// http(s) URL.
#[must_use]
pub fn default_import_name(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let from_path = url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
        .map(|s| percent_decode_str(s).decode_utf8_lossy().into_owned())
        .map(|s| match s.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
            _ => s,
        })
        .filter(|s| !s.is_empty());
    from_path.or_else(|| url.host_str().map(str::to_owned))
}
