//! The Apinata desktop app.

use apinata_app::app::{AppState, VolatileSecrets, start_with_trash};
use apinata_app::launch::{LaunchArgs, parse_args};
use apinata_app::persist::Paths;
use apinata_app::updates::Updates;
use apinata_app::watch::set::notify_factory;
use apinata_core::SystemTrash;
use apinata_core::{
    GitOptions, KEYCHAIN_TIMEOUT, KeychainStore, MemoryStore, SecretStore, TimeoutStore,
};
use gpui_kit::AppContext;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::SystemTime;

fn main() {
    // Answer a version query before GPUI starts, so packaging checks work without a display.
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    let LaunchArgs {
        version,
        roots,
        notes: arg_notes,
    } = parse_args(&args, &cwd);
    if version {
        println!("apinata-app {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    gpui_kit::application()
        .with_assets(apinata_app::icons::AppAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            apinata_app::theme::install(cx);
            apinata_app::tokio_bridge::init(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    apinata_app::update_install::quit(cx)
                }
            })
            .detach();
            // A profile directory keeps every app file in one place, as the demo does.
            let Some(paths) = Paths::for_profile(std::env::var_os("APINATA_PROFILE_DIR")) else {
                eprintln!("apinata: no home directory for settings");
                return cx.quit();
            };
            // Connecting to the keychain can block, so it runs off the main thread; without
            // one, secrets live in memory for this run and the user is told so.
            let keychain = cx.background_spawn(async { KeychainStore::new() });
            cx.spawn(async move |cx| {
                let (store, mut notes): (Arc<dyn SecretStore>, Vec<String>) = match keychain.await {
                    // A hung secret service must not stall a background read for good.
                    Ok(k) => (
                        Arc::new(TimeoutStore::new(Arc::new(k), KEYCHAIN_TIMEOUT)),
                        Vec::new(),
                    ),
                    Err(e) => {
                        cx.update(|cx| cx.set_global(VolatileSecrets));
                        (
                            Arc::new(MemoryStore::new()),
                            vec![format!(
                                "Keychain unavailable ({e}); secrets last until quit."
                            )],
                        )
                    }
                };
                notes.extend(arg_notes);
                cx.update(|cx| {
                    match start_with_trash(&paths, store, notes, Arc::new(SystemTrash::new()), cx) {
                        Ok((_, shell)) => {
                            let workspace = shell.read(cx).workspace().clone();
                            workspace.update(cx, |w, cx| {
                                w.set_watch_factory(Some(notify_factory()), cx)
                            });
                            // Git status runs only in the real app; tests opt in with a sandboxed config.
                            workspace.update(cx, |w, cx| w.set_git(Some(GitOptions::new()), cx));
                            // Roots from the command line join the restored session; open skips duplicates.
                            workspace.update(cx, |w, cx| w.open_roots(roots, cx));
                            let approvals = cx.global::<AppState>().approvals.clone();
                            approvals.update(cx, |a, cx| {
                                a.set_watch_factory(Some(notify_factory()), cx)
                            });
                            // Only the real app reaches the network: start never installs a checker.
                            let state = cx.global::<AppState>();
                            let (updates, settings) =
                                (state.updates.clone(), state.settings.clone());
                            let enabled = settings.read(cx).get().check_for_updates;
                            updates.update(cx, |u, cx| {
                                u.set_checker(Updates::core_checker(), cx);
                                u.set_stager(apinata_app::update_install::core_stager(), cx);
                                u.check_if_due(enabled, SystemTime::now(), cx);
                            });
                            apinata_app::oauth2_signin::set_opener(
                                Arc::new(|url: &str, cx: &mut gpui_kit::App| cx.open_url(url)),
                                cx,
                            );
                            apinata_app::open_external::install(
                                Arc::new(|path: &std::path::Path, cx: &mut gpui_kit::App| {
                                    cx.open_with_system(path)
                                }),
                                std::env::temp_dir(),
                                cx,
                            );
                            apinata_app::terminal::install(
                                apinata_app::terminal::Setup::system(),
                                cx,
                            );
                            // Read the key and file once, off the UI thread.
                            let tokens = apinata_app::oauth2_tokens::shared(cx);
                            apinata_app::tokio_bridge::spawn(
                                cx,
                                async move { tokens.load().await },
                            )
                            .detach();
                            cx.activate(true);
                        }
                        Err(e) => {
                            eprintln!("apinata: {e}");
                            cx.quit();
                        }
                    }
                });
            })
            .detach();
        });
}
