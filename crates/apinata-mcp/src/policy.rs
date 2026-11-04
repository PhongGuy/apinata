//! Host policy for sends and runs: the collection's trusted and denied hosts, read from the
//! per-user trust file the app writes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use apinata_core::{ApprovalInbox, ExecError, Filing, TrustStore, display_host, normalize_host};
use schemars::JsonSchema;
use serde::Serialize;

use crate::redact::Redactor;
use crate::send;

/// What `list_trusted_hosts` reports for the served collection.
#[derive(Debug, Serialize, JsonSchema, PartialEq)]
pub(crate) struct TrustedHosts {
    /// Hosts a request may be sent to without approval.
    pub trusted: Vec<String>,
    /// Hosts the user refused; requests to them are refused without filing an approval.
    pub denied: Vec<String>,
}

/// How a refused hop reached the host that was judged.
#[derive(Clone, Copy)]
enum Route<'a> {
    Direct,
    /// Through this proxy, which may be the refused host itself.
    Proxy(&'a str),
    /// The host is the address a host override maps this name to.
    Mapped(&'a str),
}

/// The trust file as last read, shared by the tools and the executor's host-policy closure.
#[derive(Clone)]
pub(crate) struct HostTrust {
    root: PathBuf,
    path: Option<PathBuf>,
    store: Arc<RwLock<TrustStore>>,
    // Hosts this server filed approvals for; a fresh filing for one means the earlier request
    // was dismissed or expired, since the inbox keeps no record of either.
    filed: Arc<Mutex<HashSet<String>>>,
}

impl HostTrust {
    /// Read the trust file at `path` once for the collection at `root`; no path trusts nothing.
    pub(crate) fn new(root: &Path, path: Option<&Path>) -> HostTrust {
        let path = path.map(Path::to_path_buf);
        let store = Arc::new(RwLock::new(read(path.as_deref())));
        HostTrust {
            root: root.to_path_buf(),
            path,
            store,
            filed: Arc::default(),
        }
    }

    /// Re-read the trust file; an unreadable or malformed one counts as empty.
    pub(crate) fn refresh(&self) {
        let fresh = read(self.path.as_deref());
        // A host the user decided on is no longer awaiting a dismissed request.
        let decided = [fresh.trusted(&self.root), fresh.denied(&self.root)].concat();
        self.filed_hosts().retain(|host| !decided.contains(host));
        *self.store.write().unwrap_or_else(|e| e.into_inner()) = fresh;
    }

    /// Whether `host` is trusted for the served collection, as last read.
    pub(crate) fn allows(&self, host: &str) -> bool {
        let store = self.store.read().unwrap_or_else(|e| e.into_inner());
        store.is_trusted(&self.root, host)
    }

    /// The host-policy closure for the executor; it follows each later `refresh`.
    pub(crate) fn policy(&self) -> Arc<dyn Fn(&str) -> bool + Send + Sync> {
        let trust = self.clone();
        Arc::new(move |host: &str| trust.allows(host))
    }

    /// The served collection's trusted and denied hosts, as last read.
    pub(crate) fn lists(&self) -> TrustedHosts {
        let store = self.store.read().unwrap_or_else(|e| e.into_inner());
        TrustedHosts {
            trusted: store.trusted(&self.root),
            denied: store.denied(&self.root),
        }
    }

    /// The answer to a refused host, or to the proxy a hop went through when `proxy` is set:
    /// files an approval unless the host is denied, pending, names no host, or holds a secret.
    fn untrusted(
        &self,
        hop: usize,
        host: &str,
        route: Route<'_>,
        request: &str,
        inbox: Option<&ApprovalInbox>,
        redactor: &Redactor,
    ) -> String {
        if normalize_host(host).is_none() {
            return format!("hop {hop} names no host that can be trusted; nothing was sent");
        }
        // A host built from a secret is never filed: the inbox would keep the secret.
        let shown = redactor.scrub_host(host);
        if shown != *host {
            return format!(
                "host `{shown}` (hop {hop}) holds a secret; nothing was sent and no \
                 approval was requested"
            );
        }
        // A refused proxy is named as the proxy; a host merely reached through one keeps the route.
        let proxy = match route {
            Route::Proxy(proxy) => Some(proxy),
            _ => None,
        };
        let shown_host = display_host(host);
        let subject = match route {
            Route::Proxy(proxy) if normalize_host(proxy).as_deref() == Some(host) => {
                format!("proxy host `{host}` (hop {hop})")
            }
            Route::Proxy(proxy) => format!("host `{host}` (hop {hop}, via proxy {proxy})"),
            // The approval is for the address the name is mapped to, whatever the name is.
            Route::Mapped(name) => format!(
                "address `{shown_host}` (hop {hop}), which a host override maps `{}` to,",
                redactor.scrub_host(name)
            ),
            Route::Direct => format!("host `{shown_host}` (hop {hop})"),
        };
        // The name is context for the person approving; a name the redactor would change
        // might hold a secret, so it is not filed at all.
        let mapped = match route {
            Route::Mapped(name) if redactor.scrub_host(name) == name => Some(name),
            _ => None,
        };
        let untrusted = format!("{subject} is not trusted for this collection");
        let filed = inbox.map(|inbox| {
            let store = self.store.read().unwrap_or_else(|e| e.into_inner());
            match mapped {
                Some(name) => inbox.request_mapped(&store, &self.root, host, request, name),
                None => inbox.request_via(&store, &self.root, host, request, proxy),
            }
        });
        match filed {
            Some(Ok(Filing::Denied)) => {
                self.filed_hosts().remove(host);
                format!("{subject} is denied for this collection; nothing was sent")
            }
            Some(Ok(Filing::Written(_))) if !self.filed_hosts().insert(host.to_owned()) => {
                format!(
                    "{untrusted}; nothing was sent. The user dismissed the earlier \
                     approval request without deciding (or it expired), so the host is \
                     still not approved; you may ask again: a new approval request was \
                     filed in the Apinata app; retry after the user approves."
                )
            }
            Some(Ok(_)) => {
                self.filed_hosts().insert(host.to_owned());
                format!(
                    "{untrusted}; nothing was sent. Approval requested in the Apinata \
                     app; retry after the user approves."
                )
            }
            Some(Err(e)) => not_filed(&untrusted, &e.to_string(), host),
            None => not_filed(&untrusted, "no approval directory", host),
        }
    }

    /// The answer to a fetch the host policy refused at redirect `hop`. It reads the trust file
    /// afresh like a send, then files the same approval under `label` unless the host is denied;
    /// without an inbox it is the usual "no approval directory" refusal.
    pub(crate) fn refuse_fetch(
        &self,
        host: &str,
        hop: usize,
        label: &str,
        inbox: Option<&ApprovalInbox>,
    ) -> String {
        self.refresh();
        let message = self.untrusted(hop, host, Route::Direct, label, inbox, &Redactor::default());
        // The approval is per host, not per tool, so the user should know what it unlocks.
        if message.contains("retry after the user approves") {
            format!("{message} Approving this host also lets send_request reach it.")
        } else {
            message
        }
    }

    fn filed_hosts(&self) -> MutexGuard<'_, HashSet<String>> {
        self.filed.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The answer to item `request`'s failed send. A refused host or proxy files an approval
    /// unless the host is denied, already pending, names no host, or holds a value `redactor`
    /// scrubs.
    pub(crate) fn refusal(
        &self,
        err: &ExecError,
        request: &str,
        inbox: Option<&ApprovalInbox>,
        redactor: &Redactor,
    ) -> String {
        match err {
            ExecError::HostNotAllowed {
                hop,
                host,
                mapped_from,
                ..
            } => {
                let route = mapped_from.as_deref().map_or(Route::Direct, Route::Mapped);
                self.untrusted(*hop, host, route, request, inbox, redactor)
            }
            ExecError::ProxyNotAllowed {
                hop, host, proxy, ..
            } => self.untrusted(*hop, host, Route::Proxy(proxy), request, inbox, redactor),
            ExecError::HostOverrideViaProxy { hop, host, .. } => format!(
                "host `{host}` (hop {hop}) has a host override but goes through a proxy, which \
                 ignores it; nothing was sent. Turn the proxy off for this collection or remove \
                 the override."
            ),
            ExecError::SecretInHost { host, .. } => format!(
                "a secret cannot be used in a host: `{host}`; nothing was sent. Move the host to \
                 a non-secret environment variable. Any secret, .env or process.env value found \
                 in the host text counts, however short."
            ),
            ExecError::SecretInProxy => "a secret cannot be used in a proxy host or port; nothing \
                was sent. Move the proxy host to a non-secret variable."
                .to_owned(),
            other => send::exec_message(other),
        }
    }
}

fn not_filed(untrusted: &str, why: &str, host: &str) -> String {
    format!(
        "{untrusted}; nothing was sent. The approval request could not be filed ({why}); the \
         user can run `apinata trust add {host}`."
    )
}

// A trust file the server cannot read must not stop it serving: it trusts nothing instead, and
// says so on stderr, which the MCP client keeps out of the conversation.
fn read(path: Option<&Path>) -> TrustStore {
    let Some(path) = path else {
        return TrustStore::empty(PathBuf::new());
    };
    TrustStore::load(path).unwrap_or_else(|e| {
        eprintln!("apinata mcp: trusting no hosts: {e}");
        TrustStore::empty(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn trust_file(dir: &Path, root: &Path, trusted: &[&str], denied: &[&str]) -> PathBuf {
        let path = dir.join("trusted-hosts.json");
        let mut store = TrustStore::empty(&path);
        for host in trusted {
            store.trust(root, host).expect("trust");
        }
        for host in denied {
            store.deny(root, host).expect("deny");
        }
        store.save().expect("save");
        path
    }

    fn refuse(trust: &HostTrust, inbox: &ApprovalInbox) -> String {
        let err = ExecError::host_not_allowed(0, "api.example.com");
        trust.refusal(&err, "r.yml", Some(inbox), &Redactor::default())
    }

    #[test]
    fn a_refused_proxy_files_an_approval_that_names_it() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let inbox = ApprovalInbox::new(state.path().join("approvals"));
        let trust = HostTrust::new(root.path(), Some(&state.path().join("trusted-hosts.json")));
        let err =
            ExecError::proxy_not_allowed(0, "proxy.corp.example", "http://proxy.corp.example:3128");
        let message = trust.refusal(&err, "r.yml", Some(&inbox), &Redactor::default());
        assert!(
            message.contains("proxy host `proxy.corp.example` (hop 0) is not trusted"),
            "{message}"
        );
        assert!(message.contains("Approval requested"), "{message}");
        let pending = inbox.pending().unwrap();
        assert_eq!(
            pending[0].proxy.as_deref(),
            Some("http://proxy.corp.example:3128")
        );
    }

    #[test]
    fn a_refused_host_behind_a_proxy_keeps_the_route() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let trust = HostTrust::new(root.path(), Some(&state.path().join("trusted-hosts.json")));
        let err =
            ExecError::proxy_not_allowed(1, "api.example.com", "http://proxy.corp.example:3128");
        let message = trust.refusal(&err, "r.yml", None, &Redactor::default());
        assert!(
            message.contains(
                "host `api.example.com` (hop 1, via proxy http://proxy.corp.example:3128) is not \
                 trusted"
            ),
            "{message}"
        );
    }

    #[test]
    fn a_host_trusted_since_filing_is_not_reported_as_dismissed() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let path = state.path().join("trusted-hosts.json");
        let inbox = ApprovalInbox::new(state.path().join("approvals"));
        let trust = HostTrust::new(root.path(), Some(&path));
        assert!(!refuse(&trust, &inbox).contains("dismissed"));

        // The user trusts it (the app deletes the request), the next tool call refreshes,
        // then the user revokes it; no send passed the policy in between.
        inbox.resolve(root.path(), "api.example.com").unwrap();
        trust_file(state.path(), root.path(), &["api.example.com"], &[]);
        trust.refresh();
        TrustStore::empty(&path).save().unwrap();
        trust.refresh();

        let message = refuse(&trust, &inbox);
        assert!(!message.contains("dismissed"), "{message}");
        assert!(
            message.contains("retry after the user approves"),
            "{message}"
        );
    }

    #[test]
    fn a_host_denied_then_undenied_is_not_reported_as_dismissed() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let path = state.path().join("trusted-hosts.json");
        let inbox = ApprovalInbox::new(state.path().join("approvals"));
        let trust = HostTrust::new(root.path(), Some(&path));
        refuse(&trust, &inbox);

        // Denied in the app, refused as denied, then the deny is removed.
        inbox.resolve(root.path(), "api.example.com").unwrap();
        trust_file(state.path(), root.path(), &[], &["api.example.com"]);
        trust.refresh();
        assert!(refuse(&trust, &inbox).contains("denied"));
        TrustStore::empty(&path).save().unwrap();
        trust.refresh();

        let message = refuse(&trust, &inbox);
        assert!(!message.contains("dismissed"), "{message}");
    }

    #[test]
    fn a_malformed_trust_file_counts_as_empty() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let path = state.path().join("trusted-hosts.json");
        std::fs::write(&path, "{ not json").unwrap();
        let trust = HostTrust::new(root.path(), Some(&path));
        assert_eq!(
            trust.lists(),
            TrustedHosts {
                trusted: vec![],
                denied: vec![]
            }
        );
    }

    #[test]
    fn lists_are_per_collection() {
        let (state, root, other) = (
            TempDir::new().unwrap(),
            TempDir::new().unwrap(),
            TempDir::new().unwrap(),
        );
        let path = trust_file(state.path(), root.path(), &["a.example"], &["evil.example"]);
        let mut store = TrustStore::load(&path).unwrap();
        store.trust(other.path(), "b.example").unwrap();
        store.save().unwrap();
        let trust = HostTrust::new(root.path(), Some(&path));
        assert_eq!(
            trust.lists(),
            TrustedHosts {
                trusted: vec!["a.example".into()],
                denied: vec!["evil.example".into()]
            }
        );
    }

    #[test]
    fn the_policy_follows_the_file_after_each_refresh() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let trust = HostTrust::new(root.path(), Some(&state.path().join("trusted-hosts.json")));
        let policy = trust.policy();
        assert!(!policy("api.example.com"));
        trust_file(state.path(), root.path(), &["api.example.com"], &[]);
        assert!(!policy("api.example.com"), "not re-read until refreshed");
        trust.refresh();
        assert!(policy("api.example.com"));
        assert!(!policy("other.example.com"));
    }

    #[test]
    fn a_fetch_to_a_trusted_host_passes_the_policy() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let path = trust_file(state.path(), root.path(), &["api.example.com"], &[]);
        let trust = HostTrust::new(root.path(), Some(&path));
        assert!((trust.policy())("api.example.com"));
    }

    #[test]
    fn an_untrusted_fetch_files_an_approval_labelled_import_collection() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let inbox = ApprovalInbox::new(state.path().join("approvals"));
        let trust = HostTrust::new(root.path(), Some(&state.path().join("trusted-hosts.json")));
        let message = trust.refuse_fetch("specs.example.com", 2, "import_collection", Some(&inbox));
        assert!(message.contains("`specs.example.com` (hop 2)"), "{message}");
        assert!(message.contains("Approval requested"), "{message}");
        assert!(message.contains("send_request"), "{message}");
        let pending = inbox.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].host, "specs.example.com");
        assert_eq!(pending[0].request, "import_collection");
    }

    #[test]
    fn a_fetch_sees_a_host_trusted_since_the_last_read() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let trust = HostTrust::new(root.path(), Some(&state.path().join("trusted-hosts.json")));
        trust_file(state.path(), root.path(), &["specs.example.com"], &[]);
        // Refusing refreshes first, so the policy now allows what the stale read refused.
        trust.refuse_fetch("specs.example.com", 0, "import_collection", None);
        assert!((trust.policy())("specs.example.com"));
    }

    #[test]
    fn a_denied_fetch_host_is_refused_without_filing() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let path = trust_file(state.path(), root.path(), &[], &["evil.example"]);
        let inbox = ApprovalInbox::new(state.path().join("approvals"));
        let trust = HostTrust::new(root.path(), Some(&path));
        for _ in 0..2 {
            let message = trust.refuse_fetch("evil.example", 0, "import_collection", Some(&inbox));
            assert!(message.contains("denied"), "{message}");
        }
        assert!(inbox.pending().unwrap().is_empty());
    }

    #[test]
    fn a_fetch_without_an_inbox_is_refused_as_unfiled() {
        let (state, root) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let trust = HostTrust::new(root.path(), Some(&state.path().join("trusted-hosts.json")));
        let message = trust.refuse_fetch("specs.example.com", 1, "import_collection", None);
        assert!(message.contains("no approval directory"), "{message}");
        assert!(message.contains("(hop 1)"), "{message}");
    }
}
