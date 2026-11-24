//! Single sign-on to a team server. `begin_sso` holds a loopback port and makes the PKCE pair
//! and state before any URL exists; the shell opens `PendingSso::url` in the browser, and
//! `finish` reads the browser's return, redeems the code once and keeps the session.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use url::Url;

use super::client::api_url;
use super::session::keep_session;
use super::{Account, TeamApi, TeamError};
use crate::loopback::{ListenError, Listener};
use crate::pkce::{Pkce, random_b64url};
use crate::{CancelHandle, SecretStore};

/// How long a sign-in waits for the browser to come back.
pub const SSO_TIMEOUT: Duration = Duration::from_secs(300);

/// A single sign-on waiting for the browser: a bound loopback port, the start URL to open, and
/// the state and verifier only this value knows.
#[non_exhaustive]
pub struct PendingSso {
    listener: Listener,
    url: String,
    state: String,
    verifier: String,
}

impl fmt::Debug for PendingSso {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingSso")
            .field("port", &self.listener.port())
            .finish_non_exhaustive()
    }
}

/// Binds the loopback listener and builds the start URL on the address `api` talks to.
/// `SsoListen` when no loopback port can be held or the system has no randomness.
pub async fn begin_sso(api: &dyn TeamApi) -> Result<PendingSso, TeamError> {
    let no_randomness = |_| TeamError::SsoListen("the system random source failed".into());
    let pkce = Pkce::generate().map_err(no_randomness)?;
    let state = random_b64url().map_err(no_randomness)?;
    let (listener, loopback) = bind().await?;
    let mut url = api_url(api.server(), &["oidc", "start"])?;
    url.query_pairs_mut()
        .append_pair("port", &listener.port().to_string())
        .append_pair("state", &state)
        .append_pair("challenge", &pkce.challenge)
        .append_pair("challenge_method", "S256")
        .append_pair("loopback", loopback);
    Ok(PendingSso {
        listener,
        url: url.into(),
        state,
        verifier: pkce.verifier,
    })
}

/// IPv4 loopback first, else IPv6, with the `loopback` value the server redirects by.
async fn bind() -> Result<(Listener, &'static str), TeamError> {
    let callback = |host: &str| {
        Url::parse(&format!("http://{host}/callback"))
            .map_err(|e| TeamError::SsoListen(e.to_string()))
    };
    let v4 = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    if let Ok(listener) = Listener::bind(v4, &callback("127.0.0.1")?).await {
        return Ok((listener, "v4"));
    }
    let v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, 0));
    match Listener::bind(v6, &callback("[::1]")?).await {
        Ok(listener) => Ok((listener, "v6")),
        Err(e) => Err(listen_failed(e)),
    }
}

fn listen_failed(e: ListenError) -> TeamError {
    match e {
        ListenError::Bind { address, message } => {
            TeamError::SsoListen(format!("{address}: {message}"))
        }
        ListenError::Timeout | ListenError::Cancelled => TeamError::SsoTimeout,
    }
}

impl PendingSso {
    /// The start URL to open in the browser, or to show for copying.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Waits up to `timeout` for the browser, checks its state before anything else, redeems
    /// the code once and keeps the session as a password sign-in does. A timeout or `cancel`
    /// is `SsoTimeout`; the port is closed and nothing is kept on any failure.
    pub async fn finish(
        self,
        api: &dyn TeamApi,
        secrets: &dyn SecretStore,
        team_file: &Path,
        timeout: Duration,
        cancel: &CancelHandle,
    ) -> Result<Account, TeamError> {
        let PendingSso {
            listener,
            state,
            verifier,
            ..
        } = self;
        let pairs = listener
            .wait(timeout, cancel)
            .await
            .map_err(listen_failed)?;
        let value = |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        // A return this app did not start is dropped before its code or error is looked at.
        if value("state") != Some(state.as_str()) {
            return Err(TeamError::SsoStateMismatch);
        }
        if let Some(error) = value("error") {
            return Err(match error {
                "account_not_linked" => TeamError::AccountNotLinked,
                "oidc_expired" => TeamError::SsoExpired,
                _ => TeamError::SsoRefused,
            });
        }
        let Some(code) = value("code") else {
            return Err(TeamError::SsoRefused);
        };
        let signed = api.redeem_sso(code, &verifier).await?;
        // The code is spent either way, so a cancel during the redeem gives the token back.
        if cancel.is_cancelled() {
            let _ = api.revoke(&signed.token, &signed.token_id).await;
            return Err(TeamError::SsoTimeout);
        }
        keep_session(api, secrets, team_file, signed).await
    }
}
