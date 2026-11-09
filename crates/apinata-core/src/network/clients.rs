//! One reqwest client per proxy rule and identity, so connection pools never cross rules.

use std::sync::{Arc, Mutex};

use crate::execute::ExecError;

use super::hosts::ResolvedHostOverride;
use super::route::ProxyRule;

/// Which client a request needs: the rule and the identity's PEM bytes, so an edited file builds anew.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ClientKey {
    pub(crate) rule: ProxyRule,
    pub(crate) identity: Option<Arc<[u8]>>,
    /// False when the request omits Accept-Encoding: the client then neither asks for nor
    /// decodes compressed bodies, so it adds no header of its own.
    pub(crate) compress: bool,
    /// The host overrides the client resolves by, ordered by host, so a changed mapping builds a
    /// new client rather than reusing a pooled connection to the old address.
    pub(crate) overrides: Arc<Vec<ResolvedHostOverride>>,
    /// True for a WebSocket handshake: the client offers only HTTP/1.1 whatever the policy, since an
    /// h2 connection cannot be upgraded.
    pub(crate) http1_only: bool,
}

/// Most recently used first; at most `CAPACITY`. Evicting only costs a rebuild and a new connection.
#[derive(Default)]
pub(crate) struct Clients {
    built: Mutex<Vec<(ClientKey, reqwest::Client)>>,
}

const CAPACITY: usize = 16;

impl Clients {
    /// The client for `key`, built with `build` when none is cached.
    pub(crate) fn get(
        &self,
        key: &ClientKey,
        build: impl FnOnce(&ClientKey) -> Result<reqwest::Client, ExecError>,
    ) -> Result<reqwest::Client, ExecError> {
        let mut built = self.built.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = built.iter().position(|(k, _)| k == key) {
            let entry = built.remove(at);
            let client = entry.1.clone();
            built.insert(0, entry);
            return Ok(client);
        }
        let client = build(key)?;
        built.insert(0, (key.clone(), client.clone()));
        built.truncate(CAPACITY);
        Ok(client)
    }
}
