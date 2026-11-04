//! Installing an offered release: the notice's download-and-restart button, the real stager
//! for this install, and the quit that installs an armed release and restarts into it.

use crate::ui::LockButton as _;
use std::path::PathBuf;
use std::sync::Arc;

use apinata_core::update::{self, InstallKind, Os, Product, Reason, Relaunch};
use gpui_kit::component::button::Button;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext as _, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Styled as _, TestSupportExt as _, Window, div,
};

use crate::app::AppState;
use crate::closing::{self, CloseScope};
use crate::updates::{InstallPhase, Prepared, Stager, Updates};
use crate::workspace::Workspace;

/// How this running app was installed, from its canonical executable and `$APPIMAGE`.
pub fn local_kind() -> InstallKind {
    let unrecognised = InstallKind::NotOwned(Reason::Unrecognised);
    let Some(os) = Os::current() else {
        return unrecognised;
    };
    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
        return unrecognised;
    };
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    update::install_kind(Product::App, os, &exe, appimage.as_deref(), false)
}

/// The real stager when this install can update itself (AppImage, `.app`, NSIS) and updates
/// are configured: `update::check`, the offered version must match, then `update::stage_app`.
pub fn core_stager() -> Option<Stager> {
    let kind = local_kind();
    let owned = matches!(
        kind,
        InstallKind::AppImage(_) | InstallKind::MacApp(_) | InstallKind::WindowsNsis
    );
    let cfg = update::build_config(Product::App).ok().filter(|_| owned)?;
    Some(Arc::new(move |cx: &mut App, version: String| {
        let (cfg, kind) = (cfg.clone(), kind.clone());
        // The manifest is read again so the download is the one it signs now, and a release
        // other than the one the notice shows is refused rather than installed.
        let job = crate::tokio_bridge::spawn(cx, async move {
            let available = update::check(&cfg).await.map_err(|e| e.to_string())?;
            let Some(a) = available.filter(|a| a.version == version) else {
                return Err("a different release is now available; check again".to_string());
            };
            let staged = update::stage_app(&kind, &a, |_, _| {})
                .await
                .map_err(|e| e.to_string())?;
            Ok(Prepared::new(version, kind, staged))
        });
        cx.background_spawn(async move { job.await.map_err(|e| e.to_string())? })
    }))
}

/// Prepares the offered release, then runs the window's normal close flow.
pub fn download_and_restart(
    updates: Entity<Updates>,
    workspace: Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let handle = window.window_handle();
    let target = updates.clone();
    // The install is armed only once the release is ready, so a press during a download
    // never arms it; a close flow that leaves the window open disarms it again.
    let on_done = move |ready: bool, cx: &mut App| {
        if !ready {
            return;
        }
        target.update(cx, |u, _| u.set_armed(true));
        let keep = target.clone();
        let closed = handle.update(cx, |_, window, cx| {
            closing::confirm_close_or_keep(workspace, CloseScope::Window, window, cx, {
                let keep = keep.clone();
                move |cx| keep.update(cx, |u, _| u.set_armed(false))
            });
        });
        if closed.is_err() {
            keep.update(cx, |u, _| u.set_armed(false));
        }
    };
    updates.update(cx, |u, cx| u.prepare(on_done, cx));
}

/// The notice's install button (`update-install`) and any failure text (`update-install-error`);
/// `None` when `can_install` is false.
pub fn install_controls(
    updates: &Entity<Updates>,
    workspace: &Entity<Workspace>,
    cx: &App,
) -> Option<AnyElement> {
    let u = updates.read(cx);
    if !u.can_install() {
        return None;
    }
    let phase = u.install_phase().clone();
    let label = match phase {
        InstallPhase::Downloading => "Downloading...",
        InstallPhase::Ready => "Restart to update",
        InstallPhase::Idle | InstallPhase::Failed(_) => "Download and restart",
    };
    let (updates, workspace) = (updates.clone(), workspace.clone());
    let button = Button::new("update-install")
        .label(label)
        .lock(phase == InstallPhase::Downloading, cx)
        .on_click(move |_, window, cx| {
            download_and_restart(updates.clone(), workspace.clone(), window, cx)
        });
    let error = match phase {
        InstallPhase::Failed(message) => Some(message),
        _ => None,
    };
    Some(
        div()
            .flex()
            .flex_row()
            .gap_2()
            .child(button)
            .when_some(error, |el, message| {
                el.child(
                    div()
                        .id("update-install-error")
                        .test_support()
                        .child(format!("Update failed: {message}")),
                )
            })
            .into_any_element(),
    )
}

/// The last window closed: install the armed release and restart into it, or just quit.
pub fn quit(cx: &mut App) {
    let updates = cx.try_global::<AppState>().map(|s| s.updates.clone());
    let Some(prepared) = updates.and_then(|u| u.update(cx, |u, _| u.take_armed())) else {
        return cx.quit();
    };
    match prepared.install() {
        Ok(Relaunch::Restart(path)) => {
            cx.set_restart_path(path);
            cx.restart();
        }
        Ok(Relaunch::Installer { program, args }) => {
            if let Err(e) = update::start_installer(&program, &args) {
                eprintln!("apinata: update install: {e}");
            }
            cx.quit();
        }
        Ok(_) => cx.quit(),
        Err(e) => {
            eprintln!("apinata: update install: {e}");
            cx.quit();
        }
    }
}
