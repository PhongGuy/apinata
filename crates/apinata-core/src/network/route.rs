//! Which proxy a hop goes through: the global settings, Bruno's bypass list and the system
//! proxy snapshot, combined into one `route` that the client and the executor both ask.

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use hyper_util::client::proxy::matcher::Matcher;
use url::Url;

use super::{PacSource, RequestProxy, ResolvedProxy};

/// The proxy used by collections that inherit the global setting.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GlobalProxy {
    /// Connect directly.
    Off,
    /// The operating system's proxy: environment variables everywhere, plus the OS settings on
    /// macOS and Windows.
    System,
    /// `proxy`, except for hosts the comma separated `bypass` list names.
    Manual {
        /// The proxy.
        proxy: ResolvedProxy,
        /// The bypass list.
        bypass: String,
    },
    /// Each hop asks the PAC script at `source` which proxy to use.
    Pac {
        /// The PAC file.
        source: PacSource,
    },
}

/// Network settings that apply to every request an executor sends.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NetworkSettings {
    /// The global proxy layer.
    pub proxy: GlobalProxy,
    /// Connect directly whatever the collection or global layer says.
    pub force_no_proxy: bool,
    /// A PEM file of extra CA certificates to trust.
    pub ca_file: Option<PathBuf>,
    /// Whether the default roots stay trusted alongside `ca_file`.
    pub keep_default_roots: bool,
}

impl Default for NetworkSettings {
    fn default() -> NetworkSettings {
        NetworkSettings {
            proxy: GlobalProxy::System,
            force_no_proxy: false,
            ca_file: None,
            keep_default_roots: true,
        }
    }
}

impl NetworkSettings {
    /// The defaults: the system proxy, the default roots and no extra CA.
    pub fn new() -> NetworkSettings {
        NetworkSettings::default()
    }

    /// The global proxy layer.
    pub fn proxy(mut self, proxy: GlobalProxy) -> NetworkSettings {
        self.proxy = proxy;
        self
    }

    /// Connect directly when `on`, overriding the collection and the global layer.
    pub fn force_no_proxy(mut self, on: bool) -> NetworkSettings {
        self.force_no_proxy = on;
        self
    }

    /// Trust the CA certificates in this PEM file too.
    pub fn ca_file(mut self, path: Option<PathBuf>) -> NetworkSettings {
        self.ca_file = path;
        self
    }

    /// Keep the default roots alongside `ca_file` when `keep`.
    pub fn keep_default_roots(mut self, keep: bool) -> NetworkSettings {
        self.keep_default_roots = keep;
        self
    }
}

/// Bruno's bypass test (its port of `proxy-from-env`): true sends `url` directly. Entries split
/// on `,`, `;` and whitespace; `*` matches all; `host:port` only that port; a leading `.` or `*`
/// is a suffix match, anything else an exact one. Entries are compared as written, not lowercased.
pub(crate) fn bypassed(url: &Url, list: &str) -> bool {
    if list.is_empty() {
        return false;
    }
    if list == "*" {
        return true;
    }
    let host = url.host_str().unwrap_or_default();
    let port = url.port_or_known_default().map_or(0, u64::from);
    list.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            // `^(.+):(\d+)$`: a trailing numeric port, which a port of 0 leaves unchecked as JS does.
            let (name, wanted) = match entry.rsplit_once(':') {
                Some((name, digits))
                    if !name.is_empty()
                        && !digits.is_empty()
                        && digits.bytes().all(|b| b.is_ascii_digit()) =>
                {
                    (name, digits.parse::<u64>().unwrap_or(u64::MAX))
                }
                _ => (entry, 0),
            };
            if wanted != 0 && wanted != port {
                return false;
            }
            match name.strip_prefix('*') {
                Some(suffix) => host.ends_with(suffix),
                None if name.starts_with('.') => host.ends_with(name),
                None => host == name,
            }
        })
}

/// The system proxy configuration, read once.
pub(crate) struct SystemProxy {
    matcher: Matcher,
}

impl SystemProxy {
    /// Reads the environment and, on macOS and Windows, the OS proxy settings.
    pub(crate) fn from_system() -> SystemProxy {
        SystemProxy::new(Matcher::from_system())
    }

    pub(crate) fn new(matcher: Matcher) -> SystemProxy {
        SystemProxy { matcher }
    }

    /// The proxy the system configuration names for `url`, with its credentials.
    pub(crate) fn route(&self, url: &Url) -> Option<ResolvedProxy> {
        let dst: http::Uri = url.as_str().parse().ok()?;
        let intercept = self.matcher.intercept(&dst)?;
        let uri = intercept.uri();
        let port = uri.port_u16().map(|p| p.to_string()).unwrap_or_default();
        let proxy = ResolvedProxy::from_system(uri.scheme_str()?, uri.host()?, &port)?;
        // HTTP proxies carry their credentials as a ready Basic header; decode it back.
        let basic = intercept.basic_auth().and_then(|header| {
            let encoded = header.to_str().ok()?.strip_prefix("Basic ")?;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?;
            let text = String::from_utf8(decoded).ok()?;
            let (user, pass) = text.split_once(':')?;
            Some((user.to_owned(), pass.to_owned()))
        });
        let raw = intercept
            .raw_auth()
            .map(|(user, pass)| (user.to_owned(), pass.to_owned()));
        Some(match basic.or(raw) {
            Some((user, pass)) => proxy.with_credentials(user, pass),
            None => proxy,
        })
    }
}

/// How one client routes every connection it opens.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ProxyRule {
    Direct,
    System,
    Custom {
        proxy: ResolvedProxy,
        bypass: String,
    },
    /// Resolved per hop by the PAC script before any client sees the rule.
    Pac(PacSource),
}

/// The rule for a request: `force_no_proxy` and the collection's own choice come before the
/// global layer.
pub(crate) fn rule_for(request: &RequestProxy, settings: &NetworkSettings) -> ProxyRule {
    let custom = |proxy: &ResolvedProxy, bypass: &String| ProxyRule::Custom {
        proxy: proxy.clone(),
        bypass: bypass.clone(),
    };
    if settings.force_no_proxy {
        return ProxyRule::Direct;
    }
    match request {
        RequestProxy::Off => ProxyRule::Direct,
        RequestProxy::Custom { proxy, bypass } => custom(proxy, bypass),
        RequestProxy::Inherit => match &settings.proxy {
            GlobalProxy::Off => ProxyRule::Direct,
            GlobalProxy::System => ProxyRule::System,
            GlobalProxy::Manual { proxy, bypass } => custom(proxy, bypass),
            GlobalProxy::Pac { source } => ProxyRule::Pac(source.clone()),
        },
    }
}

/// The proxy `rule` sends `url` through, `None` for a direct connection.
pub(crate) fn route(rule: &ProxyRule, system: &SystemProxy, url: &Url) -> Option<ResolvedProxy> {
    match rule {
        ProxyRule::Direct => None,
        ProxyRule::System => system.route(url),
        ProxyRule::Custom { proxy, bypass } => (!bypassed(url, bypass)).then(|| proxy.clone()),
        ProxyRule::Pac(_) => None,
    }
}

/// Points `builder` at `rule`: no proxy for Direct, else a closure that asks the same `route`
/// the callers judge hops with, so the connection and the host policy cannot disagree.
pub(crate) fn apply_rule(
    builder: reqwest::ClientBuilder,
    rule: &ProxyRule,
    system: &Arc<SystemProxy>,
) -> reqwest::ClientBuilder {
    match rule {
        ProxyRule::Direct => builder.no_proxy(),
        rule => {
            let (rule, system) = (rule.clone(), Arc::clone(system));
            builder.proxy(reqwest::Proxy::custom(move |dst| {
                route(&rule, &system, dst).map(|p| p.url())
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper_util::client::proxy::matcher::Matcher;

    fn url(text: &str) -> Url {
        Url::parse(text).expect("url")
    }

    #[test]
    fn bypass_follows_brunos_proxy_from_env_port() {
        let cases = [
            ("http://a.test/", "*", true),
            ("http://a.test/", "", false),
            ("http://a.test/", "   ", false),
            ("http://a.test/", "a.test", true),
            ("http://b.a.test/", "a.test", false),
            ("http://b.a.test/", ".a.test", true),
            ("http://b.a.test/", "*.a.test", true),
            ("http://a.test/", ".a.test", false),
            ("http://xa.test/", "*a.test", true),
            ("http://a.test:8080/", "a.test:8080", true),
            ("http://a.test/", "a.test:8080", false),
            ("https://a.test/", "a.test:443", true),
            ("http://a.test/", "x.test, a.test", true),
            ("http://a.test/", "x.test;a.test", true),
            ("http://a.test/", "x.test,*", true),
            ("http://[::1]:3000/", "[::1]", true),
            ("http://A.TEST/", "a.test", true),
            ("http://a.test/", "A.TEST", false),
        ];
        for (target, list, direct) in cases {
            assert_eq!(bypassed(&url(target), list), direct, "{target} {list:?}");
        }
    }

    #[test]
    fn system_routes_follow_the_matcher_and_keep_credentials() {
        let system = SystemProxy::new(
            Matcher::builder()
                .http("http://us%20er:p%40ss@10.1.2.3:3128")
                .no("skip.test")
                .build(),
        );
        let proxy = system.route(&url("http://api.test/x")).expect("proxied");
        assert_eq!(proxy.to_string(), "http://10.1.2.3:3128");
        assert_eq!(proxy.credentials(), Some(("us er", "p@ss")));
        assert!(system.route(&url("http://skip.test/")).is_none());
        assert!(
            system.route(&url("https://api.test/")).is_none(),
            "only http was configured"
        );
        let socks = SystemProxy::new(
            Matcher::builder()
                .http("socks5h://u:p@10.1.2.3:1080")
                .build(),
        );
        let proxy = socks.route(&url("http://api.test/")).expect("proxied");
        assert_eq!(
            (proxy.scheme(), proxy.credentials()),
            ("socks5h", Some(("u", "p")))
        );
    }

    #[test]
    fn pac_applies_only_to_requests_that_inherit() {
        let source = PacSource::parse("http://127.0.0.1/p.pac", None).expect("source");
        let settings = NetworkSettings::new().proxy(GlobalProxy::Pac {
            source: source.clone(),
        });
        assert!(
            matches!(rule_for(&RequestProxy::Inherit, &settings), ProxyRule::Pac(s) if s == source)
        );
        assert!(matches!(
            rule_for(&RequestProxy::Off, &settings),
            ProxyRule::Direct
        ));
        let proxy = ResolvedProxy::new("http", "10.0.0.1", "3128").expect("proxy");
        let custom = RequestProxy::Custom {
            proxy: proxy.clone(),
            bypass: String::new(),
        };
        assert!(
            matches!(rule_for(&custom, &settings), ProxyRule::Custom { proxy: p, .. } if p == proxy)
        );
        let forced = settings.force_no_proxy(true);
        assert!(matches!(
            rule_for(&RequestProxy::Inherit, &forced),
            ProxyRule::Direct
        ));
        let system = SystemProxy::new(Matcher::builder().build());
        assert!(route(&ProxyRule::Pac(source), &system, &url("http://a.test/")).is_none());
    }
}
