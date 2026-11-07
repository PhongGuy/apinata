//! The Import collection dialog: a link to fetch, or Browse for a file. A fetched body goes
//! through the same parse and write path as a chosen file, so both write identical collections.

use crate::theme::SelectedStyle;
use crate::ui::{ChooseButton as _, LockButton as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::ItemPath;
use apinata_core::import::{
    ChangedField, FetchError, ImportBundle, ImportSource, UpdateSummary, classify_source,
    default_import_name, fetch_source,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Entity, FocusHandle, Focusable, Global, PathPromptOptions, Pixels, Result, ScrollHandle,
    SharedString, Task, TestSupportExt, WeakEntity, Window, div, px,
};

use crate::command::Command;
use crate::shell::{Shell, parse_bundles_from};
use crate::style::{
    RADIUS, ROW_HEIGHT, SHORT_FORM_LABEL_WIDTH, SPACE_1, SPACE_2, TEXT_SMALL, resolve,
};
use crate::{tokio_bridge, ui};

/// An update summary that compares by its report, so a [`Phase`] stays comparable.
#[derive(Clone, Debug)]
pub struct Summary(pub UpdateSummary);

impl PartialEq for Summary {
    fn eq(&self, other: &Self) -> bool {
        format!("{:?}", self.0) == format!("{:?}", other.0)
    }
}
impl Eq for Summary {}

/// How a button in the update pickers is drawn: `Primary` is the one current choice,
/// `Marked` a choice that is kept but yields to a narrower pick below it.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickStyle {
    /// Not the current choice.
    Plain,
    Marked,
    Primary,
}

/// A small muted heading above a picker list.
fn heading(id: &'static str, text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .px(SPACE_2)
        .text_size(TEXT_SMALL)
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// What the dialog does with the spec it fetches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Write a new collection (the default).
    New,
    /// Update an open collection that came from a spec.
    Update,
}

/// Where the dialog is in its one job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for a link.
    Idle,
    /// The link is being fetched, or an update is being planned.
    Fetching,
    /// The last attempt failed; the text is shown under the field.
    Failed(String),
    /// An update is planned and nothing is written yet.
    Preview {
        /// What applying would do.
        summary: Summary,
        /// Whether requests missing from the spec would be removed.
        prune: bool,
    },
    /// The update is being applied.
    Applying,
    /// The update is applied.
    Done(Summary),
}

/// Why a collection without a spec baseline cannot be updated.
pub const NO_BASELINE: &str = "Not imported from a spec, so it cannot be updated";
/// Why an update waits on the user.
pub const UNSAVED: &str = "Save or discard your unsaved edits in this collection first";

/// An open collection offered to Update mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateTarget {
    /// The collection's root.
    pub root: PathBuf,
    /// The label shown in the picker.
    pub name: String,
    /// Whether it can be updated from a spec, at its root or in a folder; `None` until its
    /// actor has answered.
    pub baseline: Option<bool>,
    /// Whether the collection root has a spec baseline of its own.
    pub root_baseline: bool,
    /// The folders that carry a baseline of their own.
    pub folders: Vec<ItemPath>,
}

impl UpdateTarget {
    /// Why this collection cannot be chosen, or `None` when it can (or is still being asked).
    pub fn reason(&self) -> Option<String> {
        (self.baseline == Some(false)).then(|| NO_BASELINE.to_owned())
    }
}

/// The dialog's body and state.
pub struct ImportLinkDialog {
    shell: WeakEntity<Shell>,
    link: Entity<InputState>,
    phase: Phase,
    mode: Mode,
    targets: Vec<UpdateTarget>,
    selected: Option<usize>,
    // The folder of the selected collection to update: None until decided, Some(None) is its root.
    folder: Option<Option<ItemPath>>,
    // The folder the preview or result on screen is for; `folder` may already name the next one.
    shown: Option<Option<ItemPath>>,
    // Set by the first collection or folder click; from then on no answer changes the target.
    chosen: bool,
    // The request open in the active tab, with its collection root.
    active_item: Option<(PathBuf, ItemPath)>,
    prune: bool,
    // A prune toggle is waiting for its new plan; the old preview stays on screen meanwhile.
    replanning: bool,
    // Held by the dialog body once the link field is gone, so Enter still reaches the dialog.
    focus: FocusHandle,
    refocus: bool,
    // The file picked with Browse; it is the source only while the field still shows its path.
    file: Option<PathBuf>,
    // The parsed spec an update previews and applies.
    bundle: Option<Arc<ImportBundle>>,
    // The in-flight fetch; dropping it aborts the request, so Cancel and close drop it.
    task: Option<Task<()>>,
    // The baseline questions to the open collections' actors.
    asking: Option<Task<()>>,
    // The report body's scroll position, for its scrollbar.
    scroll: ScrollHandle,
}

// The most recently opened dialog; it lapses once the dialog lets go of it.
struct OpenDialog(WeakEntity<ImportLinkDialog>);
impl Global for OpenDialog {}

/// The open dialog, for tests and the pilot; None when no dialog is open.
pub fn current(cx: &App) -> Option<Entity<ImportLinkDialog>> {
    cx.try_global::<OpenDialog>().and_then(|d| d.0.upgrade())
}

/// Opens the dialog over `window`; imports go to `shell`.
pub fn open(
    shell: WeakEntity<Shell>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ImportLinkDialog> {
    let view = cx.new(|cx| {
        let link =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://host/openapi.json"));
        ImportLinkDialog {
            shell,
            link,
            phase: Phase::Idle,
            mode: Mode::New,
            targets: Vec::new(),
            selected: None,
            folder: None,
            shown: None,
            chosen: false,
            active_item: None,
            prune: false,
            replanning: false,
            focus: cx.focus_handle(),
            refocus: false,
            file: None,
            bundle: None,
            task: None,
            asking: None,
            scroll: ScrollHandle::new(),
        }
    });
    cx.set_global(OpenDialog(view.downgrade()));
    let body = view.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        let (view, enter) = (body.clone(), body.clone());
        crate::dialog_frame::place(dialog, window)
            .title("Import collection")
            .close_button(false)
            // Closing waits while an update is being written.
            .on_cancel(move |_, _, cx| {
                view.update(cx, |v, _| {
                    let applying = v.phase == Phase::Applying;
                    if !applying {
                        v.task = None;
                    }
                    !applying
                })
            })
            // Enter in the field is the primary button; the dialog stays open for the fetch.
            .on_ok(move |_, window, cx| {
                enter.update(cx, |v, cx| v.submit(window, cx));
                false
            })
            .child(body.clone())
    });
    view.read(cx)
        .link
        .read(cx)
        .focus_handle(cx)
        .focus(window, cx);
    view
}

impl ImportLinkDialog {
    /// The current screen.
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The failure shown under the field, if any.
    pub fn error(&self) -> Option<&str> {
        match &self.phase {
            Phase::Failed(text) => Some(text),
            _ => None,
        }
    }

    /// Whether the link becomes a new collection or updates an open one.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The open collections offered to Update mode, in open order.
    pub fn targets(&self) -> Vec<UpdateTarget> {
        self.targets.clone()
    }

    /// The index in [`Self::targets`] chosen for the update.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// What can be updated in the selected collection: its root (`None`, when it has a
    /// baseline) and every folder that has one, in the order they are shown.
    pub fn scopes(&self) -> Vec<Option<ItemPath>> {
        let Some(t) = self.selected.and_then(|n| self.targets.get(n)) else {
            return Vec::new();
        };
        t.root_baseline
            .then_some(None)
            .into_iter()
            .chain(t.folders.iter().cloned().map(Some))
            .collect()
    }

    /// Which of [`Self::scopes`] is chosen; at most one is.
    #[doc(hidden)]
    pub fn scopes_selected(&self) -> Vec<bool> {
        let chosen = self.folder.as_ref();
        self.scopes().iter().map(|s| chosen == Some(s)).collect()
    }

    /// Whether the folder picker is shown under the collection list.
    fn scope_picker_shown(&self) -> bool {
        let scopes = self.scopes();
        self.mode == Mode::Update
            && !matches!(self.phase, Phase::Applying | Phase::Done(_))
            && self.selected.is_some()
            && (scopes.len() > 1 || scopes.first().is_some_and(|s| s.is_some()))
    }

    /// How collection button `n` is drawn. With a folder picker below, the chosen
    /// collection is only marked, so the chosen folder is the one primary control.
    #[doc(hidden)]
    pub fn target_style(&self, n: usize) -> PickStyle {
        match (self.selected == Some(n), self.scope_picker_shown()) {
            (false, _) => PickStyle::Plain,
            (true, true) => PickStyle::Marked,
            (true, false) => PickStyle::Primary,
        }
    }

    /// Whether collection button `n` draws the chosen marker (a check and an accent outline).
    #[doc(hidden)]
    pub fn target_marker(&self, n: usize) -> bool {
        self.selected == Some(n)
    }

    /// How each folder button is drawn: the chosen one is primary.
    #[doc(hidden)]
    pub fn scope_styles(&self) -> Vec<PickStyle> {
        self.scopes_selected()
            .into_iter()
            .map(|c| {
                if c {
                    PickStyle::Primary
                } else {
                    PickStyle::Plain
                }
            })
            .collect()
    }

    /// The label of a scope in the picker: `Whole collection` for the root, else `Folder <path>`.
    #[doc(hidden)]
    pub fn scope_name(&self, scope: Option<&ItemPath>) -> String {
        match scope {
            Some(f) => format!("Folder {f}"),
            None => "Whole collection".to_string(),
        }
    }

    /// Goes back from the preview to the source step, keeping the chosen collection and folder,
    /// so the link or file can be changed without closing the dialog.
    pub fn change_source(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Preview { .. }) || self.replanning {
            return;
        }
        self.task = None;
        self.bundle = None;
        self.shown = None;
        self.prune = false;
        self.phase = Phase::Idle;
        cx.notify();
    }

    /// The folder chosen for the update, `Some(None)` for the collection root; `None` while the
    /// user still has to choose.
    pub fn folder(&self) -> Option<Option<ItemPath>> {
        self.folder.clone()
    }

    /// The plain line naming the collection and folder being updated, shown with the preview and the result.
    pub fn scope_label(&self) -> Option<String> {
        let (verb, folder) = match self.phase {
            Phase::Preview { .. } | Phase::Applying => ("Updating", self.shown.as_ref()?),
            Phase::Done(_) => ("Updated", self.shown.as_ref()?),
            _ => return None,
        };
        let name = &self.targets.get(self.selected?)?.name;
        Some(match folder {
            Some(f) => format!("{verb} {name} / Folder {f}"),
            None => format!("{verb} {name}"),
        })
    }

    /// Whether the preview removes requests missing from the spec.
    pub fn prune(&self) -> bool {
        self.prune
    }

    /// Why the update cannot go ahead right now, if something holds it.
    pub fn blocked(&self, cx: &App) -> Option<String> {
        let root = &self.targets.get(self.selected?)?.root;
        let shell = self.shell.upgrade()?;
        let ws = shell.read(cx).workspace().read(cx);
        (!ws.unsaved(Some(root), cx).is_empty()).then(|| UNSAVED.to_owned())
    }

    /// Why the primary button is off before the preview: no collection chosen, or `blocked`.
    #[doc(hidden)]
    pub fn primary_blocked(&self, cx: &App) -> Option<String> {
        match self.selected {
            None => Some("Choose a collection to update.".to_owned()),
            Some(_) => self.blocked(cx).or_else(|| {
                (self.folder.is_none()).then(|| "Choose a folder to update.".to_owned())
            }),
        }
    }

    /// The changed rows of the current preview or result, as shown.
    #[doc(hidden)]
    pub fn changed_rows(&self) -> Vec<(String, String)> {
        match &self.phase {
            Phase::Preview { summary, .. } | Phase::Done(summary) => changed_rows(&summary.0),
            _ => Vec::new(),
        }
    }

    /// Test-only: the Update button's action, without the button's disabled state.
    #[doc(hidden)]
    pub fn submit_for_test(&mut self, cx: &mut Context<Self>) {
        self.apply(cx);
    }

    /// Switches mode. Update mode asks every open collection whether it has a spec baseline.
    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        let fixed = matches!(
            self.phase,
            Phase::Fetching | Phase::Applying | Phase::Done(_)
        );
        if self.mode == mode || fixed {
            return;
        }
        self.mode = mode;
        self.prune = false;
        self.task = None;
        self.bundle = None;
        self.phase = Phase::Idle;
        if mode == Mode::Update {
            self.refresh_targets(cx);
        }
        cx.notify();
    }

    /// Chooses the collection to update; one without a baseline cannot be chosen.
    pub fn select_target(&mut self, index: usize, cx: &mut Context<Self>) {
        // The fetched spec and its preview belong to the collection chosen before them.
        if matches!(
            self.phase,
            Phase::Fetching | Phase::Preview { .. } | Phase::Applying | Phase::Done(_)
        ) {
            return;
        }
        if self
            .targets
            .get(index)
            .is_none_or(|t| t.baseline != Some(true))
        {
            return;
        }
        if self.selected != Some(index) {
            self.selected = Some(index);
            self.folder = default_scope(&self.targets[index], self.active_item.as_ref());
        }
        self.chosen = true;
        if matches!(self.phase, Phase::Failed(_)) {
            self.phase = Phase::Idle;
        }
        cx.notify();
    }

    /// Chooses the folder of the selected collection to update (`None` is its root). Once a
    /// preview is shown the plan is made again for the new folder.
    pub fn select_scope(&mut self, scope: Option<ItemPath>, cx: &mut Context<Self>) {
        let fixed = matches!(
            self.phase,
            Phase::Fetching | Phase::Applying | Phase::Done(_)
        );
        let previewing = matches!(self.phase, Phase::Preview { .. });
        if fixed || self.replanning || !self.scopes().contains(&scope) {
            return;
        }
        self.chosen = true;
        if self.folder.as_ref() == Some(&scope) {
            return;
        }
        self.folder = Some(scope);
        if previewing {
            self.plan(cx);
            return;
        }
        if matches!(self.phase, Phase::Failed(_)) {
            self.phase = Phase::Idle;
        }
        cx.notify();
    }

    // Lists the open collections and asks each actor whether it came from a spec.
    fn refresh_targets(&mut self, cx: &mut Context<Self>) {
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        let ws = shell.read(cx).workspace().clone();
        let roots: Vec<PathBuf> = ws
            .read(cx)
            .user_collections()
            .into_iter()
            .map(|c| c.root().to_path_buf())
            .collect();
        self.targets = roots
            .iter()
            .map(|r| UpdateTarget {
                root: r.clone(),
                name: ws.read(cx).collection_name(r, cx).to_string(),
                baseline: None,
                root_baseline: false,
                folders: Vec::new(),
            })
            .collect();
        self.selected = None;
        self.folder = None;
        self.chosen = false;
        // The collection the user is working in is the likeliest one to update, and a request
        // open in it points at the folder.
        let active = ws.read(cx).current_root();
        self.active_item = ws.read(cx).tabs().active().and_then(|t| {
            let item = t.item()?;
            Some((t.root.clone(), item.clone()))
        });
        let mut asked = Vec::new();
        for root in &roots {
            let (reply, info) = async_channel::bounded(1);
            let (folders_reply, folders) = async_channel::bounded(1);
            let sent = ws.update(cx, |w, cx| {
                w.send(root, Command::UpdateInfo { reply }, cx)?;
                w.send(
                    root,
                    Command::UpdateFolders {
                        reply: folders_reply,
                    },
                    cx,
                )
            });
            asked.push(sent.map(|_| (info, folders)));
        }
        self.asking = Some(cx.spawn(async move |this, cx| {
            for (i, answer) in asked.into_iter().enumerate() {
                let (root, folders) = match answer {
                    Some((info, folders)) => (
                        info.recv().await.unwrap_or(false),
                        folders.recv().await.unwrap_or_default(),
                    ),
                    None => (false, Vec::new()),
                };
                let active = active.clone();
                let _ = this.update(cx, |this, cx| {
                    if let Some(t) = this.targets.get_mut(i) {
                        t.root_baseline = root;
                        t.baseline = Some(root || !folders.is_empty());
                        t.folders = folders;
                    }
                    this.settle(i, active.as_deref());
                    cx.notify();
                });
            }
        }));
    }

    // Applies the preselect rules after the answer for target `i`; a choice by the user stands.
    fn settle(&mut self, i: usize, active: Option<&Path>) {
        let Some(target) = self.targets.get(i) else {
            return;
        };
        if self.chosen || target.baseline != Some(true) {
            return;
        }
        if self.selected.is_none() && active == Some(target.root.as_path()) {
            self.selected = Some(i);
        }
        if self.selected == Some(i) {
            self.folder = default_scope(target, self.active_item.as_ref());
        }
    }

    /// Test-only hook, not used by the app: runs the preselect rules again for every collection, as a late answer does.
    #[doc(hidden)]
    pub fn relearn_for_test(&mut self, cx: &mut Context<Self>) {
        let active = self.active_item.as_ref().map(|(r, _)| r.clone());
        for i in 0..self.targets.len() {
            self.settle(i, active.as_deref());
        }
        cx.notify();
    }

    /// Chooses whether the update removes requests missing from the spec, and plans again. A
    /// refusal on the new plan (an unavailable trash) replaces the preview with its message, by design.
    pub fn set_prune(&mut self, prune: bool, cx: &mut Context<Self>) {
        if self.prune == prune || self.replanning || !matches!(self.phase, Phase::Preview { .. }) {
            return;
        }
        self.prune = prune;
        self.plan(cx);
    }

    /// The primary action: Import, or Update once a preview is shown.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.phase {
            Phase::Preview { .. } => self.apply(cx),
            Phase::Done(_) => self.cancel(window, cx),
            _ => self.start(window, cx),
        }
    }

    // Sends `command` for the chosen collection and waits for the actor's answer.
    fn ask_update(
        &mut self,
        apply: bool,
        cx: &mut Context<Self>,
    ) -> Result<async_channel::Receiver<Result<UpdateSummary, String>>, &'static str> {
        const NOT_NOW: &str = "This collection cannot be updated right now.";
        let root = self
            .selected
            .and_then(|n| self.targets.get(n))
            .ok_or(NOT_NOW)?
            .root
            .clone();
        let bundle = self.bundle.clone().ok_or(NOT_NOW)?;
        let ws = self
            .shell
            .upgrade()
            .ok_or(NOT_NOW)?
            .read(cx)
            .workspace()
            .clone();
        let open = ws
            .read(cx)
            .user_collections()
            .iter()
            .any(|c| c.root() == root);
        if !open {
            return Err(CLOSED);
        }
        let (reply, answer) = async_channel::bounded(1);
        let prune = self.prune;
        let folder = self.folder.clone().ok_or("Choose a folder to update.")?;
        let command = if apply {
            Command::ApplyUpdate {
                bundle,
                prune,
                folder,
                reply,
            }
        } else {
            Command::PlanUpdate {
                bundle,
                prune,
                folder,
                reply,
            }
        };
        ws.update(cx, |w, cx| w.send(&root, command, cx))
            .ok_or(NOT_NOW)?;
        Ok(answer)
    }

    // Plans the update and shows the preview, or why there is none.
    fn plan(&mut self, cx: &mut Context<Self>) {
        let answer = match self.ask_update(false, cx) {
            Ok(answer) => answer,
            Err(message) => {
                self.phase = Phase::Failed(message.to_owned());
                cx.notify();
                return;
            }
        };
        self.replanning = matches!(self.phase, Phase::Preview { .. });
        if !self.replanning {
            self.phase = Phase::Fetching;
        }
        let prune = self.prune;
        let planned_for = self.folder.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = answer
                .recv()
                .await
                .unwrap_or_else(|_| Err(STOPPED.to_owned()));
            let _ = this.update(cx, |this, cx| {
                this.replanning = false;
                this.shown = planned_for;
                this.refocus = true;
                this.phase = match result {
                    Ok(summary) => Phase::Preview {
                        summary: Summary(summary),
                        prune,
                    },
                    Err(message) => {
                        this.prune = false;
                        Phase::Failed(message)
                    }
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    // Applies the previewed update; unsaved edits that appeared since keep it from running.
    fn apply(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Preview { .. })
            || self.replanning
            || self.blocked(cx).is_some()
        {
            return;
        }
        let answer = match self.ask_update(true, cx) {
            Ok(answer) => answer,
            Err(message) => {
                self.phase = Phase::Failed(message.to_owned());
                cx.notify();
                return;
            }
        };
        self.phase = Phase::Applying;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = answer
                .recv()
                .await
                .unwrap_or_else(|_| Err(STOPPED.to_owned()));
            let _ = this.update(cx, |this, cx| {
                this.refocus = true;
                this.phase = match result {
                    Ok(summary) => Phase::Done(Summary(summary)),
                    Err(message) => {
                        this.prune = false;
                        Phase::Failed(message)
                    }
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// The link field's text.
    pub fn link(&self, cx: &App) -> String {
        self.link.read(cx).value().to_string()
    }

    /// Sets the link field's text and clears a shown failure.
    pub fn set_link(&mut self, link: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.link.update(cx, |i, cx| i.set_value(link, window, cx));
        if matches!(self.phase, Phase::Failed(_)) {
            self.phase = Phase::Idle;
        }
        cx.notify();
    }

    /// Fetches the link, parses the body and hands the collections to the shell, which asks
    /// where they go. A failure stays in the dialog; a second call while fetching does nothing.
    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Fetching | Phase::Applying) {
            return;
        }
        if self.mode == Mode::Update
            && let Some(message) = self.primary_blocked(cx)
        {
            self.phase = Phase::Failed(message);
            cx.notify();
            return;
        }
        let url = self.link(cx).trim().to_owned();
        if url.is_empty() {
            self.phase = Phase::Failed("Enter a link or choose a file.".to_owned());
            cx.notify();
            return;
        }
        // The field holds a link or a local path. A path is read where a link would be fetched,
        // like a Browse pick; a text that is neither fails here, before any work starts.
        let picked = self
            .file
            .clone()
            .filter(|f| f.to_string_lossy() == url.as_str());
        let file = match picked {
            Some(path) => Some(path),
            None => match classify_source(&url) {
                Ok(ImportSource::File(path)) => Some(path),
                Ok(_) => None,
                Err(e) => {
                    self.phase = Phase::Failed(crate::import_report::failure_text(&e.to_string()));
                    cx.notify();
                    return;
                }
            },
        };
        // The collection is named after the link, or after the file, where the format takes a name.
        let name = match &file {
            Some(path) => path.file_stem().map(|s| s.to_string_lossy().into_owned()),
            None => default_import_name(&url),
        }
        .unwrap_or_else(|| "import".to_owned());
        self.phase = Phase::Fetching;
        // The link is fetched with the settings requests are sent with.
        let fetch = crate::executor::shared(cx)
            .map(|exec| exec.source_fetch())
            .unwrap_or_default();
        cx.notify();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            if let Some(file) = file {
                let parsed = cx
                    .background_spawn(async move { crate::shell::parse_bundles(&file) })
                    .await;
                let _ = this.update_in(cx, |this, window, cx| this.fetched(parsed, window, cx));
                return;
            }
            let fetched =
                tokio_bridge::spawn(cx, async move { fetch_source(&url, &fetch).await }).await;
            let parsed = match fetched {
                Ok(Ok(bytes)) => {
                    cx.background_spawn(async move {
                        parse_bundles_from(Path::new(&format!("{name}.json")), &bytes)
                    })
                    .await
                }
                Ok(Err(e)) => Err(fetch_text(&e)),
                Err(_) => Err("The fetch stopped unexpectedly. Try again.".to_owned()),
            };
            let _ = this.update_in(cx, |this, window, cx| this.fetched(parsed, window, cx));
        }));
    }

    // A failure goes under the field; success closes the dialog and the shell takes over.
    fn fetched(
        &mut self,
        parsed: Result<Vec<apinata_core::import::ImportBundle>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.task = None;
        match parsed {
            Err(message) => {
                self.phase = Phase::Failed(crate::import_report::failure_text(&message));
                cx.notify();
            }
            Ok(bundles) if self.mode == Mode::Update => {
                match <[ImportBundle; 1]>::try_from(bundles) {
                    Ok([bundle]) => {
                        self.bundle = Some(Arc::new(bundle));
                        self.plan(cx);
                    }
                    Err(_) => {
                        self.phase = Phase::Failed(MANY.to_owned());
                        cx.notify();
                    }
                }
            }
            Ok(bundles) => {
                window.close_dialog(cx);
                let shell = self.shell.clone();
                let _ = shell.update(cx, |s, cx| s.place_imported(bundles, window, cx));
            }
        }
    }

    // Drops any fetch and closes.
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase == Phase::Applying {
            return;
        }
        self.task = None;
        window.close_dialog(cx);
    }

    // Update mode: asks for a file and previews it as the link would be.
    fn browse_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Fetching | Phase::Applying) {
            return;
        }
        let reply = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose spec".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Some(file) = crate::shell::first_path(reply.await) else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let text = file.to_string_lossy().into_owned();
                this.file = Some(file);
                this.set_link(&text, window, cx);
                this.start(window, cx);
            });
        })
        .detach();
    }

    // Closes the dialog and runs the file prompt, as Import did before it had a link field.
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.task = None;
        window.close_dialog(cx);
        let _ = self.shell.update(cx, |s, cx| s.browse_import(window, cx));
    }
}

/// The folder to update in `target` before the user has chosen: the baseline folder of the open
/// request, else the root, else the only folder; `None` when the user has to choose.
fn default_scope(
    target: &UpdateTarget,
    active: Option<&(PathBuf, ItemPath)>,
) -> Option<Option<ItemPath>> {
    let from_tab = active
        .filter(|(root, _)| *root == target.root)
        .and_then(|(_, item)| {
            target
                .folders
                .iter()
                .filter(|f| item.as_str().starts_with(&format!("{f}/")))
                .max_by_key(|f| f.as_str().len())
        });
    if let Some(f) = from_tab {
        return Some(Some(f.clone()));
    }
    if target.root_baseline {
        return Some(None);
    }
    match target.folders.as_slice() {
        [only] => Some(Some(only.clone())),
        _ => None,
    }
}

const STOPPED: &str = "The update stopped unexpectedly. Try again.";
const PRUNE_NOTE: &str =
    "Only requests you have not edited are removed; they go to the trash and can be restored";
const CLOSED: &str = "That collection is no longer open";
const MANY: &str = "This spec holds several collections; update needs one";

/// One plain sentence per way a fetch can fail; the link's own credentials never appear.
fn fetch_text(error: &FetchError) -> String {
    match error {
        FetchError::Scheme(why) => format!("Cannot import this link: {why}"),
        FetchError::Transport(why) => format!("Cannot fetch this link: {why}"),
        FetchError::Timeout => "Cannot fetch this link: it timed out".to_owned(),
        FetchError::Status(code) => {
            format!("Cannot fetch this link: the server answered HTTP {code}")
        }
        FetchError::TooLarge { max_bytes } => {
            format!("Cannot fetch this link: the body is larger than {max_bytes} bytes")
        }
        FetchError::TooManyRedirects => "Cannot fetch this link: too many redirects".to_owned(),
        other => format!("Cannot fetch this link: {other}"),
    }
}

// The words a user knows a spec-owned field by.
fn field_label(f: ChangedField) -> &'static str {
    match f {
        ChangedField::Target => "URL and params",
        ChangedField::Headers => "headers",
        ChangedField::Body => "body",
        ChangedField::Docs => "docs",
        ChangedField::Tags => "tags",
        ChangedField::Vars => "variables",
        ChangedField::Method => "method",
        _ => "other fields",
    }
}

/// The changed requests of `s` as (path, field words) rows.
fn changed_rows(s: &UpdateSummary) -> Vec<(String, String)> {
    s.changed
        .iter()
        .map(|c| {
            let fields: Vec<&str> = c.fields.iter().map(|f| field_label(*f)).collect();
            (c.path.to_string(), format!("({})", fields.join(", ")))
        })
        .collect()
}

/// The rows around the report body (modes, counts, checkbox, note, footer) in a preview.
const REPORT_CHROME_ROWS: f32 = 8.;

/// The counts line above the report, such as "2 added, 1 changed, 4 unchanged".
fn counts_line(s: &UpdateSummary, applied: bool) -> String {
    let mut parts = Vec::new();
    let mut part = |n: usize, what: &str| {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    };
    part(s.added.len(), "added");
    part(s.changed.len(), "changed");
    part(s.edited.len(), "edited by you");
    part(s.orphaned.len(), "missing from the spec");
    if applied {
        part(s.pruned.len(), "removed");
    }
    parts.push(format!("{} unchanged", s.kept.len()));
    parts.join(", ")
}

/// One labelled group of the report; the heading carries the count.
fn report_section(
    id: &'static str,
    title: &str,
    items: Vec<(String, Option<String>)>,
    cx: &App,
) -> Option<impl IntoElement + use<>> {
    if items.is_empty() {
        return None;
    }
    let theme = cx.theme();
    let (muted, fg) = (theme.muted_foreground, theme.foreground);
    Some(
        v_flex()
            .id(id)
            .test_support()
            .gap(SPACE_1)
            .child(
                div()
                    .text_size(TEXT_SMALL)
                    .text_color(muted)
                    .child(format!("{title} ({})", items.len())),
            )
            .children(items.into_iter().map(move |(main, extra)| {
                h_flex()
                    .gap(SPACE_1)
                    .flex_wrap()
                    .child(div().text_color(fg).child(main))
                    .children(extra.map(|e| div().text_color(muted).child(e)))
            })),
    )
}

/// The labelled report: counts stay fixed, the sections scroll in the room left by `room`.
fn report_view(
    s: &UpdateSummary,
    applied: bool,
    room: Pixels,
    scroll: &ScrollHandle,
    cx: &App,
) -> impl IntoElement + use<> {
    let plain = |v: &[String]| v.iter().map(|p| (p.clone(), None)).collect::<Vec<_>>();
    let changed = changed_rows(s)
        .into_iter()
        .map(|(path, fields)| (path, Some(fields)))
        .collect();
    let paths = |it: &[_]| it.iter().map(|p: &_| format!("{p}")).collect::<Vec<_>>();
    let added = report_section(
        "import-update-section-added",
        "Added",
        plain(&paths(&s.added)),
        cx,
    );
    let changed = report_section("import-update-section-changed", "Changed", changed, cx);
    let edited = report_section(
        "import-update-section-edited",
        "Edited by you (spec change not applied)",
        plain(&paths(&s.edited)),
        cx,
    );
    let missing = report_section(
        "import-update-section-missing",
        "Missing from the spec",
        plain(&paths(&s.orphaned)),
        cx,
    );
    let removed = applied
        .then(|| {
            report_section(
                "import-update-section-removed",
                "Removed",
                plain(&paths(&s.pruned)),
                cx,
            )
        })
        .flatten();
    let notes = report_section(
        "import-update-section-notes",
        "Notes",
        s.notes
            .iter()
            .map(|n| (n.detail.to_string(), None))
            .collect(),
        cx,
    );
    let unchanged = report_section(
        "import-update-section-unchanged",
        "Unchanged",
        plain(&paths(&s.kept)),
        cx,
    );
    // The capped box scrolls itself, as in the import report, so a short window keeps the footer.
    v_flex()
        .w_full()
        .min_w_0()
        .gap(SPACE_2)
        .child(
            div()
                .id("import-update-counts")
                .test_support()
                .child(counts_line(s, applied)),
        )
        .child(
            div()
                .relative()
                .w_full()
                .min_w_0()
                .child(
                    v_flex()
                        .id("import-update-body")
                        .test_support()
                        .w_full()
                        .min_w_0()
                        .gap(SPACE_2)
                        .max_h(room.max(px(0.)))
                        .overflow_y_scroll()
                        .track_scroll(scroll)
                        .children(added)
                        .children(changed)
                        .children(edited)
                        .children(missing)
                        .children(removed)
                        .children(notes)
                        .children(unchanged),
                )
                .vertical_scrollbar(scroll),
        )
}

impl Render for ImportLinkDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.refocus)
            && matches!(self.phase, Phase::Preview { .. } | Phase::Done(_))
        {
            self.focus.focus(window, cx);
        }
        let busy = matches!(self.phase, Phase::Fetching | Phase::Applying);
        let error = self.error().map(|message| {
            div()
                .id("import-link-error")
                .test_support()
                .text_color(cx.theme().danger)
                .child(message.to_owned())
        });
        let blocked = self
            .blocked(cx)
            .filter(|_| self.error().is_none())
            .map(|m| {
                div()
                    .id("import-update-blocked")
                    .test_support()
                    .p(SPACE_2)
                    .border_1()
                    .border_color(cx.theme().warning)
                    .rounded(RADIUS)
                    .text_color(cx.theme().warning)
                    .child(m)
            });
        let input = ui::padded_input(
            Input::new(&self.link)
                .id("import-link")
                .w_full()
                .bg(ui::field_fill(false, cx)),
            &self.link,
        );
        // The row must span the control column, or the flex_1 field collapses to its content.
        let control = h_flex()
            .w_full()
            .min_w_0()
            .gap(SPACE_1)
            .child(div().flex_1().child(input))
            .when(self.mode == Mode::Update, |row| {
                row.child(
                    // Wrapped alone: the link field beside it keeps Enter to start the import.
                    Button::new("import-update-browse")
                        .key_context(ui::DIALOG_FOOTER_CONTEXT)
                        .label("Browse…")
                        .lock(busy, cx)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.browse_source(window, cx)),
                        ),
                )
            });
        let field =
            ui::form_row("import-link-row", "Link", control).label_width(SHORT_FORM_LABEL_WIDTH);
        // The mode and picker lists hold buttons only, so a focused one takes Enter.
        let modes = v_flex()
            .key_context(ui::DIALOG_FOOTER_CONTEXT)
            .gap(SPACE_1)
            .child(
                Button::new("import-mode-new")
                    .label("Import as a new collection")
                    .w_full()
                    .when(self.mode == Mode::New, |b| b.primary())
                    .on_click(cx.listener(|this, _, _, cx| this.set_mode(Mode::New, cx))),
            )
            .child(
                Button::new("import-mode-update")
                    .label("Update an open collection")
                    .w_full()
                    .when(self.mode == Mode::Update, |b| b.primary())
                    .on_click(cx.listener(|this, _, _, cx| this.set_mode(Mode::Update, cx))),
            );
        let mode_row = !matches!(self.phase, Phase::Applying | Phase::Done(_));
        let picking = self.mode == Mode::Update
            && !matches!(
                self.phase,
                Phase::Preview { .. } | Phase::Applying | Phase::Done(_)
            );
        let picker = picking.then(|| {
            let rows = self.targets.iter().enumerate().map(|(n, t)| {
                let style = self.target_style(n);
                let row = Button::new(SharedString::from(format!("import-update-target-{n}")))
                    .label(t.name.clone())
                    .w_full()
                    .when(style == PickStyle::Primary, |b| b.primary())
                    // A check says "chosen" in any theme; an accent outline backs it up when the
                    // folder list below holds the filled control, so no shade is relied on.
                    .when(self.target_marker(n), |b| b.icon(IconName::Check))
                    .when(style == PickStyle::Marked, |b| {
                        b.primary()
                            .outline()
                            .choose(true, SelectedStyle::ChosenOutlinePrimary, cx)
                    })
                    // Only the filled primary row lacks an edge; a marked row keeps its outline.
                    .map(|b| {
                        let locked = t.baseline != Some(true);
                        if style == PickStyle::Primary {
                            b.lock_filled(locked, cx)
                        } else {
                            b.lock(locked, cx)
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.select_target(n, cx)));
                v_flex().child(row).children(t.reason().map(|why| {
                    div()
                        .id(SharedString::from(format!("import-update-reason-{n}")))
                        .test_support()
                        .px(SPACE_2)
                        .text_size(TEXT_SMALL)
                        .text_color(cx.theme().muted_foreground)
                        .child(why)
                }))
            });
            v_flex()
                .key_context(ui::DIALOG_FOOTER_CONTEXT)
                .gap(SPACE_1)
                .child(heading(
                    "import-update-collection-heading",
                    "Collection",
                    cx,
                ))
                .children(rows)
                .children(
                    self.targets
                        .is_empty()
                        .then(|| div().child("No collection is open.")),
                )
        });
        let scopes = self.scopes();
        let styles = self.scope_styles();
        let scope_picker = self.scope_picker_shown().then(|| {
            let rows = scopes.into_iter().enumerate().map(|(n, scope)| {
                let style = styles[n];
                let label = self.scope_name(scope.as_ref());
                Button::new(SharedString::from(format!("import-update-folder-{n}")))
                    .label(label)
                    .w_full()
                    .map(|b| {
                        if style == PickStyle::Primary {
                            b.primary().lock_filled(self.replanning, cx)
                        } else {
                            b.lock(self.replanning, cx)
                        }
                    })
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_scope(scope.clone(), cx)),
                    )
            });
            v_flex()
                .key_context(ui::DIALOG_FOOTER_CONTEXT)
                .gap(SPACE_1)
                .child(heading("import-update-folder-heading", "Folder", cx))
                .children(rows)
        });
        let scope_label = self.scope_label().map(|text| {
            div()
                .id("import-update-scope")
                .test_support()
                .text_color(cx.theme().muted_foreground)
                .child(text)
        });
        let report = match &self.phase {
            Phase::Preview { summary, .. } => Some((&summary.0, false)),
            Phase::Done(summary) => Some((&summary.0, true)),
            _ => None,
        }
        .map(|(s, applied)| {
            let room = crate::dialog_frame::titled_body_room(window)
                - resolve(ROW_HEIGHT, window) * REPORT_CHROME_ROWS;
            report_view(s, applied, room, &self.scroll, cx)
        });
        let prune = matches!(self.phase, Phase::Preview { .. }).then(|| {
            v_flex()
                .gap(SPACE_1)
                .child(
                    ui::checkbox("import-update-prune", self.prune)
                        .disabled(self.replanning)
                        .label("Remove requests missing from the spec")
                        .on_click(cx.listener(|this, on: &bool, _, cx| this.set_prune(*on, cx))),
                )
                .child(
                    div()
                        .id("import-update-prune-note")
                        .test_support()
                        .text_size(TEXT_SMALL)
                        .text_color(cx.theme().muted_foreground)
                        .child(PRUNE_NOTE),
                )
        });
        // Only Update mode holds the button back before there is a preview.
        let primary_blocked = (self.mode == Mode::Update)
            .then(|| self.primary_blocked(cx))
            .flatten();
        let footer = match &self.phase {
            Phase::Done(_) => ui::dialog_footer(
                None,
                [],
                Button::new("import-update-close")
                    .label("Close")
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
            ),
            Phase::Preview { .. } | Phase::Applying => ui::dialog_footer(
                Some(
                    Button::new("import-link-cancel")
                        .label("Cancel")
                        .lock(self.phase == Phase::Applying, cx)
                        .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                ),
                [Button::new("import-update-change-source")
                    .label("Change source")
                    .lock(self.phase == Phase::Applying || self.replanning, cx)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.change_source(cx);
                        this.link.read(cx).focus_handle(cx).focus(window, cx);
                    }))],
                Button::new("import-update-apply")
                    .label("Update")
                    .primary()
                    .lock_filled(
                        self.phase == Phase::Applying
                            || self.replanning
                            || self.blocked(cx).is_some(),
                        cx,
                    )
                    .when_some(self.blocked(cx), |b, why| b.tooltip(why))
                    .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
            ),
            _ => ui::dialog_footer(
                Some(
                    Button::new("import-link-cancel")
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                ),
                (self.mode == Mode::New).then(|| {
                    Button::new("import-link-browse")
                        .label("Browse…")
                        .on_click(cx.listener(|this, _, window, cx| this.browse(window, cx)))
                }),
                Button::new("import-link-import")
                    .label(if busy { "Fetching…" } else { "Import" })
                    .primary()
                    .lock_filled(busy || primary_blocked.is_some(), cx)
                    .when_some(primary_blocked, |b, why| b.tooltip(why))
                    .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
            ),
        };
        let showing_link = !matches!(
            self.phase,
            Phase::Preview { .. } | Phase::Applying | Phase::Done(_)
        );
        v_flex()
            .id("import-link-dialog")
            .test_support()
            .track_focus(&self.focus)
            .gap(SPACE_2)
            .children(mode_row.then_some(modes))
            .children(showing_link.then_some(field))
            .children(picker)
            .children(scope_picker)
            .children(scope_label)
            .children(report)
            .children(prune)
            .children(blocked)
            .children(error)
            .child(footer)
    }
}
