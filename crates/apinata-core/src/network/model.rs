//! `config.proxy` and `config.clientCertificates` of `opencollection.yml`, read as Bruno's
//! `parseCollection` reads them and written in `stringifyCollection`'s key order.

use super::hosts::{HostOverride, read_host_overrides};
use crate::document::{Document, Fields, Origin, write_entry};
use crate::js;
use yaml_serde::{Mapping, Value};

const CONFIG: &[&str] = &["protobuf", "proxy", "clientCertificates"];
const PROXY: &[&str] = &["inherit", "config", "disabled"];
const CERTIFICATE: &[&str] = &[
    "domain",
    "type",
    "certificateFilePath",
    "privateKeyFilePath",
    "pkcs12FilePath",
    "passphrase",
    "disabled",
];

/// How a collection chooses its proxy.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CollectionProxy {
    /// Use the global setting.
    #[default]
    Inherit,
    /// Connect directly.
    Off,
    /// Use this proxy.
    Custom(ProxyConfig),
}

/// A proxy the collection names. Fields may hold `{{variables}}`.
#[derive(Debug, Clone, Eq)]
#[non_exhaustive]
pub struct ProxyConfig {
    /// `http`, `https`, `socks4`, `socks5` and so on.
    pub protocol: String,
    /// The proxy host.
    pub hostname: String,
    /// The port as written, empty for the protocol default.
    pub port: String,
    /// Credentials, when both are not blank.
    pub auth: Option<ProxyAuth>,
    /// Comma separated hosts that skip the proxy.
    pub bypass: String,
    // Whether `port` is written as a YAML number: it was read as one, or typed as digits.
    port_number: bool,
}

impl ProxyConfig {
    /// A proxy without credentials or bypass list.
    pub fn new(
        protocol: impl Into<String>,
        hostname: impl Into<String>,
        port: impl Into<String>,
    ) -> ProxyConfig {
        ProxyConfig {
            protocol: protocol.into(),
            hostname: hostname.into(),
            port: port.into(),
            auth: None,
            bypass: String::new(),
            port_number: false,
        }
    }

    /// Set the port as typed into a port field. The same text keeps the form it was read in;
    /// new text of decimal digits is written as a number, as the reference app stores a typed
    /// port, and anything else (empty too) as a string.
    pub fn set_port(&mut self, port: impl Into<String>) {
        let port = port.into();
        if port != self.port {
            self.port_number = port.parse::<u64>().is_ok();
            self.port = port;
        }
    }
}

/// Compares the public fields only; the form the port is written in does not matter.
impl PartialEq for ProxyConfig {
    fn eq(&self, other: &Self) -> bool {
        (
            &self.protocol,
            &self.hostname,
            &self.port,
            &self.auth,
            &self.bypass,
        ) == (
            &other.protocol,
            &other.hostname,
            &other.port,
            &other.auth,
            &other.bypass,
        )
    }
}

/// Proxy credentials.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProxyAuth {
    /// The user name.
    pub username: String,
    /// The password; never printed by `Debug`.
    pub password: String,
}

impl ProxyAuth {
    /// Credentials from a user name and password.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> ProxyAuth {
        ProxyAuth {
            username: username.into(),
            password: password.into(),
        }
    }
}

impl std::fmt::Debug for ProxyAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyAuth")
            .field("username", &self.username)
            .field("password", &"***")
            .finish()
    }
}

/// Where a client certificate's key material lives. Paths are relative to the collection.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CertificateKind {
    /// A certificate and a private key in PEM files.
    Pem {
        /// The certificate file.
        certificate: String,
        /// The private key file.
        private_key: String,
    },
    /// A PKCS#12 bundle.
    Pkcs12 {
        /// The bundle file.
        file: String,
    },
}

/// A client certificate offered to hosts matching `domain`.
#[derive(Clone)]
#[non_exhaustive]
pub struct ClientCertificate {
    /// The host pattern, `*` wildcards allowed.
    pub domain: String,
    /// The key material.
    pub kind: CertificateKind,
    /// The key's passphrase, empty for none; never printed by `Debug`.
    pub passphrase: String,
    /// Whether the entry is used.
    pub enabled: bool,
    origin: Option<Origin>,
}

impl ClientCertificate {
    /// An enabled PEM certificate with no passphrase.
    pub fn pem(
        domain: impl Into<String>,
        certificate: impl Into<String>,
        private_key: impl Into<String>,
    ) -> ClientCertificate {
        ClientCertificate::with_kind(
            domain.into(),
            CertificateKind::Pem {
                certificate: certificate.into(),
                private_key: private_key.into(),
            },
        )
    }

    /// An enabled PKCS#12 certificate with no passphrase.
    pub fn pkcs12(domain: impl Into<String>, file: impl Into<String>) -> ClientCertificate {
        ClientCertificate::with_kind(domain.into(), CertificateKind::Pkcs12 { file: file.into() })
    }

    fn with_kind(domain: String, kind: CertificateKind) -> ClientCertificate {
        ClientCertificate {
            domain,
            kind,
            passphrase: String::new(),
            enabled: true,
            origin: None,
        }
    }

    /// Read a `pem` or `pkcs12` entry, remembering its YAML; any other type is `None`.
    fn from_yaml(entry: &Value) -> Option<ClientCertificate> {
        let kind = match entry.get("type").and_then(Value::as_str)? {
            "pem" => CertificateKind::Pem {
                certificate: text(entry, "certificateFilePath"),
                private_key: text(entry, "privateKeyFilePath"),
            },
            "pkcs12" => CertificateKind::Pkcs12 {
                file: text(entry, "pkcs12FilePath"),
            },
            _ => return None,
        };
        let mut cert = ClientCertificate::with_kind(text(entry, "domain"), kind);
        cert.passphrase = text(entry, "passphrase");
        cert.enabled = !js::is_disabled(entry);
        cert.origin = Some(Origin::new(entry.clone(), cert.fields()));
        Some(cert)
    }

    /// The entry's YAML: as read when unchanged, else Bruno's form with unknown keys kept.
    fn to_yaml(&self) -> Value {
        write_entry(self.origin.as_ref(), self.fields(), CERTIFICATE)
    }

    fn fields(&self) -> Fields {
        let (kind, certificate, key, p12) = match &self.kind {
            CertificateKind::Pem {
                certificate,
                private_key,
            } => ("pem", Some(certificate), Some(private_key), None),
            CertificateKind::Pkcs12 { file } => ("pkcs12", None, None, Some(file)),
        };
        let string = |s: Option<&String>| s.map(|s| Value::from(s.as_str()));
        vec![
            ("domain", Some(Value::from(self.domain.as_str()))),
            ("type", Some(Value::from(kind))),
            ("certificateFilePath", string(certificate)),
            ("privateKeyFilePath", string(key)),
            ("pkcs12FilePath", string(p12)),
            (
                "passphrase",
                (!self.passphrase.is_empty()).then(|| Value::from(self.passphrase.as_str())),
            ),
            ("disabled", (!self.enabled).then_some(Value::Bool(true))),
        ]
    }
}

/// Compares the public fields only; where an entry was read from does not matter.
impl PartialEq for ClientCertificate {
    fn eq(&self, other: &Self) -> bool {
        (&self.domain, &self.kind, &self.passphrase, self.enabled)
            == (&other.domain, &other.kind, &other.passphrase, other.enabled)
    }
}

impl std::fmt::Debug for ClientCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientCertificate")
            .field("domain", &self.domain)
            .field("kind", &self.kind)
            .field("passphrase", &"***")
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// The collection's proxy choice and client certificates.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct CollectionNetwork {
    /// The proxy choice.
    pub proxy: CollectionProxy,
    /// The `pem` and `pkcs12` entries in file order; other types are not offered.
    pub certificates: Vec<ClientCertificate>,
    /// The fields an inherited or disabled proxy still keeps in the file, which a switch back
    /// to a custom proxy restores; `None` for a custom proxy or no `config` mapping.
    pub stored_proxy: Option<ProxyConfig>,
    /// The `extensions.apinata.hostOverrides` rows in file order.
    pub host_overrides: Vec<HostOverride>,
}

/// A scalar field as Bruno's `ensureString` reads it; absent or non-scalar is empty.
pub(super) fn text(entry: &Value, key: &str) -> String {
    entry
        .get(key)
        .and_then(js::yaml_to_string)
        .unwrap_or_default()
}

/// Read the model from a root document: its `config` block and host overrides.
pub(crate) fn read_network(doc: &Document) -> CollectionNetwork {
    let config = doc.get(&["config"]);
    let block = config.and_then(|c| c.get("proxy"));
    let proxy = block.map(read_proxy).unwrap_or_default();
    let stored_proxy = match proxy {
        CollectionProxy::Custom(_) => None,
        _ => block
            .and_then(|b| b.get("config"))
            .filter(|c| c.is_mapping())
            .map(read_proxy_config),
    };
    CollectionNetwork {
        proxy,
        stored_proxy,
        host_overrides: read_host_overrides(doc),
        certificates: config
            .and_then(|c| c.get("clientCertificates"))
            .and_then(Value::as_sequence)
            .map(|list| {
                list.iter()
                    .filter_map(ClientCertificate::from_yaml)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The block counts only when `inherit` is a boolean and `config` a mapping.
fn read_proxy(block: &Value) -> CollectionProxy {
    let (Some(inherit), Some(config)) = (
        block.get("inherit").and_then(Value::as_bool),
        block.get("config").filter(|c| c.is_mapping()),
    ) else {
        return CollectionProxy::Inherit;
    };
    if js::is_disabled(block) {
        return CollectionProxy::Off;
    }
    if inherit {
        return CollectionProxy::Inherit;
    }
    CollectionProxy::Custom(read_proxy_config(config))
}

// The `config` mapping's fields, as a custom proxy reads them.
fn read_proxy_config(config: &Value) -> ProxyConfig {
    let mut proxy = ProxyConfig::new(
        text(config, "protocol"),
        text(config, "hostname"),
        text(config, "port"),
    );
    proxy.port_number = config.get("port").is_some_and(Value::is_number);
    if proxy.protocol.is_empty() {
        proxy.protocol = "http".into();
    }
    proxy.bypass = text(config, "bypassProxy");
    proxy.auth = config
        .get("auth")
        .filter(|a| !js::is_disabled(a))
        .map(|a| ProxyAuth::new(text(a, "username"), text(a, "password")))
        .filter(|a| !(a.username.is_empty() && a.password.is_empty()));
    proxy
}

/// Bruno's `config` mapping for `proxy`, empty defaults when it is `None`.
fn proxy_config_yaml(proxy: Option<&ProxyConfig>) -> Value {
    let default = ProxyConfig::new("http", "", "");
    let proxy = proxy.unwrap_or(&default);
    let (username, password) = proxy
        .auth
        .as_ref()
        .map_or(("", ""), |a| (&a.username, &a.password));
    let mut auth = Mapping::new();
    auth.insert("username".into(), username.into());
    auth.insert("password".into(), password.into());
    let mut map = Mapping::new();
    map.insert("protocol".into(), proxy.protocol.as_str().into());
    map.insert("hostname".into(), proxy.hostname.as_str().into());
    // A number port holds the JavaScript text of the number it was read as (or typed digits),
    // so it parses back to that number whatever its sign, fraction or exponent.
    let port = match (proxy.port.parse::<u64>(), proxy.port.parse::<f64>()) {
        (Ok(n), _) if proxy.port_number => Value::from(n),
        (_, Ok(f)) if proxy.port_number => Value::from(f),
        _ => proxy.port.as_str().into(),
    };
    map.insert("port".into(), port);
    map.insert("auth".into(), Value::Mapping(auth));
    map.insert("bypassProxy".into(), proxy.bypass.as_str().into());
    Value::Mapping(map)
}

/// Write the proxy choice. Inherit and Off keep a well-formed `config` mapping, as Bruno's app
/// keeps the fields across modes, and write empty defaults when there is none.
pub(crate) fn write_proxy(doc: &mut Document, top: &[&str], proxy: &CollectionProxy) {
    let order: &[&[&str]] = &[top, CONFIG, PROXY];
    let path = ["config", "proxy"];
    let at = |key: &'static str| [path[0], path[1], key];
    match proxy {
        CollectionProxy::Custom(custom) => {
            doc.set(&at("inherit"), Value::Bool(false), order);
            doc.set(&at("config"), proxy_config_yaml(Some(custom)), order);
            doc.remove(&at("disabled"));
        }
        CollectionProxy::Inherit | CollectionProxy::Off => {
            let off = matches!(proxy, CollectionProxy::Off);
            doc.set(&at("inherit"), Value::Bool(!off), order);
            if !doc.get(&at("config")).is_some_and(Value::is_mapping) {
                doc.set(&at("config"), proxy_config_yaml(None), order);
            }
            if off {
                doc.set(&at("disabled"), Value::Bool(true), order);
            } else {
                doc.remove(&at("disabled"));
            }
        }
    }
}

/// Write the given entries, then the file's entries of unknown type in their original order. An
/// empty list removes the key, and an emptied `config` goes with it.
pub(crate) fn write_certificates(
    doc: &mut Document,
    top: &[&str],
    certificates: &[ClientCertificate],
) {
    let mut items: Vec<Value> = certificates
        .iter()
        .map(ClientCertificate::to_yaml)
        .collect();
    let unknown = doc
        .get(&["config", "clientCertificates"])
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|e| ClientCertificate::from_yaml(e).is_none())
        .cloned();
    items.extend(unknown);
    doc.set_list(&["config", "clientCertificates"], items, &[top, CONFIG]);
    doc.remove_if_empty(&["config"]);
}
