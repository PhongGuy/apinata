//! The workspace list: one section per joined team server, listing the Apinata workspaces the
//! user can reach with their role. Opening one clones it with the server token in git's
//! environment, binds the folder in the team file and starts team mode on it. A server admin
//! can create a seeded workspace there, and each server has its own sign out.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, Global, SharedString, TestSupportExt, WeakEntity, Window, div};

use crate::app::AppState;
use crate::clone_form::{self, FormError, Opened};
use crate::clone_job::{CloneEvent, CloneJob};
use crate::style::SPACE_2;
use crate::team_text::error_text;
use crate::ui::LockButton as _;
use crate::workspace::Workspace;
use crate::{tokio_bridge, ui};
use apinata_core::team::{
    self, Account, Binding, ConflictCopies, HttpTeamClient, Me, NewWorkspace, Role, ServerUrl,
    TeamApi as _, TeamAuth, TeamError, TeamStore, TeamToken, token_key,
};
use apinata_core::{CloneRequest, GitOptions, SecretStore};
use sha2::{Digest as _, Sha256};

/// Where a server's section is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionState {
    /// Asking the server who the user is.
    Loading,
    /// The workspaces are listed.
    Ready,
    /// The server no longer accepts the kept token, or none is kept.
    SignedOut,
    /// The server could not be asked; carries the user text.
    Failed(String),
}

/// One workspace the user can open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceRow {
    /// The owning team's slug.
    pub team: String,
    /// The workspace's slug.
    pub slug: String,
    /// The name shown to people.
    pub name: String,
    /// The user's role on it.
    pub role: Role,
    /// The folder already bound to it, when that folder still exists.
    pub opened: Option<PathBuf>,
}

/// One joined server and what it lists.
pub struct ServerSection {
    server: ServerUrl,
    account: Account,
    state: SectionState,
    rows: Vec<WorkspaceRow>,
    // The server's records behind `rows`, index for index, and the `Me` they came from.
    workspaces: Vec<team::Workspace>,
    me: Option<Me>,
    error: Option<String>,
    admin: bool,
    // The New workspace fields, shown only to a server admin.
    new_team: Entity<InputState>,
    new_slug: Entity<InputState>,
    new_name: Entity<InputState>,
    creating: bool,
    signing_out: bool,
}

impl ServerSection {
    /// The server's address.
    pub fn server(&self) -> &str {
        self.server.as_str()
    }

    /// The signed-in user's name on it.
    pub fn username(&self) -> &str {
        &self.account.username
    }

    /// Whether it is loading, listed, signed out or failed.
    pub fn state(&self) -> &SectionState {
        &self.state
    }

    /// The Apinata workspaces, sorted by team then slug.
    pub fn rows(&self) -> &[WorkspaceRow] {
        &self.rows
    }

    /// The message from the last Open, Create or Sign out, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the user may create workspaces here: the account's server admin flag after the
    /// last load.
    pub fn can_create(&self) -> bool {
        self.admin
    }
}

fn row_for(w: &team::Workspace, opened: Option<PathBuf>) -> WorkspaceRow {
    WorkspaceRow {
        team: w.team.clone(),
        slug: w.slug.clone(),
        name: w.name.clone(),
        role: w.role,
        opened,
    }
}

// The token kept for `username`; none (or not a token) reads as signed out.
fn kept_token(
    secrets: &dyn SecretStore,
    server: &ServerUrl,
    username: &str,
) -> Result<TeamToken, TeamError> {
    let text = (secrets.get(&token_key(server, username)))
        .map_err(|e| TeamError::Secrets(e.to_string()))?;
    (text.and_then(|t| TeamToken::parse(&t).ok())).ok_or(TeamError::Unauthenticated)
}

/// The dialog's body: one section per account in the team file.
pub struct WorkspacesView {
    workspace: Entity<Workspace>,
    sections: Vec<ServerSection>,
    // The clone in flight; while it runs every other Open is ignored. Dropping it cancels.
    job: Option<CloneJob>,
    cloning: bool,
}

// The most recently opened list; it lapses once the dialog lets go of it.
struct OpenView(WeakEntity<WorkspacesView>);
impl Global for OpenView {}

/// The open workspace list, for tests and the pilot; None when no list is open.
pub fn current(cx: &App) -> Option<Entity<WorkspacesView>> {
    cx.try_global::<OpenView>().and_then(|v| v.0.upgrade())
}

/// The folder for the conflict copies of the collection `binding` names: one per server, team
/// and workspace under `conflicts`, each turned into a folder name by `folder_name`.
pub fn copies_dir(conflicts: &Path, binding: &Binding) -> PathBuf {
    conflicts
        .join(folder_name(&binding.server))
        .join(folder_name(&binding.team))
        .join(folder_name(&binding.workspace))
}

/// `copies_dir`, after moving copies and delete marks kept under the folder names used before
/// names were made safe on Windows, so open clashes survive an update. A move that cannot be
/// made leaves the old folder where it is.
pub fn settle_copies(conflicts: &Path, binding: &Binding) -> PathBuf {
    let dir = copies_dir(conflicts, binding);
    let server = escape(&binding.server, |_, _| false);
    let old = conflicts
        .join(server)
        .join(&binding.team)
        .join(&binding.workspace);
    if old == dir {
        return dir;
    }
    let marks_of = |copies: &Path| ConflictCopies::new(copies.to_path_buf()).marks_dir();
    for (from, to) in [(old.clone(), dir.clone()), (marks_of(&old), marks_of(&dir))] {
        if from.exists()
            && !to.exists()
            && to.parent().is_some_and(|p| fs::create_dir_all(p).is_ok())
        {
            let _ = fs::rename(&from, &to);
        }
    }
    // Drops the old team and server folders once nothing is left in them.
    for parent in old.ancestors().skip(1).take(2) {
        let _ = fs::remove_dir(parent);
    }
    dir
}

// One folder name that Windows accepts and no other text shares. Lowercase letters, digits,
// `-` and inner dots stay; every other byte, a final dot and the first byte of a reserved
// device name become `_XX`. A name longer than `MAX_NAME` keeps a prefix and a hash after `~`,
// which no escaped name contains.
fn folder_name(text: &str) -> String {
    const MAX_NAME: usize = 64;
    const PREFIX: usize = 40;
    let stem = text.split('.').next().unwrap_or_default();
    let device = matches!(stem, "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit());
    let name = escape(text, |b, i| {
        (b == b'.' && i + 1 == text.len()) || (device && i == 0)
    });
    if name.len() <= MAX_NAME {
        return name;
    }
    // Cut before an escape that would straddle the prefix's end.
    let mut cut = PREFIX;
    while name[..cut].rfind('_').is_some_and(|at| at + 3 > cut) {
        cut -= 1;
    }
    let hash = Sha256::digest(text.as_bytes());
    let hex: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("{}~{hex}", &name[..cut])
}

// Keeps lowercase letters, digits, `-` and `.` and writes every other byte, or one `extra`
// picks by byte and index, as `_XX`.
fn escape(text: &str, extra: impl Fn(u8, usize) -> bool) -> String {
    let mut out = String::new();
    for (i, b) in text.bytes().enumerate() {
        let plain = b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-';
        if plain && !extra(b, i) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("_{b:02X}"));
        }
    }
    out
}

/// Opens the list over `window` and starts loading each server. With no account in the team
/// file it opens the join dialog instead and returns None.
pub fn open(
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<WorkspacesView>> {
    let file = cx.try_global::<AppState>()?.team.clone();
    // Sorted by server, since a session refresh moves its account to the end of the file.
    let mut accounts = match TeamStore::load(&file) {
        Ok(store) => store.accounts().to_vec(),
        Err(e) => {
            window.push_notification(ui::kind_toast(ui::ToastKind::Error, error_text(&e)), cx);
            return None;
        }
    };
    accounts.sort_by(|a, b| a.server.cmp(&b.server));
    let sections: Vec<ServerSection> = (accounts.into_iter())
        .filter_map(|account| {
            let server = ServerUrl::parse(&account.server).ok()?;
            let mut input = |hint: &str| {
                let hint = hint.to_owned();
                cx.new(|cx| InputState::new(window, cx).placeholder(hint))
            };
            Some(ServerSection {
                server,
                account,
                state: SectionState::Loading,
                rows: Vec::new(),
                workspaces: Vec::new(),
                me: None,
                error: None,
                admin: false,
                new_team: input("team"),
                new_slug: input("slug"),
                new_name: input("Name"),
                creating: false,
                signing_out: false,
            })
        })
        .collect();
    if sections.is_empty() {
        crate::team_join::open(workspace, window, cx);
        return None;
    }
    let view = cx.new(|_| WorkspacesView {
        workspace: workspace.clone(),
        sections,
        job: None,
        cloning: false,
    });
    cx.set_global(OpenView(view.downgrade()));
    view.update(cx, |v, cx| v.load(window, cx));
    let body = view.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        crate::dialog_frame::place(dialog, window)
            .title("Team workspaces")
            .close_button(false)
            .child(body.clone())
    });
    Some(view)
}

impl WorkspacesView {
    /// Every server's section, sorted by server address.
    pub fn sections(&self) -> &[ServerSection] {
        &self.sections
    }

    // The section for `server` now; a sign out may have shifted or dropped it meanwhile.
    fn position(&self, server: &ServerUrl) -> Option<usize> {
        self.sections.iter().position(|s| &s.server == server)
    }

    // Asks each server who the user is, off the main thread, and fills its section. For a
    // server admin it also reads the teams, so New workspace starts on the first one; a teams
    // failure only leaves that field empty.
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = cx.try_global::<AppState>() else {
            return;
        };
        let (secrets, file) = (Arc::clone(&state.secrets), state.team.clone());
        for section in &self.sections {
            let (server, secrets, file) =
                (section.server.clone(), Arc::clone(&secrets), file.clone());
            let username = section.account.username.clone();
            let task = tokio_bridge::spawn(cx, async move {
                let api = HttpTeamClient::new(server.clone())?;
                let me = team::current(&api, &*secrets, &file).await?;
                let store = TeamStore::load(&file)?;
                let mut first_team = None;
                if me.user.server_admin {
                    let user = (store.account(&server)).map_or(username, |a| a.username.clone());
                    if let Ok(token) = kept_token(&*secrets, &server, &user) {
                        let teams = api.teams(&token).await.unwrap_or_default();
                        first_team = teams.into_iter().next().map(|t| t.slug);
                    }
                }
                Ok::<_, TeamError>((me, store, first_team))
            });
            let server = section.server.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await.unwrap_or(Err(TeamError::Unavailable));
                let _ = this.update_in(cx, |this, window, cx| {
                    this.loaded(&server, result, window, cx)
                });
            })
            .detach();
        }
    }

    fn loaded(
        &mut self,
        server: &ServerUrl,
        result: Result<(Me, TeamStore, Option<String>), TeamError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(section) = self.position(server).map(|i| &mut self.sections[i]) else {
            return;
        };
        match result {
            Ok((me, store, first_team)) => {
                section.workspaces = me.apinata_workspaces().into_iter().cloned().collect();
                section.rows = (section.workspaces.iter())
                    .map(|w| {
                        let opened = (store.binding_for(&section.server, &w.team, &w.slug))
                            .map(|b| b.root.clone())
                            .filter(|root| root.is_dir());
                        row_for(w, opened)
                    })
                    .collect();
                section.admin = me.user.server_admin;
                if let Some(slug) = first_team
                    && section.new_team.read(cx).value().is_empty()
                {
                    (section.new_team).update(cx, |i, cx| i.set_value(slug, window, cx));
                }
                section.me = Some(me);
                section.state = SectionState::Ready;
            }
            Err(TeamError::Unauthenticated) => section.state = SectionState::SignedOut,
            Err(e) => section.state = SectionState::Failed(error_text(&e)),
        }
        cx.notify();
    }

    fn set_error(&mut self, section: usize, text: Option<String>, cx: &mut Context<Self>) {
        if let Some(s) = self.sections.get_mut(section) {
            s.error = text;
        }
        cx.notify();
    }

    // `set_error` on the section for `server`, wherever a sign out has moved it.
    fn set_error_on(&mut self, server: &ServerUrl, text: Option<String>, cx: &mut Context<Self>) {
        if let Some(index) = self.position(server) {
            self.set_error(index, text, cx);
        }
    }

    /// Opens a row: its bound folder when it has one, else a team clone of it.
    pub fn open_row(
        &mut self,
        section: usize,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_row_with(section, row, CloneJob::start, window, cx);
    }

    /// `open_row` with the clone job built by `make`, so tests can swap the git source.
    pub fn open_row_with(
        &mut self,
        section: usize,
        row: usize,
        make: impl FnOnce(CloneRequest, GitOptions, &App) -> CloneJob + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cloning {
            return;
        }
        let Some(s) = self.sections.get(section).filter(|s| !s.signing_out) else {
            return;
        };
        let (Some(found), Some(ws), Some(me)) =
            (s.rows.get(row), s.workspaces.get(row), s.me.clone())
        else {
            return;
        };
        if let Some(root) = found.opened.clone() {
            window.close_dialog(cx);
            self.workspace.update(cx, |w, cx| w.open_revealed(root, cx));
            return;
        }
        let Some(state) = cx.try_global::<AppState>() else {
            return;
        };
        let (secrets, file) = (Arc::clone(&state.secrets), state.team.clone());
        let saved = state.settings.read(cx).get().clone_parent.clone();
        let home = dirs::home_dir();
        let Some(parent) = clone_form::default_parent(saved.as_deref(), home.as_deref()) else {
            return self.set_error(section, Some(FormError::NoParent.message()), cx);
        };
        // The names are checked here, before anything is built from them on disk.
        let req = match team::clone_request(&s.server, ws, &parent) {
            Ok(req) => req,
            Err(TeamError::Protocol(m)) => {
                let text = format!("This workspace cannot be opened: {m}.");
                return self.set_error(section, Some(text), cx);
            }
            Err(e) => return self.set_error(section, Some(error_text(&e)), cx),
        };
        let text = match clone_form::check(&req.url, Some(&parent), &req.name) {
            Ok(_) => None,
            Err(FormError::Exists) => Some(format!(
                "A folder named {} already exists in {}.",
                req.name,
                parent.display()
            )),
            Err(e) => Some(e.message()),
        };
        if let Some(text) = text {
            return self.set_error(section, Some(text), cx);
        }
        if let Err(e) = clone_form::ensure_parent(&parent) {
            return self.set_error(section, Some(e.to_string()), cx);
        }
        let base = (self.workspace.read(cx).git_options().cloned()).unwrap_or_default();
        let open = Opening {
            server: s.server.clone(),
            account: s.account.clone(),
            workspace: ws.clone(),
            me,
            base,
            file,
        };
        self.cloning = true;
        self.set_error(section, None, cx);
        // The keychain blocks, so the token is read on the runtime, not here.
        let key = token_key(&open.server, &open.account.username);
        let read = tokio_bridge::spawn(cx, async move { secrets.get(&key) });
        cx.spawn_in(window, async move |this, cx| {
            let token = read.await;
            let _ = this.update_in(cx, |this, window, cx| match token {
                Ok(Ok(Some(token))) => this.clone_with(open, token, req, make, window, cx),
                Ok(Ok(None)) => {
                    this.cloning = false;
                    if let Some(i) = this.position(&open.server) {
                        this.sections[i].state = SectionState::SignedOut;
                    }
                    cx.notify();
                }
                Ok(Err(e)) => {
                    this.cloning = false;
                    let text = error_text(&TeamError::Secrets(e.to_string()));
                    this.set_error_on(&open.server, Some(text), cx);
                }
                Err(_) => {
                    this.cloning = false;
                    this.set_error_on(&open.server, Some(error_text(&TeamError::Unavailable)), cx);
                }
            });
        })
        .detach();
    }

    // Starts the clone with the team env laid over the workspace's git options.
    fn clone_with(
        &mut self,
        open: Opening,
        token: String,
        req: CloneRequest,
        make: impl FnOnce(CloneRequest, GitOptions, &App) -> CloneJob,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let auth = TeamAuth::new(open.server.as_str(), token);
        let opts = team::team_options(open.base.clone(), &auth, &open.me.identity);
        let job = make(req, opts, cx);
        let events = job.events().clone();
        self.job = Some(job);
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let CloneEvent::Finished(result) = event else {
                    continue;
                };
                let _ = this.update_in(cx, |this, window, cx| {
                    this.finished(open, result, window, cx)
                });
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.job = None;
                this.cloning = false;
                let text = "The clone stopped unexpectedly. Try again.".to_owned();
                this.set_error_on(&open.server, Some(text), cx);
            });
        })
        .detach();
    }

    // A finished clone is bound and opened; a failed one shows its error.
    fn finished(
        &mut self,
        open: Opening,
        result: Result<PathBuf, apinata_core::CloneError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.job = None;
        self.cloning = false;
        let root = match result {
            Ok(root) => root,
            Err(e) => return self.set_error_on(&open.server, clone_form::clone_error_text(&e), cx),
        };
        let ws = &open.workspace;
        let binding = Binding::new(root.clone(), &open.account, &ws.team, &ws.slug, ws.role);
        if let Err(e) = TeamStore::update(&open.file, |t| t.put_binding(binding.clone())) {
            return self.set_error_on(&open.server, Some(error_text(&e)), cx);
        }
        window.close_dialog(cx);
        if let Opened::NoCollection(root) = clone_form::classify(&root) {
            let text = clone_form::no_collection_text(&root);
            return window.push_notification(ui::info_toast(text), cx);
        }
        // The open's first snapshot resumes the bound folder, which starts its loop.
        self.workspace.update(cx, |w, cx| w.open_revealed(root, cx));
    }

    /// Closes the list and opens the join dialog for this section's server and user.
    pub fn sign_in_again(&mut self, section: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.sections.get(section) else {
            return;
        };
        let (server, username) = (s.server.to_string(), s.account.username.clone());
        window.close_dialog(cx);
        crate::team_join::open_for(&self.workspace, &server, &username, window, cx);
    }

    /// Fills the New workspace fields of `section`.
    pub fn set_new_workspace(
        &mut self,
        section: usize,
        team: &str,
        slug: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.sections.get(section) else {
            return;
        };
        for (input, text) in [
            (&s.new_team, team),
            (&s.new_slug, slug),
            (&s.new_name, name),
        ] {
            input.update(cx, |i, cx| i.set_value(text, window, cx));
        }
        cx.notify();
    }

    /// Creates the workspace the New workspace fields describe. Fields that fail the server's
    /// rules show why with nothing sent; a created workspace joins the list in sorted place.
    pub fn create_workspace(
        &mut self,
        section: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.sections.get(section) else {
            return;
        };
        if s.creating || s.signing_out || !s.admin {
            return;
        }
        let field = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let new = NewWorkspace::new(
            &field(&s.new_team),
            &field(&s.new_slug),
            &field(&s.new_name),
        );
        let new = match new {
            Ok(new) => new,
            Err(e) => return self.set_error(section, Some(error_text(&e)), cx),
        };
        let Some(state) = cx.try_global::<AppState>() else {
            return;
        };
        let secrets = Arc::clone(&state.secrets);
        let (server, username) = (s.server.clone(), s.account.username.clone());
        let task = tokio_bridge::spawn(cx, {
            let server = server.clone();
            async move {
                let token = kept_token(&*secrets, &server, &username)?;
                HttpTeamClient::new(server)?
                    .create_workspace(&token, &new)
                    .await
            }
        });
        if let Some(s) = self.sections.get_mut(section) {
            s.creating = true;
        }
        self.set_error(section, None, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await.unwrap_or(Err(TeamError::Unavailable));
            let _ = this.update_in(cx, |this, window, cx| {
                this.created(&server, result, window, cx)
            });
        })
        .detach();
    }

    fn created(
        &mut self,
        server: &ServerUrl,
        result: Result<team::Workspace, TeamError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.position(server) else {
            return;
        };
        let s = &mut self.sections[index];
        s.creating = false;
        let ws = match result {
            Ok(ws) => ws,
            Err(e) => return self.set_error(index, Some(error_text(&e)), cx),
        };
        if ws.kind == team::APINATA_KIND {
            let key = (&ws.team, &ws.slug);
            let at = (s.workspaces).partition_point(|w| (&w.team, &w.slug) < key);
            s.rows.insert(at, row_for(&ws, None));
            s.workspaces.insert(at, ws);
        }
        for input in [&s.new_slug, &s.new_name] {
            input.update(cx, |i, cx| i.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Signs out of `section`'s server: revokes its token and forgets the account, keeping the
    /// bound folders. A failure keeps the section and says why. Ignored while a clone runs or
    /// while this server is creating a workspace, so the created one still reaches the list.
    pub fn sign_out(&mut self, section: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.cloning {
            return;
        }
        let Some(s) = self.sections.get_mut(section) else {
            return;
        };
        if s.signing_out || s.creating {
            return;
        }
        let Some(state) = cx.try_global::<AppState>() else {
            return;
        };
        let (secrets, file) = (Arc::clone(&state.secrets), state.team.clone());
        let server = s.server.clone();
        s.signing_out = true;
        let task = tokio_bridge::spawn(cx, {
            let server = server.clone();
            async move { team::sign_out(&HttpTeamClient::new(server)?, &*secrets, &file).await }
        });
        self.set_error(section, None, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await.unwrap_or(Err(TeamError::Unavailable));
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(index) = this.position(&server) else {
                    return;
                };
                this.sections[index].signing_out = false;
                match result {
                    Ok(()) => {
                        this.sections.remove(index);
                        let text = format!("Signed out of {server}.");
                        window.push_notification(ui::info_toast(text), cx);
                        cx.notify();
                    }
                    Err(e) => this.set_error(index, Some(error_text(&e)), cx),
                }
            });
        })
        .detach();
    }
}

// What an Open captured when it started, so the binding uses that account and workspace.
struct Opening {
    server: ServerUrl,
    account: Account,
    workspace: team::Workspace,
    me: Me,
    base: GitOptions,
    file: PathBuf,
}

impl Render for WorkspacesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let sections = self.sections.iter().enumerate().map(|(i, s)| {
            let header = h_flex()
                .gap(SPACE_2)
                .child(
                    div()
                        .flex_1()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(format!("{} on {}", s.account.username, s.server)),
                )
                .child(
                    Button::new(("team-sign-out", i))
                        .label(if s.signing_out {
                            "Signing out…"
                        } else {
                            "Sign out"
                        })
                        .lock(s.signing_out || s.creating || self.cloning, cx)
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.sign_out(i, window, cx)),
                        ),
                );
            let body = match &s.state {
                SectionState::Loading => div()
                    .text_color(muted)
                    .child("Loading workspaces…")
                    .into_any_element(),
                SectionState::SignedOut => h_flex()
                    .gap(SPACE_2)
                    .child(format!("You are signed out of {}.", s.server))
                    .child(
                        Button::new(("team-sign-in-again", i))
                            .label("Sign in again")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.sign_in_again(i, window, cx)
                            })),
                    )
                    .into_any_element(),
                SectionState::Failed(text) => div()
                    .text_color(cx.theme().danger)
                    .child(text.clone())
                    .into_any_element(),
                SectionState::Ready if s.rows.is_empty() => div()
                    .text_color(muted)
                    .child("No Apinata workspaces on this server yet.")
                    .into_any_element(),
                SectionState::Ready => v_flex()
                    .gap(SPACE_2)
                    .children(s.rows.iter().enumerate().map(|(r, row)| {
                        let id = SharedString::from(format!("team-open-{i}-{r}"));
                        h_flex()
                            .gap(SPACE_2)
                            .child(div().flex_1().child(row.name.clone()))
                            .child(
                                div()
                                    .text_color(muted)
                                    .child(format!("{}/{}", row.team, row.slug)),
                            )
                            .child(div().text_color(muted).child(row.role.label()))
                            .child(
                                Button::new(id)
                                    .label(if self.cloning { "Opening…" } else { "Open" })
                                    .lock(self.cloning, cx)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_row(i, r, window, cx)
                                    })),
                            )
                    }))
                    .into_any_element(),
            };
            let create = (s.admin && s.state == SectionState::Ready).then(|| {
                let field = |id: &'static str, input: &Entity<InputState>| {
                    div().flex_1().child(ui::padded_input(
                        Input::new(input)
                            .id((id, i))
                            .w_full()
                            .disabled(s.creating)
                            .bg(ui::field_fill(s.creating, cx)),
                        input,
                    ))
                };
                v_flex()
                    .gap(SPACE_2)
                    .child(div().text_color(muted).child("New workspace"))
                    .child(
                        h_flex()
                            .gap(SPACE_2)
                            .child(field("team-new-team", &s.new_team))
                            .child(field("team-new-slug", &s.new_slug))
                            .child(field("team-new-name", &s.new_name))
                            .child(
                                Button::new(("team-create", i))
                                    .label(if s.creating { "Creating…" } else { "Create" })
                                    .lock(s.creating, cx)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.create_workspace(i, window, cx)
                                    })),
                            ),
                    )
            });
            let error = s.error.clone().map(|message| {
                div()
                    .id(("team-section-error", i))
                    .test_support()
                    .text_color(cx.theme().danger)
                    .child(message)
            });
            v_flex()
                .id(("team-section", i))
                .test_support()
                .gap(SPACE_2)
                .child(header)
                .child(body)
                .children(create)
                .children(error)
        });
        v_flex()
            .id("team-workspaces-dialog")
            .test_support()
            .gap(SPACE_2)
            .children(sections)
            .child(ui::dialog_footer(
                None,
                [],
                Button::new("team-workspaces-close")
                    .label("Close")
                    .primary()
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            ))
    }
}
