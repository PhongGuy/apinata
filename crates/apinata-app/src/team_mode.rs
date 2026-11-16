//! Team mode for bound folders: starting a folder's sync loop from its binding and what the
//! server says now, and re-reading the role on every events connect and each quiet minute.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use apinata_core::SecretStore;
use apinata_core::team::{
    self, Binding, Block, EventsConfig, HttpTeamClient, Resumed, ServerUrl, Standing, TeamConfig,
    TeamError, TeamState, TeamStore,
};
use futures_util::future::{self, Either};
use gpui_kit::prelude::*;
use gpui_kit::{App, AppContext, AsyncApp, Entity, Task, WeakEntity};

use crate::app::AppState;
use crate::team_text;
use crate::team_workspaces::settle_copies;
use crate::tokio_bridge;
use crate::workspace::{Workspace, WorkspaceEvent};

/// How long a running loop goes without an events connect before its role is re-read.
pub const ROLE_CHECK: Duration = Duration::from_secs(60);

/// Starts the loop of the open collection at `root` when the team file binds it, or hands a
/// loop already running there the current sign-in; nothing when git is off, it is unbound,
/// or a resume for it is already running.
pub fn resume(w: &mut Workspace, root: &Path, cx: &mut Context<Workspace>) {
    if w.git_options().is_none() || w.team_resumes().contains(root) || w.team_closing(root) {
        return;
    }
    let Some(state) = cx.try_global::<AppState>() else {
        return;
    };
    let conflicts = state.conflicts.clone();
    let source = Source {
        secrets: Arc::clone(&state.secrets),
        file: state.team.clone(),
    };
    let Some(binding) = source.binding(root) else {
        return;
    };
    let root = root.to_path_buf();
    w.team_resumes().insert(root.clone());
    let ask = source.ask(cx, binding.clone());
    cx.spawn(async move |this, cx| {
        let result = answer(ask).await;
        let _ = this.update(cx, |w, cx| {
            w.team_resumes().remove(&root);
            match result {
                // A running loop keeps its state, such as unsettled clashes, and takes the
                // new token in place; one the server had dropped starts over.
                Ok(resumed) if renewable(w, &root) => renew(w, &root, resumed, cx),
                Ok(resumed) => start(w, &root, &binding, &conflicts, resumed, source, cx),
                Err(e) => notice(&root, "did not start", &e, cx),
            }
            // A sign-in landed while this one asked with the old token: ask again with the new.
            if w.team_reruns().remove(&root) {
                resume(w, &root, cx);
            }
        });
    })
    .detach();
}

/// Resumes every open collection the team file binds to `server`, so a fresh sign-in reaches
/// loops that started without a valid token and those still holding the one it replaced.
pub fn resume_server(workspace: &Entity<Workspace>, server: &ServerUrl, cx: &mut App) {
    let Some(file) = cx.try_global::<AppState>().map(|s| s.team.clone()) else {
        return;
    };
    let Ok(store) = TeamStore::load(&file) else {
        return;
    };
    let bound: Vec<PathBuf> = (store.bindings().iter())
        .filter(|b| b.server == server.as_str())
        .map(|b| b.root.clone())
        .collect();
    workspace.update(cx, |w, cx| {
        let open: Vec<PathBuf> = (bound.into_iter())
            .filter(|root| {
                w.collections()
                    .iter()
                    .any(|c| c.opened() && c.root() == root)
            })
            .collect();
        // One still asking may have read the old token, so it runs again when it lands.
        for root in open {
            if w.team_resumes().contains(&root) {
                w.team_reruns().insert(root);
            } else {
                resume(w, &root, cx);
            }
        }
    });
}

type Asked = Task<Result<Result<Resumed, TeamError>, tokio_bridge::JoinError>>;

// Where a resume reads the token and the team file from.
struct Source {
    secrets: Arc<dyn SecretStore>,
    file: PathBuf,
}

impl Source {
    // Asks the server about `binding` on the tokio runtime.
    fn ask<C: AppContext>(&self, cx: &C, binding: Binding) -> Asked {
        let (secrets, file) = (Arc::clone(&self.secrets), self.file.clone());
        tokio_bridge::spawn(cx, async move {
            let api = HttpTeamClient::new(ServerUrl::parse(&binding.server)?)?;
            team::resume(&api, &*secrets, &file, &binding).await
        })
    }

    // The binding of `root` in the team file as it is now.
    fn binding(&self, root: &Path) -> Option<Binding> {
        TeamStore::load(&self.file).ok()?.binding_at(root).cloned()
    }
}

// A lost runtime reads as an unavailable server.
async fn answer(asked: Asked) -> Result<Resumed, TeamError> {
    asked.await.unwrap_or(Err(TeamError::Unavailable))
}

// Starts the loop from what the resume returned, tells it a standing that blocks it, and
// follows its connects.
fn start(
    w: &mut Workspace,
    root: &Path,
    binding: &Binding,
    conflicts: &Path,
    resumed: Resumed,
    source: Source,
    cx: &mut Context<Workspace>,
) {
    let Some(base) = w.git_options().cloned() else {
        return;
    };
    let copies = settle_copies(conflicts, binding);
    let config = TeamConfig::new(
        root.to_path_buf(),
        copies,
        resumed.auth.clone(),
        resumed.identity.clone(),
        resumed.role,
    )
    .base_options(base);
    let events = (resumed.events_url.clone())
        .map(|url| EventsConfig::new(url, resumed.auth.clone(), resumed.branch.clone()));
    w.start_team(root, config, events, cx);
    apply_blocks(w, root, resumed.standing);
    let Some(connects) = w.team_connects(root) else {
        return;
    };
    // The server announces no grant changes and keeps a stream open across a raised role, so
    // besides each connect the role is re-read once per quiet `ROLE_CHECK`.
    let watch = cx.spawn({
        let root = root.to_path_buf();
        async move |this: WeakEntity<Workspace>, cx: &mut AsyncApp| {
            loop {
                let tick = cx.background_executor().timer(ROLE_CHECK);
                let connect = std::pin::pin!(connects.recv());
                let quiet = match future::select(connect, tick).await {
                    Either::Left((Err(_), _)) => return,
                    Either::Left((Ok(()), _)) => false,
                    Either::Right(_) => true,
                };
                if !reread(&this, &root, &source, quiet, cx).await {
                    return;
                }
            }
        }
    });
    w.keep_team_watch(root, watch);
}

// Whether `root` has a loop a resume can hand a new token to, rather than one to replace.
fn renewable(w: &Workspace, root: &Path) -> bool {
    w.team_status(root)
        .is_some_and(|t| t.state != TeamState::Blocked(Block::Gone))
}

// Gives the running loop at `root` the token, commit identity and events stream of a fresh
// resume, and its role; a standing without a token only blocks the loop.
fn renew(w: &mut Workspace, root: &Path, resumed: Resumed, cx: &mut Context<Workspace>) {
    if matches!(resumed.standing, Standing::Ready | Standing::Offline) {
        let events = (resumed.events_url.clone())
            .map(|url| EventsConfig::new(url, resumed.auth.clone(), resumed.branch.clone()));
        w.renew_team(
            root,
            resumed.auth.clone(),
            resumed.identity.clone(),
            events,
            cx,
        );
    }
    if resumed.standing == Standing::Ready {
        w.set_team_role(root, resumed.role, cx);
    }
    apply_blocks(w, root, resumed.standing);
}

// One connect or periodic check: resume against the current binding and apply the answer to
// the running loop; a `quiet` check tells no error, so a lasting one is not repeated each
// period. False once the workspace or the binding is gone.
async fn reread(
    this: &WeakEntity<Workspace>,
    root: &Path,
    source: &Source,
    quiet: bool,
    cx: &mut AsyncApp,
) -> bool {
    let Some(binding) = source.binding(root) else {
        return false;
    };
    let ask = cx.update(|cx| source.ask(cx, binding));
    let result = answer(ask).await;
    this.update(cx, |w, cx| match result {
        Ok(resumed) => {
            if resumed.standing == Standing::Ready {
                w.set_team_role(root, resumed.role, cx);
            }
            apply_blocks(w, root, resumed.standing);
        }
        Err(_) if quiet => {}
        Err(e) => notice(root, "could not refresh", &e, cx),
    })
    .is_ok()
}

// Signed out or gone is told to the loop; Ready and Offline need nothing more.
fn apply_blocks(w: &Workspace, root: &Path, standing: Standing) {
    let Some(sync) = w.team_sync(root) else {
        return;
    };
    match standing {
        Standing::SignedOut => sync.signed_out(),
        Standing::Gone => sync.gone(),
        _ => {}
    }
}

// `what` says which step failed: the start path and the re-read of a running loop differ.
fn notice(root: &Path, what: &str, e: &TeamError, cx: &mut Context<Workspace>) {
    let name = crate::workspace::display_name(root);
    let text = format!("Team sync of {name} {what}: {}", team_text::error_text(e));
    cx.emit(WorkspaceEvent::FolderNotice(text));
}
