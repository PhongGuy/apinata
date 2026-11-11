//! Loads a GraphQL request's schema into the shared store, from its server, a schema file or
//! the disk cache, and keeps each newest load in that cache.

use std::path::{Path, PathBuf};

use apinata_core::{
    CancelHandle, IntrospectError, ItemPath, OAuth2Error, ResolveError, Schema, SchemaError,
    fetch_introspection, introspection_request,
};
use gpui_kit::{App, AppContext as _, Entity, Window};

use crate::copy_curl::plan_and_resolve_with;
use crate::graphql_schema_store::{SchemaFailure, SchemaStore};
use crate::response_pane::transport_text;
use crate::workspace::Workspace;
use crate::{executor, tokio_bridge};

/// Resolves `item` as Send would, POSTs the introspection query with the schema executor and
/// stores the answer under `url`. The fetch runs on the app, so closing the tab never leaves
/// the entry loading; a closed window before the resolve finishes stores a failure.
pub fn load_from_server(
    workspace: &Entity<Workspace>,
    root: &Path,
    item: ItemPath,
    url: String,
    window: &mut Window,
    cx: &mut App,
) {
    let generation = cx.default_global::<SchemaStore>().begin(&url);
    let no_token =
        |e: &SchemaError| matches!(e, SchemaError::Resolve(ResolveError::OAuth2NoToken { .. }));
    let planned = plan_and_resolve_with(
        workspace,
        root,
        item,
        introspection_request,
        no_token,
        window,
        cx,
    );
    cx.spawn(async move |cx| {
        let result = match planned.recv().await {
            Ok(Ok(request)) => {
                let fetch = cx.update(|cx| {
                    let exec = executor::schema(cx)?;
                    let load = async move {
                        let cancel = CancelHandle::new();
                        let answer = fetch_introspection(&exec, &request, &cancel).await?;
                        // The server may echo what the send carried, so the kept text is
                        // scrubbed of the substituted secrets and the placed credentials.
                        let secrets: Vec<String> = request
                            .sensitive_values()
                            .iter()
                            .chain(answer.derived_credentials())
                            .cloned()
                            .collect();
                        let text = answer.text().to_owned();
                        Ok::<_, IntrospectError>((answer.into_schema(), text, secrets))
                    };
                    Some(tokio_bridge::spawn(cx, load))
                });
                match fetch {
                    Some(task) => match task.await {
                        Ok(answer) => answer.map_err(|e| server_failure(e.error())),
                        Err(_) => Err("the schema load stopped".into()),
                    },
                    None => Err("the network is not available".into()),
                }
            }
            Ok(Err(message)) => Err(message.into()),
            Err(_) => Err("the schema load stopped".into()),
        };
        cx.update(|cx| finish_and_keep(url, generation, result, cx));
    })
    .detach();
}

/// Reads a schema file from `path` in the background and stores it under `url`: introspection
/// JSON (with or without its `data` wrapper) or an SDL document, told apart by content.
pub fn load_from_file(url: String, path: PathBuf, cx: &mut App) {
    let generation = cx.default_global::<SchemaStore>().begin_file(&url, &path);
    let read = cx.background_spawn(async move {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        // The file is the user's own and nothing was sent, so there is no secret to scrub.
        let schema = Schema::from_text(&text).map_err(|e| e.to_string())?;
        Ok::<_, String>((schema, text, Vec::new()))
    });
    cx.spawn(async move |cx| {
        let result = read.await.map_err(SchemaFailure::from);
        cx.update(|cx| finish_and_keep(url, generation, result, cx));
    })
    .detach();
}

/// Reads the schema kept on disk for `url` in the background, the first time `url` is used
/// and only when the store has a cache; a load started meanwhile wins over it.
pub fn load_from_cache(url: String, cx: &mut App) {
    let Some(cache) = cx.default_global::<SchemaStore>().probe(&url) else {
        return;
    };
    let read = cx.background_spawn({
        let url = url.clone();
        async move { cache.load(&url) }
    });
    cx.spawn(async move |cx| {
        if let Some(schema) = read.await {
            cx.update(|cx| cx.default_global::<SchemaStore>().restore(&url, schema));
        }
    })
    .detach();
}

// Stores the load's outcome and, when it is the newest load and succeeded, writes its text to
// the disk cache in the background.
fn finish_and_keep(
    url: String,
    generation: u64,
    result: Result<(Schema, String, Vec<String>), SchemaFailure>,
    cx: &mut App,
) {
    let store = cx.default_global::<SchemaStore>();
    let (result, kept) = match result {
        Ok((schema, text, secrets)) => (Ok(schema), Some((text, secrets))),
        Err(failure) => (Err(failure), None),
    };
    let cache = store.cache().cloned();
    if !store.finish(&url, generation, result) {
        return;
    }
    if let (Some(cache), Some((text, secrets))) = (cache, kept) {
        cx.background_spawn(async move {
            let _ = cache.store(&url, &text, &secrets);
        })
        .detach();
    }
}

// An unreachable server reads as a failed send does, keeping the raw text as the detail. Any
// other failure leads with what failed, and an answer without a schema points at the file load.
fn server_failure(error: &SchemaError) -> SchemaFailure {
    let raw = error.to_string();
    let exec = match error {
        SchemaError::Send(e) | SchemaError::Auth(OAuth2Error::Exec(e)) => Some(e),
        _ => None,
    };
    if let Some(message) = exec.and_then(transport_text) {
        return SchemaFailure {
            message,
            detail: Some(raw),
        };
    }
    let mut message = format!("Could not load schema: {}.", raw.trim_end_matches('.'));
    if matches!(
        error,
        SchemaError::Status { .. }
            | SchemaError::NotJson { .. }
            | SchemaError::NoData
            | SchemaError::Graphql { .. }
    ) {
        message
            .push_str(" The server may not share its schema; use Load schema from file instead.");
    }
    message.into()
}
