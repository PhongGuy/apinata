//! Team mode for bound folders: starting a folder's sync loop from its binding and what the
//! server says now, and re-reading the role on every events connect.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::SecretStore;
use apinata_core::team::{
    self, Binding, EventsConfig, HttpTeamClient, Resumed, ServerUrl, Standing, TeamConfig,
    TeamError, TeamStore,
};
use gpui_kit::prelude::*;
use gpui_kit::{AppContext, AsyncApp, Task, WeakEntity};

use crate::app::AppState;
use crate::team_text;
use crate::team_workspaces::copies_dir;
use crate::tokio_bridge;
use crate::workspace::{Workspace, WorkspaceEvent};

/// Starts (or restarts) the loop of the open collection at `root` when the team file binds it;
/// nothing when git is off, it is unbound, or a resume for it is already running.
pub fn resume(w: &mut Workspace, root: &Path, cx: &mut Context<Workspace>) {
    if w.git_options().is_none() || w.team_resumes().contains(root) {
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
                Ok(resumed) => start(w, &root, &binding, &conflicts, resumed, source, cx),
                Err(e) => notice(&root, &e, cx),
            }
        });
    })
    .detach();
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
    let copies = copies_dir(conflicts, binding);
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
    let watch = cx.spawn({
        let root = root.to_path_buf();
        async move |this: WeakEntity<Workspace>, cx: &mut AsyncApp| {
            while connects.recv().await.is_ok() {
                if !reread(&this, &root, &source, cx).await {
                    return;
                }
            }
        }
    });
    w.keep_team_watch(root, watch);
}

// One connect: resume against the current binding and apply the answer to the running loop.
// False once the workspace or the binding is gone.
async fn reread(
    this: &WeakEntity<Workspace>,
    root: &Path,
    source: &Source,
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
        Err(e) => notice(root, &e, cx),
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

fn notice(root: &Path, e: &TeamError, cx: &mut Context<Workspace>) {
    let name = root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let text = format!(
        "Team sync of {name} did not start: {}",
        team_text::error_text(e)
    );
    cx.emit(WorkspaceEvent::FolderNotice(text));
}
