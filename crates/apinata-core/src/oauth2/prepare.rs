//! `Executor::prepare_auth`: make sure a request's oauth2 token is in the cache before it is
//! resolved, following its decision table; plus the token panel's Get token and Refresh.

use super::cache::{Entry, Flight};
use super::client::{Grant, request_token};
use super::config::TokenConfig;
use super::interactive::sign_in;
use super::listener::CALLBACK_TIMEOUT;
use super::target::{OAuth2Target, Probe, TokenStatus};
use super::{AuthorizeOptions, OAuth2Error};
use crate::cancel::CancelHandle;
use crate::context::VarContext;
use crate::edit::OAuth2Flow;
use crate::execute::{Executor, SendTag};
use crate::request::RequestFile;
use crate::resolve::effective_oauth2;
use std::time::SystemTime;

impl Executor {
    /// No-op unless the effective auth is oauth2; leaves a usable token in the cache or fails.
    ///
    /// An expired token kept by the auth's settings counts as usable: resolving warns about it.
    pub async fn prepare_auth(
        &self,
        req: &RequestFile,
        ctx: &VarContext,
        how: &AuthorizeOptions,
        cancel: &CancelHandle,
    ) -> Result<(), OAuth2Error> {
        self.prepare_auth_as(req, ctx, how, cancel, &SendTag::default())
            .await
    }

    /// [`Executor::prepare_auth`], with a token fetch recorded into `tag`'s hold when it has one.
    pub(crate) async fn prepare_auth_as(
        &self,
        req: &RequestFile,
        ctx: &VarContext,
        how: &AuthorizeOptions,
        cancel: &CancelHandle,
        tag: &SendTag,
    ) -> Result<(), OAuth2Error> {
        let Some(spec) = effective_oauth2(req, ctx) else {
            return Ok(());
        };
        let cfg = TokenConfig::new(&spec, req, ctx)?;
        let cache = ctx.oauth2_tokens();
        cache.load().await;
        let key = cfg.key(ctx);
        let fingerprint = cfg.fingerprint();
        // Hold the key's lock from lookup to store, so a concurrent caller for the same
        // credentials waits and then finds the token, or the sign-in failure, this one left.
        let flight = cache.flight(&key);
        let arrived = flight.arrive();
        let Some(mut failed) = cancel.run(flight.lock.lock()).await else {
            return Err(OAuth2Error::Cancelled);
        };
        let cached = if how.must_refetch(&key) {
            None
        } else {
            cache.lookup(&key, &fingerprint)
        };
        let Some(mut entry) = cached else {
            if !cfg.auto_fetch || how.cache_only {
                return Err(OAuth2Error::NoToken {
                    credentials_id: cfg.credentials_id,
                });
            }
            self.fetch_in_flight(
                &cfg,
                req,
                ctx,
                how,
                cancel,
                &flight,
                &mut failed,
                arrived,
                tag,
            )
            .await?;
            how.fetched(key);
            return Ok(());
        };
        // Cache-only sends an expired token too; resolving warns about it.
        if how.cache_only || !entry.is_expired(SystemTime::now()) {
            return Ok(());
        }

        let refresh_token = entry
            .field("refresh_token")
            .filter(|t| !t.is_empty())
            .map(str::to_owned);
        if let Some(refresh_token) = refresh_token.filter(|_| cfg.auto_refresh) {
            match self
                .refresh(&cfg, req, ctx, &refresh_token, cancel, tag)
                .await
            {
                Ok(_) => return Ok(()),
                // Only the provider's answer feeds the table; a cancel or a refused or
                // unreachable host is reported with the cache untouched.
                Err(e @ (OAuth2Error::Cancelled | OAuth2Error::Exec(_))) => return Err(e),
                Err(_) if cfg.auto_fetch => cache.remove(&key).await,
                Err(_) => {
                    // Keep the expired token for this send, but never retry the refresh.
                    entry.token.remove("refresh_token");
                    cache.store(entry).await;
                    return Ok(());
                }
            }
        }
        if cfg.auto_fetch {
            self.fetch_in_flight(
                &cfg,
                req,
                ctx,
                how,
                cancel,
                &flight,
                &mut failed,
                arrived,
                tag,
            )
            .await
            .map(drop)
        } else {
            Ok(())
        }
    }

    /// Ignores the cache and runs the auth's grant (interactive ones through `how`'s authorizer).
    pub async fn fetch_token(
        &self,
        target: OAuth2Target<'_>,
        ctx: &VarContext,
        how: &AuthorizeOptions,
        cancel: &CancelHandle,
    ) -> Result<TokenStatus, OAuth2Error> {
        let req = Probe::new(target, ctx);
        let cfg = req.config(ctx)?;
        let cache = ctx.oauth2_tokens();
        cache.load().await;
        let flight = cache.flight(&cfg.key(ctx));
        let arrived = flight.arrive();
        let Some(mut failed) = cancel.run(flight.lock.lock()).await else {
            return Err(OAuth2Error::Cancelled);
        };
        let entry = self
            .fetch_in_flight(
                &cfg,
                &req,
                ctx,
                how,
                cancel,
                &flight,
                &mut failed,
                arrived,
                &SendTag::default(),
            )
            .await?;
        Ok(TokenStatus::of(&entry))
    }

    /// Refreshes the cached entry now; `NoToken` without one or without a refresh token.
    /// A failure leaves the cache as it was.
    pub async fn refresh_token(
        &self,
        target: OAuth2Target<'_>,
        ctx: &VarContext,
        cancel: &CancelHandle,
    ) -> Result<TokenStatus, OAuth2Error> {
        let req = Probe::new(target, ctx);
        let cfg = req.config(ctx)?;
        let cache = ctx.oauth2_tokens();
        cache.load().await;
        let key = cfg.key(ctx);
        let flight = cache.flight(&key);
        let Some(_held) = cancel.run(flight.lock.lock()).await else {
            return Err(OAuth2Error::Cancelled);
        };
        let refresh_token = cache
            .lookup(&key, &cfg.fingerprint())
            .and_then(|e| e.field("refresh_token").map(str::to_owned))
            .filter(|t| !t.is_empty());
        let Some(refresh_token) = refresh_token else {
            return Err(OAuth2Error::NoToken {
                credentials_id: cfg.credentials_id,
            });
        };
        let entry = self
            .refresh(&cfg, &req, ctx, &refresh_token, cancel, &SendTag::default())
            .await?;
        Ok(TokenStatus::of(&entry))
    }

    /// Posts `refresh_token` and stores the answer, keeping the old refresh token when the
    /// provider issues none. A failure stores nothing.
    async fn refresh(
        &self,
        cfg: &TokenConfig,
        req: &RequestFile,
        ctx: &VarContext,
        refresh_token: &str,
        cancel: &CancelHandle,
        tag: &SendTag,
    ) -> Result<Entry, OAuth2Error> {
        let grant = Grant::Refresh { refresh_token };
        let mut token = request_token(self, cfg, grant, req, ctx, cancel, tag).await?;
        let reissued = token
            .get("refresh_token")
            .and_then(|t| t.as_str())
            .is_some_and(|t| !t.is_empty());
        if !reissued {
            token.insert("refresh_token".to_owned(), refresh_token.into());
        }
        let entry = Entry::new(cfg.key(ctx), cfg.fingerprint(), token, SystemTime::now());
        ctx.oauth2_tokens().store(entry.clone()).await;
        Ok(entry)
    }

    /// Fetches while holding `flight`, whose guarded slot is `failed`. A caller that queued
    /// behind a failed browser sign-in returns that failure instead of opening another, and a
    /// browser sign-in that fails here is left for the callers queued behind this one. A
    /// declined sign-in is shared too, but a caller whose own handle was cancelled drops out
    /// alone, so the next caller signs in.
    #[allow(clippy::too_many_arguments)]
    async fn fetch_in_flight(
        &self,
        cfg: &TokenConfig,
        req: &RequestFile,
        ctx: &VarContext,
        how: &AuthorizeOptions,
        cancel: &CancelHandle,
        flight: &Flight,
        failed: &mut Option<(u64, OAuth2Error)>,
        arrived: u64,
        tag: &SendTag,
    ) -> Result<Entry, OAuth2Error> {
        if let Some((n, e)) = &*failed
            && *n > arrived
        {
            return Err(e.clone());
        }
        let fetched = self.fetch(cfg, req, ctx, how, cancel, tag).await;
        if let Err(e) = &fetched
            && cfg.is_interactive()
            && how.authorizer.is_some()
            && !(matches!(e, OAuth2Error::Cancelled) && cancel.is_cancelled())
        {
            flight.fail(failed, e);
        }
        fetched
    }

    /// Obtain a new token with the auth's own grant and store it; a browser grant signs in
    /// through `how`'s authorizer and needs interaction without one.
    async fn fetch(
        &self,
        cfg: &TokenConfig,
        req: &RequestFile,
        ctx: &VarContext,
        how: &AuthorizeOptions,
        cancel: &CancelHandle,
        tag: &SendTag,
    ) -> Result<Entry, OAuth2Error> {
        let token = if cfg.is_interactive() {
            let Some(authorizer) = &how.authorizer else {
                return Err(OAuth2Error::InteractionRequired {
                    credentials_id: cfg.credentials_id.clone(),
                });
            };
            sign_in(
                self,
                cfg,
                authorizer.as_ref(),
                req,
                ctx,
                cancel,
                CALLBACK_TIMEOUT,
                tag,
            )
            .await?
        } else {
            cfg.require_token_request()?;
            let grant = if cfg.flow == OAuth2Flow::Password {
                Grant::Password
            } else {
                Grant::ClientCredentials
            };
            request_token(self, cfg, grant, req, ctx, cancel, tag).await?
        };
        let entry = Entry::new(cfg.key(ctx), cfg.fingerprint(), token, SystemTime::now());
        ctx.oauth2_tokens().store(entry.clone()).await;
        Ok(entry)
    }
}
