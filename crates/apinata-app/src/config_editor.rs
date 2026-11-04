//! The settings editor of a folder or the collection: headers, auth, vars, script, tests and
//! docs, plus the collection's trusted hosts, network settings and script trust.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::{ItemPath, KeyValue, PostResponseVar, ScriptPhase, Variable};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::app::AppState;
use crate::auth_section::{AuthMode, AuthScope, AuthSection};
use crate::banner::{self, Banner, Choice};
use crate::command::{Command, Edit, Field, Notice, NoticeKind, Target};
use crate::edit_history::EditHistory;
use crate::edit_track::{self, LoadStamps, same_value};
use crate::kv_table::{
    self, KvColumns, KvEvent, KvRow, KvTable, header_rows, headers_from, post_response_block,
    post_response_rows, post_response_vars_from, var_rows, vars_from, vars_heading,
};
use crate::network_section::NetworkSection;
use crate::presets_section::PresetsSection;
use crate::request_editor::{AUTH_TYPING, RedoEdit, UndoEdit, load_auth, refresh_token_panel};
use crate::script_section::ScriptSection;
use crate::script_settings::ScriptSettingsView;
use crate::section::SectionEdit;
use crate::settings_section::DocsSection;
use crate::snapshot::ConfigView;
use crate::style;
use crate::tag_input::{TagField, TagsChanged};
use crate::token_panel::{Locate, TokenPanel};
use crate::trusted_hosts::TrustedHostsView;
use crate::ui;
use crate::workspace::{Workspace, WorkspaceEvent};

actions!(
    apinata,
    [
        /// Saves the folder's or collection's settings to its file.
        SaveConfig
    ]
);

/// Whose settings an editor shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum ConfigTarget {
    Folder(ItemPath),
    Collection,
}

impl ConfigTarget {
    /// The command target of the settings file.
    pub fn target(&self) -> Target {
        match self {
            ConfigTarget::Folder(path) => Target::Folder(path.clone()),
            ConfigTarget::Collection => Target::Collection,
        }
    }
}

/// A part of the settings the editor shows, in Bruno's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ConfigSection {
    Headers,
    Auth,
    Vars,
    Script,
    Tests,
    Docs,
    Tags,
    Hosts,
    Network,
    Presets,
    Scripts,
}

impl ConfigSection {
    // The sections every folder and collection has; `Hosts`, `Network` and `Scripts` follow
    // for the collection.
    const ALL: [ConfigSection; 6] = [
        ConfigSection::Headers,
        ConfigSection::Auth,
        ConfigSection::Vars,
        ConfigSection::Script,
        ConfigSection::Tests,
        ConfigSection::Docs,
    ];

    fn label(self) -> &'static str {
        match self {
            ConfigSection::Headers => "Headers",
            ConfigSection::Auth => "Auth",
            ConfigSection::Vars => "Vars",
            ConfigSection::Script => "Script",
            ConfigSection::Tests => "Tests",
            ConfigSection::Docs => "Docs",
            ConfigSection::Tags => "Tags",
            ConfigSection::Hosts => "Trusted hosts",
            ConfigSection::Network => "Network",
            ConfigSection::Presets => "Presets",
            ConfigSection::Scripts => "Script trust",
        }
    }
}

/// Edits the settings of a folder or the collection through the workspace; each section
/// follows the snapshots only while it has no edit in flight, as the request editor does.
pub struct ConfigEditor {
    workspace: Entity<Workspace>,
    compare: Option<Entity<crate::compare_dialog::CompareDialog>>,
    root: PathBuf,
    target: ConfigTarget,
    headers: Entity<KvTable>,
    vars: Entity<KvTable>,
    post_response: Entity<KvTable>,
    auth: Entity<AuthSection>,
    docs: Entity<DocsSection>,
    tags: Option<Entity<TagField>>,
    script: Entity<ScriptSection>,
    tests_script: Entity<ScriptSection>,
    hosts: Option<Entity<TrustedHostsView>>,
    network: Option<Entity<NetworkSection>>,
    presets: Option<Entity<PresetsSection>>,
    scripts: Option<Entity<ScriptSettingsView>>,
    header_base: Vec<KeyValue>,
    var_base: Vec<Variable>,
    post_response_base: Vec<PostResponseVar>,
    section: ConfigSection,
    sent: HashMap<u64, ConfigSection>,
    // The snapshots each field's controls loaded; its edits are built from that content.
    loaded: LoadStamps<Field>,
    // Set while a table change is sent whose rows the table no longer shows.
    lagging: bool,
    history: EditHistory,
    // The dirty flag and reload stamp the last refresh saw; a save or a reload ends typing runs.
    checkpoint: Option<(bool, u64)>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl ConfigEditor {
    /// An editor for the settings of `target` in the collection at `root`.
    pub fn new(
        workspace: Entity<Workspace>,
        root: PathBuf,
        target: ConfigTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let root = std::path::absolute(&root).unwrap_or(root);
        let headers = cx.new(|cx| {
            let mut t = KvTable::new(window, cx);
            t.set_columns(KvColumns::DESCRIBED, cx);
            t.set_header_hints(true);
            t
        });
        let vars = cx.new(|cx| KvTable::new(window, cx));
        let post_response = cx.new(|cx| {
            let mut t = KvTable::new(window, cx);
            t.set_placeholders("Name", "res.body.token", window, cx);
            t.set_headings("Name", "Expression", cx);
            t
        });
        let scope = match target {
            ConfigTarget::Collection => AuthScope::Collection,
            ConfigTarget::Folder(_) => AuthScope::Folder,
        };
        let auth = cx.new(|cx| {
            let mut a = AuthSection::new(window, cx);
            a.set_scope(scope, cx);
            a
        });
        let docs = cx.new(|cx| DocsSection::new(window, cx));
        // The collection file carries no tags: only a folder's `info.tags` filter a run.
        let tags = matches!(target, ConfigTarget::Folder(_))
            .then(|| cx.new(|cx| TagField::new(window, cx)));
        let script = cx.new(|cx| {
            ScriptSection::new(
                &[ScriptPhase::PreRequest, ScriptPhase::PostResponse],
                window,
                cx,
            )
        });
        let tests_script = cx.new(|cx| ScriptSection::new(&[ScriptPhase::Tests], window, cx));
        // Only an OAuth 2.0 auth has a token to read; the weak handle avoids a cycle.
        let locate: Locate = {
            let (root, target, auth) = (root.clone(), target.target(), auth.downgrade());
            Box::new(move |cx| {
                let shown = auth.upgrade()?.read(cx).mode();
                (shown == Some(AuthMode::OAuth2)).then(|| (root.clone(), target.clone()))
            })
        };
        let panel = cx.new(|cx| TokenPanel::new(workspace.clone(), locate, cx));
        let oauth2 = auth.read(cx).oauth2().clone();
        oauth2.update(cx, |o, cx| o.set_token_panel(Some(panel), cx));
        // Bruno's collection auth has nothing to inherit from, so it is not offered there.
        if target == ConfigTarget::Collection {
            let modes: Vec<AuthMode> = AuthMode::ALL
                .into_iter()
                .filter(|m| *m != AuthMode::Inherit)
                .collect();
            auth.update(cx, |a, cx| a.set_modes(&modes, window, cx));
        }
        // The trust file is per user and keyed by collection, so only the collection's
        // settings show it, and only once the app has its approvals.
        let trust_file = cx
            .try_global::<AppState>()
            .map(|s| s.approvals.read(cx).trust_file().to_path_buf());
        let hosts = trust_file
            .filter(|_| target == ConfigTarget::Collection)
            .map(|file| {
                let root = root.clone();
                cx.new(|cx| TrustedHostsView::new(file, root, window, cx))
            });
        let script_trust = cx
            .try_global::<AppState>()
            .and_then(|s| s.approvals.read(cx).script_trust().map(Path::to_path_buf));
        let scripts = script_trust
            .filter(|_| target == ConfigTarget::Collection)
            .map(|file| {
                let root = root.clone();
                cx.new(|cx| ScriptSettingsView::new(file, root, cx))
            });
        let network = (target == ConfigTarget::Collection).then(|| {
            let root = root.clone();
            cx.new(|cx| NetworkSection::new(root, window, cx))
        });
        let presets = (target == ConfigTarget::Collection)
            .then(|| cx.new(|cx| PresetsSection::new(window, cx)));
        let mut subs = vec![
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.refresh(window, cx)
            }),
            cx.subscribe_in(&workspace, window, |this, _, event, _, cx| match event {
                WorkspaceEvent::Notices { root, notices, .. } => {
                    this.mark_refused(root, notices, cx)
                }
                WorkspaceEvent::Delivered { root, acked, .. } => this.delivered(root, *acked),
                _ => {}
            }),
        ];
        for (table, section) in [
            (&headers, ConfigSection::Headers),
            (&vars, ConfigSection::Vars),
        ] {
            subs.push(cx.subscribe_in(
                table,
                window,
                move |this, table, event: &KvEvent, window, cx| {
                    let KvEvent::Changed(rows) = event;
                    this.lagging = kv_table::lags(table, rows, cx);
                    this.edit_rows(section, rows.clone(), window, cx);
                    this.lagging = false;
                },
            ));
        }
        subs.push(cx.subscribe_in(
            &post_response,
            window,
            |this, table, event: &KvEvent, _, cx| {
                let KvEvent::Changed(rows) = event;
                this.lagging = kv_table::lags(table, rows, cx);
                this.send_post_response(rows, cx);
                this.lagging = false;
            },
        ));
        // The text sections read their editor when its change arrives, after any refresh,
        // so their edits never lag.
        let on_edit = |this: &mut Self, edit: &SectionEdit, cx: &mut Context<Self>| {
            this.send(edit.edit.clone(), None, cx)
        };
        // A section edit is delivered after any refresh queued ahead of it, so it lags when
        // the section, reloaded since, would now send something else.
        subs.extend([
            cx.subscribe_in(&auth, window, |this, section, e: &SectionEdit, _, cx| {
                this.lagging = section.read(cx).lags(e);
                // Keystrokes in the auth fields share one typing key, so a run is one step.
                this.send_typed(
                    e.edit.clone(),
                    Some(ConfigSection::Auth),
                    Some(AUTH_TYPING),
                    cx,
                );
                this.lagging = false;
            }),
            cx.subscribe_in(&docs, window, move |this, _, e, _, cx| on_edit(this, e, cx)),
            cx.subscribe_in(&script, window, move |this, _, e, _, cx| {
                on_edit(this, e, cx)
            }),
            cx.subscribe_in(&tests_script, window, move |this, _, e, _, cx| {
                on_edit(this, e, cx)
            }),
        ]);
        if let Some(tags) = &tags {
            subs.push(cx.subscribe(tags, |this, field, e: &TagsChanged, cx| {
                this.lagging = field.read(cx).lags(&e.0);
                this.send(Edit::Tags(e.0.clone()), None, cx);
                this.lagging = false;
            }));
        }
        if let Some(network) = &network {
            subs.push(cx.subscribe(
                network,
                |this, _, action: &crate::network_section::NetworkAction, cx| {
                    let root = this.root.clone();
                    this.workspace
                        .update(cx, |w, cx| w.send(&root, Command::Network(action.0), cx));
                },
            ));
            subs.push(
                cx.subscribe_in(network, window, |this, section, e: &SectionEdit, _, cx| {
                    this.lagging = section.read(cx).lags(e, cx);
                    this.send(e.edit.clone(), Some(ConfigSection::Network), cx);
                    this.lagging = false;
                }),
            );
        }
        if let Some(presets) = &presets {
            subs.push(
                cx.subscribe_in(presets, window, |this, section, e: &SectionEdit, _, cx| {
                    this.lagging = section.read(cx).lags(&e.edit, cx);
                    this.send(e.edit.clone(), Some(ConfigSection::Presets), cx);
                    this.lagging = false;
                }),
            );
        }
        let mut editor = ConfigEditor {
            workspace,
            compare: None,
            root,
            target,
            headers,
            vars,
            post_response,
            auth,
            docs,
            tags,
            script,
            tests_script,
            hosts,
            network,
            presets,
            scripts,
            header_base: Vec::new(),
            var_base: Vec::new(),
            post_response_base: Vec::new(),
            section: ConfigSection::Headers,
            sent: HashMap::new(),
            loaded: LoadStamps::default(),
            lagging: false,
            history: EditHistory::default(),
            checkpoint: None,
            focus: cx.focus_handle(),
            _subs: subs,
        };
        editor.refresh(window, cx);
        editor
    }

    /// The section shown.
    pub fn section(&self) -> ConfigSection {
        self.section
    }

    /// Shows `section`; one this editor does not offer is ignored.
    pub fn set_section(&mut self, section: ConfigSection, cx: &mut Context<Self>) {
        if self.sections().contains(&section) {
            self.section = section;
            cx.notify();
        }
    }

    /// The folder's tag chip input; the collection has none.
    pub fn tags(&self) -> Option<&Entity<TagField>> {
        self.tags.as_ref()
    }

    /// The sections offered, in tab order; `Tags` follows for a folder, and `Hosts`, `Network` and `Scripts` follow, collection only.
    pub fn sections(&self) -> Vec<ConfigSection> {
        let mut sections = ConfigSection::ALL.to_vec();
        if self.tags.is_some() {
            sections.push(ConfigSection::Tags);
        }
        if self.hosts.is_some() {
            sections.push(ConfigSection::Hosts);
        }
        if self.network.is_some() {
            sections.push(ConfigSection::Network);
        }
        if self.presets.is_some() {
            sections.push(ConfigSection::Presets);
        }
        if self.scripts.is_some() {
            sections.push(ConfigSection::Scripts);
        }
        sections
    }

    /// The network section, present only in the collection's settings.
    pub fn network(&self) -> Option<&Entity<NetworkSection>> {
        self.network.as_ref()
    }

    /// The presets section, present only in the collection's settings.
    pub fn presets(&self) -> Option<&Entity<PresetsSection>> {
        self.presets.as_ref()
    }

    /// The trusted-hosts section, present only in the collection's settings.
    pub fn hosts(&self) -> Option<&Entity<TrustedHostsView>> {
        self.hosts.as_ref()
    }

    /// The table of a table section: headers or vars.
    pub fn table(&self, section: ConfigSection) -> Option<&Entity<KvTable>> {
        match section {
            ConfigSection::Headers => Some(&self.headers),
            ConfigSection::Vars => Some(&self.vars),
            _ => None,
        }
    }

    /// The Post-response table under the Vars table.
    pub fn post_response_table(&self) -> &Entity<KvTable> {
        &self.post_response
    }

    /// The skip hints of the enabled rows with an expression, in row order.
    pub fn post_response_hints(&self, cx: &App) -> Vec<String> {
        let rows = self.post_response.read(cx).rows(cx);
        kv_table::post_response_hints(&self.post_response_base, &rows)
    }

    /// Sends `rows` as the new post-response variables.
    pub fn edit_post_response_rows(
        &mut self,
        rows: Vec<KvRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Leave the table as typing would have, so the hints follow the rows at once.
        self.post_response
            .update(cx, |t, cx| t.load(rows.clone(), window, cx));
        self.send_post_response(&rows, cx);
    }

    fn send_post_response(&mut self, rows: &[KvRow], cx: &mut Context<Self>) {
        let vars = post_response_vars_from(&self.post_response_base, rows);
        let typed = self.post_response.read(cx).last_typed();
        self.send_typed(Edit::PostResponseVars(vars), None, typed, cx);
    }

    /// The auth section.
    pub fn auth(&self) -> &Entity<AuthSection> {
        &self.auth
    }

    /// The docs section.
    pub fn docs(&self) -> &Entity<DocsSection> {
        &self.docs
    }

    /// The Script section: the pre-request and post-response editors.
    pub fn script(&self) -> &Entity<ScriptSection> {
        &self.script
    }

    /// The Tests section: the tests editor.
    pub fn tests_script(&self) -> &Entity<ScriptSection> {
        &self.tests_script
    }

    /// Sends `rows` as the new contents of a table section and clears its refusal message.
    pub fn edit_rows(
        &mut self,
        section: ConfigSection,
        rows: Vec<KvRow>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = match section {
            ConfigSection::Headers => Edit::Headers(headers_from(&self.header_base, &rows)),
            ConfigSection::Vars => Edit::Vars(vars_from(&self.var_base, &rows)),
            _ => return,
        };
        if let Some(table) = self.table(section).cloned() {
            table.update(cx, |t, cx| t.set_error(None, None, cx));
        }
        let typed = self.table(section).and_then(|t| t.read(cx).last_typed());
        self.send_typed(edit, Some(section), typed, cx);
    }

    /// Takes back the last structured edit (headers, vars, post-response vars, tags or auth)
    /// by sending its earlier value. Does nothing when there is none or when the field was
    /// changed from outside, which forgets that field's steps.
    pub fn undo_edit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(true, cx);
    }

    /// Redoes the edit [`Self::undo_edit`] took back, under the same conditions.
    pub fn redo_edit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(false, cx);
    }

    // Applies the history's next value as an ordinary, unrecorded edit; the tables follow
    // the echo. An edit in flight on the field makes the history trust its own last value.
    fn step_history(&mut self, undo: bool, cx: &mut Context<Self>) {
        let Some(field) = self.history.next_field(undo) else {
            return;
        };
        let Some(view) = self.view(cx) else {
            return;
        };
        let busy = self.busy(field, cx);
        let holds = |edit: &Edit| view_edit(&view, edit).is_some_and(|h| same_value(&h, edit));
        let Some(edit) = edit_track::step(&mut self.history, undo, busy, &holds) else {
            return;
        };
        let section = match field {
            Field::Headers => Some(ConfigSection::Headers),
            Field::Vars => Some(ConfigSection::Vars),
            _ => None,
        };
        if let Some(table) = section.and_then(|s| self.table(s).cloned()) {
            table.update(cx, |t, cx| t.set_error(None, None, cx));
        }
        self.dispatch(edit, section, cx);
    }

    /// Whether an OAuth secret is typed here but not yet sent.
    pub fn holds_secret_typing(&self, cx: &App) -> bool {
        self.auth.read(cx).holds_pending(cx)
    }

    /// Sends the edit an OAuth secret still focused owes, as its blur would; true when one
    /// went out.
    pub fn commit_pending_secret(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.auth.update(cx, |a, cx| a.take_pending(cx)) else {
            return false;
        };
        self.send(edit, Some(ConfigSection::Auth), cx);
        true
    }

    /// The warnings a save gives for typed values it cannot take.
    pub fn unsaved(&self, cx: &App) -> Vec<String> {
        self.network()
            .map(|n| n.read(cx).unsaved())
            .unwrap_or_default()
    }

    /// Names each typed value a save cannot take in a warning, since the save keeps the last
    /// valid one instead.
    pub fn warn_unsaved(&self, window: &mut Window, cx: &mut App) {
        for warning in self.unsaved(cx) {
            window.push_notification(Notification::warning(warning), cx);
        }
    }

    /// Writes the settings to their file.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        self.commit_pending_secret(cx);
        let target = self.target.target();
        let root = self.root.clone();
        // The typed scripts are approved only once the write is confirmed.
        let texts = self.typed_scripts(cx);
        self.workspace.update(cx, |w, cx| {
            let id = w.send(&root, Command::Save(target.clone()), cx);
            w.approve_when_saved(&root, id, Some(target), texts);
        });
    }

    /// The script and test texts the user typed here, never text loaded from the file:
    /// what a save of these settings vouches for.
    pub fn typed_scripts(&self, cx: &App) -> Vec<String> {
        let mut texts = self.script.read(cx).typed_texts(cx);
        texts.extend(self.tests_script.read(cx).typed_texts(cx));
        texts
    }

    // The folder's view, or the collection's; a folder without `folder.yml` has one too.
    fn view(&self, cx: &App) -> Option<Arc<ConfigView>> {
        let snapshot = self.workspace.read(cx).snapshot(&self.root, cx)?;
        match &self.target {
            ConfigTarget::Folder(path) => snapshot.content.folders.get(path).cloned(),
            ConfigTarget::Collection => Some(snapshot.content.collection.clone()),
        }
    }

    fn busy(&self, field: Field, cx: &App) -> bool {
        self.workspace
            .read(cx)
            .in_flight(&self.root, &self.target.target(), field, cx)
    }

    // Each id stays mapped to the section that sent it until its notices are delivered.
    fn send(&mut self, edit: Edit, section: Option<ConfigSection>, cx: &mut Context<Self>) {
        self.send_typed(edit, section, None, cx);
    }

    // Headers, vars, post-response vars, tags and auth become undo steps; every other edit
    // is only noted, so a step of its field stops being valid. `typed` is the cell a
    // keystroke landed in, so a run of keystrokes there is one step.
    fn send_typed(
        &mut self,
        edit: Edit,
        section: Option<ConfigSection>,
        typed: Option<(usize, u8)>,
        cx: &mut Context<Self>,
    ) {
        match (edit.field(), &edit) {
            // The secret move rewrites values the history cannot restore.
            (_, Edit::MoveAuthSecrets) => {}
            (
                Field::Headers | Field::Vars | Field::PostResponseVars | Field::Tags | Field::Auth,
                _,
            ) => match self.view(cx) {
                Some(view) => {
                    let busy = self.busy(edit.field(), cx);
                    let read = |e: &Edit| view_edit(&view, e);
                    edit_track::track(&mut self.history, &edit, typed, busy, &read)
                }
                None => {
                    self.history.note_sent(&edit);
                }
            },
            _ => {
                self.history.note_sent(&edit);
            }
        }
        self.dispatch(edit, section, cx);
    }

    fn dispatch(&mut self, edit: Edit, section: Option<ConfigSection>, cx: &mut Context<Self>) {
        if edit.field() == Field::Network
            && let Some(network) = self.network.clone()
        {
            network.update(cx, |n, cx| n.set_notice(None, cx));
        }
        let auth = edit.field() == Field::Auth;
        if auth {
            self.auth.update(cx, |a, cx| a.set_notice(None, cx));
        }
        let field = edit.field();
        let command = Command::Edit {
            target: self.target.target(),
            edit,
        };
        let root = self.root.clone();
        // An edit carries the snapshot its field last loaded, so the actor holds it back when
        // a reload replaced that content before the field could show it.
        let basis = self.loaded.basis(&field, self.lagging);
        let id = self
            .workspace
            .update(cx, |w, cx| w.send_from(&root, command, basis, cx));
        self.history.bind(id);
        if let (Some(id), Some(section)) = (id, section) {
            self.sent.insert(id, section);
        }
        // The panel's plan queues behind the edit, so it reads the edited auth.
        if auth {
            refresh_token_panel(&self.auth, cx);
        }
        cx.notify();
    }

    // Commands up to `acked` had their notices handed over already, so only newer ones stay
    // mapped; a send before that delivery must not drop an outcome still on its way.
    fn delivered(&mut self, root: &Path, acked: u64) {
        if root == self.root {
            self.sent.retain(|&id, _| id > acked);
            self.history.settle(acked);
        }
    }

    // A `Failed` notice for an edit this editor sent shows its message under that
    // section's table, as do a network edit's warnings; the next edit there clears it.
    fn mark_refused(&mut self, root: &Path, notices: &[Notice], cx: &mut Context<Self>) {
        if root != self.root {
            return;
        }
        for notice in notices {
            let Some(section) = self.sent.remove(&notice.command) else {
                continue;
            };
            edit_track::refused(&mut self.history, notice);
            let text = crate::section::notice_text(&notice.kind);
            if section == ConfigSection::Network
                && let Some(network) = self.network.clone()
            {
                if text.is_some() {
                    network.update(cx, |n, cx| n.set_notice(text, cx));
                }
                continue;
            }
            if section == ConfigSection::Auth {
                if text.is_some() {
                    self.auth.update(cx, |a, cx| a.set_notice(text, cx));
                }
                continue;
            }
            if let (NoticeKind::Failed { message, row, .. }, Some(table)) =
                (&notice.kind, self.table(section).cloned())
            {
                let row = row.filter(|_| section == ConfigSection::Headers);
                table.update(cx, |t, cx| t.set_error(Some(message.clone()), row, cx));
            }
        }
    }

    // Each section follows the snapshot only while it has nothing in flight, so the
    // user's own echo is never written back.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.view(cx) else {
            return;
        };
        self.note_loaded(cx);
        // A save or an outside reload is a point undo stops at, in the structured history and
        // each input's own undo.
        let ended =
            |&(dirty, reloaded): &(bool, u64)| (dirty && !view.dirty) || reloaded != view.reloaded;
        let checkpoint = self.checkpoint.replace((view.dirty, view.reloaded));
        if checkpoint.as_ref().is_some_and(ended) {
            self.history.end_typing();
            self.auth.update(cx, |a, cx| a.end_typing(window, cx));
            self.script.update(cx, |s, cx| s.end_typing(window, cx));
            (self.tests_script).update(cx, |s, cx| s.end_typing(window, cx));
        }
        if !self.busy(Field::Headers, cx) {
            self.header_base = view.headers.clone();
            let rows = header_rows(&self.header_base);
            self.headers.update(cx, |t, cx| t.load(rows, window, cx));
        }
        if !self.busy(Field::Vars, cx) {
            self.var_base = view.vars.clone();
            let rows = var_rows(&self.var_base);
            self.vars.update(cx, |t, cx| t.load(rows, window, cx));
        }
        if !self.busy(Field::PostResponseVars, cx) {
            self.post_response_base = view.post_response_vars.clone();
            let rows = post_response_rows(&self.post_response_base);
            self.post_response
                .update(cx, |t, cx| t.load(rows, window, cx));
        }
        if !self.busy(Field::Auth, cx) {
            load_auth(&self.auth, &view.auth, window, cx);
        }
        if !self.busy(Field::Docs, cx) {
            self.docs.update(cx, |s, cx| s.load(&view.docs, window, cx));
        }
        if let Some(tags) = self.tags.clone()
            && !self.busy(Field::Tags, cx)
        {
            tags.update(cx, |t, cx| t.load(&view.tags, cx));
        }
        // The Script section loads whole, so an edit in flight in either editor holds it back.
        if !self.busy(Field::ScriptPreRequest, cx) && !self.busy(Field::ScriptPostResponse, cx) {
            self.script
                .update(cx, |s, cx| s.load(&view.scripts, window, cx));
        }
        if !self.busy(Field::ScriptTests, cx) {
            self.tests_script
                .update(cx, |s, cx| s.load(&view.scripts, window, cx));
        }
        if let (Some(section), Some(presets)) = (self.presets.clone(), &view.presets)
            && !self.busy(Field::Presets, cx)
        {
            section.update(cx, |s, cx| s.load(presets, window, cx));
        }
        if let (Some(section), Some(network)) = (self.network.clone(), &view.network)
            && !self.busy(Field::Network, cx)
        {
            section.update(cx, |s, cx| s.load(network, view.plain_secrets, window, cx));
        }
        self.refresh_network_notice(cx);
        cx.notify();
    }

    // Notes the snapshot behind the fields this refresh loads, which are the idle ones; the
    // Script section loads both of its phases or neither.
    fn note_loaded(&mut self, cx: &App) {
        let Some(seq) = self
            .workspace
            .read(cx)
            .snapshot(&self.root, cx)
            .map(|s| s.seq)
        else {
            return;
        };
        let script = [Field::ScriptPreRequest, Field::ScriptPostResponse];
        let idle_script = !script.iter().any(|&f| self.busy(f, cx));
        let fields = LOADED_ALONE.iter().filter(|&&f| !self.busy(f, cx)).copied();
        let loaded: Vec<Field> = fields
            .chain(script.into_iter().filter(|_| idle_script))
            .collect();
        for field in loaded {
            self.loaded.note(field, seq);
        }
    }

    // The Network section's outside-change notice, as the actor last judged it.
    fn refresh_network_notice(&mut self, cx: &mut Context<Self>) {
        if let Some(section) = self.network.clone() {
            let notice = self
                .workspace
                .read(cx)
                .snapshot(&self.root, cx)
                .and_then(|s| s.network_notice.clone());
            section.update(cx, |s, cx| s.set_change_notice(notice, cx));
        }
    }

    // The banners of the current snapshot, from the file's disk state and held edits.
    fn conflicts(&self, cx: &App) -> Option<(Target, Vec<Banner>, Vec<Edit>)> {
        let view = self.view(cx)?;
        let target = self.target.target();
        let held = self
            .workspace
            .read(cx)
            .snapshot(&self.root, cx)
            .and_then(|s| s.stale.get(&target).cloned())
            .unwrap_or_default();
        Some((target, banner::banners(view.disk, held.len()), held))
    }

    /// The open Compare dialog, if one was opened from the banner.
    pub fn compare(&self) -> Option<Entity<crate::compare_dialog::CompareDialog>> {
        self.compare.clone()
    }

    // Opens the read-only diff of disk against draft; the banner's own answer runs from it.
    fn open_compare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((target, shown, _)) = self.conflicts(cx) else {
            return;
        };
        if !shown.contains(&Banner::ChangedOnDisk) {
            return;
        }
        let this = cx.entity().downgrade();
        let answer: crate::compare_dialog::Answer = std::rc::Rc::new(move |choice, _, cx| {
            let _ = this.update(cx, |e, cx| e.answer(Banner::ChangedOnDisk, choice, cx));
        });
        let (workspace, root) = (self.workspace.clone(), self.root.clone());
        self.compare = Some(crate::compare_dialog::open(
            workspace, root, target, answer, window, cx,
        ));
    }

    /// Answers the shown `banner` with `choice`, reading the held edits afresh.
    pub fn answer(&mut self, banner: Banner, choice: Choice, cx: &mut Context<Self>) {
        let Some((target, shown, held)) = self.conflicts(cx) else {
            return;
        };
        // A click from a frame drawn before the banner went away answers nothing.
        let kind = std::mem::discriminant(&banner);
        if !shown.iter().any(|b| std::mem::discriminant(b) == kind) {
            return;
        }
        let root = self.root.clone();
        let commands = banner::commands(banner, choice, &target, &held);
        // Save and Overwrite write what these settings hold, so once the write is confirmed they
        // vouch for the typing as a save does; the text is captured now.
        let texts = self.typed_scripts(cx);
        self.workspace.update(cx, |w, cx| {
            for command in commands {
                let wrote = match &command {
                    Command::Save(t) | Command::Overwrite(t) => Some(t.clone()),
                    _ => None,
                };
                let id = w.send(&root, command, cx);
                if wrote.is_some() {
                    w.approve_when_saved(&root, id, wrote, texts.clone());
                }
            }
        });
    }

    /// The page heading: "Collection Settings" or "Folder Settings".
    pub fn title(&self, _cx: &App) -> String {
        match &self.target {
            ConfigTarget::Collection => "Collection Settings".into(),
            ConfigTarget::Folder(_) => "Folder Settings".into(),
        }
    }

    fn section_bar(&self, cx: &mut Context<Self>) -> TabBar {
        let this = cx.entity();
        let sections = self.sections();
        let tabs = sections.iter().enumerate().map(|(ix, s)| {
            Tab::new().child(
                div()
                    .line_height(relative(style::TAB_LINE_HEIGHT))
                    .id(("config-section", ix))
                    .test_support()
                    .child(s.label()),
            )
        });
        ui::tab_strip("config-sections", cx)
            .selected_index(
                sections
                    .iter()
                    .position(|s| *s == self.section)
                    .unwrap_or(0),
            )
            .children(tabs)
            .on_click(move |ix, _, cx| {
                if let Some(&section) = sections.get(*ix) {
                    this.update(cx, |e, cx| e.set_section(section, cx))
                }
            })
    }

    // Each section sits in a wrapper whose id scopes the ids of its controls; Vars heads its
    // table Pre-request and adds the Post-response block under it.
    fn section_body(&self, cx: &App) -> AnyElement {
        let (id, view): (&'static str, AnyView) = match self.section {
            ConfigSection::Headers => ("config-headers", self.headers.clone().into()),
            ConfigSection::Auth => ("config-auth", self.auth.clone().into()),
            ConfigSection::Vars => ("config-vars", self.vars.clone().into()),
            ConfigSection::Script => ("config-script", self.script.clone().into()),
            ConfigSection::Tests => ("config-tests", self.tests_script.clone().into()),
            ConfigSection::Docs => ("config-docs", self.docs.clone().into()),
            ConfigSection::Tags => match &self.tags {
                Some(tags) => ("config-tags", tags.clone().into()),
                None => ("config-docs", self.docs.clone().into()),
            },
            ConfigSection::Hosts => match &self.hosts {
                Some(hosts) => ("config-hosts", hosts.clone().into()),
                None => ("config-docs", self.docs.clone().into()),
            },
            ConfigSection::Network => match &self.network {
                Some(network) => ("config-network", network.clone().into()),
                None => ("config-docs", self.docs.clone().into()),
            },
            ConfigSection::Presets => match &self.presets {
                Some(presets) => ("config-presets", presets.clone().into()),
                None => ("config-docs", self.docs.clone().into()),
            },
            ConfigSection::Scripts => match &self.scripts {
                Some(scripts) => ("config-scripts", scripts.clone().into()),
                None => ("config-docs", self.docs.clone().into()),
            },
        };
        let body = div().id(id).test_support().w_full().child(view);
        if self.section == ConfigSection::Tags {
            return v_flex()
                .gap(style::SPACE_2)
                .w_full()
                .child(body)
                .child(ui::form_note(
                    "config-tags-note",
                    "A run can keep or skip requests by tag. A folder's tags apply to every request inside it.",
                ))
                .into_any_element();
        }
        if self.section != ConfigSection::Vars {
            return body.into_any_element();
        }
        v_flex()
            .gap(style::SPACE_2)
            .w_full()
            .child(
                v_flex()
                    .gap_1()
                    .w_full()
                    .child(vars_heading(
                        "config-pre-request-heading",
                        "Pre-request",
                        cx,
                    ))
                    .child(body),
            )
            .child(post_response_block(
                &self.post_response,
                self.post_response_hints(cx),
                (
                    "config-post-response-heading",
                    "config-post-response-vars",
                    "config-post-response-hint",
                ),
                cx,
            ))
            .into_any_element()
    }
}

// The fields `refresh` loads one by one, each while it has no edit in flight.
const LOADED_ALONE: [Field; 9] = [
    Field::Headers,
    Field::Vars,
    Field::PostResponseVars,
    Field::Auth,
    Field::Docs,
    Field::Tags,
    Field::ScriptTests,
    Field::Presets,
    Field::Network,
];

// The settings view's value of the field `edit` writes, as an edit; `None` for a kind that
// is not undoable.
fn view_edit(view: &ConfigView, edit: &Edit) -> Option<Edit> {
    Some(match edit {
        Edit::Headers(_) => Edit::Headers(view.headers.clone()),
        Edit::Vars(_) => Edit::Vars(view.vars.clone()),
        Edit::PostResponseVars(_) => Edit::PostResponseVars(view.post_response_vars.clone()),
        Edit::Tags(_) => Edit::Tags(view.tags.clone()),
        Edit::Auth(_) => Edit::Auth(view.auth.clone()),
        _ => return None,
    })
}

impl Focusable for ConfigEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ConfigEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let banners = self.conflicts(cx).map_or_else(Vec::new, |(_, shown, _)| {
            shown
                .into_iter()
                .map(|b| {
                    let this = this.clone();
                    banner::banner_row(b, move |b, choice, window, cx| {
                        this.update(cx, |e, cx| match choice {
                            Choice::Tertiary => e.open_compare(window, cx),
                            _ => e.answer(b, choice, cx),
                        })
                    })
                })
                .collect()
        });
        v_flex()
            .id("config-editor")
            .test_support()
            .size_full()
            .p(style::PANE_INSET)
            .pl(style::tab_content_inset(window))
            .pt(style::tab_content_inset(window))
            .gap_2()
            .key_context("ConfigEditor")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &UndoEdit, window, cx| this.undo_edit(window, cx)))
            .on_action(cx.listener(|this, _: &RedoEdit, window, cx| this.redo_edit(window, cx)))
            .on_action(cx.listener(|this, _: &SaveConfig, window, cx| {
                this.warn_unsaved(window, cx);
                this.save(cx)
            }))
            .child(
                div()
                    .id("config-title")
                    .test_support()
                    .child(self.title(cx)),
            )
            .children(banners)
            .child(self.section_bar(cx))
            // A tall section scrolls under the fixed title and section bar instead of running
            // past the tab's bottom edge, where the tab area clips it.
            .child(
                div()
                    .id("config-body")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .child(self.section_body(cx)),
            )
    }
}
