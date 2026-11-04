//! `bru.resetOauth2Credential`: forgets one credentials id's cached tokens, wrapped by
//! `oauth2.js`.

use std::sync::Arc;

use rquickjs::Function;
use rquickjs::prelude::Async;

use super::engine::Installer;
use super::host::HostState;

const OAUTH2_JS: &str = include_str!("oauth2.js");

/// Installs `__resetOauth2` and `oauth2.js`, which builds `bru.resetOauth2Credential` over it.
pub(crate) fn installer(state: Arc<HostState>) -> Installer {
    Box::new(move |ctx| {
        // Forget in memory before the future is built, so the script's next line already sees
        // the id gone; the future persists the clear and the phase waits for it.
        let host = move |id: String| {
            let ctx = &state.ctx;
            let (root, env) = (ctx.key_root(), ctx.environment().map(str::to_owned));
            let tokens = ctx.oauth2_tokens().clone();
            tokens.forget(&root, env.as_deref(), &id);
            async move {
                tokens.clear(&root, env.as_deref(), &id).await;
                Ok::<_, rquickjs::Error>(())
            }
        };
        ctx.globals()
            .set("__resetOauth2", Function::new(ctx.clone(), Async(host))?)?;
        ctx.eval::<(), _>(OAUTH2_JS)
    })
}
