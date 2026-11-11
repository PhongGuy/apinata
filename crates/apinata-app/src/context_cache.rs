//! The actor's variable context, built on the first `Plan` or `Variables` after anything it
//! depends on changed, so request edits and saves never touch the keychain.

use std::sync::Arc;

use apinata_core::{
    Collection, ContextError, ContextOptions, EnvOverlay, GlobalEnvironments, OverlayLayer,
    RuntimeVars, SecretStore, TokenCache, VarContext,
};

/// The last built `VarContext`, kept until a command invalidates it. Every rebuild shares
/// the collection's runtime variables and script overlay, so captures and script-set
/// collection variables outlive edits and environment switches.
pub(crate) struct ContextCache {
    context: Option<VarContext>,
    runtime: RuntimeVars,
    overlay: EnvOverlay,
    tokens: TokenCache,
    globals: Option<(Arc<GlobalEnvironments>, String)>,
    generation: u64,
}

impl ContextCache {
    pub(crate) fn new(runtime: RuntimeVars, overlay: EnvOverlay, tokens: TokenCache) -> Self {
        ContextCache {
            context: None,
            runtime,
            overlay,
            tokens,
            globals: None,
            generation: 0,
        }
    }

    /// Drop the cached context and bump the generation; the next `get` rebuilds it.
    pub(crate) fn invalidate(&mut self) {
        self.context = None;
        self.generation += 1;
    }

    // Each change below invalidates, since the cached context is pinned to one environment.

    /// Point the overlay's unpinned handles at `name`'s script values.
    pub(crate) fn select_environment(&mut self, name: Option<&str>) {
        self.overlay.select_environment(name);
        self.invalidate();
    }

    /// Drop the script values set under the deleted environment `name`.
    pub(crate) fn forget_environment(&mut self, name: &str) {
        self.overlay.clear_environment(name);
        self.invalidate();
    }

    /// Carry environment `from`'s script values over to its new name `to`.
    pub(crate) fn rename_environment(&mut self, from: &str, to: &str) {
        self.overlay.rename_environment(from, to);
        self.invalidate();
    }

    /// Build every context with global environment `globals` (a snapshot and a name), or none.
    /// Script-set global values belong to the selected name, so only a switch drops them; a
    /// fresh snapshot of the same name keeps them.
    pub(crate) fn set_globals(&mut self, globals: Option<(Arc<GlobalEnvironments>, String)>) {
        let name = |g: &Option<(Arc<GlobalEnvironments>, String)>| g.as_ref().map(|g| g.1.clone());
        if name(&self.globals) != name(&globals) {
            self.overlay.clear_layer(OverlayLayer::Global);
        }
        self.globals = globals;
        self.invalidate();
    }

    /// How many times the context has been invalidated, for views that cache its variables.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// The context for `env` in `c`, reading secrets from `store` only when none is cached.
    /// A failed build is not cached, so the next call tries again.
    pub(crate) fn get(
        &mut self,
        c: &Collection,
        env: Option<&str>,
        store: &Arc<dyn SecretStore>,
    ) -> Result<VarContext, ContextError> {
        if let Some(context) = &self.context {
            return Ok(context.clone());
        }
        let context = VarContext::new(c, self.options(env, store))?;
        self.context = Some(context.clone());
        Ok(context)
    }

    /// The options every context of this collection is built with, for `env` and `store`.
    pub(crate) fn options(
        &self,
        env: Option<&str>,
        store: &Arc<dyn SecretStore>,
    ) -> ContextOptions {
        let options = ContextOptions::new()
            .secrets(Arc::clone(store))
            .runtime_vars(self.runtime.clone())
            .env_overlay(self.overlay.clone())
            .oauth2_tokens(self.tokens.clone());
        let options = match &self.globals {
            Some((globals, name)) => options.global_env(Arc::clone(globals), name.clone()),
            None => options,
        };
        match env {
            Some(name) => options.env(name),
            None => options,
        }
    }
}
