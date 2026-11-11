//! Startup: loading the settings, keymap, session and update state files, opening the main window
//! over them, and flushing the files on quit.

use std::sync::Arc;
use std::time::Duration;

use apinata_core::{CookieJar, SecretStore, TokenCache};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{TitleBar, WindowExt};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Entity, Global, Pixels, Task, TitlebarOptions,
    WindowDecorations, WindowOptions,
};

use crate::approvals::Approvals;
use crate::commands::registry;
use crate::draft_store::{DraftStore, Drafts};
use crate::identity::{self, APP_ID};
use crate::persist::{Paths, load_json};
use crate::persisted::Persisted;
use crate::session::Session;
use crate::settings::{LinuxTitleBar, Settings};
use crate::shell::Shell;
use crate::shortcuts::{Overrides, diagnose};
use crate::updates::{UpdateState, Updates};
use crate::workspace::Workspace;

/// The app-wide settings, keymap, session, approval queue and update check, reachable from any view.
pub struct AppState {
    /// The user's settings.
    pub settings: Entity<Persisted<Settings>>,
    /// The user's shortcut overrides from keymap.json.
    pub keymap: Entity<Persisted<Overrides>>,
    /// The window and collection session.
    pub session: Entity<Persisted<Session>>,
    /// The MCP server's pending approval requests.
    pub approvals: Entity<Approvals>,
    /// The update check; it has no checker until `main` installs one.
    pub updates: Entity<Updates>,
    /// The secret store `start` was given, which holds the global proxy password.
    pub secrets: Arc<dyn SecretStore>,
}

impl Global for AppState {}

/// Set when the secret store is in memory only, so a saved secret is lost at quit.
pub struct VolatileSecrets;

impl Global for VolatileSecrets {}

/// How often the cookie jar is saved when it changed.
pub const COOKIE_SAVE_TICK: Duration = Duration::from_secs(5);

/// The main window's options, built on the kit title bar's: titled "Apinata", restored to
/// the saved bounds when they are large enough and still on one of `displays`, else placed
/// by the platform.
pub fn window_options(session: &Session, displays: &[Bounds<Pixels>]) -> WindowOptions {
    WindowOptions {
        window_bounds: session.window.as_ref().and_then(|w| w.to_window(displays)),
        titlebar: Some(TitlebarOptions {
            title: Some("Apinata".into()),
            ..TitleBar::title_bar_options()
        }),
        app_id: Some(APP_ID.into()),
        icon: identity::window_icon(),
        ..TitleBar::window_options()
    }
}

/// The decoration request for `choice`: `Some` only on Linux, where the compositor may
/// draw the title bar (`System`) or leave it to the app (`Client`).
pub fn window_decorations(choice: LinuxTitleBar) -> Option<WindowDecorations> {
    cfg!(target_os = "linux").then_some(match choice {
        LinuxTitleBar::Client => WindowDecorations::Client,
        LinuxTitleBar::System => WindowDecorations::Server,
    })
}

/// Loads the files at `paths`, opens the main window and reopens the session's
/// collections. Each of `notes`, and any file recovery message, is shown as a warning.
/// The caller has run `gpui_kit::init`; this binds every key.
pub fn start(
    paths: &Paths,
    store: Arc<dyn SecretStore>,
    notes: Vec<String>,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<Shell>)> {
    start_with_trash(paths, store, notes, Arc::new(crate::workspace::NoTrash), cx)
}

/// `start`, with deletes going to `trash`. Only the binary passes the platform's; `start`
/// passes none, so nothing else can reach the user's real trash.
pub fn start_with_trash(
    paths: &Paths,
    store: Arc<dyn SecretStore>,
    mut notes: Vec<String>,
    trash: Arc<dyn apinata_core::Trash>,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<Shell>)> {
    crate::keymap::init(cx);
    crate::zoom::install_dialog_zoom(cx);
    crate::input_rest::install(cx);
    crate::generate_code::init();
    crate::graphql_highlight::init();
    let settings = load_json::<Settings>(&paths.settings);
    let keymap = load_json::<Overrides>(&paths.keymap);
    let session = load_json::<Session>(&paths.session);
    let update_state = load_json::<UpdateState>(&paths.update_state);
    notes.extend(settings.recovered);
    notes.extend(keymap.recovered);
    notes.extend(diagnose(&registry(), &keymap.value).warnings());
    crate::keymap::apply(&keymap.value, cx);
    // Bad theme files become Console warnings; the app opens with the bundled themes anyway.
    crate::theme::load_user_themes(&paths.themes, cx);
    notes.extend(session.recovered);
    notes.extend(update_state.recovered);
    // A volatile store cannot decrypt the existing cookie file, and a write would replace it.
    let jar = Arc::new(if cx.has_global::<VolatileSecrets>() {
        CookieJar::default()
    } else {
        CookieJar::open(paths.cookies.clone(), Arc::clone(&store))
    });
    // The tick saves only a changed jar, off the UI thread as the load is.
    load_saved_cookies(Arc::clone(&jar), cx);
    let ticking = Arc::clone(&jar);
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(COOKIE_SAVE_TICK).await;
            let jar = Arc::clone(&ticking);
            cx.background_spawn(async move { crate::executor::save_cookies(&jar) })
                .await;
        }
    })
    .detach();
    // Without an executor the app still opens and edits; the warning says why sends fail.
    // The password read may block on a hung secret service, so it never runs here: a proxy
    // that needs one installs without it now and is reinstalled once the read lands.
    // Installed before the executor, whose sends check the network settings against it.
    crate::network_gate::install(paths.network_ack.clone(), cx);
    let late_password = crate::executor::wants_password(&settings.value);
    let installed = if late_password {
        crate::executor::install_with_password(&settings.value, None, Arc::clone(&jar), cx)
    } else {
        crate::executor::install_with_jar(&settings.value, store.as_ref(), Arc::clone(&jar), cx)
    };
    let installed_proxy = settings.value.proxy.clone();
    let (store_for_password, jar_for_password) = (Arc::clone(&store), Arc::clone(&jar));
    match installed {
        Ok(install_notes) => notes.extend(install_notes),
        Err(e) => notes.push(format!("Requests cannot be sent: {e}")),
    }
    // A volatile store cannot decrypt the existing token file, and a write would replace it.
    let tokens = if cx.has_global::<VolatileSecrets>() {
        TokenCache::in_memory()
    } else {
        TokenCache::open(paths.oauth2_tokens.clone(), Arc::clone(&store))
    };
    crate::oauth2_tokens::install(tokens, cx);
    let options = WindowOptions {
        window_decorations: window_decorations(settings.value.linux_title_bar),
        ..window_options(&session.value, &displays(cx))
    };
    let settings = cx.new(|_| Persisted::new(settings.value, paths.settings.clone()));
    let session = cx.new(|_| Persisted::new(session.value, paths.session.clone()));
    let keymap = cx.new(|_| Persisted::new(keymap.value, paths.keymap.clone()));
    let (trust_file, inbox) = (paths.trusted_hosts.clone(), paths.approvals.clone());
    let approvals = cx.new(|cx| Approvals::new(trust_file, inbox, cx));
    let script_trust = paths.script_trust.clone();
    approvals.update(cx, |a, cx| {
        a.set_secrets(Arc::clone(&store), cx);
        a.set_script_trust(script_trust, cx);
    });
    let update_state = cx.new(|_| Persisted::new(update_state.value, paths.update_state.clone()));
    let updates = cx.new(|_| Updates::new(update_state.clone(), None));
    cx.set_global(AppState {
        settings: settings.clone(),
        keymap: keymap.clone(),
        session: session.clone(),
        approvals,
        updates,
        secrets: Arc::clone(&store),
    });

    // A quit must wait for every final write: a dropped flush task is a cancelled write.
    let late_settings = settings.clone();
    let (quit_settings, quit_session) = (settings, session.clone());
    cx.on_app_quit(move |cx| {
        let mut writes: Vec<Task<()>> = vec![
            quit_settings.update(cx, |p, cx| p.flush(cx)),
            quit_session.update(cx, |p, cx| p.flush(cx)),
            keymap.update(cx, |p, cx| p.flush(cx)),
            update_state.update(cx, |p, cx| p.flush(cx)),
        ];
        // Loading first waits out a slow pending load, so the save holds the merged jar.
        let jar = Arc::clone(&jar);
        writes.push(cx.background_spawn(async move {
            jar.load();
            crate::executor::save_cookies(&jar);
        }));
        async move {
            for write in writes {
                write.await;
            }
        }
    })
    .detach();

    let schemas = crate::schema_cache::SchemaCache::new(paths.schemas.clone());
    cx.set_global(crate::graphql_schema_store::SchemaStore::with_cache(
        schemas,
    ));

    let responses = crate::last_response::ResponseStore::new(paths.responses.clone());
    crate::last_response::install(responses.clone(), cx);
    // The digest key reads the keychain, so it loads off the UI thread; until it does, or when
    // it cannot, sends are kept without digests. A volatile run makes no key it would lose.
    if !cx.has_global::<VolatileSecrets>() {
        let keys = Arc::clone(&store);
        cx.background_spawn(async move {
            let name = crate::last_response::DIGEST_KEY_NAME;
            if let Ok(key) = apinata_core::DigestKey::load(keys.as_ref(), name) {
                responses.set_digest_key(key);
            }
        })
        .detach();
    }

    // The store lives only in the global, so its lock is held until the app quits.
    let drafts_dir = paths.drafts.clone();
    let drafts = cx.new(|cx| DraftStore::open(drafts_dir, cx));
    cx.set_global(Drafts(drafts.clone()));

    let global_dir = paths.global_environments.clone();
    let scratch_dir = paths.scratch.clone();
    let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| {
        // X11 takes its window class only from this call, never from the options' app id.
        window.set_app_id(APP_ID);
        let workspace = cx.new(|cx| {
            let mut w = Workspace::new(store, session, cx);
            w.set_global_dir(global_dir, cx);
            w.set_scratch_dir(scratch_dir, cx);
            w.set_trash(trash);
            w
        });
        drafts.update(cx, |d, cx| d.attach(&workspace, cx));
        cx.new(|cx| Shell::new(workspace, window, cx))
    })?;
    // Restoring after the shell exists lets it hear the notices restore emits; the window's
    // notifications need the open window's root.
    let workspace = shell.read(cx).workspace().clone();
    window.update(cx, |_, _, cx| workspace.update(cx, |w, cx| w.restore(cx)))?;
    crate::oauth2_signin::install(window, cx);
    crate::script_gate::install(window, cx);
    crate::network_gate::attach(workspace.downgrade(), cx);
    if late_password {
        let late = LatePassword {
            store: store_for_password,
            jar: jar_for_password,
            settings: late_settings,
            installed_proxy,
            window,
        };
        late.spawn(cx);
    }
    // Notifications live on the window's root, which exists only once the window is open.
    window.update(cx, |_, window, cx| {
        for note in notes {
            window.push_notification(Notification::warning(note), cx);
        }
    })?;
    Ok((window, shell))
}

/// The proxy password read that follows a startup install made without it.
struct LatePassword {
    store: Arc<dyn SecretStore>,
    jar: Arc<CookieJar>,
    settings: Entity<Persisted<Settings>>,
    installed_proxy: crate::settings::ProxyPrefs,
    window: AnyWindowHandle,
}

impl LatePassword {
    // Reads off the UI thread, then reinstalls the executor with the password unless the
    // proxy settings changed meanwhile. A failed read warns once and keeps the executor.
    fn spawn(self, cx: &mut App) {
        let store = Arc::clone(&self.store);
        let read =
            cx.background_spawn(async move { store.get(&crate::executor::proxy_password_key()) });
        cx.spawn(async move |cx| {
            let result = read.await;
            cx.update(|cx| self.finish(result, cx));
        })
        .detach();
    }

    fn finish(self, result: Result<Option<String>, apinata_core::SecretError>, cx: &mut App) {
        let now = self.settings.read(cx).get().clone();
        if now.proxy != self.installed_proxy {
            return;
        }
        let warning = match result {
            Ok(None) => return,
            Ok(Some(password)) => {
                match crate::executor::install_with_password(&now, Some(password), self.jar, cx) {
                    Ok(_) => return,
                    Err(e) => format!("Requests cannot be sent: {e}"),
                }
            }
            Err(e) => crate::executor::password_note(&e),
        };
        let _ = self.window.update(cx, |_, window, cx| {
            window.push_notification(Notification::warning(warning), cx)
        });
    }
}

/// Loads `jar`'s saved cookies off the UI thread, since the load reads the keychain and the
/// disk, then notes any it dropped on the Console. Only the first load of a jar reads the file.
pub fn load_saved_cookies(jar: Arc<CookieJar>, cx: &mut App) {
    cx.spawn(async move |cx| {
        let dropped = cx.background_spawn(async move { jar.load() }).await;
        cx.update(|cx| note_dropped_cookies(dropped, cx));
    })
    .detach();
}

/// Notes on the Console, once, that loading the saved jar dropped `count` cookies; 0 notes nothing.
pub fn note_dropped_cookies(count: usize, cx: &mut App) {
    if let Some(notice) = apinata_core::dropped_cookies_notice(count) {
        let log = crate::log_feed::ensure(cx);
        log.note("Cookies", apinata_core::ConsoleLevel::Warn, &notice);
    }
}

fn displays(cx: &App) -> Vec<Bounds<Pixels>> {
    cx.displays().iter().map(|d| d.bounds()).collect()
}
