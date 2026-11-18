//! The cookie tools: the session's cookie jar, listed without values, cleared, and edited one
//! cookie at a time.

use std::time::{Duration, UNIX_EPOCH};

use apinata_core::{CookieInfo, CookieJar};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

/// One stored cookie, its value withheld.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct CookieDoc {
    pub name: String,
    pub domain: String,
    pub path: String,
    /// When it expires, in unix seconds; absent for a session cookie.
    pub expires: Option<u64>,
    pub secure: bool,
    pub http_only: bool,
}

/// What `list_cookies` returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct CookieList {
    pub cookies: Vec<CookieDoc>,
}

/// `clear_cookies`'s arguments.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClearArgs {
    /// Clear only this domain's cookies; every cookie when absent.
    pub domain: Option<String>,
}

/// What `clear_cookies` returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct Cleared {
    pub removed: usize,
}

/// Every unexpired cookie in `jar`, sorted by domain, path and name, without its value.
pub(crate) fn list(jar: &CookieJar) -> CookieList {
    let cookies = jar.list().into_iter().map(doc).collect();
    CookieList { cookies }
}

/// `c` as the cookie tools show it, without its value.
fn doc(c: CookieInfo) -> CookieDoc {
    CookieDoc {
        name: c.name,
        domain: c.domain,
        path: c.path,
        expires: c
            .expires
            .map(|at| at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())),
        secure: c.secure,
        http_only: c.http_only,
    }
}

/// Remove `domain`'s cookies, or every cookie when `domain` is `None`, and count them.
pub(crate) fn clear(jar: &CookieJar, domain: Option<&str>) -> Cleared {
    let removed = match domain {
        Some(domain) => jar.delete_domain(domain),
        None => {
            let removed = jar.list().len();
            jar.clear();
            removed
        }
    };
    Cleared { removed }
}

/// `set_cookie`'s arguments.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetArgs {
    /// The http or https URL the cookie is stored for, as if a response from it had set it.
    pub url: String,
    pub name: String,
    pub value: String,
    /// Share the cookie with this domain and its subdomains; it must be the URL's host or a
    /// parent of it. Absent keeps the cookie on the URL's exact host.
    pub domain: Option<String>,
    /// The path the cookie is scoped to; absent takes the URL's directory, as a response would.
    pub path: Option<String>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    /// When it expires, in unix seconds; absent for a session cookie.
    pub expires: Option<u64>,
}

/// What `set_cookie` returns: the stored cookie as `list_cookies` shows it.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct CookieSet {
    pub cookie: CookieDoc,
    /// Whether it replaced a cookie with the same domain, path and name.
    pub replaced: bool,
}

/// `delete_cookie`'s arguments, the fields of one `list_cookies` entry.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteArgs {
    pub domain: String,
    pub path: String,
    pub name: String,
}

/// What `delete_cookie` returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct CookieDeleted {
    /// False when no cookie had that domain, path and name.
    pub removed: bool,
}

/// RFC 6265's default-path: the URL path up to, not including, its last `/`, or `/`.
fn default_path(url: &Url) -> String {
    let path = url.path();
    match path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(at) => path[..at].to_owned(),
    }
}

/// Store one cookie as a response from `args.url` would: host-only unless `domain` names the
/// host or a parent of it, then the jar's own checks. Errors never quote the value.
pub(crate) fn set(jar: &CookieJar, args: SetArgs) -> Result<CookieSet, String> {
    let url = Url::parse(&args.url).map_err(|e| format!("invalid url: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("invalid url: it must be http or https".to_owned());
    }
    let (host, ip) = match url.host() {
        Some(url::Host::Domain(d)) => (d.trim_end_matches('.').to_ascii_lowercase(), false),
        Some(url::Host::Ipv4(a)) => (a.to_string(), true),
        Some(url::Host::Ipv6(a)) => (a.to_string(), true),
        None => return Err("invalid url: it has no host".to_owned()),
    };
    let domain = (args.domain.as_deref())
        .map(|d| {
            d.trim_matches('.')
                .trim_matches(['[', ']'])
                .to_ascii_lowercase()
        })
        .filter(|d| !d.is_empty());
    // An IP only ever matches itself, so naming it keeps the cookie host-only, as the jar
    // narrows a response's `Domain` equal to an IP host.
    let host_only = match &domain {
        None => true,
        Some(d) if *d == host => ip,
        Some(d) if !ip && host.ends_with(&format!(".{d}")) => false,
        Some(_) => return Err(format!("invalid cookie domain: does not match {host}")),
    };
    let mut cookie = CookieInfo::new(domain.unwrap_or(host), args.name, args.value);
    cookie.path = args.path.unwrap_or_else(|| default_path(&url));
    cookie.host_only = host_only;
    cookie.secure = args.secure;
    cookie.http_only = args.http_only;
    cookie.expires = args.expires.map(|s| UNIX_EPOCH + Duration::from_secs(s));
    let replaced = jar
        .list()
        .iter()
        .any(|c| c.domain == cookie.domain && c.path == cookie.path && c.name == cookie.name);
    jar.insert(cookie.clone()).map_err(|e| e.to_string())?;
    Ok(CookieSet {
        cookie: doc(cookie),
        replaced,
    })
}

/// Remove the cookie with exactly this domain, path and name, if there is one.
pub(crate) fn delete(jar: &CookieJar, args: &DeleteArgs) -> CookieDeleted {
    let domain = args.domain.trim_start_matches('.');
    CookieDeleted {
        removed: jar.delete(domain, &args.path, &args.name),
    }
}
