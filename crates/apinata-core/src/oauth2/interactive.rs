//! The browser sign-in for the authorization code and implicit grants, up to the token map
//! that goes into the cache.

use super::authorize::{Pkce, Redirect, authorization_url, read_callback, read_pasted, state_for};
use super::client::{Grant, request_token};
use super::config::TokenConfig;
use super::listener::{CallbackPlan, Listener};
use super::{AuthorizationPrompt, AuthorizeCancelled, Authorizer, OAuth2Error};
use crate::cancel::CancelHandle;
use crate::context::VarContext;
use crate::edit::OAuth2Flow;
use crate::execute::{Executor, SendTag};
use crate::request::RequestFile;
use serde_json::{Map, Value};
use std::time::Duration;

/// Opens the authorization URL through `authorizer`, reads the redirect from the loopback
/// listener or the user's paste, and returns the token: the implicit fragment's fields, or the
/// code exchange's answer. Every settings or bind error is raised before the browser opens.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn sign_in(
    exec: &Executor,
    cfg: &TokenConfig,
    authorizer: &dyn Authorizer,
    req: &RequestFile,
    ctx: &VarContext,
    cancel: &CancelHandle,
    timeout: Duration,
    tag: &SendTag,
) -> Result<Map<String, Value>, OAuth2Error> {
    let implicit = cfg.flow == OAuth2Flow::Implicit;
    cfg.require_sign_in()?;
    let state = state_for(&cfg.state)?;
    let pkce = if cfg.pkce {
        Some(Pkce::generate()?)
    } else {
        None
    };
    // Implicit always pastes: its token comes back in a fragment, which never reaches a server.
    let plan = if implicit {
        CallbackPlan::Paste {
            redirect_uri: cfg.callback_url.clone(),
        }
    } else {
        CallbackPlan::for_callback(&cfg.callback_url)
    };
    let (listener, redirect_uri) = match plan {
        CallbackPlan::Listen { bind, redirect } => {
            let listener = Listener::bind(bind, &redirect).await?;
            let redirect_uri = listener.redirect_uri().to_owned();
            (Some(listener), redirect_uri)
        }
        CallbackPlan::Paste { redirect_uri } => (None, redirect_uri),
    };
    let challenge = pkce.as_ref().map(|p| p.challenge.as_str());
    let url = authorization_url(cfg, &redirect_uri, &state, challenge)?;
    let prompt = AuthorizationPrompt::new(url, listener.is_none(), redirect_uri.clone());

    let redirect = match listener {
        Some(listener) => {
            // Poll the authorizer beside the listener so the user can cancel while it waits;
            // once the authorizer reports the browser opened, only the listener remains.
            let wait = listener.wait(timeout, cancel);
            tokio::pin!(wait);
            let mut opened = Some(authorizer.authorize(prompt));
            let pairs = loop {
                match opened.as_mut() {
                    Some(fut) => tokio::select! {
                        got = &mut wait => break got?,
                        answer = fut => match answer {
                            Err(AuthorizeCancelled) => return Err(OAuth2Error::Cancelled),
                            Ok(_) => opened = None,
                        },
                    },
                    None => break (&mut wait).await?,
                }
            };
            Redirect::Code(read_callback(&pairs, &state, &cfg.secrets)?)
        }
        None => {
            let pasted = match cancel.run(authorizer.authorize(prompt)).await {
                None | Some(Err(AuthorizeCancelled)) => return Err(OAuth2Error::Cancelled),
                Some(Ok(None)) => {
                    return Err(OAuth2Error::InvalidRedirect {
                        reason: "no URL was pasted",
                    });
                }
                Some(Ok(Some(pasted))) => pasted,
            };
            read_pasted(&pasted, &state, implicit, &cfg.secrets)?
        }
    };

    match redirect {
        Redirect::Token(token) => Ok(token),
        Redirect::Code(code) => {
            let grant = Grant::AuthorizationCode {
                code: &code,
                redirect_uri: &redirect_uri,
                verifier: pkce.as_ref().map(|p| p.verifier.as_str()),
            };
            request_token(exec, cfg, grant, req, ctx, cancel, tag).await
        }
    }
}
