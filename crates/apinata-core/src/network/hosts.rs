//! Host overrides: `extensions.apinata.hostOverrides` rows that send a host name to chosen
//! addresses, read and written beside the file's other extensions, then resolved per request.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::Url;
use yaml_serde::Value;

use super::model::text;
use crate::document::{Document, Fields, Origin, write_entry};
use crate::error::ResolveError;
use crate::host_guard::{HostPolicy, Refused, judge_addresses};
use crate::js;
use crate::trust::normalize_bare_host;

const PATH: [&str; 3] = ["extensions", "apinata", "hostOverrides"];
const ROW: &[&str] = &["host", "address", "disabled"];

/// The most addresses one row may list.
const MAX_ADDRESSES: usize = 8;

/// One host name mapped to the addresses a send connects to instead of its DNS answer.
#[derive(Clone)]
#[non_exhaustive]
pub struct HostOverride {
    /// The host name to override; may hold `{{variables}}`.
    pub host: String,
    /// One or more comma separated addresses: `ip`, `ip:port` or `[v6]:port`. A port counts
    /// only for a URL that names none; otherwise the URL's port is used.
    pub address: String,
    /// Whether the row is used.
    pub enabled: bool,
    origin: Option<Origin>,
}

impl HostOverride {
    /// An enabled row mapping `host` to `address`.
    pub fn new(host: impl Into<String>, address: impl Into<String>) -> HostOverride {
        HostOverride {
            host: host.into(),
            address: address.into(),
            enabled: true,
            origin: None,
        }
    }

    /// Check the row as written. A row holding `{{variables}}` passes, since they are filled
    /// only when a request resolves.
    pub fn check(&self) -> Result<(), ResolveError> {
        if self.host.contains("{{") || self.address.contains("{{") {
            return Ok(());
        }
        let mut row = self.clone();
        row.enabled = true;
        resolve_host_overrides(&[row], |text| Ok(text.to_owned())).map(|_| ())
    }

    /// A row is a mapping with a `host`; any other entry is kept untouched and not offered.
    fn from_yaml(entry: &Value) -> Option<HostOverride> {
        entry.get("host")?;
        let mut row = HostOverride::new(text(entry, "host"), text(entry, "address"));
        row.enabled = !js::is_disabled(entry);
        row.origin = Some(Origin::new(entry.clone(), row.fields()));
        Some(row)
    }

    fn fields(&self) -> Fields {
        vec![
            ("host", Some(Value::from(self.host.as_str()))),
            ("address", Some(Value::from(self.address.as_str()))),
            ("disabled", (!self.enabled).then_some(Value::Bool(true))),
        ]
    }
}

/// Compares the public fields only; where a row was read from does not matter.
impl PartialEq for HostOverride {
    fn eq(&self, other: &Self) -> bool {
        (&self.host, &self.address, self.enabled) == (&other.host, &other.address, other.enabled)
    }
}

impl Eq for HostOverride {}

impl std::fmt::Debug for HostOverride {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostOverride")
            .field("host", &self.host)
            .field("address", &self.address)
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// The rows in file order; entries that are not rows are skipped.
pub(crate) fn read_host_overrides(doc: &Document) -> Vec<HostOverride> {
    doc.get(&PATH)
        .and_then(Value::as_sequence)
        .map(|list| list.iter().filter_map(HostOverride::from_yaml).collect())
        .unwrap_or_default()
}

/// Write the rows, then the file's entries that are not rows in their original order. An empty
/// result removes the key and the `apinata` and `extensions` mappings that held only it.
pub(crate) fn write_host_overrides(doc: &mut Document, top: &[&str], rows: &[HostOverride]) {
    let mut items: Vec<Value> = rows
        .iter()
        .map(|r| write_entry(r.origin.as_ref(), r.fields(), ROW))
        .collect();
    let others = doc
        .get(&PATH)
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|e| HostOverride::from_yaml(e).is_none())
        .cloned();
    items.extend(others);
    let existed = doc.get(&PATH).is_some();
    doc.set_list(&PATH, items, &[top, &[], &[]]);
    if existed {
        doc.remove_if_empty(&PATH[..2]);
        doc.remove_if_empty(&PATH[..1]);
    }
}

/// An enabled host override with its name normalised and its addresses parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedHostOverride {
    host: String,
    addresses: Vec<SocketAddr>,
}

impl ResolvedHostOverride {
    /// The normalised host name.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The mapped addresses in row order; port 0 means the URL's port.
    pub fn addresses(&self) -> &[SocketAddr] {
        &self.addresses
    }

    /// `name -> address[, address]` as the timeline shows it.
    fn label(&self) -> String {
        let shown: Vec<String> = self.addresses.iter().map(shown_address).collect();
        format!("{} -> {}", self.host, shown.join(", "))
    }
}

/// An address without its port when the port is 0, as written in the row.
fn shown_address(addr: &SocketAddr) -> String {
    if addr.port() == 0 {
        addr.ip().to_string()
    } else {
        addr.to_string()
    }
}

/// Expand, validate and normalise the enabled rows. `expand` fills a row's variables. A bad row
/// or a second enabled row for one host is an error that names the row as written; the order
/// of the result is the file's.
pub(crate) fn resolve_host_overrides(
    rows: &[HostOverride],
    mut expand: impl FnMut(&str) -> Result<String, ResolveError>,
) -> Result<Vec<ResolvedHostOverride>, ResolveError> {
    let mut out: Vec<ResolvedHostOverride> = Vec::new();
    let mut first: HashMap<String, String> = HashMap::new();
    for row in rows.iter().filter(|r| r.enabled) {
        let invalid = |reason: &str| ResolveError::InvalidHostOverride {
            host: row.host.clone(),
            address: row.address.clone(),
            reason: reason.to_owned(),
        };
        let host = normalize_override_host(&expand(&row.host)?).map_err(invalid)?;
        let addresses = parse_addresses(&expand(&row.address)?).map_err(invalid)?;
        if let Some(earlier) = first.get(&host) {
            return Err(ResolveError::DuplicateHostOverride {
                host,
                first: earlier.clone(),
                second: row.address.clone(),
            });
        }
        first.insert(host.clone(), row.address.clone());
        out.push(ResolvedHostOverride { host, addresses });
    }
    Ok(out)
}

fn normalize_override_host(input: &str) -> Result<String, &'static str> {
    let host = normalize_bare_host(input).ok_or("the host is not a bare host name")?;
    if host.starts_with('[') || host.parse::<IpAddr>().is_ok() {
        return Err("an IP address cannot be overridden, only a host name");
    }
    Ok(host)
}

fn parse_addresses(input: &str) -> Result<Vec<SocketAddr>, &'static str> {
    let parts: Vec<&str> = input.split(',').map(str::trim).collect();
    if parts.len() > MAX_ADDRESSES {
        return Err("too many addresses in one row");
    }
    parts.into_iter().map(parse_address).collect()
}

fn parse_address(part: &str) -> Result<SocketAddr, &'static str> {
    // A scope id names an interface of the machine, and the parser either rejects it or
    // drops it, so say so rather than fail with a vaguer message.
    if part.contains('%') {
        return Err("a zone id (%) in an address is not supported");
    }
    let bare = part.strip_prefix('[').and_then(|p| p.strip_suffix(']'));
    let addr = SocketAddr::from_str(part)
        .ok()
        .or_else(|| {
            IpAddr::from_str(bare.unwrap_or(part))
                .ok()
                .map(|ip| SocketAddr::new(ip, 0))
        })
        .ok_or("the address is not an IP address, with an optional port")?;
    if addr.ip().to_canonical().is_unspecified() {
        return Err("an unspecified address (0.0.0.0 or ::) is not a destination");
    }
    Ok(addr)
}

/// The override for the normalised `host`, when one applies.
pub(crate) fn overridden<'a>(
    overrides: &'a [ResolvedHostOverride],
    host: &str,
) -> Option<&'a ResolvedHostOverride> {
    overrides.iter().find(|o| o.host == host)
}

/// The timeline label for a hop to `url`, when its host is overridden.
pub(crate) fn label_for(overrides: &[ResolvedHostOverride], url: &str) -> Option<String> {
    if overrides.is_empty() {
        return None;
    }
    let host = crate::trust::normalize_host(Url::parse(url).ok()?.host_str()?)?;
    overridden(overrides, &host).map(ResolvedHostOverride::label)
}

/// A resolver that answers an overridden name from its row and leaves every other name to
/// `inner`. Under a host policy it judges the mapped addresses again on each connection.
pub(crate) struct OverrideResolve {
    pub(crate) overrides: Arc<Vec<ResolvedHostOverride>>,
    pub(crate) policy: Option<HostPolicy>,
    pub(crate) inner: Arc<dyn Resolve>,
}

impl Resolve for OverrideResolve {
    fn resolve(&self, name: Name) -> Resolving {
        let host = crate::trust::normalize_host(name.as_str()).unwrap_or_default();
        let Some(found) = overridden(&self.overrides, &host).cloned() else {
            return self.inner.resolve(name);
        };
        let policy = self.policy.clone();
        Box::pin(async move {
            if let Some(policy) = &policy
                && let Err(refusal) = judge_addresses(&found, policy)
            {
                return Err(Box::new(Refused {
                    host: refusal.host,
                    mapped_from: refusal.mapped_from,
                })
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            let addrs: Addrs = Box::new(found.addresses.into_iter());
            Ok(addrs)
        })
    }
}
