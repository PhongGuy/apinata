//! What a collection's network settings were when the person last saw them, so a change made
//! outside the app (a pull, an editor, an agent) can be named before the next send.

use super::check::ShownNetwork;
use super::model::{CertificateKind, CollectionNetwork, CollectionProxy, read_network};
use crate::collection::ROOT_FILE;
use crate::context::VarContext;
use crate::document::{Document, read_regular, read_regular_text};
use crate::dynamic::Generator;
use crate::error::FileError;
use crate::interpolate::interpolate_tracked;
use crate::trust::{TrustError, with_lock, write_atomic};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::SystemTime;

/// A secret-free, normalised view of a [`CollectionNetwork`]: enabled rows only, in file order
/// (the first matching certificate wins), with no password, passphrase or user name (the user
/// name is a keyed hash, to spot a change) and, once resolved, a hash of each certificate file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NetworkSnapshot {
    proxy: ProxyView,
    host_overrides: Vec<HostView>,
    certificates: Vec<CertView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
enum ProxyView {
    #[default]
    Inherit,
    Off,
    Custom {
        protocol: String,
        host: String,
        port: String,
        bypass: String,
        auth: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct HostView {
    host: String,
    address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    creds: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct CertView {
    domain: String,
    kind: String,
    files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    creds: Option<String>,
    /// SHA-256 of each file's bytes when the snapshot was resolved; empty when not resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    content: Vec<String>,
}

/// One difference between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetworkChange {
    /// The proxy choice or its address changed; holds the new one as text.
    ProxyChanged(String),
    /// The custom proxy's bypass list changed.
    ProxyBypassChanged,
    /// The custom proxy's credentials changed.
    ProxyCredentialsChanged,
    /// An enabled host override appeared.
    HostOverrideAdded {
        /// The overridden host.
        host: String,
        /// Where it now points.
        address: String,
    },
    /// An enabled host override went away.
    HostOverrideRemoved {
        /// The overridden host.
        host: String,
        /// Where it pointed.
        address: String,
    },
    /// The credentials written in an override's host changed.
    HostOverrideCredentialsChanged {
        /// The overridden host, without the credentials.
        host: String,
    },
    /// An override of the same host points somewhere else.
    HostOverrideChanged {
        /// The overridden host.
        host: String,
        /// Where it pointed.
        from: String,
        /// Where it now points.
        to: String,
    },
    /// An enabled client certificate appeared.
    CertificateAdded {
        /// The host pattern it is offered to.
        domain: String,
    },
    /// An enabled client certificate went away.
    CertificateRemoved {
        /// The host pattern it was offered to.
        domain: String,
    },
    /// The record of what was last seen cannot be read, so nothing can be compared.
    RecordUnreadable,
    /// A certificate for the same domain names other files, or its files hold other bytes.
    CertificateChanged {
        /// The host pattern it is offered to.
        domain: String,
    },
    /// The same host overrides are listed in another order.
    HostOverridesReordered,
    /// The same client certificates are listed in another order, which can change the one
    /// a request is offered.
    CertificatesReordered,
    /// A folder, request or run variable makes this request's settings differ from the
    /// collection's; holds how they differ.
    SetByVariable(Box<NetworkChange>),
    /// This request's settings differ from the collection's in a way no other change names.
    RequestDiffers,
    /// What the send resolved does not match what the check could read, so nothing vouches
    /// for it.
    Unverifiable,
}

impl NetworkChange {
    /// The change as one sentence for a notice or a warning line.
    pub fn describe(&self) -> String {
        match self {
            NetworkChange::ProxyChanged(to) => format!("Proxy changed to {to}"),
            NetworkChange::ProxyBypassChanged => "Proxy bypass list changed".into(),
            NetworkChange::ProxyCredentialsChanged => "Proxy credentials changed".into(),
            NetworkChange::HostOverrideAdded { host, address } => {
                format!("Host override added: {host} -> {address}")
            }
            NetworkChange::HostOverrideRemoved { host, address } => {
                format!("Host override removed: {host} -> {address}")
            }
            NetworkChange::HostOverrideCredentialsChanged { host } => {
                format!("Host override credentials changed: {host}")
            }
            NetworkChange::HostOverrideChanged { host, from, to } => {
                format!("Host override changed: {host} -> {to} (was {from})")
            }
            NetworkChange::CertificateAdded { domain } => {
                format!("Client certificate added for {domain}")
            }
            NetworkChange::CertificateRemoved { domain } => {
                format!("Client certificate removed for {domain}")
            }
            NetworkChange::RecordUnreadable => "Cannot verify network settings: the record of \
                what was last seen is unreadable"
                .into(),
            NetworkChange::CertificateChanged { domain } => {
                format!("Client certificate changed for {domain}")
            }
            NetworkChange::HostOverridesReordered => "Host override order changed".into(),
            NetworkChange::CertificatesReordered => "Client certificate order changed".into(),
            NetworkChange::SetByVariable(inner) => format!(
                "{} (set by a folder, request or run variable)",
                inner.describe()
            ),
            NetworkChange::Unverifiable => {
                "The network settings for this send could not be verified".into()
            }
            NetworkChange::RequestDiffers => {
                "This request's network settings differ from the collection's".into()
            }
        }
    }

    /// `changes` joined into one line.
    pub fn summarize(changes: &[NetworkChange]) -> String {
        let lines: Vec<String> = changes.iter().map(NetworkChange::describe).collect();
        lines.join("; ")
    }
}

impl ProxyView {
    fn address(&self) -> String {
        match self {
            ProxyView::Inherit => "the global setting".into(),
            ProxyView::Off => "off".into(),
            ProxyView::Custom {
                protocol,
                host,
                port,
                auth,
                ..
            } => {
                let port = if port.is_empty() {
                    String::new()
                } else {
                    format!(":{}", shown(port))
                };
                let creds = if auth.is_some() {
                    " with credentials"
                } else {
                    ""
                };
                format!("{}://{}{port}{creds}", shown(protocol), shown(host))
            }
        }
    }
}

/// What a text expanded to: a plain value, a value that came from a secret (stored only as a
/// keyed hash), or a variable nothing defines. A `{{$dynamic}}` token stays as written inside
/// the value (see [`expand_fixed`]), so it reads the same on every send.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Expanded {
    Plain(String),
    Secret(String),
    Unresolved,
}

const HIDDEN: &str = "#h:";
/// The content hash of a certificate file that could not be read, or whose path did not resolve.
const UNREADABLE: &str = "unreadable";
/// Ends the part of a marker that is shown; what follows is only compared.
const SEPARATOR: char = '\u{1f}';

/// The names a template reads, in order: `{{ a }} x {{b}}` gives `a, b`. Nothing else of the
/// template is kept, since the rest of it may hold a credential.
fn variable_names(template: &str) -> String {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else { break };
        names.push(after[..close].trim());
        rest = &after[close + 2..];
    }
    names.join(", ")
}

/// `text` without what precedes its last `@`, and that userinfo if there was one. A host never
/// holds an `@`, so the last one ends the userinfo whatever the password holds (`/`, `?`, `#`,
/// `@`); a scheme before the userinfo stays.
fn split_userinfo(text: &str) -> (String, Option<String>) {
    let Some(at) = text.rfind('@') else {
        return (text.to_owned(), None);
    };
    let head = &text[..at];
    let scheme_end = head.find("://").map_or(0, |i| i + 3);
    (
        format!("{}{}", &text[..scheme_end], &text[at + 1..]),
        Some(head[scheme_end..].to_owned()),
    )
}

/// A field of the snapshot as text and the userinfo lifted out of it: the value without its
/// userinfo, a keyed hash of a secret one, or an unresolved marker that names the variables
/// and carries a keyed hash of the whole template (so a change to its literal part still shows).
fn text_field(salt: &str, expanded: Expanded, template: &str) -> (String, Option<String>) {
    match expanded {
        Expanded::Plain(v) => split_userinfo(v.trim()),
        Expanded::Secret(v) => (format!("{HIDDEN}{}", keyed(salt, &v)), None),
        Expanded::Unresolved => (
            format!(
                "(unresolved: {}){SEPARATOR}{}",
                variable_names(template),
                keyed(salt, template)
            ),
            None,
        ),
    }
}

// Hashes the userinfo of several fields together; none when no field had any.
fn credentials(salt: &str, parts: &[Option<String>]) -> Option<String> {
    parts.iter().any(Option::is_some).then(|| {
        let joined: Vec<&str> = parts.iter().map(|p| p.as_deref().unwrap_or("")).collect();
        keyed(salt, &joined.join("\0"))
    })
}

fn shown(text: &str) -> String {
    let text = text.split(SEPARATOR).next().unwrap_or(text);
    if text.starts_with(HIDDEN) {
        "(hidden)".into()
    } else {
        text.to_owned()
    }
}

// Host names compare case-insensitively; a dynamic token keeps its name as written.
fn lower(text: String) -> String {
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some((start, end)) = dynamic_token(rest) {
        out.push_str(&rest[..start].to_lowercase());
        out.push_str(&rest[start..end]);
        rest = &rest[end..];
    }
    out.push_str(&rest.to_lowercase());
    out
}

/// The byte range of the first token in `text` that interpolation would generate a value for.
fn dynamic_token(text: &str) -> Option<(usize, usize)> {
    crate::dynamic::next_generated(text).map(|(start, _, end)| (start, end))
}

/// `text` expanded by `expand` with each `{{$dynamic}}` token kept as written: the tokens are
/// swapped for markers no variable can name, expanded around, then put back. A field mixing
/// dynamic and other variables so records the others' values and stays stable across sends.
pub(crate) fn expand_fixed<E>(
    text: &str,
    expand: impl FnOnce(&str) -> Result<String, E>,
) -> Result<String, E> {
    let mut tokens = Vec::new();
    let mut fixed = String::new();
    let mut rest = text;
    while let Some((start, end)) = dynamic_token(rest) {
        fixed.push_str(&rest[..start]);
        fixed.push_str(&format!("\u{e000}{}\u{e001}", tokens.len()));
        tokens.push(&rest[start..end]);
        rest = &rest[end..];
    }
    fixed.push_str(rest);
    let mut out = expand(&fixed)?;
    for (i, token) in tokens.iter().enumerate() {
        out = out.replace(&format!("\u{e000}{i}\u{e001}"), token);
    }
    Ok(out)
}

/// Every text of `network` a snapshot reads: the custom proxy's fields and user name, and the
/// enabled host overrides' and certificates' fields. Passwords and passphrases are not read.
pub(crate) fn templates(network: &CollectionNetwork) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    if let CollectionProxy::Custom(p) = &network.proxy {
        out.extend([&p.protocol, &p.hostname, &p.port, &p.bypass].map(String::as_str));
        if let Some(auth) = &p.auth {
            out.push(&auth.username);
        }
    }
    for row in network.host_overrides.iter().filter(|r| r.enabled) {
        out.extend([row.host.as_str(), row.address.as_str()]);
    }
    for cert in network.certificates.iter().filter(|c| c.enabled) {
        out.push(&cert.domain);
        match &cert.kind {
            CertificateKind::Pem {
                certificate,
                private_key,
            } => out.extend([certificate.as_str(), private_key.as_str()]),
            CertificateKind::Pkcs12 { file } => out.push(file),
        }
    }
    out
}

/// Each text of `network` expanded in `ctx`'s network scope (global, collection, environment
/// and process only), labelled secret when a sensitive name was substituted into it.
pub(crate) fn scope_expansions(
    network: &CollectionNetwork,
    ctx: &VarContext,
) -> BTreeMap<String, Expanded> {
    let (vars, sensitive) = ctx.network_scope();
    // Dynamic tokens are kept as written, so the generator only meets nested ones; it is fixed.
    let generator = RefCell::new(Generator::seeded(0, SystemTime::UNIX_EPOCH));
    let expand = |text: &str| {
        if !text.contains("{{") {
            return Expanded::Plain(text.to_owned());
        }
        let tracker = ctx.tracker(sensitive.clone());
        match expand_fixed(text, |t| {
            interpolate_tracked(t, &vars, Some(&tracker), &generator)
        }) {
            Ok(v) if tracker.is_empty() => Expanded::Plain(v),
            Ok(v) => Expanded::Secret(v),
            Err(_) => Expanded::Unresolved,
        }
    };
    templates(network)
        .into_iter()
        .map(|t| (t.to_owned(), expand(t)))
        .collect()
}

/// Hashes a certificate's file from its index in the certificate and its expanded path.
pub(crate) type ContentHash<'a> = &'a dyn Fn(usize, &Expanded) -> String;

/// SHA-256 hex of the file at `path`, or the unreadable marker.
pub(crate) fn file_hash(path: &Path) -> String {
    read_regular(path).map_or_else(|_| UNREADABLE.to_owned(), |bytes| content_hash(&bytes))
}

/// SHA-256 hex of `bytes`.
pub(crate) fn content_hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The content hash of a certificate file whose path expanded to `path`, read under `root`.
pub(crate) fn path_hash(root: &Path, path: &Expanded) -> String {
    match path {
        Expanded::Plain(p) | Expanded::Secret(p) => file_hash(&root.join(p)),
        Expanded::Unresolved => UNREADABLE.to_owned(),
    }
}

impl NetworkSnapshot {
    /// The snapshot of `network` as written, with no variable expanded; `salt` keys the hashes
    /// that stand for credentials.
    pub fn from_network(network: &CollectionNetwork, salt: &str) -> NetworkSnapshot {
        Self::build(
            network,
            salt,
            &|text| Expanded::Plain(text.to_owned()),
            None,
        )
    }

    /// The snapshot of `network` as a send in `ctx`'s environment would use it. A value that
    /// came from a secret variable is kept only as a keyed hash; a variable nothing defines
    /// becomes an "unresolved" marker, so it is neither an error nor a way to hide a change; a
    /// `{{$dynamic}}` field stays its template. Each certificate file is read and hashed.
    pub fn resolve(network: &CollectionNetwork, ctx: &VarContext, salt: &str) -> NetworkSnapshot {
        Self::from_expansions(network, &scope_expansions(network, ctx), ctx.root(), salt)
    }

    /// The snapshot of `network` from `expansions` (see [`scope_expansions`]), reading each
    /// certificate file under `root`.
    pub(crate) fn from_expansions(
        network: &CollectionNetwork,
        expansions: &BTreeMap<String, Expanded>,
        root: &Path,
        salt: &str,
    ) -> NetworkSnapshot {
        // A text the map does not hold is never shown, so a miss cannot leak a value.
        let expand = |t: &str| expansions.get(t).cloned().unwrap_or(Expanded::Unresolved);
        Self::build(network, salt, &expand, Some(&|_, p| path_hash(root, p)))
    }

    /// The snapshot of `network` with each text expanded by `expand`; `content` hashes a
    /// certificate's file from its index in the certificate and its expanded path, and `None`
    /// records no content.
    pub(crate) fn build(
        network: &CollectionNetwork,
        salt: &str,
        expand: &dyn Fn(&str) -> Expanded,
        content: Option<ContentHash<'_>>,
    ) -> NetworkSnapshot {
        let proxy = match &network.proxy {
            CollectionProxy::Custom(p) => {
                let (protocol, u0) = text_field(salt, expand(&p.protocol), &p.protocol);
                let (host, u1) = text_field(salt, expand(&p.hostname), &p.hostname);
                let (port, u2) = text_field(salt, expand(&p.port), &p.port);
                let (bypass, u3) = text_field(salt, expand(&p.bypass), &p.bypass);
                let user = p
                    .auth
                    .as_ref()
                    .map(|a| text_field(salt, expand(&a.username), &a.username).0);
                // The user name and any userinfo only ever enter the record as one keyed hash.
                let auth = credentials(salt, &[user, u0, u1, u2, u3]);
                ProxyView::Custom {
                    protocol: lower(protocol),
                    host: lower(host),
                    port,
                    bypass,
                    auth,
                }
            }
            CollectionProxy::Off => ProxyView::Off,
            _ => ProxyView::Inherit,
        };
        let host_overrides: Vec<HostView> = network
            .host_overrides
            .iter()
            .filter(|row| row.enabled)
            .map(|row| {
                let (host, u0) = text_field(salt, expand(&row.host), &row.host);
                let (address, u1) = text_field(salt, expand(&row.address), &row.address);
                HostView {
                    host: lower(host),
                    address,
                    creds: credentials(salt, &[u0, u1]),
                }
            })
            .collect();
        let certificates: Vec<CertView> = network
            .certificates
            .iter()
            .filter(|cert| cert.enabled)
            .map(|cert| {
                let text = |t: &String| text_field(salt, expand(t), t);
                let (domain, u0) = text(&cert.domain);
                let (kind, paths) = match &cert.kind {
                    CertificateKind::Pem {
                        certificate,
                        private_key,
                    } => ("pem", vec![certificate, private_key]),
                    CertificateKind::Pkcs12 { file } => ("pkcs12", vec![file]),
                };
                let mut found = vec![u0];
                let mut files = Vec::new();
                for path in &paths {
                    let (f, u) = text(path);
                    files.push(f);
                    found.push(u);
                }
                let content = content.map_or_else(Vec::new, |hash| {
                    (paths.iter().enumerate())
                        .map(|(i, p)| hash(i, &expand(p)))
                        .collect()
                });
                CertView {
                    domain,
                    kind: kind.into(),
                    files,
                    creds: credentials(salt, &found),
                    content,
                }
            })
            .collect();
        NetworkSnapshot {
            proxy,
            host_overrides,
            certificates,
        }
    }

    /// Lowercase hex SHA-256 of the snapshot's canonical JSON.
    pub fn fingerprint(&self) -> String {
        // The struct serialises in declaration order and every list keeps file order, so the
        // bytes depend on the settings alone.
        let json = serde_json::to_vec(self).unwrap_or_default();
        hex(&Sha256::digest(json))
    }

    /// Whether the collection sets nothing worth telling the person about: no custom proxy,
    /// no enabled host override and no enabled certificate.
    pub fn is_empty(&self) -> bool {
        !matches!(self.proxy, ProxyView::Custom { .. })
            && self.host_overrides.is_empty()
            && self.certificates.is_empty()
    }

    /// What differs between `old` and `new`, in a stable order.
    pub fn diff(old: &NetworkSnapshot, new: &NetworkSnapshot) -> Vec<NetworkChange> {
        let mut changes = Vec::new();
        proxy_changes(&old.proxy, &new.proxy, &mut changes);
        let before = changes.len();
        let (gone, added) = difference(&old.host_overrides, &new.host_overrides);
        pair_up(
            gone,
            added,
            |row| row.host.clone(),
            |old, new| {
                if old.address == new.address {
                    NetworkChange::HostOverrideCredentialsChanged {
                        host: shown(&new.host),
                    }
                } else {
                    NetworkChange::HostOverrideChanged {
                        host: shown(&new.host),
                        from: shown(&old.address),
                        to: shown(&new.address),
                    }
                }
            },
            |row| NetworkChange::HostOverrideRemoved {
                host: shown(&row.host),
                address: shown(&row.address),
            },
            |row| NetworkChange::HostOverrideAdded {
                host: shown(&row.host),
                address: shown(&row.address),
            },
            &mut changes,
        );
        if before == changes.len() && old.host_overrides != new.host_overrides {
            changes.push(NetworkChange::HostOverridesReordered);
        }
        let before = changes.len();
        let (gone, added) = difference(&old.certificates, &new.certificates);
        pair_up(
            gone,
            added,
            |cert| cert.domain.clone(),
            |_, new| NetworkChange::CertificateChanged {
                domain: shown(&new.domain),
            },
            |cert| NetworkChange::CertificateRemoved {
                domain: shown(&cert.domain),
            },
            |cert| NetworkChange::CertificateAdded {
                domain: shown(&cert.domain),
            },
            &mut changes,
        );
        if before == changes.len() && old.certificates != new.certificates {
            changes.push(NetworkChange::CertificatesReordered);
        }
        changes
    }
}

fn proxy_changes(old: &ProxyView, new: &ProxyView, out: &mut Vec<NetworkChange>) {
    if old == new {
        return;
    }
    if let (
        ProxyView::Custom {
            protocol: p0,
            host: h0,
            port: n0,
            bypass: b0,
            auth: a0,
        },
        ProxyView::Custom {
            protocol: p1,
            host: h1,
            port: n1,
            bypass: b1,
            auth: a1,
        },
    ) = (old, new)
    {
        if (p0, h0, n0) != (p1, h1, n1) {
            out.push(NetworkChange::ProxyChanged(new.address()));
        }
        if b0 != b1 {
            out.push(NetworkChange::ProxyBypassChanged);
        }
        if a0 != a1 {
            out.push(NetworkChange::ProxyCredentialsChanged);
        }
    } else {
        out.push(NetworkChange::ProxyChanged(new.address()));
    }
}

// The items only in `old` and only in `new`, compared as multisets.
fn difference<T: Ord + Clone>(old: &[T], new: &[T]) -> (Vec<T>, Vec<T>) {
    let mut left: Vec<T> = old.to_vec();
    let mut added = Vec::new();
    for item in new {
        match left.iter().position(|x| x == item) {
            Some(ix) => {
                left.remove(ix);
            }
            None => added.push(item.clone()),
        }
    }
    (left, added)
}

// A removed and an added item with the same key read as one change; the rest stand alone.
fn pair_up<T, K: PartialEq>(
    mut gone: Vec<T>,
    added: Vec<T>,
    key: impl Fn(&T) -> K,
    changed: impl Fn(&T, &T) -> NetworkChange,
    removed: impl Fn(&T) -> NetworkChange,
    appeared: impl Fn(&T) -> NetworkChange,
    out: &mut Vec<NetworkChange>,
) {
    for new in &added {
        match gone.iter().position(|old| key(old) == key(new)) {
            Some(ix) => {
                let old = gone.remove(ix);
                out.push(changed(&old, new));
            }
            None => out.push(appeared(new)),
        }
    }
    out.extend(gone.iter().map(removed));
}

/// scrypt of `text` under `salt` (log_n 14, r 8, p 1, 32 bytes) as hex, so a record copied off
/// the machine resists guessing. A memo keyed by the salt and a SHA-256 of the text keeps the
/// many sends of a run from paying for the same hash again.
fn keyed(salt: &str, text: &str) -> String {
    type Memo = HashMap<(String, [u8; 32]), String>;
    static MEMO: LazyLock<Mutex<Memo>> = LazyLock::new(Mutex::default);
    const MEMO_MAX: usize = 4096;
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&Sha256::digest(text.as_bytes()));
    let key = (salt.to_owned(), digest);
    if let Some(hit) = MEMO
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return hit.clone();
    }
    // Constant parameters and a 32-byte output are within scrypt's limits.
    #[allow(clippy::expect_used)]
    let params = scrypt::Params::new(14, 8, 1).expect("constant scrypt parameters are valid");
    let mut out = [0u8; 32];
    // Constant parameters and a 32-byte output are within scrypt's limits.
    #[allow(clippy::expect_used)]
    scrypt::scrypt(text.as_bytes(), salt.as_bytes(), &params, &mut out)
        .expect("a 32-byte scrypt output is valid");
    let value = hex(&out);
    let mut memo = MEMO.lock().unwrap_or_else(PoisonError::into_inner);
    if memo.len() >= MEMO_MAX {
        memo.clear();
    }
    memo.insert(key, value.clone());
    value
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// How a collection's current settings compare with the last acknowledged ones.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AckState {
    /// Never seen here. Holds what the collection sets, empty when it sets nothing worth
    /// telling the person about.
    FirstSeen(Vec<NetworkChange>),
    /// The same as when last acknowledged.
    Unchanged,
    /// Different from when last acknowledged.
    Changed(Vec<NetworkChange>),
}

/// The per-user record of each collection's acknowledged network settings, keyed by canonical
/// collection root. It holds the settings themselves (secret-free by construction) so a change
/// can be named, not merely detected. Changes stay in memory until [`NetworkAck::save`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct NetworkAck {
    path: PathBuf,
    salt: String,
    collections: BTreeMap<String, Slot>,
}

// An entry that does not parse is kept as written: it reads as changed (the record cannot
// vouch for the collection) until an acknowledgement replaces it, and is never dropped.
#[derive(Debug, Clone, PartialEq)]
enum Slot {
    Seen(Entry),
    Unreadable(serde_json::Value),
}

/// The record's key for `root`: its canonical path, else its absolute path.
pub(crate) fn root_key(root: &Path) -> String {
    let path = fs::canonicalize(root).unwrap_or_else(|_| {
        if root.is_absolute() {
            root.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(|_| root.to_path_buf(), |cwd| cwd.join(root))
        }
    });
    path.to_string_lossy().into_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    fingerprint: String,
    snapshot: NetworkSnapshot,
    /// Fingerprints of single sends the person let through, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sends: Vec<String>,
}

/// The most send fingerprints one collection keeps; the oldest go first.
const MAX_SENDS: usize = 512;

#[derive(Serialize, Deserialize)]
struct AckFile {
    version: u32,
    salt: String,
    #[serde(default)]
    collections: BTreeMap<String, serde_json::Value>,
}

impl NetworkAck {
    /// `<local data dir>/apinata/network-ack-cli.json`, the command line's own record; the app
    /// keeps its own, so each side notices a change independently. `None` without a data dir.
    pub fn default_cli_path() -> Option<PathBuf> {
        dirs::data_local_dir().map(|dir| dir.join("apinata").join("network-ack-cli.json"))
    }

    /// An empty record that [`NetworkAck::save`] writes to `path`, with a fresh salt.
    pub fn empty(path: impl Into<PathBuf>) -> NetworkAck {
        NetworkAck {
            path: path.into(),
            salt: uuid::Uuid::new_v4().simple().to_string(),
            collections: BTreeMap::new(),
        }
    }

    /// Load the file at `path`; a missing file is empty. Fails on an unreadable, non-JSON or
    /// non-version-1 file. An entry that does not parse is kept as written and reads as
    /// changed.
    pub fn load(path: impl Into<PathBuf>) -> Result<NetworkAck, TrustError> {
        let path = path.into();
        // A pipe reads as no record, which the next save replaces.
        let text = match read_regular_text(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::empty(path)),
            Err(e) => {
                return Err(TrustError::Io {
                    path,
                    message: e.to_string(),
                });
            }
        };
        let malformed = |message: String| TrustError::Malformed {
            path: path.clone(),
            message,
        };
        let file: AckFile = serde_json::from_str(&text).map_err(|e| malformed(e.to_string()))?;
        if file.version != 1 {
            return Err(malformed(format!("unsupported version {}", file.version)));
        }
        let collections = file
            .collections
            .into_iter()
            .map(|(root, v)| {
                let slot = match serde_json::from_value(v.clone()) {
                    Ok(entry) => Slot::Seen(entry),
                    Err(_) => Slot::Unreadable(v),
                };
                (root, slot)
            })
            .collect();
        Ok(NetworkAck {
            path,
            salt: file.salt,
            collections,
        })
    }

    /// Loads `path`, applies `change` and saves, all under an exclusive lock on a sidecar file,
    /// so windows and processes never lose one another's writes. A file that fails to load is
    /// neither changed nor written. `change` must not call `update` on the same file.
    pub fn update<R>(
        path: impl Into<PathBuf>,
        change: impl FnOnce(&mut NetworkAck) -> R,
    ) -> Result<R, TrustError> {
        let path = path.into();
        with_lock(&path, || {
            let mut ack = Self::load(&path)?;
            let out = change(&mut ack);
            ack.save()?;
            Ok(out)
        })
    }

    /// How `network` compares with what `root` last acknowledged.
    pub fn check(&self, root: &Path, network: &CollectionNetwork) -> AckState {
        self.check_snapshot(root, &NetworkSnapshot::from_network(network, &self.salt))
    }

    /// How `snapshot`, built with [`NetworkAck::salt`], compares with what `root` last
    /// acknowledged.
    pub fn check_snapshot(&self, root: &Path, snapshot: &NetworkSnapshot) -> AckState {
        match self.collections.get(&root_key(root)) {
            Some(Slot::Unreadable(_)) => AckState::Changed(vec![NetworkChange::RecordUnreadable]),
            None if snapshot.is_empty() => AckState::FirstSeen(Vec::new()),
            None => {
                AckState::FirstSeen(NetworkSnapshot::diff(&NetworkSnapshot::default(), snapshot))
            }
            Some(Slot::Seen(entry)) if entry.fingerprint == snapshot.fingerprint() => {
                AckState::Unchanged
            }
            Some(Slot::Seen(entry)) => {
                AckState::Changed(NetworkSnapshot::diff(&entry.snapshot, snapshot))
            }
        }
    }

    /// Record `network` as seen for `root`.
    pub fn acknowledge(&mut self, root: &Path, network: &CollectionNetwork) {
        let snapshot = NetworkSnapshot::from_network(network, &self.salt);
        self.acknowledge_snapshot(root, &snapshot);
    }

    /// [`NetworkAck::check`] against the record at `path`, reading it fresh. A record that
    /// cannot be read leaves nothing to compare with, so it reads as changed rather than as
    /// never seen.
    pub fn check_file(path: &Path, root: &Path, network: &CollectionNetwork) -> AckState {
        match Self::load(path) {
            Ok(ack) => ack.check(root, network),
            Err(_) => AckState::Changed(vec![NetworkChange::RecordUnreadable]),
        }
    }

    /// The record's salt, which keys the hashes inside its snapshots.
    pub fn salt(&self) -> &str {
        &self.salt
    }

    /// Record `snapshot`, built with [`NetworkAck::salt`], as seen for `root`.
    pub fn acknowledge_snapshot(&mut self, root: &Path, snapshot: &NetworkSnapshot) {
        // Sends let through earlier were approved under the settings they replace, so they
        // last only while the acknowledged settings stay the same.
        let fingerprint = snapshot.fingerprint();
        let sends = match self.collections.remove(&root_key(root)) {
            Some(Slot::Seen(entry)) if entry.fingerprint == fingerprint => entry.sends,
            _ => Vec::new(),
        };
        let entry = Entry {
            fingerprint,
            snapshot: snapshot.clone(),
            sends,
        };
        self.collections.insert(root_key(root), Slot::Seen(entry));
    }

    /// Record what `shown` showed for `root`: the collection's settings, when it showed them,
    /// and the send it asked about. Nothing is recorded when the record's salt has changed since,
    /// and a send only while the collection entry is the one it was judged against.
    pub fn acknowledge_shown(&mut self, root: &Path, shown: &ShownNetwork) {
        if shown.salt != self.salt {
            return;
        }
        if let Some(snapshot) = &shown.collection {
            self.acknowledge_snapshot(root, snapshot);
        }
        let (Some(send), Some(Slot::Seen(entry))) =
            (&shown.send, self.collections.get_mut(&root_key(root)))
        else {
            return;
        };
        // A send judged under settings since replaced (an Acknowledge that landed while the
        // question was open) is not vouched for; the next send judges it again.
        if shown.judged.as_deref() != Some(entry.fingerprint.as_str()) {
            return;
        }
        entry.sends.retain(|s| s != send);
        entry.sends.push(send.clone());
        let over = entry.sends.len().saturating_sub(MAX_SENDS);
        entry.sends.drain(..over);
    }

    /// How the snapshot `make` builds from the record's salt compares with what `root` last
    /// acknowledged, and what a notice of it shows, for [`NetworkAck::acknowledge_shown`]. A
    /// missing record is written first so its salt lasts; a collection that sets nothing is
    /// recorded on the spot and reads as unchanged. An unreadable record shows nothing.
    pub fn check_shown(
        path: &Path,
        root: &Path,
        make: impl FnOnce(&str) -> NetworkSnapshot,
    ) -> (AckState, Option<ShownNetwork>) {
        let unreadable = || {
            (
                AckState::Changed(vec![NetworkChange::RecordUnreadable]),
                None,
            )
        };
        let judged = with_lock(path, || {
            let fresh = !path.exists();
            let Ok(mut ack) = Self::load(path) else {
                return Ok(unreadable());
            };
            let snapshot = make(&ack.salt);
            let state = match ack.check_snapshot(root, &snapshot) {
                AckState::FirstSeen(changes) if changes.is_empty() => {
                    ack.acknowledge_snapshot(root, &snapshot);
                    ack.save()?;
                    AckState::Unchanged
                }
                state => {
                    if fresh {
                        ack.save()?;
                    }
                    state
                }
            };
            Ok((state, Some(ShownNetwork::collection(&ack.salt, snapshot))))
        });
        judged.unwrap_or_else(|_| unreadable())
    }

    /// Whether the send fingerprinted `send` was let through for `root`.
    pub(crate) fn has_send(&self, root: &Path, send: &str) -> bool {
        matches!(self.collections.get(&root_key(root)), Some(Slot::Seen(e)) if e.sends.iter().any(|s| s == send))
    }

    /// [`NetworkAck::check_file`] for a snapshot that `make` builds from the record's salt, so
    /// a record that is missing or fresh keys its hashes the way the save that follows will.
    pub fn check_file_with(
        path: &Path,
        root: &Path,
        make: impl FnOnce(&str) -> NetworkSnapshot,
    ) -> AckState {
        match Self::load(path) {
            Ok(ack) => ack.check_snapshot(root, &make(&ack.salt)),
            Err(_) => AckState::Changed(vec![NetworkChange::RecordUnreadable]),
        }
    }

    /// Moves an unreadable record to `<name>.bad` (`.bad.1`, `.bad.2` and so on when one is
    /// already there), so a new one starts empty and nothing is deleted. A record that loads, or
    /// a missing one, is left alone and reported as an error. Returns where the file went.
    pub fn reset_unreadable(path: &Path) -> Result<PathBuf, TrustError> {
        with_lock(path, || {
            if Self::load(path).is_ok() {
                return Err(TrustError::Io {
                    path: path.to_path_buf(),
                    message: "the record is readable".into(),
                });
            }
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let aside = (0u32..)
                .map(|n| match n {
                    0 => path.with_file_name(format!("{name}.bad")),
                    n => path.with_file_name(format!("{name}.bad.{n}")),
                })
                .find(|candidate| fs::symlink_metadata(candidate).is_err())
                .unwrap_or_else(|| path.with_file_name(format!("{name}.bad.last")));
            fs::rename(path, &aside).map_err(|e| TrustError::Io {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?;
            Ok(aside)
        })
    }

    /// Write the file atomically as pretty JSON, creating the directory.
    pub fn save(&self) -> Result<(), TrustError> {
        let collections = self
            .collections
            .iter()
            .filter_map(|(root, slot)| {
                let value = match slot {
                    Slot::Seen(e) => serde_json::to_value(e).ok()?,
                    Slot::Unreadable(v) => v.clone(),
                };
                Some((root.clone(), value))
            })
            .collect();
        let file = AckFile {
            version: 1,
            salt: self.salt.clone(),
            collections,
        };
        let mut bytes = serde_json::to_vec_pretty(&file).map_err(|e| TrustError::Io {
            path: self.path.clone(),
            message: e.to_string(),
        })?;
        bytes.push(b'\n');
        write_atomic(&self.path, &bytes)
    }
}

/// The network settings `root`'s `opencollection.yml` holds on disk right now, ignoring any
/// unsaved edit in memory.
pub fn saved_network(root: &Path) -> Result<CollectionNetwork, FileError> {
    let doc = Document::load(&root.join(ROOT_FILE))?;
    Ok(read_network(&doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The record keeps the newest sends only, so it cannot grow without bound.
    #[test]
    fn the_sends_kept_per_collection_are_capped_oldest_first() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut ack = NetworkAck::empty(dir.path().join("ack.json"));
        let snapshot = NetworkSnapshot::default();
        ack.acknowledge_snapshot(dir.path(), &snapshot);
        for n in 0..MAX_SENDS + 10 {
            let shown = ShownNetwork {
                salt: ack.salt().to_owned(),
                collection: None,
                send: Some(format!("send-{n}")),
                judged: Some(snapshot.fingerprint()),
            };
            ack.acknowledge_shown(dir.path(), &shown);
        }
        assert!(!ack.has_send(dir.path(), "send-0"));
        assert!(ack.has_send(dir.path(), "send-10"));
        assert!(ack.has_send(dir.path(), &format!("send-{}", MAX_SENDS + 9)));
        // Another salt keyed what was shown, so nothing of it is recorded.
        let stale = ShownNetwork {
            salt: "other".into(),
            collection: None,
            send: Some("stale".into()),
            judged: Some(snapshot.fingerprint()),
        };
        ack.acknowledge_shown(dir.path(), &stale);
        assert!(!ack.has_send(dir.path(), "stale"));
    }

    // A Network-section Acknowledge that replaced the collection while a question was open
    // leaves the answer's send unrecorded: it was judged under settings no longer acknowledged.
    #[test]
    fn a_send_judged_under_replaced_settings_is_not_recorded() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut ack = NetworkAck::empty(dir.path().join("ack.json"));
        let old = NetworkSnapshot::default();
        ack.acknowledge_snapshot(dir.path(), &old);
        let mut net = CollectionNetwork::default();
        net.host_overrides
            .push(crate::network::HostOverride::new("a.test", "10.0.0.1"));
        let new = NetworkSnapshot::from_network(&net, ack.salt());
        ack.acknowledge_snapshot(dir.path(), &new);
        let answer = ShownNetwork {
            salt: ack.salt().to_owned(),
            collection: None,
            send: Some("judged-under-old".into()),
            judged: Some(old.fingerprint()),
        };
        ack.acknowledge_shown(dir.path(), &answer);
        assert!(!ack.has_send(dir.path(), "judged-under-old"));
    }
}
