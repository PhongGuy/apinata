//! `bru.runRequest`: an async host function that runs another request of the collection one
//! nesting level deeper, wrapped by `run_request.js`.

use std::sync::Arc;
use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rquickjs::Function;
use rquickjs::prelude::Async;
use serde_json::{Value, json};

use super::bru::note_read;
use super::engine::Installer;
use super::host::HostState;
use super::send::record_refusal;
use crate::execute::ExecError;
use crate::runner::{RequestResult, RunError, run_nested};

const RUN_REQUEST_JS: &str = include_str!("run_request.js");

/// Installs `__runRequest` and `run_request.js`, which builds `bru.runRequest` over it.
/// A nested run ends by `deadline`, the calling phase's, as well as by its own limits.
pub(crate) fn installer(state: Arc<HostState>, deadline: Option<Instant>) -> Installer {
    Box::new(move |ctx| {
        let host = move |path: String| {
            let state = Arc::clone(&state);
            async move {
                let answer = answer(&state, &path, deadline).await;
                Ok::<_, rquickjs::Error>(answer.to_string())
            }
        };
        ctx.globals()
            .set("__runRequest", Function::new(ctx.clone(), Async(host))?)?;
        ctx.eval::<(), _>(RUN_REQUEST_JS)
    })
}

/// The object the script's promise resolves to: the response's shape when one arrived,
/// `{error}` for a send that failed without one, otherwise `{message}` naming why none did.
/// What the nested run's scripts read stays sensitive for the caller, and its host-policy
/// refusals are recorded on the caller.
async fn answer(state: &HostState, path: &str, deadline: Option<Instant>) -> Value {
    match run_nested(state, path, deadline).await {
        Ok(result) => {
            note_read(state, result.script_sensitive_values().iter().cloned());
            if let Some(RunError::Exec(e)) = &result.error {
                record_refusal(state, e);
            }
            for e in result.script_refusals() {
                record_refusal(state, e);
            }
            shape(&result, path)
        }
        Err(message) => json!({ "message": message }),
    }
}

fn shape(result: &RequestResult, path: &str) -> Value {
    if let Some(response) = &result.response {
        return json!({
            "status": response.status(),
            "statusText": response.status_text(),
            "headers": response.headers_value(),
            "data": response.body_value(),
            "dataBuffer": STANDARD.encode(response.body()),
            "size": response.size(),
            "duration": response.response_time().as_millis() as u64,
            "url": response.url(),
        });
    }
    // A transport failure carries no status, so the key is left out rather than null.
    let message = match &result.error {
        Some(RunError::Exec(e)) if !matches!(e, ExecError::Cancelled) => {
            return json!({ "error": e.to_string() });
        }
        Some(error) => error.to_string(),
        None => format!("bru.runRequest: request skipped - {path}"),
    };
    json!({ "message": message })
}
