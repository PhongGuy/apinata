//! The workspace list: one section per joined team server, listing the Apinata workspaces the
//! user can reach with their role. Opening one clones it with the server token in git's
//! environment, binds the folder in the team file and starts team mode on it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
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
    self, Account, Binding, EventsConfig, HttpTeamClient, Me, Role, ServerUrl, TeamAuth,
    TeamConfig, TeamError, TeamStore, token_key,
};
use apinata_core::{CloneRequest, GitOptions};

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

    /// The message from the last Open, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
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
/// and workspace under `conflicts`. The server address becomes one folder name by keeping
/// lowercase letters, digits, `.` and `-` and writing every other byte as `_XX`, so no two
/// addresses share a folder.
pub fn copies_dir(conflicts: &Path, binding: &Binding) -> PathBuf {
    let mut server = String::new();
    for b in binding.server.bytes() {
        if b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-' {
            server.push(char::from(b));
        } else {
            server.push_str(&format!("_{b:02X}"));
        }
    }
    conflicts
        .join(server)
        .join(&binding.team)
        .join(&binding.workspace)
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
            Some(ServerSection {
                server: ServerUrl::parse(&account.server).ok()?,
                account,
                state: SectionState::Loading,
                rows: Vec::new(),
                workspaces: Vec::new(),
                me: None,
                error: None,
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

    // Asks each server who the user is, off the main thread, and fills its section.
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = cx.try_global::<AppState>() else {
            return;
        };
        let (secrets, file) = (Arc::clone(&state.secrets), state.team.clone());
        for (index, section) in self.sections.iter().enumerate() {
            let (server, secrets, file) =
                (section.server.clone(), Arc::clone(&secrets), file.clone());
            let task = tokio_bridge::spawn(cx, async move {
                let api = HttpTeamClient::new(server)?;
                let me = team::current(&api, &*secrets, &file).await?;
                Ok::<_, TeamError>((me, TeamStore::load(&file)?))
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await.unwrap_or(Err(TeamError::Unavailable));
                let _ = this.update(cx, |this, cx| this.loaded(index, result, cx));
            })
            .detach();
        }
    }

    fn loaded(
        &mut self,
        index: usize,
        result: Result<(Me, TeamStore), TeamError>,
        cx: &mut Context<Self>,
    ) {
        let Some(section) = self.sections.get_mut(index) else {
            return;
        };
        match result {
            Ok((me, store)) => {
                section.workspaces = me.apinata_workspaces().into_iter().cloned().collect();
                section.rows = (section.workspaces.iter())
                    .map(|w| WorkspaceRow {
                        team: w.team.clone(),
                        slug: w.slug.clone(),
                        name: w.name.clone(),
                        role: w.role,
                        opened: (store.binding_for(&section.server, &w.team, &w.slug))
                            .map(|b| b.root.clone())
                            .filter(|root| root.is_dir()),
                    })
                    .collect();
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
        let Some(s) = self.sections.get(section) else {
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
        let (secrets, file, conflicts) = (
            Arc::clone(&state.secrets),
            state.team.clone(),
            state.conflicts.clone(),
        );
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
            section,
            server: s.server.clone(),
            account: s.account.clone(),
            workspace: ws.clone(),
            me,
            base,
            file,
            conflicts,
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
                    if let Some(s) = this.sections.get_mut(open.section) {
                        s.state = SectionState::SignedOut;
                    }
                    cx.notify();
                }
                Ok(Err(e)) => {
                    this.cloning = false;
                    let text = error_text(&TeamError::Secrets(e.to_string()));
                    this.set_error(open.section, Some(text), cx);
                }
                Err(_) => {
                    this.cloning = false;
                    this.set_error(open.section, Some(error_text(&TeamError::Unavailable)), cx);
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
                    this.finished(open, auth, result, window, cx)
                });
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.job = None;
                this.cloning = false;
                let text = "The clone stopped unexpectedly. Try again.".to_owned();
                this.set_error(open.section, Some(text), cx);
            });
        })
        .detach();
    }

    // A finished clone is bound, opened and put in team mode; a failed one shows its error.
    fn finished(
        &mut self,
        open: Opening,
        auth: TeamAuth,
        result: Result<PathBuf, apinata_core::CloneError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.job = None;
        self.cloning = false;
        let root = match result {
            Ok(root) => root,
            Err(e) => return self.set_error(open.section, clone_form::clone_error_text(&e), cx),
        };
        let ws = &open.workspace;
        let binding = Binding::new(root.clone(), &open.account, &ws.team, &ws.slug, ws.role);
        if let Err(e) = TeamStore::update(&open.file, |t| t.put_binding(binding.clone())) {
            return self.set_error(open.section, Some(error_text(&e)), cx);
        }
        window.close_dialog(cx);
        if let Opened::NoCollection(root) = clone_form::classify(&root) {
            let text = clone_form::no_collection_text(&root);
            return window.push_notification(ui::info_toast(text), cx);
        }
        let identity = open.me.identity.clone();
        let copies = copies_dir(&open.conflicts, &binding);
        let config = TeamConfig::new(root.clone(), copies, auth.clone(), identity, ws.role)
            .base_options(open.base);
        let events = EventsConfig::new(ws.events_url.clone(), auth, ws.default_branch.clone());
        self.workspace.update(cx, |w, cx| {
            w.open_revealed(root.clone(), cx);
            w.start_team(&root, config, Some(events), cx);
        });
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
}

// What an Open captured when it started, so the binding uses that account and workspace.
struct Opening {
    section: usize,
    server: ServerUrl,
    account: Account,
    workspace: team::Workspace,
    me: Me,
    base: GitOptions,
    file: PathBuf,
    conflicts: PathBuf,
}

impl Render for WorkspacesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let sections = self.sections.iter().enumerate().map(|(i, s)| {
            let header = div()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(format!("{} on {}", s.account.username, s.server));
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
