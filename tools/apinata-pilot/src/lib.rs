//! Apinata under gpui-pilot's headless host: the launch glue an agent drives.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use apinata_app::app::{AppState, start_with_trash};
use apinata_app::persist::{Paths, save_json};
use apinata_app::session::Session;
use apinata_app::watch::set::notify_factory;
use apinata_core::{GitOptions, SecretStore};
use gpui_kit::{App, AssetSource};
use gpui_pilot_host::{Launch, PilotApp};

pub mod folder_trash;
pub mod secrets;

/// Apinata's own startup, run inside the host against the launch's fresh data directory.
/// Launch arguments are collection roots, opened at start (see [`prepare_session`]).
pub struct Apinata;

impl PilotApp for Apinata {
    fn assets(&self) -> Option<Arc<dyn AssetSource>> {
        Some(Arc::new(apinata_app::icons::AppAssets))
    }

    fn init(&self, cx: &mut App) {
        gpui_kit::init(cx);
        apinata_app::theme::install(cx);
        apinata_app::tokio_bridge::init(cx);
    }

    fn open(&self, launch: &Launch, cx: &mut App) -> anyhow::Result<()> {
        open_app(&launch.data_dir, &launch.args, launch.reused, cx)
    }
}

/// Opens Apinata's window on `data_dir` with the collection roots `args`, as a launch does;
/// `reused` keeps the saved session and secrets of a kept data dir.
pub fn open_app(
    data_dir: &Path,
    args: &[String],
    reused: bool,
    cx: &mut App,
) -> anyhow::Result<()> {
    let paths = Paths::in_dir(data_dir);
    prepare_session(&paths, args, reused)?;
    // Secrets live in a file in the launch data dir and no update checker is installed, so
    // a piloted run never reaches the keyring or the network on its own.
    let secrets = launch_secrets(data_dir, reused)?;
    // Deletes go to a folder in the data dir, so Undo works without the platform trash.
    let trash = Arc::new(folder_trash::FolderTrash::new(data_dir.join("pilot-trash")));
    let (_, shell) = start_with_trash(&paths, secrets, Vec::new(), trash, cx)?;
    let workspace = shell.read(cx).workspace().clone();
    workspace.update(cx, |w, cx| w.set_watch_factory(Some(notify_factory()), cx));
    // Git status as in the real app, so a piloted run shows letters and the change count.
    workspace.update(cx, |w, cx| w.set_git(Some(GitOptions::new()), cx));
    let approvals = cx.global::<AppState>().approvals.clone();
    approvals.update(cx, |a, cx| a.set_watch_factory(Some(notify_factory()), cx));
    install_opener(data_dir, cx)?;
    Ok(())
}

/// Installs the system opener so "Open in browser" shows: bodies are written under
/// `<data_dir>/pilot-open`, each opened path is appended to `<data_dir>/pilot-opened.txt`, and
/// the open itself goes to the host's stub, which never starts an app.
fn install_opener(data_dir: &Path, cx: &mut App) -> anyhow::Result<()> {
    let base = data_dir.join("pilot-open");
    std::fs::create_dir_all(&base).with_context(|| format!("creating {}", base.display()))?;
    let log = data_dir.join("pilot-opened.txt");
    apinata_app::open_external::install(
        Arc::new(move |path: &Path, cx: &mut App| {
            // The log is how a test or agent finds the file, since the stub keeps opens in memory.
            if let Err(e) = append_line(&log, path) {
                eprintln!("apinata-pilot: logging an open to {}: {e}", log.display());
            }
            cx.open_with_system(path);
        }),
        base,
        cx,
    );
    Ok(())
}

fn append_line(log: &Path, path: &Path) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)?;
    // One write per line, so a reader never sees a path without its newline.
    file.write_all(format!("{}\n", path.display()).as_bytes())
}

/// The secret store a launch uses: kept in `data_dir` across a `reused` relaunch, empty otherwise.
pub fn launch_secrets(data_dir: &Path, reused: bool) -> anyhow::Result<Arc<dyn SecretStore>> {
    let store = secrets::FileSecrets::open(data_dir, reused)
        .with_context(|| format!("opening the secret store in {}", data_dir.display()))?;
    Ok(Arc::new(store))
}

/// Seeds the session from `roots` on a fresh launch; a `reused` launch keeps the data dir a
/// previous launch left, so the app's own saved session (tabs, environment) is restored as is.
pub fn prepare_session(paths: &Paths, roots: &[String], reused: bool) -> anyhow::Result<()> {
    if reused {
        return Ok(());
    }
    seed_session(paths, roots)
}

/// Writes `paths.session` so the app opens `roots` at start. Each root must be an absolute
/// path to an existing directory. Errors: "collection path `<root>` must be absolute",
/// "collection path `<root>` is not a directory"; nothing is written on error.
pub fn seed_session(paths: &Paths, roots: &[String]) -> anyhow::Result<()> {
    // Validate every root before touching the disk, so a bad argument leaves no session behind.
    let mut open = Vec::with_capacity(roots.len());
    for root in roots {
        let path = Path::new(root);
        if !path.is_absolute() {
            bail!("collection path `{root}` must be absolute");
        }
        if !path.is_dir() {
            bail!("collection path `{root}` is not a directory");
        }
        open.push(PathBuf::from(root));
    }
    let session = Session {
        open,
        ..Session::default()
    };
    save_json(&paths.session, &session)
        .with_context(|| format!("writing {}", paths.session.display()))
}
