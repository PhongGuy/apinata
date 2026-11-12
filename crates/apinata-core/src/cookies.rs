//! The executor's cookie jar: the store every send reads and writes, open to listing and
//! deletion so the app can show and prune what a session collected.

use crate::sealed::{self, Persistence};
use crate::secrets::SecretStore;
use cookie_store::{CookieDomain, CookieExpiration, CookieStore, RawCookie};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, PoisonError, RwLock};
use std::time::{Duration, SystemTime};
use url::Url;

const KEY_NAME: &str = "cookie-jar-key";
const LABEL: &str = "cookie jar";

// The vendored public suffix list, parsed once per process and cloned into every jar's store,
// so a `Domain` attribute naming a suffix no site owns is refused. Never fetched at run time.
// The list is vendored into the binary, so a parse failure is a bug every run hits.
#[allow(clippy::expect_used)]
static PUBLIC_SUFFIXES: LazyLock<publicsuffix::List> = LazyLock::new(|| {
    include_str!("../vendor/public_suffix_list.dat")
        .parse()
        .expect("the vendored public suffix list parses")
});

// Whether `domain` is itself an entry of the list, as the store judges it. Only listed entries
// count here; dotless names fall to `is_dotless_suffix`.
fn is_public_suffix(domain: &str) -> bool {
    use publicsuffix::Psl;
    let domain = domain.trim_start_matches('.').to_ascii_lowercase();
    PUBLIC_SUFFIXES
        .suffix(domain.as_bytes())
        .is_some_and(|s| s.is_known() && s == domain.as_bytes())
}

// Whether `domain` is one label other than `localhost`, which the list's implicit `*` rule makes
// a public suffix. An IP address (`::1`) is not a label.
fn is_dotless_suffix(domain: &str) -> bool {
    let d = domain.trim_matches('.').trim_matches(['[', ']']);
    !d.is_empty()
        && !d.contains('.')
        && !d.eq_ignore_ascii_case("localhost")
        && d.parse::<std::net::IpAddr>().is_err()
}

/// What the jar does with a `Domain` attribute before the store's own rules apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DomainRule {
    /// Left to the store.
    Keep,
    /// Dropped, so the cookie stays on the exact request host.
    HostOnly,
    /// Refused as a public suffix.
    Reject,
}

// A dotless `Domain` (other than `localhost`) is a public suffix unless it is the request host,
// which, like an IP `Domain` equal to the host, narrows the cookie to that host (RFC 6265 5.3
// step 5); an IP only ever domain-matches itself.
fn domain_rule(domain: Option<&str>, url: &Url) -> DomainRule {
    use std::net::IpAddr;
    let Some(d) = domain
        .map(|d| {
            d.trim_matches('.')
                .trim_matches(['[', ']'])
                .to_ascii_lowercase()
        })
        .filter(|d| !d.is_empty())
    else {
        return DomainRule::Keep;
    };
    let ip = d.parse::<IpAddr>().ok();
    let is_host = match url.host() {
        Some(url::Host::Domain(h)) => h.trim_end_matches('.').eq_ignore_ascii_case(&d),
        Some(url::Host::Ipv4(a)) => ip == Some(IpAddr::V4(a)),
        Some(url::Host::Ipv6(a)) => ip == Some(IpAddr::V6(a)),
        None => false,
    };
    match () {
        _ if (ip.is_some() || is_dotless_suffix(&d)) && is_host => DomainRule::HostOnly,
        _ if is_dotless_suffix(&d) => DomainRule::Reject,
        _ => DomainRule::Keep,
    }
}

/// The one line every shell shows when loading dropped `count` saved cookies because their
/// domains are public suffixes; `None` for 0. It names no cookie, value or domain.
pub fn dropped_cookies_notice(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some(
            "1 saved cookie was dropped: its domain is a public suffix, so it would reach every \
             site under it."
                .to_owned(),
        ),
        n => Some(format!(
            "{n} saved cookies were dropped: their domains are public suffixes, so they would \
             reach every site under them."
        )),
    }
}

/// The cookies an executor has stored, shared by every request it sends.
pub struct CookieJar {
    store: RwLock<CookieStore>,
    /// Counts mutations, so a saver can tell whether the store changed since it last wrote.
    revision: AtomicU64,
    /// The sealed file behind an opened jar; `None` keeps the jar in memory only.
    backing: Option<Backing>,
}

impl Default for CookieJar {
    fn default() -> CookieJar {
        CookieJar {
            store: RwLock::new(CookieStore::new_with_public_suffix(Some(
                PUBLIC_SUFFIXES.clone(),
            ))),
            revision: AtomicU64::new(0),
            backing: None,
        }
    }
}

/// One `Set-Cookie` value a response hop carried, and what the jar did with it. `domain` and
/// `path` are the attributes as sent, `None` when absent. Debug never shows `value`.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SetCookie {
    /// The cookie's name; for an unparseable value, the text before its first `=`.
    pub name: String,
    /// The cookie's value; for an unparseable value, the text after its first `=`.
    pub value: String,
    /// The `Domain` attribute.
    pub domain: Option<String>,
    /// The `Path` attribute.
    pub path: Option<String>,
    /// Whether it carried `Secure`.
    pub secure: bool,
    /// Whether it carried `HttpOnly`.
    pub http_only: bool,
    /// What the jar did with it.
    pub outcome: CookieOutcome,
}

impl fmt::Debug for SetCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetCookie")
            .field("name", &self.name)
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

/// What the jar did with one received `Set-Cookie` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CookieOutcome {
    /// Stored, new or replacing one with the same name, domain and path.
    Stored,
    /// An already expired cookie that deleted the stored one it matched.
    Removed,
    /// Not stored, for the reason given.
    Rejected(CookieRejection),
}

/// Why the jar did not store a received cookie; `Display` gives a short phrase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CookieRejection {
    /// A `Secure` cookie received over plain http from a host that is not loopback.
    InsecureSecure,
    /// The `Domain` attribute does not cover the host that sent it.
    DomainMismatch,
    /// Already expired, with no stored cookie to delete.
    Expired,
    /// The value is not a cookie.
    Unparseable,
    /// The header value is not UTF-8.
    NotUtf8,
    /// The executor's cookie storing is off.
    StoringOff,
    /// The `Domain` attribute is a public suffix, such as `co.uk`, that no site owns.
    PublicSuffix,
    /// Any other refusal by the jar's storage rules.
    Other,
}

impl fmt::Display for CookieRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CookieRejection::InsecureSecure => "Secure cookie over http",
            CookieRejection::DomainMismatch => "Domain does not match host",
            CookieRejection::Expired => "Already expired",
            CookieRejection::Unparseable => "Could not be parsed",
            CookieRejection::NotUtf8 => "Not valid UTF-8",
            CookieRejection::StoringOff => "Cookie storing is off",
            CookieRejection::PublicSuffix => "Domain is a public suffix",
            CookieRejection::Other => "Rejected by the cookie jar",
        })
    }
}

impl SetCookie {
    // A row for a value that never became a cookie: the text split at its first `=`.
    fn unparsed(text: &str, why: CookieRejection) -> SetCookie {
        let (name, value) = text.split_once('=').unwrap_or((text, ""));
        SetCookie {
            name: name.trim().to_owned(),
            value: value.to_owned(),
            domain: None,
            path: None,
            secure: false,
            http_only: false,
            outcome: CookieOutcome::Rejected(why),
        }
    }

    // A row for a parsed cookie, before the jar's verdict replaces `outcome`.
    fn parsed(raw: &RawCookie<'_>, outcome: CookieOutcome) -> SetCookie {
        SetCookie {
            name: raw.name().to_owned(),
            value: raw.value().to_owned(),
            domain: raw.domain().map(str::to_owned),
            path: raw.path().map(str::to_owned),
            secure: raw.secure() == Some(true),
            http_only: raw.http_only() == Some(true),
            outcome,
        }
    }

    // A row for a cookie a script wrote, its attributes as the script gave them.
    pub(crate) fn scripted(c: &ScriptCookie, outcome: CookieOutcome) -> SetCookie {
        SetCookie {
            name: c.key.clone(),
            value: c.value.clone(),
            domain: c.domain.clone(),
            path: c.path.clone(),
            secure: c.secure,
            http_only: c.http_only,
            outcome,
        }
    }

    // A row for a stored cookie a script's delete or clear removed; the value is left out.
    pub(crate) fn removed(c: &CookieInfo) -> SetCookie {
        SetCookie {
            name: c.name.clone(),
            value: String::new(),
            domain: Some(c.domain.clone()),
            path: Some(c.path.clone()),
            secure: c.secure,
            http_only: c.http_only,
            outcome: CookieOutcome::Removed,
        }
    }

    // Reads one header value: the parsed cookie, or the finished row of why it is none.
    fn read(bytes: &[u8]) -> Result<RawCookie<'_>, SetCookie> {
        let text = std::str::from_utf8(bytes).map_err(|_| {
            SetCookie::unparsed(&String::from_utf8_lossy(bytes), CookieRejection::NotUtf8)
        })?;
        RawCookie::parse(text).map_err(|_| SetCookie::unparsed(text, CookieRejection::Unparseable))
    }
}

/// The rows for `Set-Cookie` values received while storing is off: each one `StoringOff`.
pub(crate) fn not_stored<I>(set_cookies: I) -> Vec<SetCookie>
where
    I: IntoIterator,
    I::Item: AsRef<[u8]>,
{
    let off = CookieOutcome::Rejected(CookieRejection::StoringOff);
    (set_cookies.into_iter())
        .map(|v| match SetCookie::read(v.as_ref()) {
            Ok(raw) => SetCookie::parsed(&raw, off),
            Err(row) => SetCookie {
                outcome: off,
                ..row
            },
        })
        .collect()
}

// Whether `url` may set a Secure cookie: https, or a loopback host, as the jar's send rule has it.
fn secure_origin(url: &Url) -> bool {
    match url.host() {
        _ if url.scheme() == "https" => true,
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

// The jar's verdict on one parsed cookie from `url`, storing it when allowed.
fn judge(store: &mut CookieStore, raw: &RawCookie<'_>, url: &Url) -> CookieOutcome {
    use cookie_store::{CookieError as Refused, StoreAction};
    if raw.secure() == Some(true) && !secure_origin(url) {
        return CookieOutcome::Rejected(CookieRejection::InsecureSecure);
    }
    let mut raw = raw.clone();
    match domain_rule(raw.domain(), url) {
        DomainRule::Reject => return CookieOutcome::Rejected(CookieRejection::PublicSuffix),
        DomainRule::HostOnly => raw.unset_domain(),
        DomainRule::Keep => {}
    }
    match store.insert_raw(&raw, url) {
        Ok(StoreAction::ExpiredExisting) => CookieOutcome::Removed,
        Ok(_) => CookieOutcome::Stored,
        Err(e) => CookieOutcome::Rejected(match e {
            Refused::DomainMismatch => CookieRejection::DomainMismatch,
            Refused::Expired => CookieRejection::Expired,
            Refused::Parse => CookieRejection::Unparseable,
            Refused::PublicSuffix => CookieRejection::PublicSuffix,
            _ => CookieRejection::Other,
        }),
    }
}

/// Where an opened jar is saved, and what its one load found.
struct Backing {
    path: PathBuf,
    keys: Arc<dyn SecretStore>,
    /// Set once the load finishes: the file key, or why the jar stays in memory.
    loaded: OnceLock<Result<[u8; 32], String>>,
    /// The revision last written; its lock also serialises saves.
    saved: Mutex<u64>,
}

impl Backing {
    /// The file key, present only once the load succeeded.
    fn key(&self) -> Option<[u8; 32]> {
        self.loaded.get().and_then(|r| r.as_ref().ok()).copied()
    }
}

/// One cookie as the sealed file records it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedCookie {
    domain: String,
    path: String,
    name: String,
    value: String,
    /// Unix seconds; `None` for a session cookie.
    expires: Option<u64>,
    secure: bool,
    http_only: bool,
    host_only: bool,
    /// "Strict", "Lax" or "None"; absent in older files, and an unknown word loads as unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    same_site: Option<String>,
}

impl From<&cookie_store::Cookie<'_>> for SavedCookie {
    fn from(c: &cookie_store::Cookie<'_>) -> SavedCookie {
        let c = info(c);
        SavedCookie {
            expires: c.expires.map(|at| {
                at.duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())
            }),
            domain: c.domain,
            path: c.path,
            name: c.name,
            value: c.value,
            secure: c.secure,
            http_only: c.http_only,
            host_only: c.host_only,
            same_site: c.same_site.map(|s| s.as_str().to_owned()),
        }
    }
}

impl From<SavedCookie> for CookieInfo {
    fn from(c: SavedCookie) -> CookieInfo {
        CookieInfo {
            domain: c.domain,
            path: c.path,
            name: c.name,
            value: c.value,
            expires: c
                .expires
                .map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs)),
            secure: c.secure,
            http_only: c.http_only,
            host_only: c.host_only,
            same_site: c.same_site.as_deref().and_then(SameSite::from_word),
        }
    }
}

/// Why the jar refused a cookie.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CookieError {
    /// A field fails validation; `field` is "domain", "path", "name", "value" or "expires".
    #[error("invalid cookie {field}: {reason}")]
    Invalid {
        /// The refused field.
        field: &'static str,
        /// What the field must satisfy.
        reason: String,
    },
}

/// One stored cookie as the jar holds it.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CookieInfo {
    /// The domain the cookie is stored under, without a leading dot.
    pub domain: String,
    /// The path the cookie is scoped to.
    pub path: String,
    /// The cookie's name.
    pub name: String,
    /// The cookie's value.
    pub value: String,
    /// When a persistent cookie expires; `None` for a session cookie.
    pub expires: Option<SystemTime>,
    /// Whether the cookie is sent over secure connections only.
    pub secure: bool,
    /// Whether the cookie carried `HttpOnly`.
    pub http_only: bool,
    /// Whether the cookie is sent to its exact host only, having set no `Domain`.
    pub host_only: bool,
    /// The cookie's `SameSite` attribute; `None` when it set none.
    pub same_site: Option<SameSite>,
}

/// A cookie's `SameSite` attribute: whether it goes along on requests started by other sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SameSite {
    /// Sent only on requests the cookie's own site starts.
    Strict,
    /// Also sent when the user follows a link from another site.
    Lax,
    /// Sent on every request, whichever site starts it.
    None,
}

impl SameSite {
    /// The attribute's value as a `Set-Cookie` header spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            SameSite::Strict => "Strict",
            SameSite::Lax => "Lax",
            SameSite::None => "None",
        }
    }

    fn from_word(word: &str) -> Option<SameSite> {
        [SameSite::Strict, SameSite::Lax, SameSite::None]
            .into_iter()
            .find(|s| s.as_str().eq_ignore_ascii_case(word))
    }
}

// The store's own type stays private, so the public enum names no dependency type.
fn same_site_of(s: cookie::SameSite) -> SameSite {
    match s {
        cookie::SameSite::Strict => SameSite::Strict,
        cookie::SameSite::Lax => SameSite::Lax,
        cookie::SameSite::None => SameSite::None,
    }
}

fn raw_same_site(s: SameSite) -> cookie::SameSite {
    match s {
        SameSite::Strict => cookie::SameSite::Strict,
        SameSite::Lax => cookie::SameSite::Lax,
        SameSite::None => cookie::SameSite::None,
    }
}

impl CookieInfo {
    /// A host-only session cookie at path "/", not secure, not HttpOnly.
    pub fn new(
        domain: impl Into<String>,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> CookieInfo {
        CookieInfo {
            domain: domain.into(),
            path: "/".to_owned(),
            name: name.into(),
            value: value.into(),
            expires: None,
            secure: false,
            http_only: false,
            host_only: true,
            same_site: None,
        }
    }
}

/// Leaves the value out, since it is often a session secret.
impl fmt::Debug for CookieInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CookieInfo")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("name", &self.name)
            .field("expires", &self.expires)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("host_only", &self.host_only)
            .field("same_site", &self.same_site)
            .finish_non_exhaustive()
    }
}

/// Shows only how many cookies are stored.
impl fmt::Debug for CookieJar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.read().iter_unexpired().count();
        f.debug_struct("CookieJar")
            .field("cookies", &count)
            .finish()
    }
}

/// One stored cookie as `CookieInfo` shows it.
fn info(c: &cookie_store::Cookie<'_>) -> CookieInfo {
    CookieInfo {
        domain: c
            .domain
            .as_cow()
            .map(|d| d.into_owned())
            .unwrap_or_default(),
        path: c.path.as_ref().to_owned(),
        name: c.name().to_owned(),
        value: c.value().to_owned(),
        expires: match c.expires {
            CookieExpiration::AtUtc(at) => Some(SystemTime::from(at)),
            CookieExpiration::SessionEnd => None,
        },
        secure: c.secure().unwrap_or(false),
        http_only: c.http_only().unwrap_or(false),
        host_only: matches!(c.domain, CookieDomain::HostOnly(_)),
        same_site: c.same_site().map(same_site_of),
    }
}

// One cookie into `store`, as if `https://<host>/` had set it: a Domain attribute only for a
// domain-wide cookie, so the store records host-only cookies against the exact host.
fn put(store: &mut CookieStore, c: &CookieInfo) -> Result<(), cookie_store::CookieError> {
    let host = c.domain.trim_start_matches('.');
    // A bare IPv6 address (`::1`) needs its brackets to parse as a URL host.
    let url_host = match host.parse::<std::net::Ipv6Addr>() {
        Ok(_) => format!("[{host}]"),
        Err(_) => host.to_owned(),
    };
    let url = Url::parse(&format!("https://{url_host}/"))
        .map_err(|_| cookie_store::CookieError::Parse)?;
    let mut raw = RawCookie::new(c.name.clone(), c.value.clone());
    raw.set_path(c.path.clone());
    if !c.host_only || c.domain.starts_with('.') {
        raw.set_domain(host.to_owned());
    }
    raw.set_secure(c.secure);
    raw.set_http_only(c.http_only);
    raw.set_same_site(c.same_site.map(raw_same_site));
    if let Some(at) = c.expires {
        raw.set_expires(time::OffsetDateTime::from(at));
    }
    store.insert_raw(&raw, &url).map(drop)
}

const NAME_RULE: &str = "must be non-empty, without spaces, control characters, ; or =";
const VALUE_RULE: &str = "must not contain control characters or ;";

fn bad_name(name: &str) -> bool {
    name.is_empty()
        || name.contains(|ch: char| ch.is_control() || ch.is_whitespace() || ch == ';' || ch == '=')
}

fn bad_value(value: &str) -> bool {
    value.contains(|ch: char| ch.is_control() || ch == ';')
}

/// A cookie a script asks to store for a URL, before the jar's rules apply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScriptCookie {
    pub(crate) key: String,
    #[serde(default)]
    pub(crate) value: String,
    pub(crate) domain: Option<String>,
    pub(crate) path: Option<String>,
    /// Unix milliseconds.
    pub(crate) expires: Option<f64>,
    /// Seconds from now; wins over `expires`.
    pub(crate) max_age: Option<f64>,
    #[serde(default)]
    pub(crate) secure: bool,
    #[serde(default)]
    pub(crate) http_only: bool,
    #[serde(default)]
    pub(crate) host_only: bool,
}

/// 9999-12-31T23:59:59Z in Unix milliseconds, the latest expiry a script cookie can carry.
const MAX_MS: f64 = 253_402_300_799_000.0;

/// RFC 6265's default-path: the URL path up to, not including, its last `/`, or `/`.
fn default_path(url: &Url) -> String {
    let path = url.path();
    match path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(at) => path[..at].to_owned(),
    }
}

// The raw cookie `url` would have sent for a script cookie, refused with the jar's verdict and
// a reason that never quotes the value. These are the checks the store leaves out: the exact
// scheme, the name prefixes and a single-label domain, refused here with a script's own reason.
fn script_raw(
    url: &Url,
    c: &ScriptCookie,
) -> Result<RawCookie<'static>, (CookieRejection, String)> {
    let other = |reason: String| (CookieRejection::Other, reason);
    if !matches!(url.scheme(), "http" | "https") {
        return Err(other("the URL must be http or https".to_owned()));
    }
    let (host, ip) = match url.host() {
        Some(url::Host::Domain(d)) => (d.to_ascii_lowercase(), false),
        Some(url::Host::Ipv4(a)) => (a.to_string(), true),
        Some(url::Host::Ipv6(a)) => (a.to_string(), true),
        None => return Err(other("the URL has no host".to_owned())),
    };
    if bad_name(&c.key) {
        return Err(other(format!("its name {NAME_RULE}")));
    }
    if bad_value(&c.value) {
        return Err(other(format!("its value {VALUE_RULE}")));
    }
    let domain = c
        .domain
        .as_deref()
        .map(|d| {
            d.trim_matches('.')
                .trim_matches(['[', ']'])
                .to_ascii_lowercase()
        })
        .filter(|d| !d.is_empty());
    let path = c
        .path
        .clone()
        .filter(|p| p.starts_with('/'))
        .unwrap_or_else(|| default_path(url));
    if c.key.starts_with("__Secure-") && !c.secure {
        return Err(other("a __Secure- cookie must be secure".to_owned()));
    }
    if c.key.starts_with("__Host-")
        && (!c.secure || path != "/" || (domain.is_some() && !c.host_only))
    {
        return Err(other(
            "a __Host- cookie must be secure, set no domain and use path /".to_owned(),
        ));
    }
    // A domain cookie by default, except for an IP host or a `__Host-` name, which stay on
    // their exact host; an explicit domain must then be that host.
    let host_only = ip || c.host_only || c.key.starts_with("__Host-");
    // An explicit domain is compared without its trailing dot, so `com.` is still one label.
    let bare_host = host.trim_end_matches('.');
    if let Some(d) = &domain {
        if host_only && d != bare_host {
            return Err((
                CookieRejection::DomainMismatch,
                format!("its domain does not match {host}"),
            ));
        }
        // `localhost` is exempt, as a response's `Domain=localhost` is (`is_dotless_suffix`).
        if !d.contains('.') && d != bare_host && d != "localhost" {
            return Err(other("its domain is a single label".to_owned()));
        }
    }
    let mut raw = RawCookie::new(c.key.clone(), c.value.clone());
    raw.set_path(path);
    if !host_only {
        raw.set_domain(domain.unwrap_or(host));
    }
    raw.set_secure(c.secure);
    raw.set_http_only(c.http_only);
    // Script numbers are clamped into what the date types hold: a negative or zero max-age, or a
    // date before 1970, expires the cookie; anything past the year 9999 stops there.
    if let Some(secs) = c.max_age.filter(|s| !s.is_nan()) {
        raw.set_max_age(time::Duration::seconds(
            secs.clamp(0.0, MAX_MS / 1000.0) as i64
        ));
    } else if let Some(ms) = c.expires.filter(|ms| !ms.is_nan()) {
        let nanos = ms.clamp(0.0, MAX_MS) as i128 * 1_000_000;
        if let Ok(at) = time::OffsetDateTime::from_unix_timestamp_nanos(nanos) {
            raw.set_expires(at);
        }
    }
    Ok(raw)
}

/// Every problem in the fields a person can type, in the order the dialog shows them.
fn problems(c: &CookieInfo) -> Vec<CookieError> {
    let mut out = Vec::new();
    let mut invalid = |field, reason: &str| {
        out.push(CookieError::Invalid {
            field,
            reason: reason.to_owned(),
        });
    };
    // A host is valid when a URL parser keeps it whole: no port, path or forbidden character.
    let host = c.domain.strip_prefix('.').unwrap_or(&c.domain);
    let parsed = Url::parse(&format!("https://{host}/")).ok();
    if host.is_empty()
        || parsed.as_ref().and_then(Url::host_str) != Some(&host.to_ascii_lowercase())
    {
        invalid("domain", "must be a host name or IP address");
    } else if !c.host_only || c.domain.starts_with('.') {
        // The jar would quietly narrow or drop such a cookie; a person is told instead.
        if matches!(
            parsed.as_ref().and_then(Url::host),
            Some(url::Host::Ipv4(_) | url::Host::Ipv6(_))
        ) {
            invalid("domain", "must be host-only for an IP address");
        } else if is_public_suffix(host) || is_dotless_suffix(host) {
            invalid("domain", "is a public suffix");
        }
    }
    if !c.path.starts_with('/') {
        invalid("path", "must start with /");
    }
    if bad_name(&c.name) {
        invalid("name", NAME_RULE);
    }
    if bad_value(&c.value) {
        invalid("value", VALUE_RULE);
    }
    if c.expires.is_some_and(|at| at <= SystemTime::now()) {
        invalid("expires", "must be in the future");
    }
    out
}

fn validate(c: &CookieInfo) -> Result<(), CookieError> {
    problems(c).into_iter().next().map_or(Ok(()), Err)
}

fn sorted(mut list: Vec<CookieInfo>) -> Vec<CookieInfo> {
    list.sort_by(|a, b| (&a.domain, &a.path, &a.name).cmp(&(&b.domain, &b.path, &b.name)));
    list
}

impl CookieJar {
    /// A jar backed by the sealed file at `path`, its key in `keys`. Nothing is read until `load`.
    pub fn open(path: impl Into<PathBuf>, keys: Arc<dyn SecretStore>) -> CookieJar {
        CookieJar {
            backing: Some(Backing {
                path: path.into(),
                keys,
                loaded: OnceLock::new(),
                saved: Mutex::new(0),
            }),
            ..CookieJar::default()
        }
    }

    /// Blocking: reads the key and the file once and merges the saved cookies in; a cookie
    /// already in memory with the same domain, path and name wins. A call made while another
    /// runs blocks until it finishes; later calls do nothing. Returns how many saved cookies
    /// were dropped for a public-suffix domain, counted only by the call that read the file.
    /// That call also rewrites the file without them; a failed write leaves the jar marked
    /// changed, so a later `save` retries it.
    pub fn load(&self) -> usize {
        let Some(backing) = &self.backing else {
            return 0;
        };
        let mut dropped = 0;
        backing.loaded.get_or_init(|| {
            let key = sealed::file_key(&*backing.keys, KEY_NAME, LABEL)?;
            let saved: Vec<SavedCookie> = sealed::read(&backing.path, &key).unwrap_or_default();
            dropped = self.merge(saved);
            Ok(key)
        });
        // Saving needs the key the init just stored, so it runs once the init has returned.
        if dropped > 0 {
            {
                let _guard = self.write();
                self.bump();
            }
            let _ = self.save();
        }
        dropped
    }

    /// Blocking: writes the file when the jar changed since the last save; a no-op otherwise,
    /// and always a no-op unless `persistence()` is `Encrypted`.
    pub fn save(&self) -> Result<(), String> {
        let Some(backing) = &self.backing else {
            return Ok(());
        };
        // One save at a time: the snapshot, the write and the saved mark move together.
        let mut saved = backing.saved.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(key) = backing.key() else {
            return Ok(());
        };
        let (revision, cookies) = {
            let store = self.read();
            let revision = self.revision.load(Ordering::SeqCst);
            let cookies: Vec<SavedCookie> = store.iter_unexpired().map(SavedCookie::from).collect();
            (revision, cookies)
        };
        if revision == *saved {
            return Ok(());
        }
        sealed::write(&backing.path, &key, &cookies, LABEL)?;
        *saved = revision;
        Ok(())
    }

    /// Blocking: merges the cookies of the plain JSON file at `path` into the jar, dropping
    /// expired ones and any the store refuses. A missing file is an empty jar. An unreadable
    /// or malformed file is an error that never quotes the file's content. Returns how many
    /// cookies were dropped for a public-suffix domain.
    pub fn read_file(&self, path: &std::path::Path) -> Result<usize, String> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(format!("cannot read cookie jar {}: {e}", path.display())),
        };
        // serde's message can quote input, so only its position is reported.
        let saved: Vec<SavedCookie> = serde_json::from_slice(&bytes).map_err(|e| {
            format!(
                "cookie jar {} is not valid (line {}, column {})",
                path.display(),
                e.line(),
                e.column()
            )
        })?;
        Ok(self.merge(saved))
    }

    /// Blocking: writes the unexpired cookies to `path` as plain JSON, atomically through a
    /// sibling temporary file, creating parent directories. The file is mode 0600 on Unix. A symlink at `path` is replaced
    /// by a regular file, not followed. Concurrent writers to one path are last-writer-wins.
    pub fn write_file(&self, path: &std::path::Path) -> Result<(), String> {
        use std::io::Write;
        let failed =
            |e: &dyn fmt::Display| format!("cannot write cookie jar {}: {e}", path.display());
        let cookies: Vec<SavedCookie> = self
            .read()
            .iter_unexpired()
            .map(SavedCookie::from)
            .collect();
        let json = serde_json::to_vec_pretty(&cookies).map_err(|e| failed(&e))?;
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| failed(&e))?;
        let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| failed(&e))?;
        // The temp file starts 0600 already; setting it keeps that true whatever the library does,
        // and the rename replaces an older, wider file with this one.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| failed(&e))?;
        }
        file.write_all(&json).map_err(|e| failed(&e))?;
        file.persist(path).map_err(|e| failed(&e.error))?;
        Ok(())
    }

    /// Merges saved cookies in: expired ones and ones already in memory are skipped, and one
    /// the store will not take is dropped, as a bad `Set-Cookie` would be. A domain-wide cookie
    /// for a public suffix is dropped too, and a domain-wide one for an IP loads host-only.
    /// Returns how many were dropped for a public suffix, the only loss worth telling the user.
    fn merge(&self, saved: Vec<SavedCookie>) -> usize {
        let now = SystemTime::now();
        let mut dropped = 0;
        // Merging restores what was saved, so it is not a change and leaves the revision.
        let mut store = self.write();
        for mut cookie in saved.into_iter().map(CookieInfo::from) {
            let domain = cookie.domain.trim_start_matches('.').to_ascii_lowercase();
            let wide = !cookie.host_only || cookie.domain.starts_with('.');
            if cookie.expires.is_some_and(|at| at <= now)
                || store.contains(&domain, &cookie.path, &cookie.name)
            {
                continue;
            }
            if wide && (is_public_suffix(&domain) || is_dotless_suffix(&domain)) {
                dropped += 1;
                continue;
            }
            // An IP only ever matched itself, so host-only keeps what the cookie meant.
            if wide
                && domain
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok()
            {
                cookie.domain = domain;
                cookie.host_only = true;
            }
            let _ = put(&mut store, &cookie);
        }
        dropped
    }

    /// Where the jar keeps its cookies: `MemoryOnly` for `CookieJar::default()`.
    pub fn persistence(&self) -> Persistence {
        let Some(backing) = &self.backing else {
            return Persistence::MemoryOnly {
                reason: "cookies are not saved".into(),
            };
        };
        match backing.loaded.get() {
            None => Persistence::NotLoaded,
            Some(Ok(_)) => Persistence::Encrypted,
            Some(Err(reason)) => Persistence::MemoryOnly {
                reason: reason.clone(),
            },
        }
    }

    // A panic elsewhere while holding the lock leaves the store itself consistent, so a
    // poisoned lock is used as is rather than failing every later send.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, CookieStore> {
        self.store.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, CookieStore> {
        self.store.write().unwrap_or_else(PoisonError::into_inner)
    }

    // Called with the write guard held, so a reader of the store sees a matching revision.
    fn bump(&self) {
        self.revision.fetch_add(1, Ordering::SeqCst);
    }

    /// The `Cookie` header value for `url`, built as reqwest's own jar builds it, or `None`
    /// when no stored cookie matches.
    pub(crate) fn header_for(&self, url: &Url) -> Option<String> {
        let header = self
            .read()
            .get_request_values(url)
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        (!header.is_empty()).then_some(header)
    }

    /// Stores each `Set-Cookie` value received from `url` that the jar's rules allow, and
    /// returns one row per value, in order, saying what became of it.
    pub(crate) fn store<I>(&self, set_cookies: I, url: &Url) -> Vec<SetCookie>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        let mut store = self.write();
        let rows: Vec<SetCookie> = (set_cookies.into_iter())
            .map(|v| match SetCookie::read(v.as_ref()) {
                Ok(raw) => SetCookie::parsed(&raw, judge(&mut store, &raw, url)),
                Err(row) => row,
            })
            .collect();
        // Only a stored or removed cookie is a change worth saving.
        let changed = rows
            .iter()
            .any(|c| matches!(c.outcome, CookieOutcome::Stored | CookieOutcome::Removed));
        if changed {
            self.bump();
        }
        rows
    }

    /// Stores a script's cookie as if a response from `url` had set it, so the store's
    /// domain-match applies on top of the script checks. An expired cookie removes its match.
    /// Returns the jar's verdict and, for a refusal, the reason for a warning, which never
    /// holds the value.
    pub(crate) fn store_for(
        &self,
        url: &Url,
        cookie: &ScriptCookie,
    ) -> (CookieOutcome, Option<String>) {
        use cookie_store::{CookieError as Refused, StoreAction};
        let mut raw = match script_raw(url, cookie) {
            Ok(raw) => raw,
            Err((why, reason)) => return (CookieOutcome::Rejected(why), Some(reason)),
        };
        let refused = |why, reason: String| (CookieOutcome::Rejected(why), Some(reason));
        let suffix = "its domain is a public suffix";
        match domain_rule(raw.domain(), url) {
            // A backstop: `script_raw` already refuses a single label other than the host.
            DomainRule::Reject => return refused(CookieRejection::PublicSuffix, suffix.to_owned()),
            DomainRule::HostOnly => raw.unset_domain(),
            DomainRule::Keep => {}
        }
        let mut store = self.write();
        match store.insert_raw(&raw, url) {
            Ok(action) => {
                self.bump();
                match action {
                    StoreAction::ExpiredExisting => (CookieOutcome::Removed, None),
                    _ => (CookieOutcome::Stored, None),
                }
            }
            // An expired cookie with nothing to delete is dropped without a warning, recorded as
            // already expired.
            Err(Refused::Expired) => (CookieOutcome::Rejected(CookieRejection::Expired), None),
            Err(Refused::PublicSuffix) => refused(CookieRejection::PublicSuffix, suffix.to_owned()),
            Err(Refused::DomainMismatch) => refused(
                CookieRejection::DomainMismatch,
                format!(
                    "its domain does not match {}",
                    url.host_str().unwrap_or_default()
                ),
            ),
            Err(Refused::Parse) => {
                refused(CookieRejection::Unparseable, Refused::Parse.to_string())
            }
            Err(e) => refused(CookieRejection::Other, e.to_string()),
        }
    }

    /// Removes one cookie named `name` that a request to `url` would send: the one whose path
    /// is the URL's path, else the one with the longest path. Returns the cookie removed.
    pub(crate) fn delete_for(&self, url: &Url, name: &str) -> Option<CookieInfo> {
        let mut store = self.write();
        let hits: Vec<CookieInfo> = sorted(store.matches(url).into_iter().map(info).collect())
            .into_iter()
            .filter(|c| c.name == name)
            .collect();
        // On a tie the first in `list` order goes, as `getCookie` reads that one.
        let longest = hits.iter().map(|c| c.path.len()).max().unwrap_or(0);
        let doomed = hits
            .iter()
            .find(|c| c.path == url.path())
            .or_else(|| hits.iter().find(|c| c.path.len() == longest));
        let gone = doomed.filter(|c| store.remove(&c.domain, &c.path, name).is_some());
        if gone.is_some() {
            self.bump();
        }
        gone.cloned()
    }

    /// Removes every cookie a request to `url` would send, returning them in `list` order.
    pub(crate) fn delete_all_for(&self, url: &Url) -> Vec<CookieInfo> {
        let mut store = self.write();
        let doomed = sorted(store.matches(url).into_iter().map(info).collect());
        let removed: Vec<CookieInfo> = (doomed.into_iter())
            .filter(|c| store.remove(&c.domain, &c.path, &c.name).is_some())
            .collect();
        if !removed.is_empty() {
            self.bump();
        }
        removed
    }

    /// Every field of `cookie` that `insert` would refuse, in the dialog's field order; empty
    /// when the fields are valid. Stores nothing.
    pub fn check(cookie: &CookieInfo) -> Vec<CookieError> {
        problems(cookie)
    }

    /// Adds `cookie`, replacing any cookie with the same domain, path and name.
    pub fn insert(&self, cookie: CookieInfo) -> Result<(), CookieError> {
        validate(&cookie)?;
        let mut store = self.write();
        put(&mut store, &cookie).map_err(|e| CookieError::Invalid {
            field: "domain",
            reason: e.to_string(),
        })?;
        self.bump();
        Ok(())
    }

    /// Every unexpired cookie, sorted by domain, path and name.
    pub fn list(&self) -> Vec<CookieInfo> {
        sorted(self.read().iter_unexpired().map(info).collect())
    }

    /// The unexpired cookies a request to `url` would send, in `list` order.
    pub(crate) fn matching(&self, url: &Url) -> Vec<CookieInfo> {
        sorted(self.read().matches(url).into_iter().map(info).collect())
    }

    /// Removes the cookie stored under this domain (in any case), exact path and exact name;
    /// true if one was.
    pub fn delete(&self, domain: &str, path: &str, name: &str) -> bool {
        // The store keys domains lower-cased and has no case-insensitive lookup.
        let domain = domain.to_ascii_lowercase();
        let mut store = self.write();
        let removed = store.remove(&domain, path, name).is_some();
        self.bump();
        removed
    }

    /// Removes every unexpired cookie whose stored domain is `domain` in any case, returning
    /// how many; these are the ones `list` showed.
    pub fn delete_domain(&self, domain: &str) -> usize {
        // Stored domains are lower case, so fold once for the filter and for `remove`.
        let domain = domain.to_ascii_lowercase();
        let mut store = self.write();
        let doomed: Vec<(String, String)> = store
            .iter_unexpired()
            .filter(|c| c.domain.as_cow().is_some_and(|d| d == domain.as_str()))
            .map(|c| (c.path.as_ref().to_owned(), c.name().to_owned()))
            .collect();
        let removed = doomed
            .iter()
            .filter(|(path, name)| store.remove(&domain, path, name).is_some())
            .count();
        self.bump();
        removed
    }

    /// Removes every cookie.
    pub fn clear(&self) {
        self.clear_all();
    }

    /// Removes every cookie, returning the unexpired ones in `list` order.
    pub(crate) fn clear_all(&self) -> Vec<CookieInfo> {
        let mut store = self.write();
        let removed = sorted(store.iter_unexpired().map(info).collect());
        store.clear();
        self.bump();
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A jar holding `sid` and `theme` for `example.com`, set by a host and a `Domain`
    /// attribute spelled in mixed case.
    fn jar() -> CookieJar {
        let jar = CookieJar::default();
        let url = Url::parse("http://WWW.Example.COM/").expect("url");
        jar.store(
            [
                "sid=1; Domain=Example.COM; Path=/",
                "theme=dark; Domain=example.com; Path=/",
            ],
            &url,
        );
        jar
    }

    #[test]
    fn the_jar_stores_domains_lower_cased() {
        let domains: Vec<String> = jar().list().into_iter().map(|c| c.domain).collect();
        assert_eq!(domains, ["example.com", "example.com"]);
    }

    #[test]
    fn delete_matches_the_domain_whatever_its_case_but_not_the_name() {
        let jar = jar();
        assert!(!jar.delete("example.com", "/", "SID"));
        assert!(jar.delete("EXAMPLE.Com", "/", "sid"));
        assert!(!jar.delete("EXAMPLE.Com", "/", "sid"));
        let left: Vec<String> = jar.list().into_iter().map(|c| c.name).collect();
        assert_eq!(left, ["theme"]);
    }

    #[test]
    fn delete_domain_matches_the_domain_whatever_its_case() {
        let jar = jar();
        assert_eq!(jar.delete_domain("Example.COM"), 2);
        assert!(jar.list().is_empty());
        assert_eq!(jar.delete_domain("example.com"), 0);
    }

    #[test]
    fn every_mutation_bumps_the_revision_and_reads_do_not() {
        use std::sync::atomic::Ordering;
        let jar = CookieJar::default();
        let rev = |jar: &CookieJar| jar.revision.load(Ordering::SeqCst);
        let url = Url::parse("http://a.test/").unwrap();
        let r0 = rev(&jar);
        jar.store(["x=1"], &url);
        let r1 = rev(&jar);
        assert!(r1 > r0);
        jar.store(std::iter::empty::<&str>(), &url);
        let _ = (jar.list(), jar.header_for(&url), jar.matching(&url));
        assert_eq!(rev(&jar), r1, "no parsed cookie and reads change nothing");
        jar.insert(CookieInfo::new("a.test", "y", "2")).unwrap();
        let r2 = rev(&jar);
        assert!(r2 > r1);
        assert!(jar.delete("a.test", "/", "y"));
        let r3 = rev(&jar);
        assert!(r3 > r2);
        assert_eq!(jar.delete_domain("a.test"), 1);
        let r4 = rev(&jar);
        assert!(r4 > r3);
        jar.clear();
        assert!(rev(&jar) > r4);
    }

    // The outcomes of one store, as (name, value, outcome).
    fn outcomes(
        jar: &CookieJar,
        values: &[&[u8]],
        url: &str,
    ) -> Vec<(String, String, CookieOutcome)> {
        let url = Url::parse(url).unwrap();
        jar.store(values.iter().copied(), &url)
            .into_iter()
            .map(|c| (c.name, c.value, c.outcome))
            .collect()
    }

    #[test]
    fn store_reports_one_outcome_per_value_in_order() {
        use CookieRejection::*;
        let jar = CookieJar::default();
        let rejected = CookieOutcome::Rejected;
        let got = outcomes(
            &jar,
            &[
                b"a=1; Path=/",
                b"d=4; Domain=other.example",
                b"s=5; Secure",
                b"x=1; Max-Age=0",
                b"novalue",
                b" =v",
                b"n=caf\xff",
            ],
            "http://app.test/",
        );
        let row = |n: &str, v: &str, o| (n.to_owned(), v.to_owned(), o);
        assert_eq!(
            got,
            [
                row("a", "1", CookieOutcome::Stored),
                row("d", "4", rejected(DomainMismatch)),
                row("s", "5", rejected(InsecureSecure)),
                row("x", "1", rejected(Expired)),
                row("novalue", "", rejected(Unparseable)),
                row("", "v", rejected(Unparseable)),
                row("n", "caf\u{fffd}", rejected(NotUtf8)),
            ]
        );
        let names: Vec<String> = jar.list().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["a"]);
    }

    #[test]
    fn an_expired_value_removes_its_match_and_https_keeps_secure() {
        let jar = CookieJar::default();
        let https = "https://app.test/";
        assert_eq!(
            outcomes(&jar, &[b"s=5; Secure; Path=/"], https)[0].2,
            CookieOutcome::Stored
        );
        let gone = outcomes(&jar, &[b"s=; Path=/; Max-Age=0"], https);
        assert_eq!(gone[0].2, CookieOutcome::Removed);
        assert!(jar.list().is_empty());
        let local = outcomes(&jar, &[b"t=1; Secure"], "http://localhost:8080/");
        assert_eq!(local[0].2, CookieOutcome::Stored);
    }

    #[test]
    fn reasons_read_as_short_phrases() {
        use CookieRejection::*;
        let texts: Vec<String> = [
            InsecureSecure,
            DomainMismatch,
            Expired,
            Unparseable,
            NotUtf8,
            StoringOff,
            PublicSuffix,
            Other,
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(
            texts,
            [
                "Secure cookie over http",
                "Domain does not match host",
                "Already expired",
                "Could not be parsed",
                "Not valid UTF-8",
                "Cookie storing is off",
                "Domain is a public suffix",
                "Rejected by the cookie jar",
            ]
        );
    }

    #[test]
    fn a_set_cookies_debug_never_shows_its_value() {
        let jar = CookieJar::default();
        let url = Url::parse("http://app.test/").unwrap();
        let got = jar.store([b"sid=SECRETV; HttpOnly".as_slice()], &url);
        let shown = format!("{got:?}");
        assert!(!shown.contains("SECRETV"), "{shown}");
        assert!(shown.contains("sid") && shown.contains("Stored"), "{shown}");
    }

    fn saved(name: &str, value: &str, expires: Option<u64>) -> String {
        format!(
            r#"{{"domain":"a.test","path":"/","name":"{name}","value":"{value}","expires":{},"secure":false,"httpOnly":false,"hostOnly":true}}"#,
            expires.map_or("null".to_owned(), |e| e.to_string())
        )
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn saved_domain_wide_suffix_cookies_are_dropped_on_load() {
        let entry = |domain: &str, host_only: bool| {
            format!(
                r#"{{"domain":"{domain}","path":"/","name":"n","value":"v","expires":null,"secure":false,"httpOnly":false,"hostOnly":{host_only}}}"#
            )
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jar.json");
        let text = [
            entry("co.uk", false),
            entry("internal", false),
            entry("github.io", true),
            entry("127.0.0.1", false),
            entry("::1", false),
            entry("localhost", false),
        ]
        .join(",");
        std::fs::write(&path, format!("[{text}]")).unwrap();
        let jar = CookieJar::default();
        jar.read_file(&path).unwrap();
        let kept: Vec<(String, bool)> = (jar.list().into_iter())
            .map(|c| (c.domain, c.host_only))
            .collect();
        assert_eq!(
            kept,
            [
                ("127.0.0.1".to_owned(), true),
                ("[::1]".to_owned(), true),
                ("github.io".to_owned(), true),
                ("localhost".to_owned(), false),
            ]
        );
    }

    // Two public-suffix cookies (`co.uk`, `com`), one dotless (`internal`), a valid one and an
    // expired one: only the first three count as dropped.
    fn dropping_fixture() -> Vec<SavedCookie> {
        let entry = |domain: &str, name: &str, expires: Option<u64>| {
            serde_json::from_str::<SavedCookie>(&format!(
                r#"{{"domain":"{domain}","path":"/","name":"{name}","value":"VAL{name}","expires":{},"secure":false,"httpOnly":false,"hostOnly":false}}"#,
                expires.map_or("null".to_owned(), |e| e.to_string())
            ))
            .unwrap()
        };
        vec![
            entry("co.uk", "suffixa", None),
            entry("com", "suffixb", None),
            entry("internal", "dotless", None),
            entry("example.com", "kept", None),
            entry("example.com", "stale", Some(1)),
        ]
    }

    #[test]
    fn read_file_counts_the_cookies_dropped_for_a_public_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jar.json");
        std::fs::write(&path, serde_json::to_vec(&dropping_fixture()).unwrap()).unwrap();
        let jar = CookieJar::default();
        assert_eq!(jar.read_file(&path), Ok(3));
        let names: Vec<String> = jar.list().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["kept"]);
    }

    #[test]
    fn a_sealed_load_counts_the_dropped_cookies_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let keys: Arc<dyn SecretStore> = Arc::new(crate::MemoryStore::new());
        let key = sealed::file_key(&*keys, KEY_NAME, LABEL).unwrap();
        sealed::write(&path, &key, &dropping_fixture(), LABEL).unwrap();
        let jar = CookieJar::open(&path, keys);
        assert_eq!(jar.load(), 3);
        assert_eq!(jar.load(), 0);
        assert_eq!(jar.list().len(), 1);
    }

    #[test]
    fn a_sealed_load_that_dropped_cookies_rewrites_the_file_without_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cookies.json");
        let keys: Arc<dyn SecretStore> = Arc::new(crate::MemoryStore::new());
        let key = sealed::file_key(&*keys, KEY_NAME, LABEL).unwrap();
        sealed::write(&path, &key, &dropping_fixture(), LABEL).unwrap();
        assert_eq!(CookieJar::open(&path, Arc::clone(&keys)).load(), 3);
        let on_disk: Vec<SavedCookie> = sealed::read(&path, &key).unwrap();
        let names: Vec<String> = on_disk.into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["kept"]);
        let next = CookieJar::open(&path, keys);
        assert_eq!(next.load(), 0);
        assert_eq!(next.list().len(), 1);
    }

    #[test]
    fn the_dropped_cookies_notice_names_only_the_count() {
        assert_eq!(dropped_cookies_notice(0), None);
        assert_eq!(
            dropped_cookies_notice(1).as_deref(),
            Some(
                "1 saved cookie was dropped: its domain is a public suffix, so it would reach every site under it."
            )
        );
        let three = dropped_cookies_notice(3).unwrap();
        assert_eq!(
            three,
            "3 saved cookies were dropped: their domains are public suffixes, so they would reach every site under them."
        );
        for word in ["suffixa", "VAL", "co.uk", "internal"] {
            assert!(!three.contains(word), "{three}");
        }
    }

    #[test]
    fn a_missing_file_reads_as_an_empty_jar() {
        let dir = tempfile::tempdir().unwrap();
        let jar = CookieJar::default();
        jar.read_file(&dir.path().join("none.json")).unwrap();
        assert!(jar.list().is_empty());
    }

    #[test]
    fn written_cookies_read_back_and_expired_ones_are_dropped_on_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/jar.json");
        let text = format!(
            "[{},{},{}]",
            saved("keep", "1", Some(now_secs() + 3600)),
            saved("old", "2", Some(1)),
            saved("sess", "3", None)
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        let jar = CookieJar::default();
        jar.read_file(&path).unwrap();
        let mut names: Vec<String> = jar.list().into_iter().map(|c| c.name).collect();
        names.sort();
        assert_eq!(names, ["keep", "sess"]);
        let out = dir.path().join("deep/er/out.json");
        jar.write_file(&out).unwrap();
        let again = CookieJar::default();
        again.read_file(&out).unwrap();
        assert_eq!(again.list().len(), 2);
    }

    #[test]
    fn a_malformed_file_is_an_error_that_never_quotes_a_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jar.json");
        std::fs::write(&path, r#"[{"value":"hunter2""#).unwrap();
        let err = CookieJar::default().read_file(&path).unwrap_err();
        assert!(!err.contains("hunter2"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"[{"value":"hunter2""#
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_file_sets_mode_0600_even_over_an_existing_0644_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jar.json");
        std::fs::write(&path, "[]").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        jar().write_file(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_set_cookie_whose_domain_is_a_public_suffix_is_rejected() {
        let jar = CookieJar::default();
        let suffix = CookieOutcome::Rejected(CookieRejection::PublicSuffix);
        let icann = outcomes(&jar, &[b"a=1; Domain=co.uk; Path=/"], "https://x.co.uk/");
        assert_eq!(icann[0].2, suffix);
        let private = outcomes(&jar, &[b"b=2; Domain=github.io"], "https://x.github.io/");
        assert_eq!(private[0].2, suffix);
        assert!(jar.list().is_empty());
    }

    #[test]
    fn a_set_cookie_for_a_registrable_domain_is_still_stored() {
        let jar = CookieJar::default();
        let got = outcomes(
            &jar,
            &[b"a=1; Domain=example.co.uk"],
            "https://x.example.co.uk/",
        );
        assert_eq!(got[0].2, CookieOutcome::Stored);
        assert_eq!(jar.list()[0].domain, "example.co.uk");
    }

    #[test]
    fn loopback_hosts_keep_their_cookies() {
        let jar = CookieJar::default();
        for url in ["http://127.0.0.1/", "http://localhost/"] {
            assert_eq!(outcomes(&jar, &[b"h=1"], url)[0].2, CookieOutcome::Stored);
        }
        let wide = outcomes(&jar, &[b"w=1; Domain=localhost"], "http://localhost/");
        assert_eq!(wide[0].2, CookieOutcome::Stored);
        assert_eq!(jar.list().len(), 3);
    }

    // Stores one header from `url` into a fresh jar and checks it is the only cookie, host-only or not.
    fn assert_stored_as(header: &str, url: &str, host_only: bool) {
        let jar = CookieJar::default();
        let got = outcomes(&jar, &[header.as_bytes()], url);
        assert_eq!(got[0].2, CookieOutcome::Stored, "{header} from {url}");
        let list = jar.list();
        assert_eq!(list.len(), 1, "{header} from {url}");
        assert_eq!(list[0].host_only, host_only, "{header} from {url}");
    }

    #[test]
    fn a_dotless_domain_other_than_the_host_is_a_public_suffix() {
        let suffix = CookieOutcome::Rejected(CookieRejection::PublicSuffix);
        for (header, url) in [
            ("a=1; Domain=internal", "https://api.internal/"),
            ("a=1; Domain=test", "http://myapp.test/"),
            ("a=1; Domain=.local.", "http://myapp.local/"),
        ] {
            let jar = CookieJar::default();
            assert_eq!(
                outcomes(&jar, &[header.as_bytes()], url)[0].2,
                suffix,
                "{header}"
            );
            assert!(jar.list().is_empty(), "{header}");
        }
    }

    #[test]
    fn a_dotless_domain_equal_to_the_host_is_stored_host_only() {
        assert_stored_as("a=1; Domain=api", "http://api:8080/", true);
        assert_stored_as("a=1; Domain=API.", "http://api/", true);
    }

    #[test]
    fn localhost_and_two_label_local_domains_stay_domain_wide() {
        assert_stored_as("a=1; Domain=localhost", "http://app.localhost/", false);
        assert_stored_as("a=1; Domain=myapp.test", "http://myapp.test/", false);
        assert_stored_as("a=1; Domain=.myapp.local", "http://api.myapp.local/", false);
    }

    #[test]
    fn an_ip_domain_equal_to_the_host_is_stored_host_only_and_sent_back() {
        for (header, url) in [
            ("a=1; Domain=127.0.0.1", "http://127.0.0.1:3000/"),
            ("a=1; Domain=10.0.0.5", "http://10.0.0.5/"),
            ("a=1; Domain=::1", "http://[::1]/"),
        ] {
            assert_stored_as(header, url, true);
            let jar = CookieJar::default();
            outcomes(&jar, &[header.as_bytes()], url);
            let url = Url::parse(url).unwrap();
            assert_eq!(jar.header_for(&url).as_deref(), Some("a=1"), "{header}");
        }
    }

    #[test]
    fn an_ip_domain_for_another_ip_is_a_mismatch() {
        let jar = CookieJar::default();
        let got = outcomes(&jar, &[b"a=1; Domain=10.0.0.6"], "http://10.0.0.5/");
        assert_eq!(
            got[0].2,
            CookieOutcome::Rejected(CookieRejection::DomainMismatch)
        );
    }

    #[test]
    fn a_script_cookie_for_a_dotless_host_ends_host_only() {
        let jar = CookieJar::default();
        let url = Url::parse("http://api/").unwrap();
        assert_eq!(
            jar.store_for(&url, &script("api")),
            (CookieOutcome::Stored, None)
        );
        assert!(jar.list()[0].host_only);
    }

    #[test]
    fn a_script_cookie_for_localhost_from_a_localhost_subdomain_is_domain_wide() {
        let jar = CookieJar::default();
        let url = Url::parse("http://api.localhost/").unwrap();
        assert_eq!(
            jar.store_for(&url, &script("LocalHost.")),
            (CookieOutcome::Stored, None)
        );
        assert!(!jar.list()[0].host_only);
        for to in ["http://localhost/", "http://other.localhost/"] {
            let to = Url::parse(to).unwrap();
            assert_eq!(jar.header_for(&to).as_deref(), Some("k=SECRETV"), "{to}");
        }
    }

    #[test]
    fn a_script_cookie_for_another_single_label_is_still_refused() {
        let jar = CookieJar::default();
        let url = Url::parse("http://api.internal/").unwrap();
        let (outcome, err) = jar.store_for(&url, &script("internal"));
        assert_eq!(outcome, CookieOutcome::Rejected(CookieRejection::Other));
        assert_eq!(err.as_deref(), Some("its domain is a single label"));
        assert!(jar.list().is_empty());
    }

    #[test]
    fn a_manual_domain_wide_cookie_for_a_dotless_name_or_an_ip_is_invalid() {
        let wide = |domain: &str| CookieInfo {
            host_only: false,
            ..CookieInfo::new(domain, "a", "1")
        };
        let invalid = |reason: &str| {
            vec![CookieError::Invalid {
                field: "domain",
                reason: reason.to_owned(),
            }]
        };
        assert_eq!(
            CookieJar::check(&wide("internal")),
            invalid("is a public suffix")
        );
        assert_eq!(
            CookieJar::check(&wide("127.0.0.1")),
            invalid("must be host-only for an IP address")
        );
        assert!(CookieJar::check(&wide("localhost")).is_empty());
        assert!(CookieJar::check(&CookieInfo::new("127.0.0.1", "a", "1")).is_empty());
    }

    fn script(domain: &str) -> ScriptCookie {
        serde_json::from_value(serde_json::json!({
            "key": "k", "value": "SECRETV", "domain": domain
        }))
        .unwrap()
    }

    #[test]
    fn a_script_cookie_for_a_public_suffix_is_refused_without_its_value() {
        let jar = CookieJar::default();
        let url = Url::parse("https://x.co.uk/").unwrap();
        let (outcome, err) = jar.store_for(&url, &script("co.uk"));
        let err = err.expect("a warning");
        assert_eq!(
            outcome,
            CookieOutcome::Rejected(CookieRejection::PublicSuffix)
        );
        assert!(err.contains("public suffix"), "{err}");
        assert!(!err.contains("SECRETV"), "{err}");
        assert!(jar.list().is_empty());
    }

    #[test]
    fn a_script_cookie_for_a_registrable_domain_is_stored() {
        let jar = CookieJar::default();
        let url = Url::parse("https://x.example.co.uk/").unwrap();
        let stored = jar.store_for(&url, &script("example.co.uk"));
        assert_eq!(stored, (CookieOutcome::Stored, None));
        assert_eq!(jar.list()[0].domain, "example.co.uk");
    }

    #[test]
    fn a_manual_domain_cookie_for_a_public_suffix_is_an_invalid_domain() {
        let wide = CookieInfo {
            host_only: false,
            ..CookieInfo::new("co.uk", "a", "1")
        };
        let invalid = CookieError::Invalid {
            field: "domain",
            reason: "is a public suffix".to_owned(),
        };
        assert_eq!(CookieJar::check(&wide), std::slice::from_ref(&invalid));
        assert_eq!(
            CookieJar::check(&CookieInfo::new(".co.uk", "a", "1")),
            std::slice::from_ref(&invalid)
        );
        let jar = CookieJar::default();
        assert_eq!(jar.insert(wide), Err(invalid));
        assert!(jar.list().is_empty());
    }

    #[test]
    fn a_manual_host_only_cookie_under_a_public_suffix_is_valid() {
        let host = CookieInfo::new("example.co.uk", "a", "1");
        assert!(CookieJar::check(&host).is_empty());
        let jar = CookieJar::default();
        jar.insert(host).unwrap();
        assert_eq!(jar.list().len(), 1);
    }

    // (Domain attribute, sending host, whether a suffix rule refuses it): `*.ck` with its
    // `!www.ck` exception, and an IDN suffix in punycode and in Unicode.
    const SUFFIX_RULES: [(&str, &str, bool); 4] = [
        ("foo.ck", "a.foo.ck", true),
        ("www.ck", "a.www.ck", false),
        ("xn--55qx5d.cn", "x.xn--55qx5d.cn", true),
        ("\u{516c}\u{53f8}.cn", "x.\u{516c}\u{53f8}.cn", true),
    ];

    #[test]
    fn wildcard_exception_and_idn_rules_apply_to_set_cookie() {
        for (domain, host, refused) in SUFFIX_RULES {
            let jar = CookieJar::default();
            let header = format!("a=1; Domain={domain}");
            let got = outcomes(&jar, &[header.as_bytes()], &format!("https://{host}/"));
            let want = if refused {
                CookieOutcome::Rejected(CookieRejection::PublicSuffix)
            } else {
                CookieOutcome::Stored
            };
            assert_eq!(got[0].2, want, "{domain}");
        }
    }

    #[test]
    fn wildcard_exception_and_idn_rules_apply_to_script_cookies() {
        for (domain, host, refused) in SUFFIX_RULES {
            let jar = CookieJar::default();
            let url = Url::parse(&format!("https://{host}/")).unwrap();
            let got = jar.store_for(&url, &script(domain));
            if refused {
                let err = got.1.expect("a warning");
                assert!(err.contains("public suffix"), "{domain}: {err}");
                let suffix = CookieOutcome::Rejected(CookieRejection::PublicSuffix);
                assert_eq!(got.0, suffix, "{domain}");
            } else {
                assert_eq!(got, (CookieOutcome::Stored, None), "{domain}");
            }
        }
    }
}
