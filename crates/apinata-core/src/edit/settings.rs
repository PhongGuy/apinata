//! A request's `settings` block as the app edits it.

use crate::HttpVersionPolicy;
use crate::js::js_trim;
use crate::order;
use crate::resolve::{to_bool, to_max_redirects};
use yaml_serde::Value;

/// A request timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Timeout {
    /// No limit; written as `0`.
    Unlimited,
    /// A limit in milliseconds, at least 1.
    Millis(u64),
    /// Use the default timeout; written as `inherit`, and read also for an absent or null key.
    Inherit,
}

/// Whether a request checks the server's TLS certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TlsVerify {
    /// Follow the app or command-line default; the file has no `verifySsl` key.
    #[default]
    Inherit,
    /// Always verify; written as `verifySsl: true`.
    On,
    /// Accept any certificate; written as `verifySsl: false`.
    Off,
}

/// How `settings.verifySsl` reads, and whether its value was not understood. Only the YAML
/// boolean `false` turns the check off; anything else, the string `"false"` too, keeps it on.
pub(crate) fn read_verify(node: Option<&Value>) -> (TlsVerify, bool) {
    match node {
        None | Some(Value::Null) => (TlsVerify::Inherit, false),
        Some(Value::String(s)) if s == "inherit" => (TlsVerify::Inherit, false),
        Some(Value::Bool(false)) => (TlsVerify::Off, false),
        Some(Value::Bool(true)) => (TlsVerify::On, false),
        Some(_) => (TlsVerify::On, true),
    }
}

/// How `settings.httpVersion` reads, and whether its value was not understood, which inherits.
pub(crate) fn read_http_version(node: Option<&Value>) -> (Option<HttpVersionPolicy>, bool) {
    match node {
        None | Some(Value::Null) => (None, false),
        Some(Value::String(s)) if s == "inherit" => (None, false),
        Some(Value::String(s)) => match s.parse() {
            Ok(policy) => (Some(policy), false),
            Err(_) => (None, true),
        },
        Some(_) => (None, true),
    }
}

/// The settings Bruno stores per request, plus Apinata's own certificate and version keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RequestSettings {
    /// Whether the URL is percent-encoded before sending.
    pub encode_url: bool,
    /// The timeout.
    pub timeout: Timeout,
    /// Whether redirects are followed.
    pub follow_redirects: bool,
    /// How many redirects are followed.
    pub max_redirects: u32,
    /// Whether `Authorization` is kept across a redirect.
    pub forward_authorization_header: bool,
    /// Whether the server's certificate is checked (`verifySsl`).
    pub verify_tls: TlsVerify,
    /// The HTTP version (`httpVersion`); `None` follows the app or command-line default.
    pub http_version: Option<HttpVersionPolicy>,
}

impl RequestSettings {
    /// What a file without these keys means: no time limit, and `Authorization` kept across a
    /// redirect. A file Apinata creates or imports starts from
    /// [`RequestSettings::for_new_file`] instead.
    pub fn new() -> RequestSettings {
        RequestSettings {
            encode_url: true,
            timeout: Timeout::Unlimited,
            follow_redirects: true,
            max_redirects: 5,
            forward_authorization_header: true,
            verify_tls: TlsVerify::Inherit,
            http_version: None,
        }
    }

    /// The settings a new request file starts with: [`RequestSettings::new`] with the timeout
    /// inherited, so the send-time default timeout applies, and `Authorization` not forwarded
    /// across a redirect, as Bruno writes for a request it creates. New files and the importers
    /// use it.
    pub fn for_new_file() -> RequestSettings {
        RequestSettings {
            timeout: Timeout::Inherit,
            forward_authorization_header: false,
            ..RequestSettings::new()
        }
    }

    /// Read a `settings` block with Bruno's coercions; a missing block gives the defaults, with
    /// the timeout inherited.
    pub(crate) fn from_yaml(block: Option<&Value>) -> RequestSettings {
        let get = |key: &str| block.and_then(|block| block.get(key));
        // Absent and null take the default when sent, as `inherit` does, so they read as it.
        let timeout = match get("timeout") {
            None | Some(Value::Null) => Timeout::Inherit,
            Some(Value::String(s)) if s == "inherit" => Timeout::Inherit,
            // Fractions floor; zero, negatives and non-numbers mean no limit.
            Some(Value::Number(n)) => match n.as_f64() {
                Some(ms) if ms.is_finite() && ms >= 1.0 => Timeout::Millis(ms as u64),
                _ => Timeout::Unlimited,
            },
            _ => Timeout::Unlimited,
        };
        RequestSettings {
            encode_url: to_bool(get("encodeUrl")),
            timeout,
            follow_redirects: to_bool(get("followRedirects")),
            max_redirects: to_max_redirects(get("maxRedirects")),
            forward_authorization_header: to_bool(get("forwardAuthorizationHeader")),
            verify_tls: read_verify(get("verifySsl")).0,
            http_version: read_http_version(get("httpVersion")).0,
        }
    }

    /// The keys that differ from `before`, with the values written; `None` removes the key.
    pub(crate) fn changes_from(
        &self,
        before: &RequestSettings,
    ) -> Vec<(&'static str, Option<Value>)> {
        let mut changes = Vec::new();
        if self.encode_url != before.encode_url {
            changes.push(("encodeUrl", Some(Value::Bool(self.encode_url))));
        }
        if self.timeout != before.timeout {
            let timeout = match self.timeout {
                Timeout::Unlimited => Value::from(0u64),
                Timeout::Millis(ms) => Value::from(ms),
                Timeout::Inherit => Value::from("inherit"),
            };
            changes.push(("timeout", Some(timeout)));
        }
        if self.follow_redirects != before.follow_redirects {
            changes.push(("followRedirects", Some(Value::Bool(self.follow_redirects))));
        }
        if self.max_redirects != before.max_redirects {
            changes.push((
                "maxRedirects",
                Some(Value::from(u64::from(self.max_redirects))),
            ));
        }
        if self.forward_authorization_header != before.forward_authorization_header {
            changes.push((
                "forwardAuthorizationHeader",
                Some(Value::Bool(self.forward_authorization_header)),
            ));
        }
        if self.verify_tls != before.verify_tls {
            let verify = match self.verify_tls {
                TlsVerify::Inherit => None,
                TlsVerify::On => Some(Value::Bool(true)),
                TlsVerify::Off => Some(Value::Bool(false)),
            };
            changes.push(("verifySsl", verify));
        }
        if self.http_version != before.http_version {
            let version = self.http_version.map(|v| Value::from(v.to_string()));
            changes.push(("httpVersion", version));
        }
        changes
    }
}

/// Whether a saved `settings.<key>` node has the type the writer itself would produce, so it can
/// be put back as is rather than only read through a fallback.
pub(crate) fn well_typed(key: &str, node: &Value) -> bool {
    let boolean = || match node {
        Value::Bool(_) => true,
        Value::String(s) => s == "true" || s == "false",
        _ => false,
    };
    match key {
        "timeout" => {
            matches!(node, Value::Null | Value::Number(_)) || node.as_str() == Some("inherit")
        }
        "encodeUrl" | "followRedirects" | "forwardAuthorizationHeader" => boolean(),
        "maxRedirects" => node.as_f64().is_some_and(|n| n >= 0.0),
        "verifySsl" => matches!(node, Value::Bool(_)),
        "httpVersion" => node
            .as_str()
            .is_some_and(|s| s.parse::<HttpVersionPolicy>().is_ok()),
        _ => false,
    }
}

impl Default for RequestSettings {
    fn default() -> Self {
        RequestSettings::new()
    }
}

/// Where a new `settings` key goes: the top-level order, then the settings order.
pub(crate) const ORDER: &[&[&str]] = &[order::TOP, order::SETTINGS];

/// `settings.omitHeaders` as Bruno's `normalizeOmitHeaders` reads it: string entries, trimmed,
/// blanks dropped.
pub(crate) fn omitted_names(block: Option<&Value>) -> Vec<String> {
    block
        .and_then(|block| block.get("omitHeaders"))
        .and_then(Value::as_sequence)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(js_trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
