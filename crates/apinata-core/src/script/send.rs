//! `bru.sendRequest` and `axios`: an async host function that sends a script's request through
//! the run's executor and guard, wrapped by `send.js`.

use std::sync::{Arc, PoisonError};

use rquickjs::Function;
use rquickjs::prelude::Async;
use serde::Deserialize;
use serde_json::{Value, json};

use super::engine::Installer;
use super::host::HostState;
use crate::execute::{ExecError, SendTag};
use crate::resolve::{ScriptSend, script_request};
use crate::secret_names::log_values;
use crate::session_log::SendSource;

const SEND_JS: &str = include_str!("send.js");

/// The config `send.js` normalised from axios's: headers in order, `timeout` in milliseconds.
#[derive(Deserialize)]
struct Config {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    timeout: Option<f64>,
}

/// Installs `__send` and `send.js`, which builds `bru.sendRequest` over it.
pub(crate) fn installer(state: Arc<HostState>) -> Installer {
    Box::new(move |ctx| {
        let host = move |config: String| {
            let state = Arc::clone(&state);
            async move { Ok::<_, rquickjs::Error>(answer(&state, &config).await) }
        };
        ctx.globals()
            .set("__send", Function::new(ctx.clone(), Async(host))?)?;
        ctx.eval::<(), _>(SEND_JS)
    })
}

/// `{"v": {status, statusText, headers, data}}` for any response, or `{"e": message}`.
async fn answer(state: &HostState, config: &str) -> String {
    match send(state, config).await {
        Ok(response) => json!({ "v": response }),
        Err(message) => json!({ "e": message }),
    }
    .to_string()
}

async fn send(state: &HostState, config: &str) -> Result<Value, String> {
    let config: Config =
        serde_json::from_str(config).map_err(|e| format!("invalid request config: {e}"))?;
    // Values the script read so far are judged against the host, as an interpolated one would be.
    let sensitive = state
        .sensitive
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    // A zero or unusable timeout means none, as in axios; a huge one is capped as timers are.
    let timeout = config
        .timeout
        .filter(|ms| ms.is_finite() && *ms > 0.0)
        .map(super::timers::wait);
    let request = ScriptSend {
        method: config.method,
        url: config.url,
        headers: config.headers,
        body: config.body,
        timeout,
    };
    let resolved = script_request(request, &state.request, &state.ctx, &sensitive)
        .map_err(|e| e.to_string())?;
    if let Some(message) = state.guard.as_ref().and_then(|guard| guard(&resolved)) {
        return Err(message);
    }
    // The entry hides the calling request's secret candidates and what the script read so far.
    let mut extra = Vec::new();
    if state.exec.session_log().is_some() {
        extra = log_values(&state.request, &state.ctx);
        extra.extend(sensitive.iter().cloned());
    }
    let tag = SendTag {
        source: SendSource::Script,
        extra,
        hold: state.nesting.hold().cloned(),
        ..SendTag::default()
    };
    let response = state
        .exec
        .send_as(&resolved, &state.cancel, &tag)
        .await
        .map_err(|e| {
            record_refusal(state, &e);
            e.to_string()
        })?;
    Ok(json!({
        "status": response.status(),
        "statusText": response.status_text(),
        "headers": response.headers_value(),
        "data": response.body_value(),
    }))
}

/// Keeps a host policy refusal of `error` on the send's state, once per host and proxy.
pub(super) fn record_refusal(state: &HostState, error: &ExecError) {
    let key = |e: &ExecError| match e {
        ExecError::HostNotAllowed { host, .. } => Some((host.clone(), None)),
        ExecError::ProxyNotAllowed { host, proxy, .. } => Some((host.clone(), Some(proxy.clone()))),
        _ => None,
    };
    let Some(wanted) = key(error) else { return };
    let mut refused = state.refused.lock().unwrap_or_else(PoisonError::into_inner);
    if !refused.iter().any(|e| key(e).as_ref() == Some(&wanted)) {
        refused.push(error.clone());
    }
}
