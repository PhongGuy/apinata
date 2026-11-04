//! The one encrypted OAuth 2.0 token cache every collection's sends and Copy as curl share.

use apinata_core::TokenCache;
use gpui_kit::{App, Global};

struct SharedTokens(TokenCache);

impl Global for SharedTokens {}

/// Makes `cache` the app's token cache.
pub fn install(cache: TokenCache, cx: &mut App) {
    cx.set_global(SharedTokens(cache));
}

/// The installed cache; installs an in-memory one on first use when none was.
pub fn shared(cx: &mut App) -> TokenCache {
    if let Some(tokens) = cx.try_global::<SharedTokens>() {
        return tokens.0.clone();
    }
    let cache = TokenCache::in_memory();
    install(cache.clone(), cx);
    cache
}
