//! What a request's network block resolves to: the proxy to use and the client certificate.

use std::fmt;
use std::path::PathBuf;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

use super::hosts::ResolvedHostOverride;
use crate::error::ResolveError;

/// What `encodeURIComponent` escapes: everything but alphanumerics and `-_.!~*'()`.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// A validated proxy address with optional credentials.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedProxy {
    scheme: String,
    host: String,
    port: u16,
    credentials: Option<(String, String)>,
}

impl ResolvedProxy {
    /// `protocol` is http, https, socks4 or socks5 (ASCII case-insensitive); `host` a DNS name,
    /// IPv4 or bracketed IPv6 literal, normalised like a URL host; an empty `port` is the scheme's
    /// default (80, 443, 1080, 1080). Fails with [`ResolveError::InvalidProxy`].
    pub fn new(protocol: &str, host: &str, port: &str) -> Result<ResolvedProxy, ResolveError> {
        ResolvedProxy::checked(protocol, host, port, false)
    }

    /// A proxy the system configuration or a PAC answer names, which may also be `socks4a` or
    /// `socks5h`.
    pub(crate) fn from_system(protocol: &str, host: &str, port: &str) -> Option<ResolvedProxy> {
        ResolvedProxy::checked(protocol, host, port, true).ok()
    }

    fn checked(
        protocol: &str,
        host: &str,
        port: &str,
        remote_dns: bool,
    ) -> Result<ResolvedProxy, ResolveError> {
        let invalid = |reason: &str| ResolveError::InvalidProxy {
            reason: reason.to_owned(),
        };
        let scheme = protocol.to_ascii_lowercase();
        let default = match scheme.as_str() {
            "http" => 80,
            "https" => 443,
            "socks4" | "socks5" => 1080,
            "socks4a" | "socks5h" if remote_dns => 1080,
            _ => return Err(invalid("the protocol is not http, https, socks4 or socks5")),
        };
        let port = if port.is_empty() {
            default
        } else if port.bytes().all(|b| b.is_ascii_digit()) {
            port.parse()
                .map_err(|_| invalid("the port is not between 0 and 65535"))?
        } else {
            return Err(invalid("the port is not a number"));
        };
        let bad_host = || invalid("the hostname is not a valid host");
        if host.is_empty()
            || host
                .chars()
                .any(|c| c.is_whitespace() || "@/?#\\".contains(c))
        {
            return Err(bad_host());
        }
        // Parsed under `http` whatever the scheme, so SOCKS hosts are normalised as URL hosts are.
        let url = Url::parse(&format!("http://{host}:{port}/")).map_err(|_| bad_host())?;
        let host = url.host_str().ok_or_else(bad_host)?.to_owned();
        Ok(ResolvedProxy {
            scheme,
            host,
            port,
            credentials: None,
        })
    }

    /// The proxy with these credentials.
    pub fn with_credentials(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> ResolvedProxy {
        self.credentials = Some((username.into(), password.into()));
        self
    }

    /// `http`, `https`, `socks4` or `socks5`; a system or PAC proxy may be `socks4a` or `socks5h`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The normalised host; an IPv6 literal keeps its brackets.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port, the scheme's default when none was given.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The proxy as reqwest takes it, credentials percent-encoded as `encodeURIComponent` does.
    // The scheme, host and port were validated when the proxy was parsed.
    #[allow(clippy::expect_used)]
    pub(crate) fn url(&self) -> Url {
        let userinfo = match &self.credentials {
            Some((user, pass)) => format!(
                "{}:{}@",
                utf8_percent_encode(user, URI_COMPONENT),
                utf8_percent_encode(pass, URI_COMPONENT)
            ),
            None => String::new(),
        };
        let text = format!("{}://{userinfo}{}:{}", self.scheme, self.host, self.port);
        Url::parse(&text).expect("a validated scheme, host and port form a URL")
    }

    /// The proxy's host and port as an `http` URL for judging under a host policy: SOCKS schemes
    /// are not special, so under them `url` would read an IP literal as a domain.
    // The host and port were validated when the proxy was parsed.
    #[allow(clippy::expect_used)]
    pub(crate) fn address(&self) -> Url {
        Url::parse(&format!("http://{}:{}/", self.host, self.port))
            .expect("a validated host and port form a URL")
    }

    /// `text` with the user name and password, raw and percent-encoded as `url` writes them,
    /// replaced by `***`. Longer values go first, so one inside the other is still caught.
    pub(crate) fn redact(&self, text: &str) -> String {
        let Some((user, pass)) = self.credentials() else {
            return text.to_owned();
        };
        let mut needles: Vec<String> = [user, pass]
            .into_iter()
            .filter(|v| !v.is_empty())
            .flat_map(|v| {
                [
                    v.to_owned(),
                    utf8_percent_encode(v, URI_COMPONENT).to_string(),
                ]
            })
            .collect();
        needles.sort_by_key(|n| std::cmp::Reverse(n.len()));
        needles
            .iter()
            .fold(text.to_owned(), |text, needle| text.replace(needle, "***"))
    }

    /// The user name and password, when set.
    #[allow(dead_code)] // read by the route tests and the proxy's secret check
    pub(crate) fn credentials(&self) -> Option<(&str, &str)> {
        self.credentials
            .as_ref()
            .map(|(user, pass)| (user.as_str(), pass.as_str()))
    }
}

impl fmt::Display for ResolvedProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}:{}", self.scheme, self.host, self.port)
    }
}

// Credentials are secrets, so Debug says only whether they are set.
impl fmt::Debug for ResolvedProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedProxy")
            .field("proxy", &self.to_string())
            .field("credentials", &self.credentials.is_some())
            .finish()
    }
}

/// The proxy choice of one request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RequestProxy {
    /// Use the global setting.
    #[default]
    Inherit,
    /// Connect directly.
    Off,
    /// Use `proxy`, except for hosts the comma separated `bypass` list names.
    Custom {
        /// The proxy.
        proxy: ResolvedProxy,
        /// The bypass list, expanded.
        bypass: String,
    },
}

/// A certificate file: where it is read from and how the collection wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CertPath {
    /// The expanded path, joined to the collection root when relative.
    pub read: PathBuf,
    /// The path as written, before expansion.
    pub written: String,
}

/// The files of a chosen client certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CertificateFiles {
    /// A PEM certificate and private key.
    Pem {
        /// The certificate.
        certificate: CertPath,
        /// The private key.
        private_key: CertPath,
    },
    /// A PKCS#12 bundle.
    Pkcs12 {
        /// The bundle.
        file: CertPath,
    },
}

/// The client certificate chosen for a request, its templates expanded.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedCertificate {
    domain: String,
    files: CertificateFiles,
    passphrase: String,
}

impl ResolvedCertificate {
    pub(crate) fn new(
        domain: String,
        files: CertificateFiles,
        passphrase: String,
    ) -> ResolvedCertificate {
        ResolvedCertificate {
            domain,
            files,
            passphrase,
        }
    }

    /// The expanded domain pattern that matched.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The certificate's files.
    pub fn files(&self) -> &CertificateFiles {
        &self.files
    }

    /// The key's passphrase, empty for none.
    pub(crate) fn passphrase(&self) -> &str {
        &self.passphrase
    }
}

impl fmt::Debug for ResolvedCertificate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedCertificate")
            .field("domain", &self.domain)
            .field("files", &self.files)
            .field("passphrase", &"***")
            .finish()
    }
}

/// A request's proxy and client certificate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct RequestNetwork {
    pub(crate) proxy: RequestProxy,
    pub(crate) certificate: Option<ResolvedCertificate>,
    pub(crate) secret_in_proxy: bool,
    pub(crate) host_overrides: Vec<ResolvedHostOverride>,
    /// What the network change check judges this send from; set by every resolve.
    pub(crate) basis: Option<std::sync::Arc<super::check::NetworkBasis>>,
}

impl RequestNetwork {
    /// The proxy choice.
    pub fn proxy(&self) -> &RequestProxy {
        &self.proxy
    }

    /// The first enabled certificate whose domain matches the request URL.
    pub fn certificate(&self) -> Option<&ResolvedCertificate> {
        self.certificate.as_ref()
    }

    /// The enabled host overrides, which apply to every hop of the send.
    pub fn host_overrides(&self) -> &[ResolvedHostOverride] {
        &self.host_overrides
    }

    /// Whether a sensitive value was substituted into the proxy's protocol, host or port.
    pub fn secret_in_proxy(&self) -> bool {
        self.secret_in_proxy
    }
}

/// Bruno's certificate test: `^https:\/\/` then the domain with `.` literal and `*` as `.*`,
/// anchored at the start only. A domain that does not compile matches nothing.
pub(crate) fn domain_matches(domain: &str, url: &str) -> bool {
    let pattern = format!(
        "^https:\\/\\/{}",
        domain.replace('.', "\\.").replace('*', ".*")
    );
    regress::Regex::new(&pattern).is_ok_and(|re| re.find(url).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_removes_raw_and_encoded_credentials() {
        let proxy = ResolvedProxy::new("http", "127.0.0.1", "3128")
            .expect("proxy")
            .with_credentials("pw", "pw x/1");
        let text = "http://pw:pw%20x%2F1@h failed for pw x/1 and pw";
        assert_eq!(
            proxy.redact(text),
            "http://***:***@h failed for *** and ***"
        );
        let bare = ResolvedProxy::new("http", "127.0.0.1", "").expect("proxy");
        assert_eq!(bare.redact("pw"), "pw");
    }
}
