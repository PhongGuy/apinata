//! Loads a GraphQL request's schema into the shared store, from its server or from a schema file.

use std::path::{Path, PathBuf};

use apinata_core::{
    CancelHandle, ItemPath, ResolveError, Schema, SchemaError, fetch_schema, introspection_request,
};
use gpui_kit::{App, AppContext as _, Entity, Window};

use crate::copy_curl::plan_and_resolve_with;
use crate::graphql_schema_store::SchemaStore;
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
                        fetch_schema(&exec, &request, &cancel).await
                    };
                    Some(tokio_bridge::spawn(cx, load))
                });
                match fetch {
                    Some(task) => match task.await {
                        Ok(answer) => answer.map_err(|e| e.to_string()),
                        Err(_) => Err("the schema load stopped".to_owned()),
                    },
                    None => Err("the network is not available".to_owned()),
                }
            }
            Ok(Err(message)) => Err(message),
            Err(_) => Err("the schema load stopped".to_owned()),
        };
        cx.update(|cx| {
            cx.default_global::<SchemaStore>()
                .finish(&url, generation, result)
        });
    })
    .detach();
}

/// Reads a schema file from `path` in the background and stores it under `url`: introspection
/// JSON (with or without its `data` wrapper) or an SDL document, told apart by content.
pub fn load_from_file(url: String, path: PathBuf, cx: &mut App) {
    let generation = cx.default_global::<SchemaStore>().begin(&url);
    let read = cx.background_spawn(async move {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        Schema::from_text(&text).map_err(|e| e.to_string())
    });
    cx.spawn(async move |cx| {
        let result = read.await;
        cx.update(|cx| {
            cx.default_global::<SchemaStore>()
                .finish(&url, generation, result)
        });
    })
    .detach();
}
