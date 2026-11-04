//! The opt-in send restriction: each hop's host is judged before any I/O for it. Loopback
//! literals and `localhost` names always pass; every other host must satisfy the caller's policy.

use std::error::Error as StdError;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

use crate::network::{ResolvedHostOverride, overridden};
use crate::trust::{is_loopback, normalize_host};

/// The caller's verdict on a normalised host that is neither loopback nor local.
pub(crate) type HostPolicy = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Whether the normalised `host` is `localhost` or a name under it.
pub(crate) fn is_local_name(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost")
}

/// Passes `url` when its host is a loopback literal, a local name or one `policy` trusts;
/// otherwise returns the normalised host, empty when the URL has none. The policy is never
/// asked about loopback or local hosts.
pub(crate) fn judge(url: &Url, policy: &HostPolicy) -> Result<(), String> {
    let name = url.host_str().and_then(normalize_host).unwrap_or_default();
    let allowed = match url.host() {
        Some(Host::Ipv4(ip)) => is_loopback(ip.into()) || policy(&name),
        Some(Host::Ipv6(ip)) => is_loopback(ip.into()) || policy(&name),
        Some(Host::Domain(_)) => is_local_name(&name) || policy(&name),
        None => false,
    };
    if allowed { Ok(()) } else { Err(name) }
}

/// A refused hop: the host the policy said no to and, for an overridden name, that name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) host: String,
    pub(crate) mapped_from: Option<String>,
}

/// [`judge`] for a send with host overrides. An overridden name is judged by where it will
/// connect, whatever the name is: each mapped address must be loopback or trusted, and an
/// unspecified address is never a destination. Other names are judged as `judge` does.
pub(crate) fn judge_with_overrides(
    url: &Url,
    policy: &HostPolicy,
    overrides: &[ResolvedHostOverride],
) -> Result<(), Refusal> {
    let name = url.host_str().and_then(normalize_host).unwrap_or_default();
    if let (Some(Host::Domain(_)), Some(found)) = (url.host(), overridden(overrides, &name)) {
        return judge_addresses(found, policy);
    }
    judge(url, policy).map_err(|host| Refusal {
        host,
        mapped_from: None,
    })
}

/// The address half of [`judge_with_overrides`], also asked again when the connection resolves.
pub(crate) fn judge_addresses(
    found: &ResolvedHostOverride,
    policy: &HostPolicy,
) -> Result<(), Refusal> {
    for addr in found.addresses() {
        let ip = addr.ip();
        let host = match ip {
            IpAddr::V4(v4) => Host::<String>::Ipv4(v4),
            IpAddr::V6(v6) => Host::Ipv6(v6),
        }
        .to_string();
        let allowed = !ip.to_canonical().is_unspecified() && (is_loopback(ip) || policy(&host));
        if !allowed {
            return Err(Refusal {
                host,
                mapped_from: Some(found.host().to_owned()),
            });
        }
    }
    Ok(())
}

/// Turns a host name into addresses; the production lookup asks the system resolver.
pub(crate) trait Lookup: Send + Sync + 'static {
    fn lookup(&self, host: String)
    -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>>;
}

/// The system resolver, through tokio's `lookup_host`.
pub(crate) struct SystemLookup;

impl Lookup for SystemLookup {
    fn lookup(
        &self,
        host: String,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>> {
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
            Ok(addrs.map(|a| a.ip()).collect())
        })
    }
}

/// A resolver that re-applies the host rules for every new connection, so a name the
/// policy no longer trusts is refused before any DNS query is made.
pub(crate) struct Guard {
    pub(crate) policy: HostPolicy,
    pub(crate) lookup: Arc<dyn Lookup>,
}

/// The resolver's refusal, carrying the normalised host; `refused_host` finds it in a chain.
#[derive(Debug, thiserror::Error)]
#[error("host `{host}` refused by the host policy")]
pub(crate) struct Refused {
    pub(crate) host: String,
    pub(crate) mapped_from: Option<String>,
}

impl Resolve for Guard {
    fn resolve(&self, name: Name) -> Resolving {
        let host = normalize_host(name.as_str()).unwrap_or_default();
        let raw = name.as_str().to_owned();
        let (policy, lookup) = (Arc::clone(&self.policy), Arc::clone(&self.lookup));
        Box::pin(async move {
            // Local names never reach DNS; anything not trusted fails before a query is made.
            let ips = if is_local_name(&host) {
                vec![
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    IpAddr::V6(Ipv6Addr::LOCALHOST),
                ]
            } else if !host.is_empty() && policy(&host) {
                lookup.lookup(raw).await?
            } else {
                return Err(Box::new(Refused {
                    host,
                    mapped_from: None,
                }) as Box<dyn StdError + Send + Sync>);
            };
            // Port 0 is replaced by the URL's port.
            let addrs: Addrs = Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

/// The first `Refused` in `error`'s source chain, if any.
pub(crate) fn refusal_in(error: &(dyn StdError + 'static)) -> Option<Refusal> {
    let mut current = Some(error);
    while let Some(e) = current {
        if let Some(Refused { host, mapped_from }) = e.downcast_ref::<Refused>() {
            return Some(Refusal {
                host: host.clone(),
                mapped_from: mapped_from.clone(),
            });
        }
        current = e.source();
    }
    None
}

/// The host of the first `Refused` in `error`'s source chain, if any.
pub(crate) fn refused_host(error: &(dyn StdError + 'static)) -> Option<String> {
    refusal_in(error).map(|r| r.host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Collection, ContextOptions, ExecError, ExecOptions, Executor, ResolvedRequest, VarContext,
        resolve,
    };
    use std::str::FromStr;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A lookup that records every name it is asked for and answers 127.0.0.1.
    #[derive(Default)]
    struct Recording(Mutex<Vec<String>>);

    impl Lookup for Recording {
        fn lookup(
            &self,
            host: String,
        ) -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>> {
            self.0.lock().unwrap().push(host);
            Box::pin(async { Ok(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]) })
        }
    }

    fn names(lookup: &Recording) -> Vec<String> {
        lookup.0.lock().unwrap().clone()
    }

    /// A GET of `url` resolved from a one-request collection.
    fn resolved(url: &str) -> ResolvedRequest {
        let dir = tempfile::TempDir::new().unwrap();
        let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
        let req =
            format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n");
        std::fs::write(dir.path().join("opencollection.yml"), root).unwrap();
        std::fs::write(dir.path().join("r.yml"), req).unwrap();
        let c = Collection::open(dir.path()).unwrap();
        let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
        resolve(&c.tree().requests()[0], &ctx).unwrap()
    }

    /// An executor under `policy` whose trusted names go to a fresh recorder.
    fn executor(policy: HostPolicy) -> (Executor, Arc<Recording>) {
        let lookup = Arc::new(Recording::default());
        let opts = ExecOptions::new().host_policy(policy);
        let exec = Executor::with_lookup(opts, Arc::clone(&lookup) as Arc<dyn Lookup>).unwrap();
        (exec, lookup)
    }

    async fn ok_server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(path("/ok"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        server
    }

    /// A policy that says yes to its first `yes` questions and no after.
    fn first(yes: usize) -> HostPolicy {
        let asked = AtomicUsize::new(0);
        Arc::new(move |_: &str| asked.fetch_add(1, Ordering::SeqCst) < yes)
    }

    #[tokio::test]
    async fn untrusted_and_localhost_names_are_never_looked_up() {
        let server = ok_server().await;
        let port = server.address().port();
        let (exec, lookup) = executor(Arc::new(|_: &str| false));
        let err = exec
            .send(&resolved(&format!("http://untrusted.example:{port}/ok")))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ExecError::HostNotAllowed { hop: 0, .. }),
            "{err:?}"
        );
        for host in ["api.localhost", "API.Localhost.", "localhost"] {
            let res = exec
                .send(&resolved(&format!("http://{host}:{port}/ok")))
                .await;
            assert_eq!(res.expect(host).status(), 200, "{host}");
        }
        assert!(names(&lookup).is_empty(), "{:?}", names(&lookup));
    }

    #[tokio::test]
    async fn a_trusted_name_is_looked_up_and_reaches_the_server() {
        let server = ok_server().await;
        let port = server.address().port();
        let (exec, lookup) = executor(Arc::new(|host: &str| host == "api.test"));
        let res = exec
            .send(&resolved(&format!("http://API.test:{port}/ok")))
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(names(&lookup), ["api.test"]);
    }

    #[tokio::test]
    async fn the_resolver_re_applies_the_rules_before_any_lookup() {
        let lookup = Arc::new(Recording::default());
        let guard = Guard {
            policy: Arc::new(|_: &str| false),
            lookup: Arc::clone(&lookup) as Arc<dyn Lookup>,
        };
        let err = guard
            .resolve(Name::from_str("evil.example").unwrap())
            .await
            .err()
            .unwrap();
        assert_eq!(refused_host(err.as_ref()).as_deref(), Some("evil.example"));
        let local: Vec<SocketAddr> = guard
            .resolve(Name::from_str("Foo.LOCALHOST.").unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(
            local,
            ["127.0.0.1:0".parse().unwrap(), "[::1]:0".parse().unwrap()]
        );
        assert!(names(&lookup).is_empty());

        // The pre-hop check says yes, the resolver's second question says no.
        let server = ok_server().await;
        let (exec, lookup) = executor(first(1));
        let url = format!("http://api.test:{}/ok", server.address().port());
        let err = exec.send(&resolved(&url)).await.unwrap_err();
        assert!(
            matches!(&err, ExecError::HostNotAllowed { hop: 0, host, .. } if host == "api.test"),
            "{err:?}"
        );
        assert!(names(&lookup).is_empty());
    }

    #[tokio::test]
    async fn pooling_is_off_so_every_send_asks_the_resolver_again() {
        // Send one asks twice (pre-hop, resolver); send two passes its pre-hop check, so only
        // a fresh connection's resolver call can refuse it.
        let server = ok_server().await;
        let (exec, lookup) = executor(first(3));
        let url = format!("http://api.test:{}/ok", server.address().port());
        assert_eq!(exec.send(&resolved(&url)).await.unwrap().status(), 200);
        let err = exec.send(&resolved(&url)).await.unwrap_err();
        assert!(
            matches!(err, ExecError::HostNotAllowed { hop: 0, .. }),
            "{err:?}"
        );
        assert_eq!(names(&lookup), ["api.test"]);
    }

    #[tokio::test]
    async fn a_redirect_hop_refused_by_the_resolver_reports_its_hop() {
        // Hop 0 is a loopback literal, so the policy's only yes goes to hop 1's pre-check
        // and the resolver's question for hop 1 gets the no.
        let server = MockServer::start().await;
        let port = server.address().port();
        Mock::given(path("/from"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("http://api.test:{port}/ok")),
            )
            .mount(&server)
            .await;
        let (exec, lookup) = executor(first(1));
        let err = exec
            .send(&resolved(&format!("{}/from", server.uri())))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ExecError::HostNotAllowed { hop: 1, host, .. } if host == "api.test"),
            "{err:?}"
        );
        assert!(names(&lookup).is_empty());
    }

    /// A lookup that fails every name, as an unknown host does.
    struct NoSuchHost;

    impl Lookup for NoSuchHost {
        fn lookup(
            &self,
            _host: String,
        ) -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>> {
            Box::pin(async { Err(io::Error::other("failed to lookup address information")) })
        }
    }

    // The real hyper-util chain names a failed lookup `dns error`; a dependency bump that
    // renames it fails here instead of quietly degrading the cause to Other.
    #[tokio::test]
    async fn a_failed_lookup_is_a_dns_connect_error() {
        let opts = ExecOptions::new().host_policy(Arc::new(|_: &str| true));
        let exec = Executor::with_lookup(opts, Arc::new(NoSuchHost) as Arc<dyn Lookup>).unwrap();
        let err = exec
            .send(&resolved("http://nowhere.test:9/"))
            .await
            .expect_err("no address");
        assert!(
            matches!(
                err,
                ExecError::Connect {
                    cause: crate::ConnectCause::Dns,
                    ..
                }
            ),
            "{err:?}"
        );
    }
}
