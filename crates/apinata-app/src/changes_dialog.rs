//! The Changes dialog: a collection's changed files beside the selected file's changed sections
//! and line diff, read off the UI thread.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::file_diff_with;
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    IndentInline, InputEvent, OutdentInline, Position, Textarea, TextareaState,
};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, ThemeStyled as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Entity, FocusHandle, Global, KeyDownEvent, ListHorizontalSizingBehavior,
    Pixels, Role, ScrollHandle, SharedString, Subscription, TestSupportExt,
    UniformListScrollHandle, WeakEntity, Window, actions, div, px, uniform_list,
};

use crate::confirm;
use crate::diff_doc::{self, DiffBody, DiffDoc, DiffRow};
use crate::diff_rows::{DiffPalette, diff_row};
use crate::form_drafts;
use crate::git_status::{GitChange, GitChanges, GitStage, QUIET_NOTE, commit_label};
use crate::style;
use crate::ui::{self, LockButton as _, RingRoom as _, RingSides};
use crate::workspace::{GitWrite, GitWriteDone, Workspace};

actions!(
    apinata,
    [
        /// Commits what is staged from the message field.
        CommitChanges
    ]
);

/// Key context of the message field's wrapper; Ctrl+Enter binds at `ChangesMessage > Input`.
pub const MESSAGE_CONTEXT: &str = "ChangesMessage";

// The list's note when nothing differs from the last commit.
const NO_CHANGES: &str = "No changes since the last commit";

// A row's note when the index holds a file gone from its path; it says nothing of elsewhere.
const GONE_NOTE: &str = "staged, gone from its path";

// What the box of such a row warns of: its copy in the index is all that was at that path.
const GONE_UNSTAGE: &str = "Unstaging drops the staged copy, and nothing is left at this path";

// What is on disk for a gone row, as far as core knows.
enum Left {
    Nothing,
    // A file holding the staged copy.
    Kept(String),
    // The file the path opens through a link, with other content.
    Opens(String),
}

// A gone row's note, naming the file that holds its staged copy or that the path now opens.
fn gone_note(left: &Left) -> String {
    match left {
        Left::Kept(kept) => format!("staged, file now at {kept}"),
        Left::Opens(other) => format!("staged, path now opens {other}"),
        Left::Nothing => GONE_NOTE.to_owned(),
    }
}

// A gone row's warning, on its box and in the question before it drops; a file holding the same
// content keeps the edit safe.
fn gone_warning(left: &Left) -> String {
    match left {
        Left::Kept(kept) => format!("Unstaging drops the staged copy; the file at {kept} stays"),
        Left::Opens(other) => {
            format!("Unstaging drops the staged copy; the path now opens another file ({other})")
        }
        Left::Nothing => GONE_UNSTAGE.to_owned(),
    }
}

// The most recently opened dialog's view; it lapses once the dialog lets go of it.
struct OpenView(WeakEntity<ChangesView>);
impl Global for OpenView {}

/// The open dialog's view, for tests and the pilot; None when no dialog is open.
pub fn current(cx: &App) -> Option<Entity<ChangesView>> {
    cx.try_global::<OpenView>().and_then(|v| v.0.upgrade())
}

/// Opens the Changes dialog of the collection at `root`, selecting `select` when it is listed,
/// else the first file, and refreshes the status behind it. Opens nothing while the workspace
/// has no git state for `root`.
pub fn open_changes(
    workspace: &Entity<Workspace>,
    root: &Path,
    select: Option<PathBuf>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(changes) = workspace.read(cx).git_changes(root) else {
        return;
    };
    let view = cx.new(|cx| {
        let observing = cx.observe(workspace, |v: &mut ChangesView, _, cx| v.follow(cx));
        let runs = workspace.read(cx).git_runs(root);
        let draft = form_drafts::commit(root, cx);
        let message = cx.new(|cx| {
            let mut m = TextareaState::new(window, cx)
                .placeholder("Commit message")
                .rows(3);
            // A message left from an earlier open comes back with the caret after it.
            if let Some(text) = draft {
                m.set_value(text.as_str(), window, cx);
                let last = text.matches('\n').count() as u32;
                m.set_cursor_position(Position::new(last, u32::MAX), window, cx);
            }
            m
        });
        // Typing changes what the Commit button allows and is kept as the root's draft;
        // `set_value` emits no event of its own.
        let message_changes = cx.subscribe(&message, |v: &mut ChangesView, m, e, cx| {
            if matches!(e, InputEvent::Change) {
                let text = m.read(cx).value().to_string();
                form_drafts::set_commit(&v.root, &text, cx);
                cx.notify();
            }
        });
        let mut view = ChangesView {
            workspace: workspace.clone(),
            root: root.to_owned(),
            seen: None,
            runs,
            files: Vec::new(),
            stages: Vec::new(),
            busy: false,
            message,
            _message_changes: message_changes,
            notice: None,
            clear_message: false,
            branch: None,
            truncated_note: None,
            selected: None,
            doc: None,
            rows: Arc::from([]),
            generation: 0,
            diff_loads: 0,
            fresh: true,
            lines_scroll: UniformListScrollHandle::new(),
            files_scroll: ScrollHandle::new(),
            list_focus: cx.focus_handle().tab_stop(true),
            focus_when_filled: None,
            _observing: observing,
        };
        view.show(Some(changes), select, cx);
        view
    });
    cx.set_global(OpenView(view.downgrade()));
    // A commit made outside the app leaves the shown status stale; the view follows the
    // refresh once it lands.
    workspace.update(cx, |w, cx| w.refresh_git(root, cx));
    let focus_on_open = {
        let v = view.read(cx);
        (!v.files.is_empty()).then(|| v.list_focus.clone())
    };
    let body = view.clone();
    window.open_dialog(cx, move |d, window, _| {
        // Two panes need more room than the default width, but never more than the window.
        let width = style::resolve(style::CHANGES_DIALOG_WIDTH, window)
            .min(window.viewport_size().width * 0.9);
        crate::dialog_frame::place(d, window)
            .title("Changes")
            .close_button(false)
            // Enter never closes it, so a typed message is never lost; Commit is a button.
            .on_ok(|_, _, _| false)
            .width(width)
            .child(body.clone())
    });
    // `open_dialog` focused the kit's host; a list with rows takes focus so Up and Down
    // work at once.
    match focus_on_open {
        Some(list) => window.focus(&list, cx),
        // An empty list takes focus once a refresh fills it, if the dialog's focus is unmoved.
        None => {
            let host = window.focused(cx);
            view.update(cx, |v, _| v.focus_when_filled = host);
        }
    }
}

/// The dialog body: the changed files, the selected one, and its diff once loaded.
pub struct ChangesView {
    workspace: Entity<Workspace>,
    root: PathBuf,
    // The workspace state the list was built from; `None` is the quiet state.
    seen: Option<Arc<GitChanges>>,
    // The workspace's finished status runs when the diff was last requested.
    runs: u64,
    files: Vec<(PathBuf, GitChange)>,
    // Each file's stage state, parallel to `files`.
    stages: Vec<GitStage>,
    // A stage or unstage is running; further toggles are ignored until it returns.
    busy: bool,
    // The commit message field; cleared only when a commit lands.
    message: Entity<TextareaState>,
    // The outcome of the last commit, shown above the buttons until the next one starts.
    notice: Option<Notice>,
    // Set by a landed commit; the next render empties the field, which needs a window.
    clear_message: bool,
    // The branch the Commit button names.
    branch: Option<String>,
    truncated_note: Option<String>,
    selected: Option<usize>,
    doc: Option<DiffDoc>,
    // The loaded doc's rows, shared with the virtualized list so a redraw copies none.
    rows: Arc<[DiffRow]>,
    // Bumped on every load; a diff that lands under an older value is dropped.
    generation: u64,
    // Diffs started, counted so a test can pin an open to one read.
    diff_loads: usize,
    // Set at open: the diff was just read, so the first finished status run need not reload it.
    fresh: bool,
    lines_scroll: UniformListScrollHandle,
    files_scroll: ScrollHandle,
    // Held by the file list after a click, which turns the selected row's edge to the accent.
    list_focus: FocusHandle,
    // What held focus when the dialog opened on an empty list, until the list fills.
    focus_when_filled: Option<FocusHandle>,
    _observing: Subscription,
    _message_changes: Subscription,
}

/// How the last commit ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The commit landed; its abbreviated id.
    Committed(String),
    /// Git refused or failed; its text, verbatim.
    Failed(String),
}

impl ChangesView {
    /// The changed files in workspace order (path order).
    pub fn files(&self) -> &[(PathBuf, GitChange)] {
        &self.files
    }

    /// Each listed file's stage state, parallel to [`files`](Self::files).
    pub fn stages(&self) -> &[GitStage] {
        &self.stages
    }

    /// Whether a stage or unstage is running.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// How many listed files are staged, fully or partly.
    pub fn staged_count(&self) -> usize {
        self.stages
            .iter()
            .filter(|s| !matches!(s, GitStage::Unstaged))
            .count()
    }

    /// Stages the file at `ix`, or unstages it when its box is ticked. Does nothing while an
    /// operation runs or for an index past the list.
    pub fn toggle(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some((op, _)) = self.row_op(ix) {
            self.notice = None;
            self.start(op, cx);
        }
    }

    // What a toggle of the row at `ix` runs, and the word for it.
    fn row_op(&self, ix: usize) -> Option<(GitWrite, &'static str)> {
        let paths = vec![self.files.get(ix)?.0.clone()];
        Some(match self.stages.get(ix) {
            Some(stage) if stage.is_full() => (GitWrite::Unstage(paths), "Unstage"),
            _ => (GitWrite::Stage(paths), "Stage"),
        })
    }

    // How many gone rows an action on the row at `ix` drops, that row included: git takes a path
    // with all under it, an entry replaces any file over or under it, and unstaging resets the
    // origin of every rename at or under the row's path too.
    fn gone_around(&self, ix: usize) -> usize {
        let Some((at, _)) = self.files.get(ix) else {
            return 0;
        };
        let unstages = self.stages.get(ix).is_some_and(|s| s.is_full());
        let origins: Vec<&PathBuf> = (self.files.iter())
            .filter(|_| unstages)
            .filter_map(|(path, change)| match change {
                GitChange::Renamed { from } if path.starts_with(at) => Some(from),
                _ => None,
            })
            .collect();
        let touches =
            |path: &Path, other: &Path| path.starts_with(other) || other.starts_with(path);
        (self.files.iter().zip(&self.stages))
            .filter(|((path, _), stage)| {
                **stage == GitStage::StagedGone
                    && (touches(path, at) || origins.iter().any(|o| touches(path, o)))
            })
            .count()
    }

    // Toggles the row at `ix` by Space, Enter or its box, asking first when that drops a gone
    // row's copy; a gone row that drops only itself asks in the words of its box's warning.
    fn press(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((op, action)) = self.row_op(ix) else {
            return;
        };
        let ask = match self.gone_around(ix) {
            0 => None,
            1 if self.stages.get(ix) == Some(&GitStage::StagedGone) => {
                let mut ask = confirm::drop_staged_ask(1, action);
                let path = self.files[ix].0.to_string_lossy();
                ask.detail = Some(gone_warning(&self.left_at(&path)).into());
                Some(ask)
            }
            n => Some(confirm::drop_staged_ask(n, action)),
        };
        self.confirmed(op, ask, window, cx);
    }

    // Unstages everything when every box is ticked, else stages everything; either drops the
    // staged copy of each gone row.
    fn toggle_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let all = vec![PathBuf::from(".")];
        let (op, action) = match self.stages.iter().all(|s| s.is_full()) {
            true => (GitWrite::Unstage(all), "Unstage all"),
            false => (GitWrite::Stage(all), "Stage all"),
        };
        let gone = self
            .stages
            .iter()
            .filter(|s| **s == GitStage::StagedGone)
            .count();
        // `.` reaches past the cap, where any file may be gone, so a cut list always asks.
        let ask = match (self.truncated(), gone) {
            (true, n) => Some(confirm::drop_staged_past_cap_ask(n, action)),
            (false, 0) => None,
            (false, n) => Some(confirm::drop_staged_ask(n, action)),
        };
        self.confirmed(op, ask, window, cx);
    }

    // Runs `op`, after `ask` when there is one; Cancel runs nothing, so every row stays as it was.
    fn confirmed(
        &mut self,
        op: GitWrite,
        ask: Option<confirm::Ask>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(ask) = ask else {
            self.notice = None;
            self.start(op, cx);
            return;
        };
        let answer = confirm::ask(ask, window, cx);
        cx.spawn(async move |this, cx| {
            if answer.recv().await == Ok(1) {
                this.update(cx, |v, cx| {
                    if !v.busy {
                        v.notice = None;
                        v.start(op, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    // Runs `op` through the workspace and holds `busy` until it returns; no git leaves it idle.
    fn start(&mut self, op: GitWrite, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let Some(task) = self
            .workspace
            .update(cx, |w, cx| w.run_git_write(&root, op, cx))
        else {
            return;
        };
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let done = task.await;
            // A landed commit used the root's draft, even when the dialog closed meanwhile.
            if matches!(done, GitWriteDone::Committed { .. }) {
                cx.update(|cx| form_drafts::clear_commit(&root, cx));
            }
            this.update(cx, |v, cx| v.finished(done, cx))
        })
        .detach();
    }

    fn finished(&mut self, done: GitWriteDone, cx: &mut Context<Self>) {
        self.busy = false;
        match done {
            GitWriteDone::Committed { short_id, .. } => {
                self.notice = Some(Notice::Committed(short_id));
                // The message belongs to the commit that just landed.
                self.clear_message = true;
            }
            GitWriteDone::Failed(text) => self.notice = Some(Notice::Failed(text)),
            _ => {}
        }
        cx.notify();
    }

    /// The commit message as typed.
    pub fn message(&self, cx: &App) -> String {
        self.message.read(cx).value().to_string()
    }

    /// Whether the commit message field is read-only and whether it is disabled, as drawn.
    pub fn message_lock(&self, cx: &App) -> (bool, bool) {
        let p = self.message.read(cx).presentation();
        (p.is_readonly(), p.is_disabled())
    }

    /// Replaces the commit message, which is kept as the root's draft as typing is.
    pub fn set_message(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.message
            .update(cx, |m, cx| m.set_value(text, window, cx));
        form_drafts::set_commit(&self.root, text, cx);
        cx.notify();
    }

    /// Moves focus into the commit message field.
    pub fn focus_message(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.message.update(cx, |m, cx| m.focus(window, cx));
    }

    /// Whether Commit is allowed: something is staged, the message has text and nothing runs.
    pub fn can_commit(&self, cx: &App) -> bool {
        self.staged_count() > 0 && !self.busy && !self.message(cx).trim().is_empty()
    }

    /// Commits what is staged with the typed message; does nothing unless `can_commit`.
    pub fn commit(&mut self, cx: &mut Context<Self>) {
        if self.can_commit(cx) {
            self.notice = None;
            let message = self.message(cx);
            self.start(GitWrite::Commit(message), cx);
        }
    }

    /// The last commit's outcome.
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    /// Whether the list stops at core's cap.
    pub fn truncated(&self) -> bool {
        self.truncated_note.is_some()
    }

    /// The selected file's collection-relative path.
    pub fn selected(&self) -> Option<&Path> {
        self.selected.map(|ix| self.files[ix].0.as_path())
    }

    /// How many diffs this view has started reading, for tests that pin a load to once.
    pub fn diff_loads(&self) -> usize {
        self.diff_loads
    }

    /// The selected file's diff; `None` while it loads.
    pub fn doc(&self) -> Option<&DiffDoc> {
        self.doc.as_ref()
    }

    // A new git state rebuilds the list; a finished status run with an equal state reloads
    // the selected diff, whose file may have changed; any other workspace notify does nothing.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let ws = self.workspace.read(cx);
        let (now, runs) = (ws.git_changes(&self.root), ws.git_runs(&self.root));
        let same = match (&now, &self.seen) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let ran = runs != self.runs;
        self.runs = runs;
        if !same {
            self.fresh = false;
            let keep = self.selected().map(Path::to_path_buf);
            self.show(now, keep, cx);
        } else if ran {
            // The first run after open only confirms what the open already read.
            if !std::mem::replace(&mut self.fresh, false) {
                self.load(cx);
            }
        }
    }

    // Shows `changes`, selecting `keep` when it is still listed, else the first file; `None`
    // empties the list and the pane for the quiet state.
    fn show(
        &mut self,
        changes: Option<Arc<GitChanges>>,
        keep: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.files = changes
            .iter()
            .flat_map(|c| c.files.iter().map(|(p, c)| (p.clone(), c.clone())))
            .collect();
        self.stages = changes
            .iter()
            .flat_map(|c| {
                c.files
                    .keys()
                    .map(|p| c.stage.get(p).copied().unwrap_or(GitStage::Unstaged))
            })
            .collect();
        self.truncated_note = changes.as_ref().and_then(|c| c.truncated_note());
        self.branch = changes.as_ref().and_then(|c| c.branch.clone());
        self.seen = changes;
        self.selected = None;
        let at = keep
            .as_ref()
            .and_then(|k| self.files.iter().position(|(p, _)| p == k))
            .or_else(|| self.first_request())
            .unwrap_or(0);
        match self.files.get(at) {
            // The same file keeps its diff on screen until the reload lands.
            Some((path, _)) if Some(path) == keep.as_ref() => {
                self.selected = Some(at);
                self.load(cx);
            }
            Some(_) => self.select(at, cx),
            None => {
                self.clear_doc();
                // Drops any diff still loading for the old list.
                self.generation += 1;
                cx.notify();
            }
        }
    }

    /// Selects the file at `ix` and loads its diff off the UI thread; an index past the list
    /// does nothing.
    pub fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.files.len() {
            return;
        }
        if self.selected != Some(ix) {
            self.clear_doc();
        }
        self.selected = Some(ix);
        self.load(cx);
    }

    // The first listed file core can diff (a .yml or .yaml: request, folder or environment, loaded
    // or not), so the dialog opens on a diff rather than a note file that sorts first.
    fn first_request(&self) -> Option<usize> {
        self.files
            .iter()
            .position(|(path, _)| diff_doc::is_request_file(path))
    }

    // Loads the selected file's diff off the UI thread; what is on screen stays until it lands.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        cx.notify();
        let Some((rel, _)) = self.selected.and_then(|ix| self.files.get(ix)) else {
            return;
        };
        if !diff_doc::is_request_file(rel) {
            self.set_doc(DiffDoc::not_a_request());
            return;
        }
        let Some(options) = self.workspace.read(cx).git_options().cloned() else {
            return;
        };
        self.diff_loads += 1;
        let (root, rel, generation) = (self.root.clone(), rel.clone(), self.generation);
        let diff = cx.background_spawn(async move { file_diff_with(&root, &rel, &options) });
        cx.spawn(async move |this, cx| {
            let result = diff.await;
            this.update(cx, |v, cx| {
                // A newer load owns the pane now; this result is stale.
                if v.generation != generation {
                    return;
                }
                v.set_doc(match &result {
                    Ok(diff) => DiffDoc::from_diff(diff),
                    Err(error) => DiffDoc::from_error(error),
                });
                cx.notify();
            })
        })
        .detach();
    }

    // Up and Down in the focused list move the selection one row, stopping at either end.
    fn step(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Keys bubble up from the bulk button in the header; only the list itself steps rows.
        if event.keystroke.modifiers.modified() || !self.list_focus.is_focused(window) {
            return;
        }
        let at = self.selected;
        let next = match event.keystroke.key.as_str() {
            "up" => at.and_then(|ix| ix.checked_sub(1)),
            "down" => Some(at.map_or(0, |ix| ix + 1)),
            // Space stages or unstages the selected row, ignored while busy.
            "space" | " " => {
                if let Some(ix) = at {
                    cx.stop_propagation();
                    self.press(ix, window, cx);
                }
                return;
            }
            _ => return,
        };
        cx.stop_propagation();
        if let Some(ix) = next.filter(|&ix| ix < self.files.len()) {
            self.select(ix, cx);
            self.files_scroll.scroll_to_item(ix);
        }
    }

    fn clear_doc(&mut self) {
        self.doc = None;
        self.rows = Arc::from([]);
    }

    // A reload of the file on screen keeps its scroll position; a newly selected file starts
    // at the top.
    fn set_doc(&mut self, doc: DiffDoc) {
        self.rows = match &doc.body {
            DiffBody::Lines(rows) => rows.as_slice().into(),
            _ => Arc::from([]),
        };
        if self.doc.replace(doc).is_none() {
            self.lines_scroll = UniformListScrollHandle::new();
        }
    }

    // Beside the diff the list keeps a fixed width; stacked over it, its share of the panes.
    fn file_list(
        &self,
        stacked: Option<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (active, muted, radius) = (theme.list_active, theme.muted_foreground, theme.radius);
        // The selected row reads like the tree's: neutral fill and a left edge, accent only
        // while the list itself holds focus, not its bulk button.
        let focused = self.list_focus.is_focused(window);
        let edge = crate::sidebar::selection_edge_color(focused, cx);
        let edge_kind = if focused {
            "changes-file-focus-edge"
        } else {
            "changes-file-edge"
        };
        let list_focus = self.list_focus.clone();
        let this = cx.entity();
        let header = self.summary_header(cx);
        let rows = self.files.iter().enumerate().map(|(ix, (path, change))| {
            let on = self.selected == Some(ix);
            let path = path.to_string_lossy().into_owned();
            let (dir, name) = match path.rfind(['/', '\\']) {
                Some(cut) => (path[..=cut].to_owned(), path[cut + 1..].to_owned()),
                None => (String::new(), path.clone()),
            };
            let gone = self.stages.get(ix) == Some(&GitStage::StagedGone);
            let note = match self.stages.get(ix) {
                Some(GitStage::Both) => Some(("changes-partly", "partly staged".to_owned())),
                Some(GitStage::StagedGone) => {
                    Some(("changes-gone", gone_note(&self.left_at(&path))))
                }
                _ => None,
            };
            let origin = match change {
                GitChange::Renamed { from } => Some(from.to_string_lossy().into_owned()),
                _ => None,
            };
            let tall = note.is_some() || origin.is_some();
            let spoken = match (&origin, &note) {
                (Some(origin), _) => format!("{} {path} from {origin}", change.word()),
                (None, Some((_, note))) if gone => format!("{} {path}, {note}", change.word()),
                (None, _) => format!("{} {path}", change.word()),
            };
            h_flex()
                .id(("changes-file", ix))
                .test_support()
                .role(Role::ListBoxOption)
                .aria_label(spoken)
                .aria_selected(on)
                .gap(style::SPACE_2)
                .px(style::SPACE_2)
                // Only a minimum, since a wrapped name makes a row taller than one line.
                .min_h(style::ROW_HEIGHT)
                .py(if tall { style::SPACE_1 } else { style::SPACE_0 })
                .flex_none()
                .relative()
                .rounded(radius)
                .when(on, |row| {
                    row.bg(active).child(
                        div()
                            .id((edge_kind, ix))
                            .test_support()
                            .absolute()
                            .inset_0()
                            .rounded(radius)
                            .border_l_2()
                            .border_color(edge),
                    )
                })
                .cursor_pointer()
                .child(self.stage_box(ix, &path, this.clone()))
                .on_click({
                    let list_focus = list_focus.clone();
                    cx.listener(move |this, _, window, cx| {
                        window.focus(&list_focus, cx);
                        this.select(ix, cx);
                    })
                })
                .child(
                    // The status word and directory stay one line; a name that does not fit
                    // beside them wraps to a line of its own at the row's full width. A rename's
                    // origin and a stage note each add a line, so none is clipped.
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            h_flex()
                                .flex_wrap()
                                .gap_x(style::SPACE_2)
                                .min_w_0()
                                .child(
                                    h_flex()
                                        .gap(style::SPACE_2)
                                        .min_w_0()
                                        .child(
                                            // A fixed column, so the names line up whatever the
                                            // word.
                                            div()
                                                .id(("changes-file-word", ix))
                                                .test_support()
                                                .flex_none()
                                                .w(style::CHANGES_STATUS_WIDTH)
                                                .text_size(style::TEXT_SMALL)
                                                .text_color(muted)
                                                .child(change.word()),
                                        )
                                        .when(!dir.is_empty(), |line| {
                                            line.child(
                                                div()
                                                    .id(("changes-file-dir", ix))
                                                    .test_support()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_color(muted)
                                                    .child(dir),
                                            )
                                        }),
                                )
                                .child({
                                    // A name too long even for its own line ends in an ellipsis;
                                    // hovering it shows the whole path.
                                    div()
                                        .id(("changes-file-name", ix))
                                        .test_support()
                                        .min_w_0()
                                        .truncate()
                                        .tooltip(ui::text_tooltip(
                                            "changes-file-tooltip",
                                            SharedString::from(path.clone()),
                                        ))
                                        .child(name)
                                }),
                        )
                        .when_some(origin, |col, origin| {
                            let text = format!("from {origin}");
                            col.child(
                                div()
                                    .id(("changes-from", ix))
                                    .test_support()
                                    .flex_none()
                                    .truncate()
                                    .text_size(style::TEXT_SMALL)
                                    .text_color(muted)
                                    .aria_label(text.clone())
                                    .tooltip(ui::text_tooltip(
                                        "changes-from-tooltip",
                                        SharedString::from(origin),
                                    ))
                                    .child(text),
                            )
                        })
                        .when_some(note, |col, (id, text)| {
                            col.child(
                                div()
                                    .id((id, ix))
                                    .test_support()
                                    .flex_none()
                                    .text_size(style::TEXT_SMALL)
                                    .text_color(muted)
                                    .aria_label(text.clone())
                                    .child(text),
                            )
                        }),
                )
        });
        let quiet = self.seen.is_none().then(|| {
            div()
                .id("changes-quiet")
                .test_support()
                .aria_label(QUIET_NOTE)
                .p(style::SPACE_2)
                .text_color(muted)
                .child(QUIET_NOTE)
        });
        let empty = (self.seen.is_some() && self.files.is_empty()).then(|| {
            div()
                .id("changes-empty")
                .test_support()
                .p(style::SPACE_2)
                .text_color(muted)
                .truncate()
                .tooltip(|window, cx| Tooltip::new(NO_CHANGES).build(window, cx))
                .child(NO_CHANGES)
        });
        let truncated = self.truncated_note.clone().map(|note| {
            div()
                .id("changes-truncated")
                .test_support()
                .p(style::SPACE_2)
                .text_size(style::TEXT_SMALL)
                .text_color(muted)
                .child(note)
        });
        let list = div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(
                v_flex()
                    .id("changes-rows")
                    .test_support()
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.files_scroll)
                    .children(rows)
                    .children(quiet)
                    .children(empty)
                    .children(truncated),
            )
            .vertical_scrollbar(&self.files_scroll);
        // The header sits above the scrolling rows, so the counts and the bulk button stay put.
        v_flex()
            .id("changes-files")
            .test_support()
            .track_focus(&self.list_focus)
            .on_key_down(cx.listener(Self::step))
            // Enter in the focused list stages like Space instead of reaching the dialog's
            // Confirm; the bulk button in the header takes its own Enter first.
            .on_action(cx.listener(|this, _: &Confirm, window, cx| {
                if let Some(ix) = this.selected.filter(|_| this.list_focus.is_focused(window)) {
                    cx.stop_propagation();
                    this.press(ix, window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .role(Role::ListBox)
            .relative()
            .pt(style::FOCUS_RING_ROOM)
            .flex_none()
            .map(|d| match stacked {
                Some(height) => d.w_full().h(height),
                None => d.w(style::CHANGES_FILES_WIDTH).h_full(),
            })
            .children(header)
            .child(list)
            // The dialog body clips flush with the list's top, so the ring goes on an overlay
            // inset by the ring's room: its outside band then lands inside the list's own box.
            .when(focused, |list| {
                list.child(
                    div()
                        .id("changes-files-ring")
                        .test_support()
                        .absolute()
                        .top(style::FOCUS_RING_ROOM)
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .focus_ring_style(window, cx),
                )
            })
    }

    // The row's checkbox: ticked when a commit takes the whole file; its name is for accessibility
    // only. Unticking a file gone from disk loses the indexed copy, so that box says so.
    fn stage_box(&self, ix: usize, path: &str, this: Entity<Self>) -> impl IntoElement + use<> {
        let stage = self.stages.get(ix).copied().unwrap_or(GitStage::Unstaged);
        let gone = stage == GitStage::StagedGone;
        let warning = gone_warning(&self.left_at(path));
        let name = match gone {
            true => format!("Stage {path}. {warning}"),
            false => format!("Stage {path}"),
        };
        div()
            .id(("changes-stage-box", ix))
            .flex_none()
            .when(gone, |d| {
                d.tooltip(ui::text_tooltip("changes-gone-tooltip", warning))
            })
            .child(
                ui::checkbox(("changes-stage", ix), stage.is_full())
                    .accessible_name(name)
                    .disabled(self.busy)
                    .tab_stop(false)
                    // A gone row's own box warns on hover and asks the same on a click.
                    .on_click(move |_, window, cx| {
                        this.update(cx, |v, cx| v.press(ix, window, cx))
                    }),
            )
    }

    // What is on disk for the gone row at `path`: where its file went, or the other file its
    // path now opens, when core knows.
    fn left_at(&self, path: &str) -> Left {
        let Some(changes) = self.seen.as_ref() else {
            return Left::Nothing;
        };
        let shown = |p: &Path| p.to_string_lossy().into_owned();
        match changes.kept_at.get(Path::new(path)) {
            Some(kept) => Left::Kept(shown(kept)),
            None => match changes.opens_at.get(Path::new(path)) {
                Some(other) => Left::Opens(shown(other)),
                None => Left::Nothing,
            },
        }
    }

    // The counts and the one bulk button; absent in the quiet and empty states.
    fn summary_header(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.seen.is_none() || self.files.is_empty() {
            return None;
        }
        // The count matches the ticked boxes.
        let staged = self.stages.iter().filter(|s| s.is_full()).count();
        let total = self.files.len();
        let all = staged == total;
        let this = cx.entity();
        Some(
            h_flex()
                .gap(style::SPACE_2)
                .px(style::SPACE_2)
                .h(style::ROW_HEIGHT)
                .flex_none()
                .justify_between()
                .child(
                    div()
                        .id("changes-summary")
                        .test_support()
                        .aria_label(format!("{staged} staged of {total}"))
                        .text_size(style::TEXT_SMALL)
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{staged} staged of {total}")),
                )
                .child(
                    // Enter in a dialog arrives as its Confirm, which would close it; the
                    // focused bulk button takes it as a press instead.
                    div()
                        .on_action({
                            let this = this.clone();
                            move |_: &Confirm, window, cx| {
                                cx.stop_propagation();
                                this.update(cx, |v, cx| v.toggle_all(window, cx));
                            }
                        })
                        .child(
                            Button::new("changes-toggle-all")
                                .label(if all { "Unstage all" } else { "Stage all" })
                                .small()
                                .lock(self.busy, cx)
                                .on_click(move |_, window, cx| {
                                    this.update(cx, |v, cx| v.toggle_all(window, cx))
                                }),
                        ),
                ),
        )
    }

    fn diff_pane(&self, stacked: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(doc) = &self.doc else {
            return div().into_any_element();
        };
        let sections = h_flex()
            .id("changes-sections")
            .test_support()
            .flex_wrap()
            .gap(style::SPACE_1)
            .children(doc.sections.iter().enumerate().map(|(i, s)| {
                div()
                    .id(("changes-section", i))
                    .test_support()
                    .child(Tag::secondary().small().child(*s))
            }))
            // An empty block as tall as a small chip holds the row's height, so the body does
            // not jump up without one; unlike a hidden chip it leaves no text in the tree.
            .when(doc.sections.is_empty(), |row| {
                let chip = chip_height(window.rem_size());
                row.child(div().id("changes-sections-spacer").test_support().h(chip))
            });
        let body = match (&doc.body, doc.note()) {
            (DiffBody::Lines(_), _) => self.lines(self.rows.clone(), window, cx),
            (_, Some(note)) => div()
                .id("changes-note")
                .test_support()
                .aria_label(note.to_owned())
                .text_color(cx.theme().muted_foreground)
                .child(note.to_owned())
                .into_any_element(),
            (_, None) => div().into_any_element(),
        };
        // Beside the list the pane takes the row's height; stacked, the room left under it. It
        // takes the list's ring room on top beside it, so the chips line up with the list header.
        v_flex()
            .when(!stacked, |d| d.ring_room(RingSides::TOP))
            .flex_1()
            .min_w_0()
            .min_h_0()
            .when(!stacked, |d| d.h_full())
            .gap(style::SPACE_2)
            .child(sections)
            .child(body)
            .into_any_element()
    }

    // The rows are virtualized like the response body's: a gutter mark, both line numbers in
    // the mono font, and the row's screen-safe text; colour only tints what the mark says.
    fn lines(
        &self,
        rows: Arc<[DiffRow]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = DiffPalette::new(&rows, window, cx);
        // The widest row sets the list's content width, so a long line scrolls sideways.
        let widest = (0..rows.len())
            .max_by_key(|&ix| rows[ix].text.chars().count())
            .unwrap_or(0);
        let list = uniform_list(
            "changes-diff-lines",
            rows.len(),
            move |range: Range<usize>, _: &mut Window, _: &mut App| {
                range
                    .map(|ix| diff_row(ix, &rows[ix], &palette))
                    .collect::<Vec<_>>()
            },
        )
        .with_width_from_item(Some(widest))
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .track_scroll(&self.lines_scroll)
        .size_full();
        let border = cx.theme().input;
        let radius = cx.theme().radius;
        let sideways = self.lines_scroll.0.borrow().base_handle.clone();
        let pane = div()
            .id("changes-diff")
            .test_support()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .border_1()
            .border_color(border)
            .rounded(radius)
            .overflow_hidden()
            .child(list)
            .vertical_scrollbar(&self.lines_scroll);
        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(pane)
            .children(ui::scroll_bar_x("diff-scrollbar", &sideways))
            .into_any_element()
    }
}

/// A small Tag's height at `rem`: a 1.25 line over 0.625 rem text (whole pixels once laid
/// out), 0.125 rem of padding above and below, and 1 px borders.
fn chip_height(rem: Pixels) -> Pixels {
    (rem * (0.625 * 1.25)).floor() + rem * 0.25 + px(2.)
}

/// The panes' least height for the diff to keep six `row`s under its chip row. Stacked, the
/// list takes `stacked_list_height` of the panes first, so the panes must grow to cover it.
pub fn min_panes_height(row: Pixels, rem: Pixels, stacked: bool) -> Pixels {
    let diff = row * 6.;
    if !stacked {
        return diff;
    }
    let (pane_gap, chip_gap) = (
        style::snap(style::SPACE_3.to_pixels(rem)),
        style::snap(style::SPACE_2.to_pixels(rem)),
    );
    let beside = diff + pane_gap + chip_gap + chip_height(rem);
    // The list is two fifths unless three rows are more, so the diff has three fifths or the
    // panes less three rows.
    (beside / 0.6).max(beside + row * 3.)
}

/// The stacked file list's height for panes `body` tall: two fifths, never under three rows of `row`.
pub fn stacked_list_height(body: Pixels, row: Pixels) -> Pixels {
    (body * 0.4).max(row * 3.)
}

impl Render for ChangesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.files.is_empty()
            && let Some(host) = self.focus_when_filled.take()
            && host.is_focused(window)
        {
            let list = self.list_focus.clone();
            cx.defer_in(window, move |_, window, cx| window.focus(&list, cx));
        }
        if std::mem::take(&mut self.clear_message) {
            let message = self.message.clone();
            cx.defer_in(window, move |_, window, cx| {
                message.update(cx, |m, cx| m.set_value("", window, cx));
            });
        }
        // A narrow dialog stacks the list over the diff, as form rows stack their labels.
        let (stacked, watch) = ui::narrower_than(
            "changes-panes".into(),
            style::CHANGES_STACK_WIDTH,
            window,
            cx,
        );
        let row = style::resolve(style::ROW_HEIGHT, window);
        let gap = style::resolve(style::DIALOG_FOOTER_GAP, window);
        // The panes fill the room above the status bar, less the message, note, notice and
        // footer rows with their gaps. A long failure notice yields first, so the diff keeps
        // four rows (plus its chip row) and the notice scrolls inside what is left. The estimate
        // runs above the measured rows, so the diff's ring room comes out of that slack.
        let base = crate::dialog_frame::titled_body_room(window) - (row * 4. + gap * 4.)
            + style::FOCUS_RING_ROOM;
        let min_panes = min_panes_height(row, window.rem_size(), stacked);
        let notice_cap = (base - min_panes - gap)
            .max(row)
            .min(style::resolve(style::CHANGES_NOTICE_MAX_HEIGHT, window));
        let notice_room = self.notice.as_ref().map_or(px(0.), |n| match n {
            Notice::Committed(_) => row + gap,
            Notice::Failed(_) => notice_cap + gap,
        });
        let room = base - notice_room;
        let height = style::resolve(style::CHANGES_DIALOG_HEIGHT, window).min(room.max(min_panes));
        let panes = if stacked {
            v_flex()
        } else {
            h_flex().items_start()
        };
        v_flex()
            .id("changes-dialog")
            .test_support()
            .child(
                panes
                    .id("changes-panes")
                    .test_support()
                    .relative()
                    .w_full()
                    .h(height)
                    .gap(style::SPACE_3)
                    .child(self.file_list(
                        stacked.then(|| {
                            stacked_list_height(height, style::resolve(style::ROW_HEIGHT, window))
                        }),
                        window,
                        cx,
                    ))
                    .child(self.diff_pane(stacked, window, cx))
                    .child(watch),
            )
            .child(self.commit_area(notice_cap, window, cx))
            .child(ui::dialog_footer(
                None,
                [Button::new("changes-close")
                    .label("Close")
                    .ghost()
                    .on_click(|_, window, cx| window.close_dialog(cx))],
                Button::new("changes-commit")
                    .label(commit_label(self.branch.as_deref()))
                    .primary()
                    .lock_filled(!self.can_commit(cx), cx)
                    .on_click(cx.listener(|this, _, _, cx| this.commit(cx))),
            ))
    }
}

impl ChangesView {
    // The message field, the no-push note and the last commit's notice, above the footer.
    fn commit_area(
        &self,
        notice_cap: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let notice = self.notice.as_ref().map(|n| match n {
            Notice::Committed(id) => {
                let text = format!("Committed {id}");
                div()
                    .id("changes-notice")
                    .test_support()
                    .aria_label(text.clone())
                    .text_color(theme.success)
                    .child(text)
                    .into_any_element()
            }
            Notice::Failed(text) => div()
                .id("changes-notice")
                .test_support()
                .aria_label(text.clone())
                .max_h(notice_cap)
                .overflow_y_scroll()
                .font_family(theme.mono_font_family.clone())
                .text_size(style::TEXT_SMALL)
                .text_color(theme.warning)
                .child(text.clone())
                .into_any_element(),
        });
        v_flex()
            .gap(style::SPACE_2)
            .mt(style::SPACE_2)
            .child(
                div()
                    .id("changes-message")
                    .test_support()
                    .key_context(MESSAGE_CONTEXT)
                    .on_action(cx.listener(|this, _: &CommitChanges, _, cx| this.commit(cx)))
                    // A commit message has no use for indentation, so Tab keeps moving focus
                    // instead of trapping the keyboard in the field.
                    .capture_action(|_: &IndentInline, window, cx| {
                        window.focus_next(cx);
                        cx.stop_propagation();
                    })
                    .capture_action(|_: &OutdentInline, window, cx| {
                        window.focus_prev(cx);
                        cx.stop_propagation();
                    })
                    .child(ui::ringed_textarea(
                        ui::lock_textarea(
                            Textarea::new(&self.message),
                            &self.message,
                            self.busy,
                            window,
                            cx,
                        ),
                        &self.message,
                    )),
            )
            .child(
                div()
                    .id("changes-push-note")
                    .test_support()
                    .text_size(style::TEXT_SMALL)
                    .text_color(theme.muted_foreground)
                    .child("Committing does not push."),
            )
            .children(notice)
    }
}
