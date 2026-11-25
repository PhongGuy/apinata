//! Single sign-on to a team server. `begin_sso` holds a loopback port and makes the PKCE pair
//! and state before any URL exists; the shell opens `PendingSso::url` in the browser, and
//! `finish` reads the browser's return, or the address pasted through `SsoPaste` when the
//! browser cannot reach the loopback port, redeems the code once and keeps the session.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use aws_lc_rs::constant_time::verify_slices_are_equal;
use tokio::sync::oneshot;
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
    paste: SsoPaste,
    pasted: oneshot::Receiver<Vec<(String, String)>>,
}

/// The query pairs of the one answer that ends the wait.
type Answer = Vec<(String, String)>;

/// Hands `finish` the address the browser ended on, for when the browser cannot reach the
/// loopback port. Clones share one sign-in; only the first valid answer, pasted or from the
/// listener, is used.
#[derive(Clone)]
pub struct SsoPaste {
    gate: Arc<PasteGate>,
}

struct PasteGate {
    redirect: Url,
    state: String,
    /// Taken by whichever answer wins, so an empty slot means the wait is over.
    slot: Mutex<Option<oneshot::Sender<Answer>>>,
}

impl fmt::Debug for SsoPaste {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SsoPaste")
            .field("redirect", &self.gate.redirect.as_str())
            .finish_non_exhaustive()
    }
}

/// Why a pasted address was refused. The message never repeats what was pasted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PasteRefused {
    /// Not an address, or not the loopback address this sign-in waits on.
    NotThisAddress,
    /// No `state`, more than one, or not the one this sign-in made.
    WrongState,
    /// The address carries neither a code nor an error.
    NoCode,
    /// The sign-in is no longer waiting.
    Finished,
}

impl fmt::Display for PasteRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PasteRefused::NotThisAddress => "that is not the address this sign-in waits on",
            PasteRefused::WrongState => "that address belongs to another sign-in",
            PasteRefused::NoCode => "that address carries no sign-in code",
            PasteRefused::Finished => "the sign-in is no longer waiting",
        })
    }
}

impl std::error::Error for PasteRefused {}

impl SsoPaste {
    /// Checks `text` as the address the browser ended on and, when it is this sign-in's
    /// redirect with its state and a code or error, ends the wait with it. A refusal leaves
    /// the wait going; once the wait is over every call is `Finished`.
    pub fn submit(&self, text: &str) -> Result<(), PasteRefused> {
        let gate = &self.gate;
        let mut slot = gate.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.as_ref().is_none_or(oneshot::Sender::is_closed) {
            return Err(PasteRefused::Finished);
        }
        let answer = gate.check(text)?;
        match slot.take() {
            Some(sender) => sender.send(answer).map_err(|_| PasteRefused::Finished),
            None => Err(PasteRefused::Finished),
        }
    }
}

impl PasteGate {
    fn check(&self, text: &str) -> Result<Answer, PasteRefused> {
        let url = Url::parse(text.trim()).map_err(|_| PasteRefused::NotThisAddress)?;
        let want = &self.redirect;
        // Compared part by part, so `localhost` never stands in for the address bound.
        let same = url.scheme() == want.scheme()
            && url.username().is_empty()
            && url.password().is_none()
            && url.host() == want.host()
            && url.port_or_known_default() == want.port_or_known_default()
            && url.path() == want.path();
        if !same {
            return Err(PasteRefused::NotThisAddress);
        }
        let pairs: Answer = url.query_pairs().into_owned().collect();
        let mut states = pairs.iter().filter(|(k, _)| k == "state");
        let state_ok = match (states.next(), states.next()) {
            (Some((_, got)), None) => same_secret(got, &self.state),
            _ => false,
        };
        if !state_ok {
            return Err(PasteRefused::WrongState);
        }
        let has = |key: &str| pairs.iter().any(|(k, v)| k == key && !v.is_empty());
        if !has("error") && !has("code") {
            return Err(PasteRefused::NoCode);
        }
        Ok(pairs)
    }

    /// Claims the win for the listener; false when a paste already took it.
    fn close(&self) -> bool {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        slot.take().is_some()
    }
}

/// Compares two secrets in time that does not depend on where they differ.
fn same_secret(got: &str, want: &str) -> bool {
    verify_slices_are_equal(got.as_bytes(), want.as_bytes()).is_ok()
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
    let listener = listener.expect_state(state.clone());
    let redirect =
        Url::parse(listener.redirect_uri()).map_err(|e| TeamError::SsoListen(e.to_string()))?;
    let (sender, pasted) = oneshot::channel();
    let paste = SsoPaste {
        gate: Arc::new(PasteGate {
            redirect,
            state: state.clone(),
            slot: Mutex::new(Some(sender)),
        }),
    };
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
        paste,
        pasted,
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

    /// A handle for pasting the address the browser ended on while `finish` waits.
    pub fn paste_handle(&self) -> SsoPaste {
        self.paste.clone()
    }

    /// Waits up to `timeout` for the browser or a paste, whichever brings a valid answer first,
    /// checks its state before anything else, redeems the code once and keeps the session as a
    /// password sign-in does. A timeout or `cancel` is `SsoTimeout`; the port is closed and
    /// nothing is kept on any failure.
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
            paste,
            mut pasted,
            ..
        } = self;
        // A winning paste stops the listener through its own cancel, so the port still closes.
        let (stop_listening, guard) = cancel.child_until_dropped();
        let listen = listener.wait(timeout, &stop_listening);
        tokio::pin!(listen);
        let heard = tokio::select! {
            heard = &mut listen => {
                // A paste that claimed the win just before the listener answered still wins;
                // it sent while holding the slot, so its answer is already waiting.
                if paste.gate.close() {
                    heard
                } else {
                    pasted.try_recv().map_err(|_| ListenError::Cancelled)
                }
            }
            Ok(pairs) = &mut pasted => {
                drop(guard);
                let _ = listen.await;
                Ok(pairs)
            }
        };
        let pairs = heard.map_err(listen_failed)?;
        let value = |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        // The listener already waits past a return this app did not start; this guards it.
        if !value("state").is_some_and(|got| same_secret(got, &state)) {
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
