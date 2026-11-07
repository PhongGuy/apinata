//! The app's one executor: built once at startup and shared by every collection and window,
//! so all sends read and write a single cookie jar, as Bruno's do.

use std::sync::Arc;

use apinata_core::{
    CookieJar, ExecOptions, Executor, GlobalProxy, HttpVersionPolicy, NetworkSettings, PacSource,
    ResolvedProxy, SCHEMA_BODY_LIMIT, SecretKey, SecretStore,
};
use gpui_kit::{App, Global};

use crate::settings::{HttpVersionPref, ProxyMode, Settings};

/// The session-wide executor every send and run uses.
pub struct SharedExecutor(pub Arc<Executor>);

impl Global for SharedExecutor {}

/// The executor downloads use: the shared jar and network, with no session log.
pub struct UnloggedExecutor(pub Arc<Executor>);

impl Global for UnloggedExecutor {}

/// The executor schema loads use: the shared jar and network, with the schema body limit.
pub struct SchemaExecutor(pub Arc<Executor>);

impl Global for SchemaExecutor {}

/// The executor options `settings` asks for: TLS verification, the response body cap, the
/// cookie switches and the HTTP version policy.
pub fn exec_options(settings: &Settings) -> ExecOptions {
    ExecOptions::new()
        .insecure(settings.insecure)
        // An empty setting means no limit; core would otherwise cap a streamed body at its default.
        .max_body_bytes(Some(settings.max_body_bytes.unwrap_or(u64::MAX)))
        .store_cookies(settings.store_cookies)
        .send_cookies(settings.send_cookies)
        .http_version(match settings.http_version {
            HttpVersionPref::Http1Only => HttpVersionPolicy::Http1Only,
            HttpVersionPref::Auto => HttpVersionPolicy::Auto,
            HttpVersionPref::Http2PriorKnowledge => HttpVersionPolicy::Http2PriorKnowledge,
        })
}

/// The secret store key of the global proxy password.
pub fn proxy_password_key() -> SecretKey {
    SecretKey::new("", "app", "proxy-password")
}

/// `text` as the app's PAC source: `PacSource::parse(text, None)`, refused with
/// "the PAC URL must not contain credentials" when it carries a user name or password.
pub fn parse_pac(text: &str) -> Result<PacSource, String> {
    const CREDENTIALS: &str = "the PAC URL must not contain credentials";
    // A file URL cannot hold userinfo, so an `@` in its authority would fail as an invalid
    // host; refuse any `@` before the path as credentials so every scheme reads the same.
    if let Some((_, rest)) = text.trim().split_once("://") {
        let end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
        if rest[..end].contains('@') {
            return Err(CREDENTIALS.to_owned());
        }
    }
    let source = PacSource::parse(text, None).map_err(|e| e.to_string())?;
    // The shown form redacts credentials, so it differs from the URL exactly when there are some.
    if source.to_string() != source.as_str() {
        return Err(CREDENTIALS.to_owned());
    }
    Ok(source)
}

/// The core network settings `settings` asks for, with `password` for a manual proxy that
/// names a user. `Err` holds why a manual proxy or PAC source is not usable.
pub fn network_settings(
    settings: &Settings,
    password: Option<String>,
) -> Result<NetworkSettings, String> {
    let prefs = &settings.proxy;
    let proxy = match prefs.mode {
        ProxyMode::Off => GlobalProxy::Off,
        ProxyMode::System => GlobalProxy::System,
        ProxyMode::Manual => {
            let mut proxy = ResolvedProxy::new(&prefs.protocol, &prefs.host, &prefs.port)
                .map_err(|e| e.to_string())?;
            if !prefs.username.is_empty() {
                proxy = proxy.with_credentials(&prefs.username, password.unwrap_or_default());
            }
            GlobalProxy::Manual {
                proxy,
                bypass: prefs.bypass.clone(),
            }
        }
        ProxyMode::Pac => GlobalProxy::Pac {
            source: parse_pac(&prefs.pac_source)?,
        },
    };
    Ok(NetworkSettings::new()
        .proxy(proxy)
        .ca_file(settings.ca_file.clone())
        .keep_default_roots(settings.keep_default_cas))
}

/// Builds the executor from `settings` and sets it as the `SharedExecutor` global. Settings
/// alone never fail it: a bad manual proxy or PAC source falls back to the system proxy and an
/// unreadable password sends without it, each returned as a startup note. `Err` means no
/// executor.
pub fn install(
    settings: &Settings,
    store: &dyn SecretStore,
    cx: &mut App,
) -> Result<Vec<String>, String> {
    install_with_jar(settings, store, Arc::new(CookieJar::default()), cx)
}

/// Whether `settings` need the proxy password: only a manual proxy with a user does, so only
/// it may touch the keychain.
pub fn wants_password(settings: &Settings) -> bool {
    settings.proxy.mode == ProxyMode::Manual && !settings.proxy.username.is_empty()
}

/// `install`, with every send reading and writing `jar`. Reads the password from `store`
/// on the calling thread; a caller on the UI thread uses `install_with_password` instead.
pub fn install_with_jar(
    settings: &Settings,
    store: &dyn SecretStore,
    jar: Arc<CookieJar>,
    cx: &mut App,
) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    let password = if wants_password(settings) {
        store.get(&proxy_password_key()).unwrap_or_else(|e| {
            notes.push(password_note(&e));
            None
        })
    } else {
        None
    };
    notes.extend(install_with_password(settings, password, jar, cx)?);
    Ok(notes)
}

/// The startup note for a proxy password the store could not give.
pub fn password_note(e: &dyn std::fmt::Display) -> String {
    format!("Proxy password unavailable ({e}); sending without it.")
}

/// `install_with_jar` with the password already read (or `None`). Also sets the
/// `SchemaExecutor` global, which shares the jar and network settings.
pub fn install_with_password(
    settings: &Settings,
    password: Option<String>,
    jar: Arc<CookieJar>,
    cx: &mut App,
) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    let network = network_settings(settings, password).or_else(|reason| {
        let what = match settings.proxy.mode {
            ProxyMode::Pac => "PAC file",
            _ => "Manual proxy",
        };
        notes.push(format!(
            "{what} ignored ({reason}); using the system proxy."
        ));
        let mut system = settings.clone();
        system.proxy.mode = ProxyMode::System;
        network_settings(&system, None)
    })?;
    // Downloads get the same options without the session log; the logged options add it
    // before the schema options are cloned, so both of those executors share it.
    let mut base = exec_options(settings).network(network).cookie_jar(jar);
    if let Some(check) = crate::network_gate::check(cx) {
        base = base.network_check(check);
    }
    let unlogged = Executor::new(base.clone()).map_err(|e| e.to_string())?;
    let options = base.session_log(crate::log_feed::ensure(cx));
    let schema_options = options.clone().max_body_bytes(Some(SCHEMA_BODY_LIMIT));
    let exec = Executor::new(options).map_err(|e| e.to_string())?;
    let schema = Executor::new(schema_options).map_err(|e| e.to_string())?;
    cx.set_global(SharedExecutor(Arc::new(exec)));
    cx.set_global(SchemaExecutor(Arc::new(schema)));
    cx.set_global(UnloggedExecutor(Arc::new(unlogged)));
    Ok(notes)
}

/// Saves `jar`, printing a failure to stderr; the next save retries it.
pub fn save_cookies(jar: &CookieJar) {
    if let Err(e) = jar.save() {
        eprintln!("apinata: {e}");
    }
}

/// The shared executor, or `None` when `install` has not succeeded.
pub fn shared(cx: &App) -> Option<Arc<Executor>> {
    cx.try_global::<SharedExecutor>().map(|s| Arc::clone(&s.0))
}

/// The schema-load executor, or `None` when `install` has not succeeded.
pub fn schema(cx: &App) -> Option<Arc<Executor>> {
    cx.try_global::<SchemaExecutor>().map(|s| Arc::clone(&s.0))
}

/// The executor for downloads (no session log), or `None` when `install` has not succeeded.
pub fn unlogged(cx: &App) -> Option<Arc<Executor>> {
    cx.try_global::<UnloggedExecutor>()
        .map(|s| Arc::clone(&s.0))
}
