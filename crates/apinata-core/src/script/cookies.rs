//! `bru.cookies`: host ops reading and writing the executor's jar, for the request URL or, through
//! `jar()`, for any URL a script names. Writes follow the jar's response-cookie rules.

use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use rquickjs::Function;
use serde_json::{Value, json};
use url::Url;

use super::ScriptPhase;
use super::bru::interpolate_text;
use super::engine::Installer;
use super::host::HostState;
use crate::cookies::{CookieInfo, ScriptCookie, SetCookie};

const COOKIES_JS: &str = include_str!("cookies.js");

/// Installs `__cookies`, `__cookieOp` and `cookies.js`, which builds `bru.cookies` over them;
/// the rows its writes record belong to `phase`.
pub(crate) fn installer(state: Arc<HostState>, phase: ScriptPhase) -> Installer {
    Box::new(move |ctx| {
        let reads = Arc::clone(&state);
        let host = move || cookies(&reads);
        ctx.globals()
            .set("__cookies", Function::new(ctx.clone(), host)?)?;
        let writes = Arc::clone(&state);
        let op = move |request: String| cookie_op(&writes, phase, &request);
        ctx.globals()
            .set("__cookieOp", Function::new(ctx.clone(), op)?)?;
        ctx.eval::<(), _>(COOKIES_JS)
    })
}

/// One jar operation the JS layer asks for.
#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Op {
    List {
        url: Option<String>,
    },
    Set {
        url: Option<String>,
        cookie: ScriptCookie,
    },
    Delete {
        url: Option<String>,
        name: String,
    },
    DeleteAll {
        url: Option<String>,
    },
    Clear,
}

/// Runs one operation against this executor's jar, the only jar a script can reach. Answers
/// `{"v": result}`, `{"e": message}` for a bad URL, and adds `"w"` when a cookie was refused.
fn cookie_op(state: &HostState, phase: ScriptPhase, request: &str) -> String {
    let op: Op = match serde_json::from_str(request) {
        Ok(op) => op,
        Err(e) => return json!({ "e": e.to_string() }).to_string(),
    };
    // `None` names the request URL; one that does not parse makes every operation a no-op.
    let target = |url: Option<String>| -> Result<Option<Url>, String> {
        match url {
            None => Ok(request_url(state)),
            Some(raw) => Url::parse(&interpolate_text(state, &raw))
                .map(Some)
                .map_err(|_| "Invalid URL".to_owned()),
        }
    };
    let jar = state.exec.cookies();
    let mut warning = None;
    let answer = match op {
        Op::List { url } => target(url).map(|url| {
            let list = url.map(|u| jar.matching(&u)).unwrap_or_default();
            Value::Array(list.iter().map(cookie_json).collect())
        }),
        Op::Set { url, cookie } => target(url).map(|url| {
            if let Some(u) = url {
                let (outcome, refused) = jar.store_for(&u, &cookie);
                warning = refused.map(|reason| {
                    format!("cookie {} was not stored: {reason}", shown(&cookie.key))
                });
                state.record_cookie(phase, SetCookie::scripted(&cookie, outcome));
            }
            Value::Null
        }),
        // Each cookie a delete or clear actually removed is recorded; one that matched
        // nothing records nothing.
        Op::Delete { url, name } => target(url).map(|url| {
            let gone = url.and_then(|u| jar.delete_for(&u, &name));
            gone.iter()
                .for_each(|c| state.record_cookie(phase, SetCookie::removed(c)));
            Value::Null
        }),
        Op::DeleteAll { url } => target(url).map(|url| {
            let gone = url.map(|u| jar.delete_all_for(&u)).unwrap_or_default();
            gone.iter()
                .for_each(|c| state.record_cookie(phase, SetCookie::removed(c)));
            Value::Null
        }),
        Op::Clear => {
            let gone = jar.clear_all();
            gone.iter()
                .for_each(|c| state.record_cookie(phase, SetCookie::removed(c)));
            Ok(Value::Null)
        }
    };
    match (answer, warning) {
        (Ok(v), None) => json!({ "v": v }).to_string(),
        (Ok(v), Some(w)) => json!({ "v": v, "w": w }).to_string(),
        (Err(e), _) => json!({ "e": e }).to_string(),
    }
}

/// A cookie name for a warning: quoted with control characters escaped, cut after 64
/// characters, so a hostile name cannot break or flood the console.
fn shown(name: &str) -> String {
    let cut: String = name.chars().take(64).collect();
    let more = if cut.len() < name.len() { "..." } else { "" };
    format!("\"{}{more}\"", cut.escape_default())
}

/// A cookie as scripts read it.
fn cookie_json(c: &CookieInfo) -> Value {
    let expires = c
        .expires
        .map(|at| DateTime::<Utc>::from(at).to_rfc3339_opts(SecondsFormat::Millis, true));
    json!({
        "key": c.name, "value": c.value, "domain": c.domain, "path": c.path,
        "secure": c.secure, "httpOnly": c.http_only, "expires": expires, "hostOnly": c.host_only,
    })
}

/// The JSON array of cookie objects for the request URL.
fn cookies(state: &HostState) -> String {
    let list = request_url(state)
        .map(|url| state.exec.cookies().matching(&url))
        .unwrap_or_default();
    Value::Array(list.iter().map(cookie_json).collect()).to_string()
}

/// The request URL: the response's URL once there is a response, else the prepared URL with its
/// placeholders expanded. An unparsable URL is `None`.
fn request_url(state: &HostState) -> Option<Url> {
    let raw = match state.response.get() {
        Some(response) => response.url().to_owned(),
        None => {
            let url = state
                .prepared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .url()
                .to_owned();
            interpolate_text(state, &url)
        }
    };
    Url::parse(&raw).ok()
}
