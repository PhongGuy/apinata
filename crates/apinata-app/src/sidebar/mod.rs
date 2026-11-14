//! The sidebar: every open collection as one tree.

pub mod batch;
pub mod drop;
pub mod menu;
pub mod model;
pub mod multi;
pub mod paste;
pub mod terms;

use crate::ui::LockButton as _;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use apinata_core::{DropTarget, ItemPath, RequestKind};
use gpui_kit::Role;
use gpui_kit::assets::IconName;
use gpui_kit::base::actions::{Cancel, SelectDown, SelectUp};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::tree::{TreeEvent, TreeState, tree};
use gpui_kit::component::{ActiveTheme, Icon, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, Context, DismissEvent, DragMoveEvent, Edges,
    ElementId, Entity, EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement,
    Modifiers, MouseButton, ParentElement, Pixels, Render, RenderOnce, ScrollStrategy,
    SharedString, StatefulInteractiveElement, Styled, Subscription, TestSupportExt, WeakEntity,
    Window, actions, anchored, canvas, deferred, div, px,
};

use crate::command::{Command, NoticeKind, TreeOp};
use crate::commands::label;
use crate::export::ExportFormat;
use crate::git_status::{self, GitChange, GitChanges};
use crate::history_palette::OpenHistory;
use crate::keymap::{NewGraphqlRequest, NewRequest, SHELL_CONTEXT};
use crate::menu_focus;
use crate::shell::collection_row;
use crate::snapshot::{CollectionSnapshot, Content, DiskState, RowKind, Unsupported};
use crate::stale::rebase;
use crate::tab_area::{ConvertToGraphql, ConvertToHttp, CopyAsCurl, GenerateCode};
use crate::tabs::TabId;
use crate::workspace::{Workspace, WorkspaceEvent};
use crate::{style, ui};
use batch::{Batch, Verb, bulk_delete_ask, permanent_delete_ask, single_delete_ask};
use drop::{DragPreview, DraggedRow, DropZone, drop_target, drop_zone, multi_drop};
use menu::{BulkEntry, MenuEntry, NEW_FOLDER_NAME, bulk_entries, clone_name, menu_entries};
use model::{Expanded, Node, NodeKind, OpenDraft};
use multi::{Multi, SelKey, Tally, row_keys, selectable, visible_keys};
use paste::{ClipMode, paste_target};

/// The key context the sidebar's bindings live in.
pub const KEY_CONTEXT: &str = "Sidebar";

/// How many finished deletes stay undoable with Ctrl+Z; an older one's trash is let go.
pub const UNDO_DELETE_CAP: usize = 10;

/// The key context around the filter input, so window keys can be masked there alone.
pub const FILTER_CONTEXT: &str = "SidebarFilter";

/// The filter field's placeholder, one word so it fits beside the key hint at the default width.
pub const FILTER_PLACEHOLDER: &str = "Filter";

/// The tooltip on the filter field, where the `tag:` and `method:` terms are named.
pub const FILTER_TOOLTIP: &str = "Filter by name. Use tag:name or method:get.";

// A collection root's hint shrinks this many times slower than its name: twins share the name,
// so the hint is what tells them apart and keeps its room until the name is gone.
const HINT_SHRINK: f32 = 1. / 10_000.;

// The group every tree row carries, so a row's run button shows while the row is hovered.
const ROW_GROUP: &str = "sidebar-row";
/// The notice a Duplicate of a row with unsaved edits pushes.
const DUPLICATE_UNSAVED: &str = "The copy includes unsaved edits.";

actions!(
    sidebar,
    [
        /// Moves focus to the filter input and selects its text.
        FocusFilter,
        /// Clears the filter, or moves focus to the tree once it is empty.
        ClearFilter,
        /// Moves focus from the filter to the tree and selects its first row.
        EnterTree,
        /// Starts renaming the selected folder, request or example.
        RenameSelected,
        /// Clones the selected request.
        CloneSelected,
        /// Opens the selected row's directory in the system file manager.
        ShowInFileManager,
        /// Opens a terminal in the selected row's directory.
        OpenInTerminal,
        /// Deletes the selected folder, request or other item, after asking.
        DeleteSelected,
        /// Copies the selected items onto the sidebar clipboard.
        CopySelected,
        /// Cuts the selected items; they move when pasted.
        CutSelected,
        /// Pastes the sidebar clipboard into the selected folder.
        PasteSelected,
        /// Creates a folder beside the selection and starts renaming it.
        NewFolder,
        /// Renumbers the selected folder's items, or the selection's folder, by name.
        SortByName,
        /// Keeps the selected request open in a tab, or toggles the selected folder.
        OpenSelected,
        /// Previews the selected request.
        PreviewSelected,
        /// Moves focus from the tree to the Tab stop before it (the tree's shift+tab).
        LeaveTree,
        /// Empties the multi-selection, keeping the focus row.
        ClearSelection,
        /// Grows or shrinks the multi-selection one row up.
        ExtendUp,
        /// Grows or shrinks the multi-selection one row down.
        ExtendDown,
        /// Runs the multi-selection in a runner tab of its own.
        RunSelected,
        /// Closes every folder of every open collection.
        CollapseAll,
        /// Opens every folder of every open collection.
        ExpandAll,
        /// Restores the newest delete still undoable, or passes the key on when there is none.
        UndoDelete,
        /// Opens the selected row's context menu at the row, as a right click on it would.
        OpenRowMenu,
        /// Opens the selected folder's settings tab.
        FolderSettings,
        /// Saves the selected draft through the save dialog, whichever tab is active.
        SaveSelectedDraft,
        /// Closes the selected draft's tab, whichever tab is active.
        CloseSelectedDraft
    ]
);

/// The id of a row's marker element: `"{kind}:{node}"`, for kinds `chevron`, `icon`,
/// `method`, `dirty`, `rollup`, `removed`, `broken`, `git` and `rename`.
pub fn marker_id(kind: &str, node: &str) -> SharedString {
    format!("{kind}:{node}").into()
}

/// What the sidebar asks of the rest of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum SidebarEvent {
    /// Open a request: a single click previews it, a double click keeps it open.
    OpenItem {
        /// The collection's root.
        root: PathBuf,
        /// The request within it.
        path: ItemPath,
        /// Whether the tab stays open rather than previewing.
        permanent: bool,
        /// Whether the keyboard moves on to the opened request's URL, as Enter on a row does.
        focus_url: bool,
    },
    /// Open the settings tab of a folder, or of the collection when `folder` is `None`.
    OpenSettings {
        /// The collection's root.
        root: PathBuf,
        /// The folder within it.
        folder: Option<ItemPath>,
    },
    /// Open the runner tab of a folder, or of the collection when `folder` is `None`.
    Run {
        /// The collection's root.
        root: PathBuf,
        /// The folder within it.
        folder: Option<ItemPath>,
    },
    /// Open the runner tab of a selection of requests and folders.
    RunSelection {
        /// The collection's root.
        root: PathBuf,
        /// The selected items, normalised, without opaque rows, in sidebar order.
        items: Vec<ItemPath>,
    },
    /// Open example `index` of a request: a single click previews it, a double click keeps it.
    OpenExample {
        /// The collection's root.
        root: PathBuf,
        /// The request holding the example.
        path: ItemPath,
        /// The example's index in the request's `examples`.
        index: usize,
        /// Whether the tab stays open rather than previewing.
        permanent: bool,
    },
    /// Save the open draft tab showing `path`, which asks for its name first.
    SaveDraft {
        /// The collection's root.
        root: PathBuf,
        /// The draft's item path.
        path: ItemPath,
    },
    /// Close the open draft tab showing `path`, asking first when it has edits.
    CloseDraft {
        /// The collection's root.
        root: PathBuf,
        /// The draft's item path.
        path: ItemPath,
    },
}

impl EventEmitter<SidebarEvent> for Sidebar {}

// A command this view tracks: its root, the open generation it was sent to, and its id. Ids
// restart in a reopened collection, so the generation keeps an old instance's from matching.
type Tracked = (PathBuf, u64, u64);

/// A create or clone this view sent: whether its new row renames, and the notice its success
/// shows.
#[derive(Clone, Copy)]
struct PendingRow {
    rename: bool,
    notice: Option<&'static str>,
}

/// The tree of open collections, rebuilt from the workspace on every change.
pub struct Sidebar {
    renders: usize,
    // The selected row last seen, so only a move of the selection changes the current collection.
    seen_selection: Option<SharedString>,
    workspace: Entity<Workspace>,
    tree: Entity<TreeState>,
    // The tree keeps its own handle private; this one wraps it to tell when it holds focus.
    tree_focus: FocusHandle,
    // One per collection row, on its New request and New folder buttons, so a focused
    // button does not count as the tree holding focus.
    button_focus: Vec<FocusHandle>,
    expanded: Expanded,
    // Roots whose saved expansion has been applied; only these write back to the session.
    restored: HashSet<PathBuf>,
    filter: Entity<InputState>,
    filter_text: String,
    nodes: Vec<Node>,
    rows: Rc<HashMap<SharedString, Node>>,
    /// Each open collection's git changes, copied from the workspace on every rebuild.
    git: Rc<HashMap<PathBuf, Arc<GitChanges>>>,
    /// The folder and collection rows with a changed or unsaved item inside; a row shows the
    /// roll-up only while collapsed, which the tree can flip without a rebuild.
    rollup: Rc<HashSet<SharedString>>,
    rename: Option<Rename>,
    // A selection the tree lost to a rebuild, and one a move notice relocated; see `rebuild`.
    lost_selection: Option<(PathBuf, ItemPath)>,
    moved_selection: Option<(PathBuf, ItemPath)>,
    // A wanted selection a rebuild handed to a neighbour, with that neighbour: the workspace
    // notifies before it emits a Moved, so the move it belongs to still finds it.
    handed_off: Option<((PathBuf, ItemPath), (PathBuf, ItemPath))>,
    // The example selected before the last rebuild that changed the nodes; see `example_deleted`.
    prior_example: Option<(PathBuf, ItemPath, usize)>,
    // The active request tab the selection last followed; see `rebuild`.
    followed: Option<TabId>,
    pending_focus: Option<PendingFocus>,
    // The folder or root row under the pointer. Layout cannot follow a CSS-like group hover, so a
    // floating run slot learns of it from here and makes room for itself; see `RunSlot`.
    hovered: Option<SharedString>,
    // The row whose right-click menu is open: the menu acts on it, so it stays marked.
    menu_target: Option<SharedString>,
    // The context menu the keyboard opened, drawn at its row.
    key_menu: Option<KeyMenu>,
    // The rows chosen with modifier clicks; the kit tree's own selection stays the focus row.
    multi: Multi,
    // Multi-item commands in flight, each summarised by one toast when finished.
    batches: Vec<Batch>,
    // Finished deletes into the trash, oldest first, that Ctrl+Z or their toast can undo.
    deletes: Vec<UndoEntry>,
    next_delete: u64,
    // Whether the shell draws the tree now; only a drawn tree can take focus.
    drawn: bool,
    // What Copy or Cut last took; see `paste`.
    clipboard: Option<paste::Clipboard>,
    // Cross-collection cut pastes in flight, by target command: the source root, the
    // generation the copy was taken from, and the item to delete once the copy is written.
    cuts: HashMap<Tracked, (PathBuf, u64, ItemPath)>,
    // Moves sent for a same-collection cut; a confirmed one takes its item off the clipboard.
    moves: HashMap<Tracked, ItemPath>,
    // Keys a rebuild could not find, held until the move notice that explains them arrives.
    lost_multi: Vec<SelKey>,
    // Every row key of the last rebuild, so a notice can re-admit the keys that came back.
    live_keys: HashSet<SelKey>,
    // Creates and clones this view sent, until their notice answers.
    pending: HashMap<Tracked, PendingRow>,
    // The row a drag is over and the zone of it, for the drop indicator and the drop itself.
    hover: Option<(SharedString, DropZone)>,
    // The colour each request row's method tag painted, refilled on every render.
    tag_paints: ui::TagPaints,
    // Whether the filter field was last laid out too narrow for its key hint.
    filter_cramped: Rc<std::cell::Cell<bool>>,
    _subscriptions: Vec<Subscription>,
}

/// One undoable delete: the items the instance `generation` of `root` trashed, by ticket.
struct UndoEntry {
    id: u64,
    root: PathBuf,
    generation: u64,
    trashed: Vec<(u64, ItemPath)>,
}

/// An inline rename in progress; dropping it drops the input's subscription.
struct Rename {
    root: PathBuf,
    path: ItemPath,
    old: String,
    // The example being renamed, else the request or folder at `path` itself.
    example: Option<usize>,
    // A folder this view just created, which a cancelled naming removes again.
    fresh: bool,
    // The name Enter refused and why, until the name is edited.
    error: Option<(SharedString, String)>,
    input: Entity<InputState>,
    _subscription: Subscription,
}

/// A row to select, and maybe rename, as soon as a rebuild shows it.
struct PendingFocus {
    root: PathBuf,
    path: ItemPath,
    rename: bool,
}

impl Sidebar {
    /// Record that the pointer entered or left the folder or root row `id`, redrawing on a change.
    fn set_hovered(&mut self, id: &SharedString, entered: bool, cx: &mut Context<Self>) {
        let next = if entered {
            Some(id.clone())
        } else if self.hovered.as_ref() == Some(id) {
            None
        } else {
            return;
        };
        if self.hovered != next {
            self.hovered = next;
            cx.notify();
        }
    }

    /// Builds the sidebar over `workspace` and keeps it in step with every workspace notify.
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(FILTER_PLACEHOLDER));
        // The tree's own handle is not a Tab stop, so this one stands in for it in the Tab
        // order and hands focus straight on, where the tree's keys bind.
        let tree_focus = cx.focus_handle().tab_stop(true);
        let mut subscriptions = vec![
            // Arriving with nothing selected selects the first row so the focus shows; it
            // only selects, since opening a request stays on click.
            cx.on_focus(&tree_focus, window, |this, window, cx| {
                this.tree.update(cx, |t, cx| {
                    if t.selected_index().is_none() && t.entry(0).is_some() {
                        t.set_selected_index(Some(0), cx);
                    }
                    t.focus(window, cx)
                });
                this.settle_selection(cx);
            }),
            cx.subscribe_in(&filter, window, Self::filter_event),
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.forget_closed(cx);
                this.rebuild(window, cx)
            }),
            cx.subscribe(&tree, Self::tree_event),
            cx.observe(&tree, |this, _, cx| {
                this.sync_multi_focus(cx);
                this.follow_selection(cx);
            }),
            cx.subscribe_in(&workspace, window, Self::workspace_event),
        ];
        subscriptions.extend(ui::redraw_on_settings(cx));
        let mut sidebar = Sidebar {
            renders: 0,
            seen_selection: None,
            filter_cramped: Rc::default(),
            workspace,
            tree,
            tree_focus,
            button_focus: Vec::new(),
            expanded: Expanded::new(),
            restored: HashSet::new(),
            filter,
            filter_text: String::new(),
            deletes: Vec::new(),
            next_delete: 0,
            drawn: true,
            nodes: Vec::new(),
            rows: Rc::default(),
            git: Rc::default(),
            rollup: Rc::default(),
            rename: None,
            lost_selection: None,
            moved_selection: None,
            handed_off: None,
            prior_example: None,
            followed: None,
            pending_focus: None,
            hovered: None,
            menu_target: None,
            key_menu: None,
            multi: Multi::default(),
            batches: Vec::new(),
            clipboard: None,
            cuts: HashMap::new(),
            moves: HashMap::new(),
            lost_multi: Vec::new(),
            live_keys: HashSet::new(),
            pending: HashMap::new(),
            hover: None,
            tag_paints: ui::TagPaints::default(),
            _subscriptions: subscriptions,
        };
        sidebar.rebuild(window, cx);
        sidebar
    }

    /// The tree state holding every collection's rows.
    pub fn tree(&self) -> &Entity<TreeState> {
        &self.tree
    }

    /// The colour the row with tree id `id` painted its method tag in the last render;
    /// `None` when that row drew no recorded tag.
    pub fn painted_tag_color(&self, id: &SharedString) -> Option<Hsla> {
        self.tag_paints.borrow().get(id).copied()
    }

    /// Whether the row with tree id `id` draws expanded (chevron down, open folder);
    /// `None` when there is no such row.
    pub fn row_expanded(&self, id: &SharedString) -> Option<bool> {
        self.rows.get(id).map(|n| n.expanded)
    }

    /// How many times this view has rendered; tests read it to check what a change redraws.
    #[doc(hidden)]
    pub fn renders(&self) -> usize {
        self.renders
    }

    /// The filter input above the tree.
    pub fn filter(&self) -> &Entity<InputState> {
        &self.filter
    }

    /// The selected row's collection root and item path; collection rows have the root path.
    /// An invalid row or the Unsaved group, which shares the root path, gives `None`.
    pub fn selected(&self, cx: &App) -> Option<(PathBuf, ItemPath)> {
        let item = self.tree.read(cx).selected_item()?;
        let node = self.rows.get(&item.id)?;
        (!matches!(
            node.kind,
            NodeKind::Invalid | NodeKind::UnsavedGroup | NodeKind::Empty
        ))
        .then(|| (node.root.clone(), node.path.clone()))
    }

    /// The multi-selection's members, in set order.
    pub fn multi_keys(&self) -> Vec<(PathBuf, ItemPath)> {
        self.multi
            .keys()
            .iter()
            .map(|k| (k.root.clone(), k.path.clone()))
            .collect()
    }

    /// Tracks `batch` as an in-flight multi-item command group.
    pub fn push_batch(&mut self, batch: Batch) {
        self.batches.push(batch);
    }

    /// The roots of the batches still tracked, in the order they were sent.
    pub fn batch_roots(&self) -> Vec<PathBuf> {
        self.batches
            .iter()
            .map(|b| b.root().to_path_buf())
            .collect()
    }

    /// Whether two or more rows are selected together.
    pub fn multi_active(&self) -> bool {
        self.multi.is_active()
    }

    /// Whether `RunSelected` has something to run: several rows, or one folder or collection row.
    pub fn run_selected_available(&self, cx: &App) -> bool {
        self.multi.is_active()
            || self.selected_node(cx).is_some_and(|n| {
                let entries = menu_entries(n.kind);
                entries.contains(&MenuEntry::RunFolder)
                    || entries.contains(&MenuEntry::RunCollection)
            })
    }

    /// The name of the one selected row, as the tree shows it.
    pub fn selected_name(&self, cx: &App) -> Option<SharedString> {
        let node = self.selected_node(cx).filter(|_| !self.multi.is_active())?;
        Some(node.name.clone().into())
    }

    /// Whether the menu the keyboard opened is open.
    pub fn key_menu_open(&self) -> bool {
        self.key_menu.is_some()
    }

    /// Whether the tree itself holds focus, not a button on one of its rows.
    pub fn tree_holds_focus(&self, window: &Window, cx: &App) -> bool {
        let in_menu = (self.key_menu.as_ref())
            .is_some_and(|k| k.menu.focus_handle(cx).contains_focused(window, cx));
        !in_menu
            && self.tree_focus.contains_focused(window, cx)
            && !self
                .button_focus
                .iter()
                .any(|f| f.contains_focused(window, cx))
    }

    /// Whether exactly one row is selected and its menu offers `entry`; the same test the
    /// keyboard commands apply before acting.
    pub fn selection_offers(&self, entry: MenuEntry, cx: &App) -> bool {
        !self.multi.is_active()
            && self
                .selected_node(cx)
                .is_some_and(|n| menu_entries(n.kind).contains(&entry))
    }

    // A request row's kind, as its collection's snapshot holds it.
    fn request_kind(&self, node: &Node, cx: &App) -> Option<RequestKind> {
        let ws = self.workspace.read(cx);
        ws.request(&node.root, &node.path, cx).map(|v| v.kind)
    }

    fn key_of(&self, id: &SharedString) -> Option<SelKey> {
        SelKey::of(self.rows.get(id)?)
    }

    fn focus_key(&self, cx: &App) -> Option<SelKey> {
        SelKey::of(self.selected_node(cx)?)
    }

    fn id_of(&self, key: &SelKey) -> Option<SharedString> {
        self.find(&key.root, &key.path).map(|n| n.id.clone())
    }

    fn clear_multi(&mut self, cx: &mut Context<Self>) {
        if !self.multi.is_empty() {
            self.multi.clear();
            cx.notify();
        }
    }

    // A kit selection that lands outside the set (a plain click, Up or Down, a reveal) ends it.
    fn sync_multi_focus(&mut self, cx: &mut Context<Self>) {
        if self.multi.is_empty() {
            return;
        }
        let Some(item) = self.tree.read(cx).selected_item() else {
            return;
        };
        let id = item.id.clone();
        if !self.key_of(&id).is_some_and(|k| self.multi.contains(&k)) {
            self.clear_multi(cx);
        }
    }

    // Takes a selection the sidebar made itself as seen, so only a user's move of the
    // selection changes the current collection.
    fn settle_selection(&mut self, cx: &App) {
        self.seen_selection = self.tree.read(cx).selected_item().map(|i| i.id.clone());
    }

    // Selecting a row in another collection makes that collection current.
    fn follow_selection(&mut self, cx: &mut Context<Self>) {
        let id = self.tree.read(cx).selected_item().map(|i| i.id.clone());
        if id == self.seen_selection {
            return;
        }
        self.seen_selection = id;
        if let Some((root, _)) = self.selected(cx) {
            self.make_current(&root, cx);
        }
    }

    fn make_current(&self, root: &Path, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |w, cx| w.set_current_root(root, cx));
    }

    // Escape empties the set; with none, it passes on so a rename or dialog still ends.
    fn clear_selection(&mut self, _: &ClearSelection, _: &mut Window, cx: &mut Context<Self>) {
        if self.multi.is_empty() {
            return cx.propagate();
        }
        self.clear_multi(cx);
    }

    fn extend_up(&mut self, _: &ExtendUp, window: &mut Window, cx: &mut Context<Self>) {
        self.extend(false, window, cx);
    }

    fn extend_down(&mut self, _: &ExtendDown, window: &mut Window, cx: &mut Context<Self>) {
        self.extend(true, window, cx);
    }

    // Shift+arrow grows the set from the anchor and moves focus to the row it reached.
    fn extend(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(focus) = self.focus_key(cx) else {
            return;
        };
        let order = visible_keys(&self.nodes);
        if let Some(target) = self
            .multi
            .extend(down, &focus, &order)
            .and_then(|k| self.id_of(&k))
        {
            self.focus_row(&target, window, cx);
        }
        cx.notify();
    }

    // A plain click starts over from the row it selects.
    fn plain_click(&mut self, cx: &mut Context<Self>) {
        self.clear_multi(cx);
    }

    /// Secondary-click (`secondary`) toggles a row, shift-click selects the range to it;
    /// neither opens it.
    pub fn modified_click(
        &mut self,
        id: &SharedString,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.key_of(id) else {
            return;
        };
        let focus = self.focus_key(cx);
        let order = visible_keys(&self.nodes);
        let next = if secondary {
            self.multi.toggle(key, focus.as_ref(), &order)
        } else {
            Some(self.multi.select_range(key, focus.as_ref(), &order))
        };
        self.tree.update(cx, |t, cx| t.focus(window, cx));
        if let Some(target) = next.and_then(|k| self.id_of(&k)) {
            self.focus_row(&target, window, cx);
        }
        cx.notify();
    }

    /// Moves focus to the filter input and selects its text.
    pub fn focus_filter(&mut self, _: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all(window, cx);
        })
    }

    /// Escape in the sidebar: empties typed text and keeps focus where it is; an empty filter
    /// hands focus to the tree, or passes the key on when the tree already has it.
    pub fn clear_filter(&mut self, _: &ClearFilter, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter.read(cx).value().is_empty() {
            if self.tree_focus.contains_focused(window, cx) {
                return cx.propagate();
            }
            window.focus(&self.tree_focus, cx);
            return;
        }
        self.filter.update(cx, |i, cx| i.set_value("", window, cx));
        self.filter_text.clear();
        self.rebuild(window, cx);
    }

    /// Down in the filter: focuses the tree on its first shown row, so a filtered list is
    /// walked from the top; with no row shown it does nothing.
    pub fn enter_tree(&mut self, _: &EnterTree, window: &mut Window, cx: &mut Context<Self>) {
        let first = self.tree.read(cx).entry(0).map(|e| e.item().id.clone());
        if let Some(id) = first {
            self.focus_row(&id, window, cx);
        }
    }

    /// Starts renaming the selected row, if any, unless a rename is already under way.
    pub fn rename_selected(
        &mut self,
        _: &RenameSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // F2 inside the rename input reaches here through the tree; a rename already under
        // way keeps its typed name.
        if self.rename.is_some() || self.multi.is_active() {
            return;
        }
        if let Some(node) = self.selected_node(cx).cloned() {
            self.begin_rename(&node, window, cx);
        }
    }

    /// Clones the selected row when its menu offers Duplicate.
    pub fn clone_selected(
        &mut self,
        _: &CloneSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.multi.is_active() {
            return;
        }
        if let Some(node) = self.selected_node(cx).cloned()
            && menu_entries(node.kind).contains(&MenuEntry::CloneItem)
        {
            self.choose(&node, MenuEntry::CloneItem, window, cx);
        }
    }

    /// Runs the Show in file manager entry on the selected row.
    pub fn show_in_file_manager(
        &mut self,
        _: &ShowInFileManager,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_selected(MenuEntry::ShowInFileManager, window, cx);
    }

    /// Runs the Open in terminal entry on the selected row.
    pub fn open_in_terminal(
        &mut self,
        _: &OpenInTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_selected(MenuEntry::OpenInTerminal, window, cx);
    }

    /// Opens the selected row's context menu below the row, its first entry highlighted, the
    /// keyboard's way to the menu a right click opens. Escape closes it and the tree takes
    /// focus back.
    pub fn open_row_menu(&mut self, _: &OpenRowMenu, window: &mut Window, cx: &mut Context<Self>) {
        // A rename's input sits in the tree, so its keys reach here; they stay typing.
        if self.rename.is_some() {
            return;
        }
        // A second press closes the menu, as a second right click does.
        if let Some(k) = &self.key_menu {
            let menu = k.menu.focus_handle(cx);
            window.defer(cx, move |window, cx| {
                menu.dispatch_action(&Cancel, window, cx)
            });
            return;
        }
        let Some(row) = self.tree.read(cx).selected_item().map(|i| i.id.clone()) else {
            return;
        };
        // The menu reads the sidebar while it is built, so it is built once this update ends.
        let view = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            let menu = PopupMenu::build(window, cx, |menu, window, cx| {
                let tree = view.clone();
                // The tree's handle is private, so the menu restores focus to it through the
                // sidebar once it closes, unless an entry moved focus elsewhere.
                menu_focus::watch(
                    cx,
                    window,
                    |_| menu_focus::Before::Nothing,
                    move |window, cx| {
                        tree.update(cx, |s, cx| {
                            if s.rename.is_none() {
                                s.tree.update(cx, |t, cx| t.focus(window, cx))
                            }
                        })
                        .ok();
                    },
                );
                fill_row_menu(&view, &row, menu, window, cx)
            });
            if menu.read(cx).is_empty() {
                return;
            }
            view.update(cx, |s, cx| s.show_key_menu(row, menu, window, cx))
                .ok();
        });
    }

    // The keyboard's menu is drawn on its row, so it closes when that row is gone, no longer
    // selected or being renamed, or when focus has left it. Focus it held goes to the tree.
    fn drop_stale_key_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(k) = self.key_menu.as_ref().filter(|k| !k.fresh) else {
            return;
        };
        let selected = self.tree.read(cx).selected_item().map(|i| i.id.clone());
        let held = k.menu.focus_handle(cx).contains_focused(window, cx);
        let stale = self.rename.is_some()
            || !self.rows.contains_key(&k.row)
            || selected.as_ref() != Some(&k.row)
            || !held;
        if !stale {
            return;
        }
        self.key_menu = None;
        if held && self.rename.is_none() {
            let tree = self.tree.clone();
            window.defer(cx, move |window, cx| {
                tree.update(cx, |t, cx| t.focus(window, cx))
            });
        }
    }

    // Shows the keyboard's `menu` at `row` and gives it focus; it forgets itself once closed.
    fn show_key_menu(
        &mut self,
        row: SharedString,
        menu: Entity<PopupMenu>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dismissed = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, _, cx| {
            this.key_menu = None;
            cx.notify();
        });
        menu.focus_handle(cx).focus(window, cx);
        self.key_menu = Some(KeyMenu {
            row,
            menu,
            fresh: true,
            _dismissed: dismissed,
        });
        cx.notify();
    }

    /// Opens the settings tab of the selected folder row.
    pub fn folder_settings(
        &mut self,
        _: &FolderSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_selected(MenuEntry::Settings, window, cx);
    }

    /// Saves the selected draft row's draft, as its Save menu entry does.
    pub fn save_selected_draft(
        &mut self,
        _: &SaveSelectedDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_selected(MenuEntry::SaveDraft, window, cx);
    }

    /// Closes the selected draft row's tab, as its Close menu entry does.
    pub fn close_selected_draft(
        &mut self,
        _: &CloseSelectedDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_selected(MenuEntry::CloseDraft, window, cx);
    }

    /// The kind of the one request row selected while the tree holds focus: the row the request
    /// commands then act on in place of the active tab.
    pub fn tree_request_kind(&self, window: &Window, cx: &App) -> Option<RequestKind> {
        if self.multi.is_active() || !self.tree_holds_focus(window, cx) {
            return None;
        }
        let node = self
            .selected_node(cx)
            .filter(|n| n.kind == NodeKind::Request)?;
        self.request_kind(node, cx)
    }

    // With the tree focused on one request row, `entry` acts on that row; anywhere else the
    // command passes on to the active tab.
    fn on_tree_request(&mut self, entry: MenuEntry, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.tree_request_kind(window, cx) else {
            return cx.propagate();
        };
        let Some(node) = self.selected_node(cx).cloned() else {
            return cx.propagate();
        };
        // A conversion to the kind the request already has does nothing.
        let converts = match entry {
            MenuEntry::ConvertToGraphql => kind == RequestKind::Http,
            MenuEntry::ConvertToHttp => kind == RequestKind::Graphql,
            _ => true,
        };
        if !converts {
            return;
        }
        self.choose(&node, entry, window, cx);
    }

    // Runs `entry` on the single selected row when its menu offers it.
    fn choose_selected(&mut self, entry: MenuEntry, window: &mut Window, cx: &mut Context<Self>) {
        if self.multi.is_active() {
            return;
        }
        if let Some(node) = self.selected_node(cx).cloned()
            && menu_entries(node.kind).contains(&entry)
        {
            self.choose(&node, entry, window, cx);
        }
    }

    /// Runs the Delete menu entry on the selected row; a collection row is never deleted.
    pub fn delete_selected(
        &mut self,
        _: &DeleteSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.multi.is_active() {
            self.confirm_delete_set(window, cx);
            return;
        }
        if let Some(node) = self.selected_node(cx).cloned()
            && menu_entries(node.kind).contains(&MenuEntry::Delete)
        {
            self.choose(&node, MenuEntry::Delete, window, cx);
        }
    }

    /// Runs the active multi-selection as one runner tab. With a single row selected it runs
    /// that folder or collection, as its run button does, and ignores any other row.
    pub fn run_selected(&mut self, _: &RunSelected, window: &mut Window, cx: &mut Context<Self>) {
        if !self.multi.is_active() {
            if let Some(node) = self.selected_node(cx).cloned() {
                for entry in [MenuEntry::RunFolder, MenuEntry::RunCollection] {
                    if menu_entries(node.kind).contains(&entry) {
                        self.choose(&node, entry, window, cx);
                    }
                }
            }
            return;
        }
        let Some((root, items, _)) = self.run_plan(cx) else {
            return;
        };
        if !items.is_empty() {
            cx.emit(SidebarEvent::RunSelection { root, items });
        }
    }

    // The root, the items a run would plan and how many requests they hold, or None without a
    // snapshot. The items are normalised and stripped of opaque rows.
    fn run_plan(&self, cx: &App) -> Option<(PathBuf, Vec<ItemPath>, usize)> {
        let root = self.multi.keys().first().map(|k| k.root.clone())?;
        let snapshot = self.workspace.read(cx).snapshot(&root, cx)?;
        let rows = &snapshot.content.rows;
        let top = self.multi.normalised(&row_keys(&root, rows));
        let items = multi::run_items(rows, &top);
        let keys: Vec<SelKey> = items
            .iter()
            .map(|p| SelKey::new(root.clone(), p.clone()))
            .collect();
        let count = multi::run_count(rows, &keys);
        Some((root, items, count))
    }

    /// Closes every folder of every open collection; the collections themselves stay open.
    pub fn collapse_all(&mut self, _: &CollapseAll, window: &mut Window, cx: &mut Context<Self>) {
        self.set_folders_open(None, false, window, cx);
    }

    /// Opens every folder of every open collection.
    pub fn expand_all(&mut self, _: &ExpandAll, window: &mut Window, cx: &mut Context<Self>) {
        self.set_folders_open(None, true, window, cx);
    }

    // Adds the folders the session saved for `root` once its snapshot is in, keeping only
    // those the loaded tree still has; a renamed or deleted folder drops out silently.
    fn restore_expanded(&mut self, root: &Path, snap: &CollectionSnapshot, cx: &App) {
        if !self.restored.insert(root.to_path_buf()) {
            return;
        }
        let workspace = self.workspace.read(cx);
        let session = workspace.session().read(cx).get();
        let Some(saved) = session.collections.get(root) else {
            return;
        };
        // A saved list replaces the default seed (the open collection row), so a row collapsed
        // by hand stays collapsed; no saved list keeps the seed.
        let Some(saved) = &saved.expanded else {
            return;
        };
        let live: HashSet<&ItemPath> = snap.content.rows.iter().map(|r| &r.path).collect();
        let set: HashSet<ItemPath> = saved
            .iter()
            .filter_map(|p| ItemPath::new(p))
            .filter(|p| p.is_root() || live.contains(p))
            .collect();
        self.expanded.insert(root.to_path_buf(), set);
    }

    // Writes each restored root's expanded set into the session when it differs from the
    // stored one. The session store debounces, so a burst of clicks costs one file write.
    fn save_expanded(&mut self, cx: &mut Context<Self>) {
        let session = self.workspace.read(cx).session().clone();
        let changed: Vec<(PathBuf, Vec<String>)> = {
            let stored = session.read(cx).get();
            self.restored
                .iter()
                .filter(|root| root.to_str().is_some())
                .filter_map(|root| {
                    let mut now: Vec<String> = self
                        .expanded
                        .get(root)?
                        .iter()
                        .map(|p| p.as_str().to_owned())
                        .collect();
                    now.sort();
                    let before = stored
                        .collections
                        .get(root)
                        .and_then(|c| c.expanded.as_ref());
                    (before.map_or(&[][..], |v| &v[..]) != &now[..]).then(|| (root.clone(), now))
                })
                .collect()
        };
        if changed.is_empty() {
            return;
        }
        session.update(cx, |p, cx| {
            p.update(cx, |s| {
                for (root, now) in changed {
                    s.collections.entry(root).or_default().expanded = Some(now);
                }
            })
        });
    }

    /// The folders a collection keeps open, its own root path included.
    pub fn expanded_paths(&self, root: &Path) -> Vec<ItemPath> {
        self.expanded
            .get(root)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    // Writes the kept set of `only` (or of every collection) into the same `expanded` map the
    // chevron path writes, then rebuilds. Folders come from the snapshot rows, not the built
    // nodes, so an active filter that hides some of them does not shrink Expand all.
    fn set_folders_open(
        &mut self,
        only: Option<&Path>,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let roots: Vec<PathBuf> = self.nodes.iter().map(|n| n.root.clone()).collect();
        for root in roots {
            if only.is_some_and(|only| only != root.as_path()) {
                continue;
            }
            // The collection row stays open either way, so Collapse all leaves its rows visible.
            let mut set: HashSet<ItemPath> = [ItemPath::root()].into();
            if open && let Some(snapshot) = self.workspace.read(cx).snapshot(&root, cx) {
                let rows = &snapshot.content.rows;
                set.extend(
                    rows.iter()
                        .filter(|r| r.kind == crate::snapshot::RowKind::Folder)
                        .map(|r| r.path.clone()),
                );
            }
            self.expanded.insert(root, set);
        }
        self.rebuild(window, cx);
    }

    /// Creates a folder where `new_request` would put a draft, then renames it inline.
    pub fn new_folder(&mut self, _: &NewFolder, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.read(cx).current_root() else {
            return;
        };
        let parent = self.selection_parent(&root, cx);
        let op = TreeOp::CreateFolder {
            parent,
            name: NEW_FOLDER_NAME.into(),
        };
        self.send_tracked(&root, op, true, None, window, cx);
    }

    /// Opens a draft in the workspace's current collection, under the selected folder or the
    /// selected request's folder when the selection is in that collection, else at its root.
    pub fn new_request(&mut self, _: &NewRequest, _: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.read(cx).current_root() else {
            self.workspace.update(cx, |w, cx| w.new_scratch_request(cx));
            return;
        };
        let parent = self.selection_parent(&root, cx);
        self.workspace
            .update(cx, |w, cx| w.new_draft(&root, parent, cx));
    }

    /// `new_request` for a GraphQL draft.
    pub fn new_graphql_request(
        &mut self,
        _: &NewGraphqlRequest,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.workspace.read(cx).current_root() else {
            self.workspace.update(cx, |w, cx| {
                let root = w.open_scratch(cx)?;
                w.new_draft_of_kind(&root, ItemPath::root(), RequestKind::Graphql, cx)
            });
            return;
        };
        let parent = self.selection_parent(&root, cx);
        self.workspace.update(cx, |w, cx| {
            w.new_draft_of_kind(&root, parent, RequestKind::Graphql, cx)
        });
    }

    /// Sorts the items of the folder `selection_parent` names, as the menu entry does.
    pub fn sort_by_name(&mut self, _: &SortByName, _: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.read(cx).current_root() else {
            return;
        };
        let folder = self.selection_parent(&root, cx);
        self.send_sort(&root, folder, cx);
    }

    // The core renumbers the whole folder in one checked batch, or touches nothing.
    fn send_sort(&mut self, root: &Path, folder: ItemPath, cx: &mut Context<Self>) {
        let op = TreeOp::SortByName { folder };
        self.workspace
            .update(cx, |w, cx| w.send(root, Command::Tree(op), cx));
    }

    // The folder a new item goes in: the selected folder or the one an Empty row stands in,
    // the selected request's or example's folder, or the collection root otherwise.
    fn selection_parent(&self, root: &Path, cx: &App) -> ItemPath {
        let node = self
            .tree
            .read(cx)
            .selected_item()
            .and_then(|i| self.rows.get(&i.id));
        node.filter(|n| n.root == root)
            .and_then(|n| match n.kind {
                NodeKind::Folder | NodeKind::Empty => Some(n.path.clone()),
                NodeKind::Request | NodeKind::Example => n.path.parent(),
                _ => None,
            })
            .unwrap_or_else(ItemPath::root)
    }

    /// Folders and requests only; any other row is ignored. Focuses an input holding the
    /// name, all selected.
    pub fn start_rename(
        &mut self,
        root: &Path,
        path: &ItemPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(node) = self.find(root, path).cloned() {
            self.begin_rename(&node, window, cx);
        }
    }

    // Renames `node` itself, so an example row brings its own index and name rather than
    // its request's, with which it shares a path.
    fn begin_rename(&mut self, node: &Node, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(
            node.kind,
            NodeKind::Folder | NodeKind::Request | NodeKind::Example
        ) {
            return;
        }
        let old = node.name.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(old.clone()));
        input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all(window, cx);
        });
        self.rename = Some(Rename {
            root: node.root.clone(),
            path: node.path.clone(),
            old,
            example: node.example,
            fresh: false,
            error: None,
            _subscription: cx.subscribe_in(&input, window, Self::rename_event),
            input,
        });
        // The rename input takes focus, so its row must be on screen: a new folder can land
        // anywhere in a tree taller than the sidebar.
        let id = node.id.clone();
        self.tree.update(cx, |t, _| {
            if let Some(ix) = t.index_of(&id) {
                t.scroll_to_item(ix, ScrollStrategy::Nearest);
            }
        });
        cx.notify();
    }

    /// The row a drag is over and the zone of it, if any.
    pub fn hover(&self) -> Option<(SharedString, DropZone)> {
        self.hover.clone()
    }

    // Records the zone under the pointer while it is inside the target's row, and forgets this
    // row's zone once the pointer leaves it, so a drag released elsewhere leaves no indicator.
    // A set payload shows an indicator only where `multi_drop` would accept it.
    fn drag_over(&mut self, target: &Node, e: &DragMoveEvent<DraggedRow>, cx: &mut Context<Self>) {
        let id = &target.id;
        let over = e.bounds.contains(&e.event.position);
        let zone = match target.kind {
            NodeKind::Empty => DropZone::Inside,
            _ => drop_zone(e.event.position.y, e.bounds),
        };
        let dragged = e.drag(cx);
        let allowed = dragged.also.is_empty()
            || (dragged.root == target.root
                && multi_drop(&dragged.items(), &target.path, target.kind, zone).is_some());
        let hover = if over && allowed {
            Some((id.clone(), zone))
        } else if self.hover.as_ref().is_some_and(|(h, _)| h == id) {
            None
        } else {
            return;
        };
        if hover != self.hover {
            self.hover = hover;
            cx.notify();
        }
    }

    // Moves the dragged item, or every dragged item of a set in sidebar order, to the zone last
    // hovered on `target`; a drop from another collection, or one that the drop rule refuses,
    // sends nothing. A set's moves are tracked as one batch with a single summary.
    fn drop_on(&mut self, target: &Node, dragged: &DraggedRow, cx: &mut Context<Self>) {
        let Some((id, zone)) = self.hover.take() else {
            return;
        };
        cx.notify();
        if id != target.id || dragged.root != target.root {
            return;
        }
        if !dragged.also.is_empty() {
            self.drop_set(target, dragged, zone, cx);
            return;
        }
        if let Some(to) = drop_target(&dragged.path, &target.path, target.kind, zone) {
            let op = TreeOp::Move {
                item: dragged.path.clone(),
                to,
            };
            self.workspace
                .update(cx, |w, cx| w.send(&target.root, Command::Tree(op), cx));
        }
    }

    fn drop_set(
        &mut self,
        target: &Node,
        dragged: &DraggedRow,
        zone: DropZone,
        cx: &mut Context<Self>,
    ) {
        let root = target.root.clone();
        let Some(snapshot) = self.workspace.read(cx).snapshot(&root, cx) else {
            return;
        };
        let mut items = dragged.items();
        let order: std::collections::HashMap<ItemPath, usize> = snapshot
            .content
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.path.clone(), i))
            .collect();
        items.sort_by_key(|p| order.get(p).copied().unwrap_or(usize::MAX));
        let Some(moving) = multi_drop(&items, &target.path, target.kind, zone) else {
            return;
        };
        let dir = if target.kind == NodeKind::Collection {
            ItemPath::root()
        } else {
            target.path.clone()
        };
        let sent: Vec<(u64, ItemPath)> = self.workspace.update(cx, |w, cx| {
            moving
                .into_iter()
                .filter_map(|item| {
                    let op = TreeOp::Move {
                        item: item.clone(),
                        to: DropTarget::Inside(dir.clone()),
                    };
                    Some((w.send(&root, Command::Tree(op), cx)?, item))
                })
                .collect()
        });
        if sent.is_empty() {
            return;
        }
        let ids: Vec<u64> = sent.iter().map(|(id, _)| *id).collect();
        self.workspace.update(cx, |w, _| w.track_batch(&root, &ids));
        let generation = self.generation(&root, cx).unwrap_or(u64::MAX);
        (self.batches).push(Batch::new(Verb::Move, root, generation, sent));
    }

    /// The collection root and item path being renamed, if any.
    pub fn renaming(&self) -> Option<(PathBuf, ItemPath)> {
        self.rename
            .as_ref()
            .map(|r| (r.root.clone(), r.path.clone()))
    }

    /// Why the typed name was refused, while the rename shows it.
    pub fn rename_error(&self) -> Option<&str> {
        Some(self.rename.as_ref()?.error.as_ref()?.1.as_str())
    }

    // The refusal for a name a sibling of the same kind already shows, which would leave two
    // rows alike. Folders clash only with folders, request files with request files.
    fn name_clash(&self, r: &Rename, cx: &App) -> Option<String> {
        let name = r.input.read(cx).value().trim().to_string();
        if r.example.is_some() || name.is_empty() || name == r.old {
            return None;
        }
        let snapshot = self.workspace.read(cx).snapshot(&r.root, cx)?;
        let rows = &snapshot.content.rows;
        let folder = rows.iter().find(|row| row.path == r.path)?.kind == RowKind::Folder;
        let parent = r.path.parent()?;
        rows.iter().find(|row| {
            row.path != r.path
                && row.path.parent().as_ref() == Some(&parent)
                && (row.kind == RowKind::Folder) == folder
                && row.name == name
        })?;
        let place = if parent.is_root() {
            let collection = self.nodes.iter().find(|n| n.root == r.root)?;
            collection.name.clone()
        } else {
            rows.iter().find(|row| row.path == parent)?.name.clone()
        };
        let what = if folder { "folder" } else { "request" };
        Some(format!(
            "A {what} named \"{name}\" is already in \"{place}\""
        ))
    }

    // Enter and clicking away both commit a real change and end the rename; only Enter hands
    // focus back to the tree. Escape ends it through the sidebar's own action handler, since
    // the input lets an idle Escape propagate.
    fn rename_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // A taken name keeps the field open with its refusal, so one keystroke fixes it.
            InputEvent::PressEnter { .. } => {
                let clash = (self.rename.as_ref()).and_then(|r| self.name_clash(r, cx));
                if let Some(r) = &mut self.rename
                    && let Some(error) = clash
                {
                    r.error = Some((r.input.read(cx).value(), error));
                    cx.notify();
                    return;
                }
                if let Some(r) = self.rename.take() {
                    self.commit_rename(r, false, cx);
                }
                self.end_rename(window, cx);
            }
            // Switching to another window keeps the field focused, so the rename goes on. A
            // field left behind cannot stay open, so a taken name is dropped with a toast.
            InputEvent::Blur if window.is_window_active() => {
                if let Some(r) = self.rename.take() {
                    match self.name_clash(&r, cx) {
                        Some(error) => {
                            let outcome = if r.fresh {
                                "the folder was not created"
                            } else {
                                "the name was not changed"
                            };
                            self.discard_fresh(r, cx);
                            let message = format!("{error}, so {outcome}.");
                            window.push_notification(Notification::error(message), cx);
                        }
                        None => self.commit_rename(r, true, cx),
                    }
                }
                cx.notify();
            }
            // A driver's Enter echoes its text into the input, which changes nothing; only
            // a real edit answers the refusal.
            InputEvent::Change => {
                if let Some(r) = &mut self.rename
                    && r.error
                        .take_if(|(refused, _)| *refused != r.input.read(cx).value())
                        .is_some()
                {
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    // Sends the typed name unless it is blank or unchanged; a name core refuses shows its
    // error as any failed tree op does. On blur, a just-created folder left unnamed goes again.
    fn commit_rename(&mut self, r: Rename, blur: bool, cx: &mut Context<Self>) {
        let name = r.input.read(cx).value().trim().to_string();
        if name == r.old {
            if blur {
                self.discard_fresh(r, cx);
            }
            return;
        }
        if name.is_empty() {
            return;
        }
        let op = match r.example {
            Some(index) => TreeOp::RenameExample {
                item: r.path,
                index,
                name,
            },
            None => TreeOp::Rename {
                item: r.path,
                name,
                rename_file: true,
            },
        };
        self.workspace
            .update(cx, |w, cx| w.send(&r.root, Command::Tree(op), cx));
    }

    // Escape cancels the rename, and with it the naming of a folder this view just created.
    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = self.rename.take() {
            self.discard_fresh(r, cx);
        }
        self.end_rename(window, cx);
    }

    // Removes a just-created folder whose naming was dropped. Core keeps it if anything is
    // inside, and a failure shows as any failed tree op does.
    fn discard_fresh(&mut self, r: Rename, cx: &mut Context<Self>) {
        if r.fresh {
            let op = TreeOp::DeleteEmptyFolder { item: r.path };
            self.workspace
                .update(cx, |w, cx| w.send(&r.root, Command::Tree(op), cx));
        }
    }

    // Drops the rename and hands focus back to the tree it started from.
    fn end_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rename = None;
        self.tree.update(cx, |t, cx| t.focus(window, cx));
        cx.notify();
    }

    // Keeps view state attached to items that moved, then rebuilds. The workspace notifies
    // before it emits notices, so an example delete sees the selection from before the rebuild.
    fn workspace_event(
        &mut self,
        _: &Entity<Workspace>,
        event: &WorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Events name the instance that sent them: by the time one arrives, the root may be
        // open again as another instance whose command ids start over.
        let (root, notices, generation) = match event {
            WorkspaceEvent::Notices {
                root,
                notices,
                generation,
            } => (root, notices, *generation),
            // Every notice up to `acked` is in, so a command still tracked there never answers.
            WorkspaceEvent::Delivered {
                root,
                acked,
                generation,
            } => {
                // A delivery closes the update a hand-off was made in, so one with no Moved
                // notice, such as a delete on disk, never gives the selection back.
                self.handed_off.take_if(|(wanted, _)| wanted.0 == *root);
                return self.forget_tracked(|r, g, id| {
                    r == root.as_path() && g == *generation && id <= *acked
                });
            }
            _ => return,
        };
        self.record_batches(root, generation, notices, window, cx);
        // A move is reported only once it is on disk, so a closed instance's move still holds
        // for the reopened tree; its example deletes shift rows that tree no longer shows.
        let stale = self.generation(root, cx) != Some(generation);
        let mut followed = false;
        for notice in notices {
            let (from, to) = match &notice.kind {
                NoticeKind::Moved { from, to } => (from, to),
                // Only a create or clone this view sent focuses its new row, never another's.
                NoticeKind::Created(path) => {
                    let key = (root.clone(), generation, notice.command);
                    if let Some(PendingRow { rename, notice }) = self.pending.remove(&key) {
                        if let Some(notice) = notice {
                            window.push_notification(ui::info_toast(notice), cx);
                        }
                        if let Some(parent) = path.parent() {
                            self.expanded
                                .entry(root.clone())
                                .or_default()
                                .insert(parent);
                        }
                        self.pending_focus = Some(PendingFocus {
                            root: root.clone(),
                            path: path.clone(),
                            rename,
                        });
                    }
                    continue;
                }
                NoticeKind::Failed { .. } => {
                    self.pending
                        .remove(&(root.clone(), generation, notice.command));
                    continue;
                }
                NoticeKind::ExampleDeleted { item, index } => {
                    if !stale {
                        self.example_deleted(root, item, *index, window, cx);
                    }
                    continue;
                }
                _ => continue,
            };
            let moved = |at: &Path, path: &ItemPath| {
                (at == root.as_path())
                    .then(|| rebase(path, from, to))
                    .flatten()
            };
            // A neighbour still holding a selection this move explains gives it back.
            let current = self.selected(cx);
            let handed = self
                .handed_off
                .take_if(|(wanted, near)| {
                    current.as_ref() == Some(near) && moved(&wanted.0, &wanted.1).is_some()
                })
                .map(|(wanted, _)| wanted);
            let selection = self
                .moved_selection
                .take()
                .or(handed)
                .or(current)
                .or_else(|| self.lost_selection.take());
            self.moved_selection = selection.map(|(r, p)| match moved(&r, &p) {
                Some(p) => {
                    followed = true;
                    (r, p)
                }
                None => (r, p),
            });
            self.multi.rebase(root, from, to);
            for key in &mut self.lost_multi {
                if key.root == *root
                    && let Some(p) = rebase(&key.path, from, to)
                {
                    key.path = p;
                }
            }
            if let Some(r) = &mut self.rename
                && let Some(p) = moved(&r.root, &r.path)
            {
                r.path = p;
            }
            if let Some(f) = &mut self.pending_focus
                && let Some(p) = moved(&f.root, &f.path)
            {
                f.path = p;
            }
            if let Some(set) = self.expanded.get_mut(root) {
                *set = set
                    .drain()
                    .map(|p| rebase(&p, from, to).unwrap_or(p))
                    .collect();
            }
        }
        self.handed_off = None;
        // The folders above the selection's final path open once, so its row exists; a later
        // collapse sticks.
        if followed && let Some((r, path)) = &self.moved_selection {
            let set = self.expanded.entry(r.clone()).or_default();
            let mut parent = path.parent();
            while let Some(p) = parent {
                parent = p.parent();
                set.insert(p);
            }
        }
        // Keys the rebuild before this notice could not find come back when a move explains them.
        let lost = std::mem::take(&mut self.lost_multi);
        self.multi
            .admit(lost.into_iter().filter(|k| self.live_keys.contains(k)));
        self.rebuild(window, cx);
    }

    // Example ids carry their index, so a delete shifts the later examples of its request:
    // a selected one follows its example down, or falls back to the request when it was the
    // one deleted, and an inline rename at or after the index ends. The workspace notifies
    // before it emits notices, so the rebuild for the delete has run and `prior_example` holds
    // the selection from before it.
    fn example_deleted(
        &mut self,
        root: &Path,
        item: &ItemPath,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rename.as_ref().is_some_and(|r| {
            r.root == root && r.path == *item && r.example.is_some_and(|e| e >= index)
        }) {
            self.end_rename(window, cx);
        }
        let Some((_, _, selected)) = self
            .prior_example
            .take()
            .filter(|(r, p, _)| r == root && p == item)
        else {
            return;
        };
        let Some(request) = self.find(root, item).cloned() else {
            return;
        };
        let id = match selected.cmp(&index) {
            Ordering::Less => return,
            Ordering::Equal => request.id,
            Ordering::Greater => model::example_id(request.collection, item, selected - 1),
        };
        self.select(&id, cx);
    }

    /// Runs a context menu entry on `node`. New request opens a draft. New folder and
    /// Duplicate send at once and focus the new row when its `Created` notice arrives; a new
    /// folder also renames it inline.
    fn choose(
        &mut self,
        node: &Node,
        entry: MenuEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (root, path) = (node.root.clone(), node.path.clone());
        match entry {
            MenuEntry::NewRequest => {
                self.workspace
                    .update(cx, |w, cx| w.new_draft(&root, path, cx));
            }
            MenuEntry::NewGraphqlRequest => {
                self.workspace.update(cx, |w, cx| {
                    w.new_draft_of_kind(&root, path, RequestKind::Graphql, cx)
                });
            }
            MenuEntry::SortByName => self.send_sort(&root, path, cx),
            MenuEntry::NewFolder => self.send_tracked(
                &root,
                TreeOp::CreateFolder {
                    parent: path,
                    name: NEW_FOLDER_NAME.into(),
                },
                true,
                None,
                window,
                cx,
            ),
            MenuEntry::CloneItem => {
                // Duplicate copies what the user sees, so once the copy exists say so when that
                // differed from disk.
                let dirty = self.dirty_within(&root, std::iter::once(&path));
                let notice = dirty.then_some(DUPLICATE_UNSAVED);
                let name = clone_name(&node.name);
                let op = TreeOp::Clone { item: path, name };
                self.send_tracked(&root, op, false, notice, window, cx);
            }
            MenuEntry::CopyItem => self.copy_row(node, ClipMode::Copy, cx),
            MenuEntry::CutItem => self.copy_row(node, ClipMode::Cut, cx),
            MenuEntry::PasteItem => {
                if let Some((root, folder)) = paste_target(node) {
                    self.paste(&root, folder, window, cx);
                }
            }
            MenuEntry::CopyAsCurl => {
                crate::copy_curl::copy_as_curl(&self.workspace, &root, path, window, cx)
            }
            MenuEntry::GenerateCode => {
                crate::generate_code::open_generate_code(&self.workspace, &root, path, window, cx)
            }
            MenuEntry::ConvertToGraphql | MenuEntry::ConvertToHttp => {
                let to = if entry == MenuEntry::ConvertToGraphql {
                    RequestKind::Graphql
                } else {
                    RequestKind::Http
                };
                crate::convert_kind::convert(&self.workspace, &root, path, to, window, cx);
            }
            // The popup restores focus after its click handler, so the input is focused later.
            MenuEntry::Rename => {
                let (view, node) = (cx.entity().downgrade(), node.clone());
                window.defer(cx, move |window, cx| {
                    view.update(cx, |s, cx| s.begin_rename(&node, window, cx))
                        .ok();
                });
            }
            MenuEntry::CollectionSettings => {
                cx.emit(SidebarEvent::OpenSettings { root, folder: None });
            }
            MenuEntry::Settings => {
                let folder = Some(path);
                cx.emit(SidebarEvent::OpenSettings { root, folder });
            }
            MenuEntry::RunCollection => cx.emit(SidebarEvent::Run { root, folder: None }),
            MenuEntry::RunFolder => {
                let folder = Some(path);
                cx.emit(SidebarEvent::Run { root, folder });
            }
            MenuEntry::Delete => self.confirm_delete(node, window, cx),
            MenuEntry::ExportPostman => {
                let format = ExportFormat::Postman {
                    keep_scripts: false,
                };
                crate::export::export_collection(&self.workspace, &root, format, window, cx)
            }
            MenuEntry::ExportPostmanKeepScripts => {
                let format = ExportFormat::Postman { keep_scripts: true };
                crate::export::export_collection(&self.workspace, &root, format, window, cx)
            }
            MenuEntry::ExportOpenApi => {
                let format = ExportFormat::OpenApi;
                crate::export::export_collection(&self.workspace, &root, format, window, cx)
            }
            MenuEntry::ExportHtml => {
                let format = ExportFormat::Html;
                crate::export::export_collection(&self.workspace, &root, format, window, cx)
            }
            MenuEntry::CollapseAll => self.set_folders_open(Some(&root), false, window, cx),
            MenuEntry::ExpandAll => self.set_folders_open(Some(&root), true, window, cx),
            MenuEntry::Close => {
                crate::shell::close_collection_at(&self.workspace, root, window, cx);
            }
            MenuEntry::ShowInFileManager | MenuEntry::OpenInTerminal => {
                let Some(dir) = menu::entry_dir(node.kind, &root, &path) else {
                    return;
                };
                let failure = if entry == MenuEntry::ShowInFileManager {
                    (!crate::open_external::open_path(&dir, cx))
                        .then(|| "Could not open the file manager".to_string())
                } else {
                    crate::terminal::open_in(&dir, cx).err()
                };
                if let Some(message) = failure {
                    window.push_notification(Notification::error(message), cx);
                }
            }
            MenuEntry::ShowChanges => {
                let file = PathBuf::from(git_status::item_file(&path, false));
                crate::changes_dialog::open_changes(&self.workspace, &root, Some(file), window, cx);
            }
            MenuEntry::SaveDraft => cx.emit(SidebarEvent::SaveDraft { root, path }),
            MenuEntry::CloseDraft => cx.emit(SidebarEvent::CloseDraft { root, path }),
        }
    }

    // A command whose answer can never arrive must not wait for one: it would only hold
    // memory, since a later command of another instance never shares its key.
    fn forget_tracked(&mut self, gone: impl Fn(&Path, u64, u64) -> bool) {
        self.pending.retain(|(r, g, id), _| !gone(r, *g, *id));
        self.cuts.retain(|(r, g, id), _| !gone(r, *g, *id));
        self.moves.retain(|(r, g, id), _| !gone(r, *g, *id));
    }

    // A closed collection's model is gone with every answer it still owed, even when the same
    // root opened again as a new instance. A cut goes with it: it may only move items from the
    // instance it was taken from.
    fn forget_closed(&mut self, cx: &App) {
        let cut = self
            .clipboard
            .as_ref()
            .filter(|c| c.mode == paste::ClipMode::Cut);
        let idle = self.pending.is_empty() && self.cuts.is_empty() && self.moves.is_empty();
        if idle && cut.is_none() && self.batches.is_empty() && self.deletes.is_empty() {
            return;
        }
        let open: HashSet<(PathBuf, u64)> = (self.workspace.read(cx).collections().iter())
            .map(|c| (c.root().to_path_buf(), c.generation()))
            .collect();
        if cut.is_some_and(|c| !open.contains(&(c.root.clone(), c.generation))) {
            self.clipboard = None;
        }
        // Nor can it send the notices its batches wait for.
        (self.batches).retain(|b| open.contains(&(b.root().to_path_buf(), b.generation())));
        // A closed instance took its trash tickets with it, so there is nothing to forget.
        (self.deletes).retain(|d| open.contains(&(d.root.clone(), d.generation)));
        self.forget_tracked(|r, g, _| !open.contains(&(r.to_path_buf(), g)));
    }

    // The open generation of the collection at `root`, if it is open.
    fn generation(&self, root: &Path, cx: &App) -> Option<u64> {
        (self.workspace.read(cx).collections().iter())
            .find(|c| c.root() == root)
            .map(|c| c.generation())
    }

    // Clears an active filter first: a new row it hid would keep its focus and rename
    // pending until the filter was cleared, then take focus unasked. `notice` shows only once
    // the op succeeds.
    fn send_tracked(
        &mut self,
        root: &Path,
        op: TreeOp,
        rename: bool,
        notice: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self
            .workspace
            .update(cx, |w, cx| w.send(root, Command::Tree(op), cx));
        let (Some(id), Some(generation)) = (id, self.generation(root, cx)) else {
            return;
        };
        let row = PendingRow { rename, notice };
        self.pending
            .insert((root.to_path_buf(), generation, id), row);
        if !self.filter_text.trim().is_empty() {
            self.filter.update(cx, |i, cx| i.set_value("", window, cx));
            self.filter_text.clear();
            self.rebuild(window, cx);
        }
    }

    // Feeds the notices to every batch of `root`, then toasts the summary of each finished one.
    fn record_batches(
        &mut self,
        root: &Path,
        generation: u64,
        notices: &[crate::command::Notice],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_cuts(root, generation, notices, cx);
        self.record_outside_trash(root, generation, notices, window, cx);
        if self.batches.is_empty() {
            return;
        }
        // Only the instance open now has a snapshot to settle against, and only its own
        // batches may be settled by it.
        let current = self.generation(root, cx) == Some(generation);
        let acked = (self.workspace.read(cx).snapshot(root, cx))
            .filter(|_| current)
            .map(|s| s.acked);
        for batch in &mut self.batches {
            for notice in notices {
                batch.record(root, generation, notice);
            }
            if let Some(acked) = acked
                && batch.root() == root
                && batch.generation() == generation
            {
                batch.settle(acked);
            }
        }
        let (done, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.batches)
            .into_iter()
            .partition(Batch::is_finished);
        self.batches = open;
        for notice in done.iter().filter_map(Batch::unsaved_notice) {
            window.push_notification(ui::info_toast(notice), cx);
        }
        for notice in done.iter().filter_map(Batch::restored_message) {
            window.push_notification(ui::info_toast(notice), cx);
        }
        for (kind, summary) in done.iter().filter_map(Batch::summary) {
            window.push_notification(ui::kind_toast(kind, summary), cx);
        }
        for batch in done.iter().filter(|b| !b.tickets().is_empty()) {
            let (root, generation) = (batch.root().to_path_buf(), batch.generation());
            if batch.undo_owed() {
                self.send_restores(&root, generation, batch.trashed(), cx);
                continue;
            }
            let entry = self.push_delete(root, generation, batch.trashed(), cx);
            let sidebar = cx.weak_entity();
            let undo = move |_: &mut Window, cx: &mut App| {
                sidebar.update(cx, |s, cx| s.undo_entry(entry, cx)).ok();
            };
            crate::undo_toast::show_undo_toast(batch.trashed_message(), entry, undo, window, cx);
        }
    }

    // A removal into the trash the sidebar did not send, such as a spec update's, is undone as
    // a delete is: one entry and Undo toast for each command's items.
    fn record_outside_trash(
        &mut self,
        root: &Path,
        generation: u64,
        notices: &[crate::command::Notice],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut by_command: Vec<(u64, Vec<(u64, ItemPath)>)> = Vec::new();
        for notice in notices {
            let NoticeKind::Trashed { item, ticket } = &notice.kind else {
                continue;
            };
            let ours = |b: &Batch| b.tracks(root, generation, notice.command);
            if self.batches.iter().any(ours) {
                continue;
            }
            let entry = (*ticket, item.clone());
            match by_command.iter_mut().find(|(id, _)| *id == notice.command) {
                Some((_, trashed)) => trashed.push(entry),
                None => by_command.push((notice.command, vec![entry])),
            }
        }
        for (_, trashed) in by_command {
            let message = batch::removed_message(&trashed);
            let entry = self.push_delete(root.to_path_buf(), generation, trashed, cx);
            let sidebar = cx.weak_entity();
            let undo = move |_: &mut Window, cx: &mut App| {
                sidebar.update(cx, |s, cx| s.undo_entry(entry, cx)).ok();
            };
            crate::undo_toast::show_undo_toast(message, entry, undo, window, cx);
        }
    }

    // Keeps a finished delete undoable, letting the oldest go past the cap. An evicted
    // entry's tickets are forgotten only by the instance that issued them.
    fn push_delete(
        &mut self,
        root: PathBuf,
        generation: u64,
        trashed: Vec<(u64, ItemPath)>,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.next_delete += 1;
        let id = self.next_delete;
        (self.deletes).push(UndoEntry {
            id,
            root,
            generation,
            trashed,
        });
        while self.deletes.len() > UNDO_DELETE_CAP {
            let old = self.deletes.remove(0);
            if self.generation(&old.root, cx) == Some(old.generation) {
                self.workspace.update(cx, |w, cx| {
                    for (ticket, _) in &old.trashed {
                        let op = TreeOp::ForgetTrash { ticket: *ticket };
                        w.send(&old.root, Command::Tree(op), cx);
                    }
                });
            }
        }
        id
    }

    // The toast's Undo: restores its own entry once; one already undone or evicted is gone.
    fn undo_entry(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(ix) = self.deletes.iter().position(|d| d.id == id) {
            let d = self.deletes.remove(ix);
            self.send_restores(&d.root, d.generation, d.trashed, cx);
        }
    }

    /// Whether `UndoDelete` has a delete to undo: one still landing into the trash, or a
    /// finished one of a collection instance still open.
    pub fn can_undo_delete(&self, cx: &App) -> bool {
        let ws = self.workspace.read(cx);
        let open = |root: &Path, generation: u64| {
            (ws.collections().iter()).any(|c| c.root() == root && c.generation() == generation)
        };
        self.batches.iter().any(|b| b.awaits_undo())
            || (self.deletes.iter()).any(|d| open(&d.root, d.generation))
    }

    /// Ctrl+Z outside any editor or input restores the newest delete; with none left the key
    /// goes on to the shell's undo of the active tab. A delete still landing is the newest and
    /// has no undo ticket yet, so the key claims that delete, undone the moment it lands.
    pub fn undo_delete(&mut self, _: &UndoDelete, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(batch) = self.batches.iter_mut().rev().find(|b| b.awaits_undo()) {
            batch.owe_undo();
        } else if !self.undo_newest(window, cx) {
            cx.propagate();
        }
    }

    // Restores the newest delete of a still open instance and closes its toast; false when
    // there is none.
    fn undo_newest(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let open: HashSet<(PathBuf, u64)> = (self.workspace.read(cx).collections().iter())
            .map(|c| (c.root().to_path_buf(), c.generation()))
            .collect();
        (self.deletes).retain(|d| open.contains(&(d.root.clone(), d.generation)));
        let Some(d) = self.deletes.pop() else {
            return false;
        };
        window.remove_notification1::<crate::undo_toast::UndoToast>(
            crate::undo_toast::toast_key(d.id),
            cx,
        );
        self.send_restores(&d.root, d.generation, d.trashed, cx);
        true
    }

    // Sends one restore per trashed item and tracks them as a batch, so an undo is reported
    // by one toast. Last deleted goes first, so each restore finds its siblings numbered as
    // that delete left them. Tickets belong to the instance `generation` that trashed them,
    // so a reopened collection gets nothing.
    fn send_restores(
        &mut self,
        root: &Path,
        generation: u64,
        trashed: Vec<(u64, ItemPath)>,
        cx: &mut Context<Self>,
    ) {
        if self.generation(root, cx) != Some(generation) {
            return;
        }
        let sent: Vec<(u64, ItemPath)> = self.workspace.update(cx, |w, cx| {
            (trashed.into_iter().rev())
                .filter_map(|(ticket, item)| {
                    let op = TreeOp::Restore { ticket };
                    Some((w.send(root, Command::Tree(op), cx)?, item))
                })
                .collect()
        });
        if sent.is_empty() {
            return;
        }
        let ids: Vec<u64> = sent.iter().map(|(id, _)| *id).collect();
        self.workspace.update(cx, |w, _| w.track_batch(root, &ids));
        (self.batches).push(Batch::new(
            Verb::Restore,
            root.to_path_buf(),
            generation,
            sent,
        ));
    }

    // The top-level members of an active set, in sidebar order, for the rows' drag payloads.
    fn drag_set(&self, cx: &App) -> Option<std::rc::Rc<DragSet>> {
        let root = self.multi.keys().first().map(|k| k.root.clone())?;
        if !self.multi.is_active() {
            return None;
        }
        let snapshot = self.workspace.read(cx).snapshot(&root, cx)?;
        let items: Vec<ItemPath> = self
            .multi
            .normalised(&row_keys(&root, &snapshot.content.rows))
            .into_iter()
            .map(|k| k.path)
            .collect();
        let members = self.multi.keys().iter().map(|k| k.path.clone()).collect();
        Some(std::rc::Rc::new(DragSet {
            root,
            items,
            members,
        }))
    }

    // The top-level items a bulk verb would act on, by kind.
    fn bulk_tally(&self, cx: &App) -> Tally {
        let Some(root) = self.multi.keys().first().map(|k| k.root.clone()) else {
            return Tally::default();
        };
        self.workspace
            .read(cx)
            .snapshot(&root, cx)
            .map_or(Tally::default(), |snap| {
                let rows = &snap.content.rows;
                multi::tally(rows, &self.multi.normalised(&row_keys(&root, rows)))
            })
    }

    // Asks once for the whole selection, then deletes its top-level members in tree order.
    fn confirm_delete_set(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.multi.keys().first().map(|k| k.root.clone()) else {
            return;
        };
        let Some(snapshot) = self.workspace.read(cx).snapshot(&root, cx) else {
            return;
        };
        let keys = self
            .multi
            .normalised(&row_keys(&root, &snapshot.content.rows));
        if keys.is_empty() {
            return;
        }
        let rows = &snapshot.content.rows;
        let dirty = self.dirty_within(&root, keys.iter().map(|k| &k.path));
        let items = multi::tally(rows, &keys);
        let unpicked = self.multi.unpicked(rows);
        let unpicked_rows = multi::tally_rows(unpicked.iter().copied());
        let paths: Vec<ItemPath> = keys.into_iter().map(|k| k.path).collect();
        let trash = self.workspace.read(cx).trash_available();
        // The trash makes a prompt needless unless unsaved edits, or rows nobody selected inside
        // a selected folder, would go with the items.
        if trash && !dirty && unpicked.is_empty() {
            self.send_deletes(&root, paths, true, window, cx);
            return;
        }
        let ask = if trash {
            bulk_delete_ask(items, unpicked_rows, dirty)
        } else {
            permanent_delete_ask(items, unpicked_rows, dirty)
        };
        let answer = crate::confirm::ask(ask, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.recv().await == Ok(1) {
                this.update_in(cx, |s, window, cx| {
                    s.send_deletes(&root, paths, trash, window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    // Whether a request with unsaved edits sits at or under any of `paths` of `root`.
    fn dirty_within<'a>(&self, root: &Path, paths: impl Iterator<Item = &'a ItemPath>) -> bool {
        let paths: Vec<&ItemPath> = paths.collect();
        self.rows.values().any(|n| {
            n.dirty
                && n.root == root
                && paths
                    .iter()
                    .any(|p| n.path == **p || crate::stale::is_within(&n.path, p))
        })
    }

    // Sends one delete per item and tracks them as a batch, so the sidebar reports them once:
    // into the trash with an Undo toast, or permanently.
    fn send_deletes(
        &mut self,
        root: &Path,
        items: Vec<ItemPath>,
        trash: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Whatever started it, a delete Ctrl+Z can undo leaves the keyboard on the tree, where
        // that key undoes it; an editor's Ctrl+Z stays its own once focus goes back there.
        if trash && self.drawn {
            self.tree.update(cx, |t, cx| t.focus(window, cx));
        }
        let sent: Vec<(u64, ItemPath)> = self.workspace.update(cx, |w, cx| {
            items
                .into_iter()
                .filter_map(|item| {
                    let op = if trash {
                        TreeOp::Trash { item: item.clone() }
                    } else {
                        TreeOp::Delete { item: item.clone() }
                    };
                    Some((w.send(root, Command::Tree(op), cx)?, item))
                })
                .collect()
        });
        let ids: Vec<u64> = sent.iter().map(|(id, _)| *id).collect();
        self.workspace.update(cx, |w, _| w.track_batch(root, &ids));
        let generation = self.generation(root, cx).unwrap_or(u64::MAX);
        let batch = Batch::new(Verb::Delete, root.to_path_buf(), generation, sent);
        (self.batches).push(if trash { batch.into_trash() } else { batch });
    }

    /// Tells the sidebar whether the shell draws it now.
    pub fn set_drawn(&mut self, drawn: bool) {
        self.drawn = drawn;
    }

    // Deletes only on the first answer, "Delete". With a trash and nothing unsaved a request or
    // folder goes there at once, with an Undo toast in place of the prompt.
    fn confirm_delete(&mut self, node: &Node, window: &mut Window, cx: &mut Context<Self>) {
        let (root, item, example) = (node.root.clone(), node.path.clone(), node.example);
        let folder = node.kind == NodeKind::Folder;
        let trash = example.is_none() && self.workspace.read(cx).trash_available();
        if trash && !self.dirty_within(&root, std::iter::once(&item)) {
            self.send_deletes(&root, vec![item], true, window, cx);
            return;
        }
        let ask = match example {
            Some(_) => crate::confirm::delete_ask(format!("Delete \"{}\"?", node.name), None),
            None => single_delete_ask(&node.name, folder, !trash),
        };
        let answer = crate::confirm::ask(ask, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.recv().await == Ok(1) {
                this.update_in(cx, |s, window, cx| match example {
                    Some(index) => s.workspace.update(cx, |w, cx| {
                        w.delete_example(&root, item, index, cx);
                    }),
                    None if trash => s.send_deletes(&root, vec![item], true, window, cx),
                    None => {
                        s.workspace.update(cx, |w, cx| {
                            w.send(&root, Command::Tree(TreeOp::Delete { item }), cx);
                        });
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    // Example rows share their request's path and the Unsaved group the collection's, so a
    // lookup by path never returns one.
    fn find(&self, root: &Path, path: &ItemPath) -> Option<&Node> {
        self.rows.values().find(|n| {
            !matches!(
                n.kind,
                NodeKind::Invalid | NodeKind::Example | NodeKind::UnsavedGroup | NodeKind::Empty
            ) && n.root == root
                && n.path == *path
        })
    }

    // Rebuilds the nodes from the newest snapshots; the tree's items are replaced only on a
    // real change. That clears the tree's selection, so the wanted selection (a moved one
    // first, else the tree's, else one an earlier rebuild lost) is found again by path.
    fn rebuild(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let open: Vec<(PathBuf, Option<Arc<CollectionSnapshot>>)> = workspace
            .listed_collections(cx)
            .iter()
            .map(|c| (c.root().to_path_buf(), workspace.snapshot(c.root(), cx)))
            .collect();
        for (root, _) in &open {
            self.expanded
                .entry(root.clone())
                .or_insert_with(|| [ItemPath::root()].into());
        }
        for (root, snap) in &open {
            if let Some(snap) = snap {
                self.restore_expanded(root, snap, cx);
            }
        }
        // Follow the active request only when another tab becomes active, so a later click
        // elsewhere stays. Keyed by tab, since a rename moves the tab before its notice arrives.
        // Its folders open before the build so its row exists to select.
        let follow = workspace
            .tabs()
            .active()
            .and_then(|tab| Some((tab.id, tab.root.clone(), tab.item()?.clone())));
        if follow.is_none() {
            self.followed = None;
        }
        let follow = follow.filter(|(id, _, _)| self.followed != Some(*id));
        if let Some((_, root, item)) = &follow {
            let set = self.expanded.entry(root.clone()).or_default();
            let mut parent = item.parent();
            while let Some(p) = parent {
                parent = p.parent();
                set.insert(p);
            }
        }
        let names: Vec<String> = open
            .iter()
            .map(|(root, snap)| collection_name(root, snap.as_deref()))
            .collect();
        let inputs: Vec<(&Path, &str, Option<&Content>)> = open
            .iter()
            .zip(&names)
            .map(|((root, snap), name)| {
                (
                    root.as_path(),
                    name.as_str(),
                    snap.as_ref().map(|s| &s.content),
                )
            })
            .collect();
        // Draft rows mirror the open draft tabs in tab order and are never cached, so a row
        // lives exactly as long as its tab.
        let drafts: Vec<OpenDraft> = workspace
            .tabs()
            .entries()
            .iter()
            .filter_map(|tab| {
                let path = tab.item().filter(|p| p.is_draft())?;
                // A view the snapshot has not delivered yet still gets its row, named like its
                // tab, so the group does not wait for a later rebuild.
                let view = workspace.request(&tab.root, path, cx);
                Some(OpenDraft {
                    root: tab.root.clone(),
                    path: path.clone(),
                    name: view
                        .as_ref()
                        .map_or_else(|| path.file_name().to_owned(), |v| v.name.clone()),
                    method: view.as_ref().map(|v| v.method.clone()).unwrap_or_default(),
                    dirty: view.is_some_and(|v| v.dirty),
                })
            })
            .collect();
        let nodes = model::build(
            &inputs,
            &drafts,
            &self.expanded,
            &self.filter_text,
            open.iter()
                .map(|(root, _)| root.as_path())
                .find(|root| workspace.is_scratch(root)),
        );
        // Computed before any early-out so a closed root empties the set.
        self.live_keys = open
            .iter()
            .filter_map(|(root, snap)| Some(row_keys(root, &snap.as_ref()?.content.rows)))
            .flatten()
            .collect();
        let dropped = self.multi.retain(|k| self.live_keys.contains(k));
        self.lost_multi.extend(dropped);
        self.git = Rc::new(
            open.iter()
                .filter_map(|(root, _)| Some((root.clone(), workspace.git_changes(root)?)))
                .collect(),
        );
        self.rollup = Rc::new(rolled_up(&nodes, &self.git));
        // A busy marker can change with the nodes unchanged, so the view always redraws.
        cx.notify();
        self.save_expanded(cx);
        let current = self.selected(cx);
        // An example row shares its request's path, so it is restored by its tree id.
        let example = self
            .selected_node(cx)
            .filter(|n| n.kind == NodeKind::Example)
            .map(|n| n.id.clone());
        if current.is_some() {
            self.lost_selection = None;
        }
        // The workspace notifies, the sidebar rebuilds one or more times, then the Moved batch
        // arrives; a hand-off lasts while its neighbour stays selected, until that batch ends.
        self.handed_off
            .take_if(|(_, near)| current.as_ref() != Some(near));
        let wanted = self
            .moved_selection
            .take()
            .or(current.clone())
            .or_else(|| self.lost_selection.take());
        let changed = nodes != self.nodes;
        // Where the selection sat before the items change, so a removed row hands it on.
        let old_ix = self.tree.read(cx).selected_index();
        if changed {
            self.prior_example = self
                .selected_node(cx)
                .and_then(|n| Some((n.root.clone(), n.path.clone(), n.example?)));
            let mut rows = HashMap::new();
            index(&nodes, &mut rows);
            let items = model::to_tree_items(&nodes);
            self.tree.update(cx, |t, cx| t.set_items(items, cx));
            self.nodes = nodes;
            self.rows = Rc::new(rows);
        }
        if let Some((root, path)) = wanted
            && (changed || current.as_ref() != Some(&(root.clone(), path.clone())))
        {
            let kept = example.filter(|id| {
                self.rows
                    .get(id)
                    .is_some_and(|n| n.root == root && n.path == path)
            });
            match kept.or_else(|| self.find(&root, &path).map(|n| n.id.clone())) {
                Some(id) => self.select(&id, cx),
                // A row the filter hides, or whose snapshot is not loaded, comes back later;
                // one that is gone hands the selection to its nearest surviving row, unless a
                // multi-selection survives, which a single selection would clear.
                None => {
                    let loaded = open.iter().any(|(r, snap)| *r == root && snap.is_some());
                    let near = old_ix
                        .filter(|_| loaded && self.filter_text.trim().is_empty())
                        .filter(|_| self.multi.is_empty())
                        .and_then(|ix| self.nearest(ix, &root, &path, cx));
                    match near {
                        Some(ix) => {
                            self.tree.update(cx, |t, cx| {
                                t.set_selected_index(Some(ix), cx);
                                t.scroll_to_item(ix, ScrollStrategy::Nearest);
                            });
                            self.handed_off = self.selected(cx).map(|near| ((root, path), near));
                        }
                        None => self.lost_selection = Some((root, path)),
                    }
                }
            }
        }
        // A row the filter hides, or a snapshot not loaded yet, leaves the follow for a later
        // rebuild. Selecting and scrolling never move focus.
        if let Some((tab, root, item)) = follow
            && let Some(id) = self.find(&root, &item).map(|n| n.id.clone())
        {
            self.select(&id, cx);
            self.tree.update(cx, |t, _| {
                if let Some(ix) = t.index_of(&id) {
                    t.scroll_to_item(ix, ScrollStrategy::Nearest);
                }
            });
            self.lost_selection = None;
            self.followed = Some(tab);
        }
        self.apply_pending_focus(window, cx);
        self.settle_selection(cx);
    }

    // The row that takes a removed row's place: its old index clamped to the rows left, or
    // the row above when only that one shares the removed row's parent.
    fn nearest(&self, old: usize, root: &Path, path: &ItemPath, cx: &App) -> Option<usize> {
        let tree = self.tree.read(cx);
        let len = (0..).take_while(|&ix| tree.entry(ix).is_some()).count();
        let at = old.min(len.checked_sub(1)?);
        // An Empty row stands where the folder's last row was: the row after it, or the
        // folder itself at the end, takes the place instead.
        let empty = |ix: usize| {
            let id = &tree.entry(ix)?.item().id;
            Some(self.rows.get(id)?.kind == NodeKind::Empty)
        };
        let at = match empty(at) {
            Some(true) if at + 1 < len => at + 1,
            Some(true) => at - 1,
            _ => at,
        };
        let parent = |ix: usize| {
            let node = self.rows.get(&tree.entry(ix)?.item().id)?;
            Some((node.root.clone(), node.path.parent()))
        };
        let want = Some((root.to_path_buf(), path.parent()));
        if at > 0 && parent(at) != want && parent(at - 1) == want {
            Some(at - 1)
        } else {
            Some(at)
        }
    }

    fn select(&mut self, id: &SharedString, cx: &mut Context<Self>) {
        self.tree.update(cx, |t, cx| {
            let ix = t.index_of(id);
            t.set_selected_index(ix, cx);
        });
    }

    // Selects, and maybe renames, the pending row once it exists; until then it stays pending.
    fn apply_pending_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(f) = &self.pending_focus else {
            return;
        };
        let Some((id, kind)) = self.find(&f.root, &f.path).map(|n| (n.id.clone(), n.kind)) else {
            return;
        };
        let Some(f) = self.pending_focus.take() else {
            return;
        };
        self.select(&id, cx);
        // A new request row opens the way a single click does, so the editor shows the row the
        // selection marks.
        if kind == NodeKind::Request {
            cx.emit(SidebarEvent::OpenItem {
                root: f.root.clone(),
                path: f.path.clone(),
                permanent: false,
                focus_url: false,
            });
        }
        // Only a new folder renames on arrival, so its rename is a fresh one.
        if f.rename {
            self.start_rename(&f.root, &f.path, window, cx);
            if let Some(r) = &mut self.rename
                && r.root == f.root
                && r.path == f.path
            {
                r.fresh = true;
            }
        }
    }

    // Every filter edit rebuilds with its text; a repeated `Change` rebuilds to the same nodes.
    fn filter_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.filter_text = input.read(cx).value().to_string();
                self.rebuild(window, cx);
            }
            InputEvent::PressEnter {
                secondary: false, ..
            } => self.open_first_match(window, cx),
            _ => {}
        }
    }

    // Keeps the first visible request open, then hands the keyboard to the tree with that
    // row selected so the arrows continue from the match.
    fn open_first_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tree = self.tree.read(cx);
        let first = (0..)
            .map_while(|ix| tree.entry(ix))
            .filter_map(|e| self.rows.get(&e.item().id))
            .find(|n| n.kind == NodeKind::Request);
        let Some(n) = first else {
            return;
        };
        let (id, root, path) = (n.id.clone(), n.root.clone(), n.path.clone());
        self.select(&id, cx);
        self.tree.update(cx, |t, cx| t.focus(window, cx));
        cx.emit(SidebarEvent::OpenItem {
            root,
            path,
            permanent: true,
            focus_url: false,
        });
    }

    // The selected row by tree id, so an example row is itself and not its request.
    fn selected_node(&self, cx: &App) -> Option<&Node> {
        let node = self.rows.get(&self.tree.read(cx).selected_item()?.id)?;
        (!matches!(
            node.kind,
            NodeKind::Invalid | NodeKind::UnsavedGroup | NodeKind::Empty
        ))
        .then_some(node)
    }

    // The selected row when it is not being renamed; a rename's input sits inside the tree,
    // so its keys must pass through to the text.
    fn selected_row(&self, cx: &App) -> Option<&Node> {
        if self.rename.is_some() {
            return None;
        }
        self.rows.get(&self.tree.read(cx).selected_item()?.id)
    }

    // Enter does what a double click does: keeps a request, toggles a folder or collection.
    // A kept request also takes the keyboard to its URL; Space previews and stays in the tree.
    fn open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        if self.multi.is_active() {
            return;
        }
        let Some(n) = self.selected_row(cx) else {
            return cx.propagate();
        };
        match n.kind {
            NodeKind::Request => {
                let event = SidebarEvent::OpenItem {
                    root: n.root.clone(),
                    path: n.path.clone(),
                    permanent: true,
                    focus_url: true,
                };
                cx.emit(event);
            }
            NodeKind::Opaque => match n.unsupported {
                Some(kind) => show_unsupported(kind, window, cx),
                None => cx.propagate(),
            },
            NodeKind::Folder | NodeKind::Collection if !self.filter_text.trim().is_empty() => {
                let (root, path) = (n.root.clone(), n.path.clone());
                self.reveal_folder(root, path, window, cx);
            }
            NodeKind::Folder | NodeKind::Collection => {
                let id = n.id.clone();
                self.toggle_folder(&id, window, cx);
            }
            _ => cx.propagate(),
        }
    }

    /// Selects a collection's root row and scrolls it to the top of the list, or as near as
    /// the list allows, clearing a filter that hides it first. Focus does not move.
    pub fn reveal_root(&mut self, root: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let top = ItemPath::root();
        if self.find(root, &top).is_none() && !self.filter_text.trim().is_empty() {
            self.filter.update(cx, |i, cx| i.set_value("", window, cx));
            self.filter_text.clear();
            self.rebuild(window, cx);
        }
        let Some(id) = self.find(root, &top).map(|n| n.id.clone()) else {
            return;
        };
        self.select(&id, cx);
        // The root goes to the top, not just into view, so the collection's requests show below it.
        self.tree.update(cx, |t, _| {
            if let Some(ix) = t.index_of(&id) {
                t.scroll_to_item(ix, ScrollStrategy::Top);
            }
        });
    }

    /// Opens a folder and its ancestors, clears the filter, then selects the folder's row and
    /// scrolls it into view. Focus does not move.
    pub fn reveal_folder(
        &mut self,
        root: PathBuf,
        path: ItemPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Its ancestors open too, or a folder under a collapsed one would vanish unfiltered.
        // A request or example has no children to open, so only its parents join the set.
        let leaf = self.workspace.read(cx).request(&root, &path, cx).is_some();
        let set = self.expanded.entry(root.clone()).or_default();
        let mut next = if leaf {
            path.parent()
        } else {
            Some(path.clone())
        };
        while let Some(p) = next {
            next = p.parent();
            set.insert(p);
        }
        self.filter.update(cx, |i, cx| i.set_value("", window, cx));
        self.filter_text.clear();
        self.rebuild(window, cx);
        let Some(id) = self.find(&root, &path).map(|n| n.id.clone()) else {
            return;
        };
        self.select(&id, cx);
        self.tree.update(cx, |t, _| {
            if let Some(ix) = t.index_of(&id) {
                t.scroll_to_item(ix, ScrollStrategy::Nearest);
            }
        });
    }

    // Space does what a single click does on a request; anywhere else it passes on.
    fn preview_selected(
        &mut self,
        _: &PreviewSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.multi.is_active() {
            return;
        }
        match self.selected_row(cx) {
            Some(n) if n.kind == NodeKind::Request => {
                let event = SidebarEvent::OpenItem {
                    root: n.root.clone(),
                    path: n.path.clone(),
                    permanent: false,
                    focus_url: false,
                };
                cx.emit(event);
            }
            Some(Node {
                kind: NodeKind::Opaque,
                unsupported: Some(kind),
                ..
            }) => show_unsupported(*kind, window, cx),
            _ => cx.propagate(),
        }
    }

    // Focuses the tree and selects a row: the selecting half of the tree's own mouse-down,
    // for rows that stop it.
    fn focus_row(&mut self, id: &SharedString, window: &mut Window, cx: &mut Context<Self>) {
        self.tree.update(cx, |t, cx| t.focus(window, cx));
        self.select(id, cx);
    }

    // Focuses the tree, selects a folder row and flips its expansion: what the tree's own
    // mouse-down would have done had the row not stopped it.
    fn toggle_folder(&mut self, id: &SharedString, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_row(id, window, cx);
        self.toggle_expanded(id, window, cx);
    }

    // Flips a row's kept expansion and rebuilds; a filter shows folders expanded, so a flip
    // made under one is not kept.
    fn toggle_expanded(&mut self, id: &SharedString, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.rows.get(id) else {
            return;
        };
        let (root, path) = (node.root.clone(), node.path.clone());
        if self.filter_text.trim().is_empty() || node.kind == NodeKind::Request {
            let set = self.expanded.entry(root).or_default();
            if !set.remove(&path) {
                set.insert(path);
            }
        }
        self.rebuild(window, cx);
    }

    // The tree flipped a row itself (keys or its own click): the row's chevron and icon follow
    // at once, and the flip is kept for later rebuilds unless a filter shows everything open.
    fn tree_event(&mut self, _: Entity<TreeState>, event: &TreeEvent, cx: &mut Context<Self>) {
        let (id, open) = match event {
            TreeEvent::Expanded(id) => (id, true),
            TreeEvent::Collapsed(id) => (id, false),
        };
        // The Unsaved group always shows its drafts, and its root path is the collection's. A
        // collapse the tree made itself (Left) is undone by pushing the items again.
        if self
            .rows
            .get(id)
            .is_some_and(|n| n.kind == NodeKind::UnsavedGroup)
        {
            if !open {
                let items = model::to_tree_items(&self.nodes);
                let selected = self.tree.read(cx).selected_item().map(|i| i.id.clone());
                self.tree.update(cx, |t, cx| {
                    t.set_items(items, cx);
                    if let Some(ix) = selected.and_then(|id| t.index_of(&id)) {
                        t.set_selected_index(Some(ix), cx);
                    }
                });
            }
            return;
        }
        let Some(node) = Rc::make_mut(&mut self.rows).get_mut(id) else {
            return;
        };
        node.expanded = open;
        let (root, path) = (node.root.clone(), node.path.clone());
        // A toggled root or folder is a pick of its collection, even when already selected.
        self.make_current(&root, cx);
        cx.notify();
        if !self.filter_text.trim().is_empty() {
            return;
        }
        let set = self.expanded.entry(root).or_default();
        if open {
            set.insert(path);
        } else {
            set.remove(&path);
        }
        self.save_expanded(cx);
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let rows = Rc::clone(&self.rows);
        let collections = self.workspace.read(cx).listed_collections(cx).len();
        while self.button_focus.len() < collections {
            self.button_focus.push(cx.focus_handle());
        }
        let tree_focused = self.rename.is_none() && self.tree_holds_focus(window, cx);
        let button_focus = self.button_focus.clone();
        self.tag_paints.borrow_mut().clear();
        let paints = Rc::clone(&self.tag_paints);
        let git = Rc::clone(&self.git);
        let rollup = Rc::clone(&self.rollup);
        let hovered = self.hovered.clone();
        let busy: Vec<bool> = self
            .workspace
            .read(cx)
            .listed_collections(cx)
            .iter()
            .map(|c| c.busy())
            .collect();
        // The hint names the key bound in this view's own context, wherever focus is.
        // It steps aside when the field is too narrow for it; the field's width does not
        // depend on the hint, so a crossing settles in one extra frame.
        let cramped = Rc::clone(&self.filter_cramped);
        let was_cramped = cramped.get();
        let filter_hint = Kbd::binding_for_action(&FocusFilter, Some(KEY_CONTEXT), window)
            .filter(|_| !was_cramped);
        let no_match = !self.filter_text.trim().is_empty() && self.nodes.is_empty();
        // With no rows at all the header has nothing to act on.
        let idle = self.nodes.is_empty() && !no_match;
        // Member row ids, built once per frame; empty unless the set is active.
        let members: std::rc::Rc<std::collections::HashSet<SharedString>> =
            std::rc::Rc::new(if self.multi.is_active() {
                self.multi
                    .keys()
                    .iter()
                    .filter_map(|k| self.id_of(k))
                    .collect()
            } else {
                Default::default()
            });
        let multi_on = !members.is_empty();
        let drag_set = self.drag_set(cx);
        let list_active = cx.theme().list_active;
        let list_hover = cx.theme().list_hover;
        let menu_target = self.menu_target.clone();
        self.drop_stale_key_menu(window, cx);
        // The kit keeps a menu's cursor private; once this frame draws the keyboard's menu,
        // the kit's own Down moves the cursor to its first entry.
        if let Some(k) = self.key_menu.as_mut().filter(|k| k.fresh) {
            k.fresh = false;
            let handle = k.menu.focus_handle(cx);
            window.defer(cx, move |window, cx| {
                handle.dispatch_action(&SelectDown, window, cx)
            });
        }
        let key_menu = (self.key_menu.as_ref()).map(|k| (k.row.clone(), k.menu.clone()));
        let menu_margin = style::resolve(style::SPACE_2, window);
        // While the keyboard's menu is open the tree must not scroll its row away under it.
        let scroll_shield = key_menu.is_some().then(|| key_menu_shield(window));
        let muted = cx.theme().muted_foreground;
        // The scratch pad is listed: say what it is, since it looks like a collection.
        let scratch_only = {
            let ws = self.workspace.read(cx);
            (ws.listed_collections(cx).iter()).any(|c| ws.is_scratch(c.root()))
        };
        let scratch_dir = self.workspace.read(cx).scratch_dir();
        let view = cx.entity().downgrade();
        let menu_view = view.clone();
        let hover = self.hover.clone();
        // An example's row is found from its request's, whose collection index it shares.
        let renaming = self.rename.as_ref().map(|r| {
            let request = self.find(&r.root, &r.path);
            let id = match r.example {
                Some(ix) => request.map(|n| model::example_id(n.collection, &r.path, ix)),
                None => request.map(|n| n.id.clone()),
            };
            (
                id,
                r.input.clone(),
                r.error.clone().map(|(_, e)| SharedString::from(e)),
            )
        });
        div()
            .id("sidebar-view")
            .children(scroll_shield)
            .test_support()
            .key_context(KEY_CONTEXT)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::focus_filter))
            .on_action(cx.listener(Self::collapse_all))
            .on_action(cx.listener(Self::expand_all))
            // Step back from the tree's stand-in, not the tree, whose own handle sits after it
            // in the Tab order; the stand-in's brief focus is gone before any frame sees it.
            // Any control inside a row matches the same binding. Asking the tree for focus
            // moves a control's focus to it and leaves the tree's own focus where it was, which
            // is the cue to go on to History.
            .on_action(cx.listener(|this, _: &LeaveTree, window, cx| {
                let before = window.focused(cx);
                let on_stand_in = this.tree_focus.is_focused(window);
                this.tree.update(cx, |t, cx| t.focus(window, cx));
                if on_stand_in || before == window.focused(cx) {
                    window.focus(&this.tree_focus, cx);
                    window.focus_prev(cx);
                }
            }))
            .on_action(cx.listener(Self::rename_selected))
            .on_action(cx.listener(Self::clone_selected))
            .on_action(cx.listener(Self::show_in_file_manager))
            .on_action(cx.listener(Self::open_in_terminal))
            .on_action(cx.listener(Self::copy_action))
            .on_action(cx.listener(Self::cut_action))
            .on_action(cx.listener(Self::paste_action))
            .on_action(cx.listener(Self::delete_selected))
            .on_action(cx.listener(Self::clear_filter))
            .on_action(cx.listener(Self::enter_tree))
            .on_action(cx.listener(Self::run_selected))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::preview_selected))
            .on_action(cx.listener(Self::clear_selection))
            .on_action(cx.listener(Self::extend_up))
            .on_action(cx.listener(Self::extend_down))
            .on_action(cx.listener(Self::open_row_menu))
            .on_action(cx.listener(Self::folder_settings))
            .on_action(cx.listener(Self::save_selected_draft))
            .on_action(cx.listener(Self::close_selected_draft))
            .on_action(cx.listener(|this, _: &CopyAsCurl, window, cx| {
                this.on_tree_request(MenuEntry::CopyAsCurl, window, cx)
            }))
            .on_action(cx.listener(|this, _: &GenerateCode, window, cx| {
                this.on_tree_request(MenuEntry::GenerateCode, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ConvertToGraphql, window, cx| {
                this.on_tree_request(MenuEntry::ConvertToGraphql, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ConvertToHttp, window, cx| {
                this.on_tree_request(MenuEntry::ConvertToHttp, window, cx)
            }))
            // A plain arrow moves the kit's selection out of the set; the set ends first.
            // Inside the keyboard's menu the arrows walk its entries, and the set stays.
            .capture_action(cx.listener(|this, _: &SelectUp, _, cx| {
                if this.key_menu.is_none() {
                    this.clear_multi(cx)
                }
            }))
            .capture_action(cx.listener(|this, _: &SelectDown, _, cx| {
                if this.key_menu.is_none() {
                    this.clear_multi(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.rename.is_some() {
                    this.cancel_rename(window, cx);
                }
            }))
            .child(
                div()
                    .id("sidebar-filter")
                    .test_support()
                    .key_context(FILTER_CONTEXT)
                    .p(style::SPACE_1)
                    .child(
                        h_flex()
                            .gap(style::SPACE_1)
                            .child(
                                // The tooltip sits on the field alone, so the buttons beside it
                                // keep their own.
                                div()
                                    .id("sidebar-filter-input")
                                    .test_support()
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .tooltip(|window, cx| {
                                        Tooltip::new(FILTER_TOOLTIP).build(window, cx)
                                    })
                                    .child(
                                        canvas(
                                            move |bounds, window, _| {
                                                let now = bounds.size.width
                                                    < style::filter_hint_min_field(window);
                                                cramped.set(now);
                                                if now != was_cramped {
                                                    window.request_animation_frame();
                                                }
                                            },
                                            |_, _, _, _| {},
                                        )
                                        .absolute()
                                        .size_full(),
                                    )
                                    .child(ui::padded_input(
                                        Input::new(&self.filter)
                                            .disabled(idle)
                                            .prefix(
                                                div()
                                                    .id("sidebar-filter-icon")
                                                    .test_support()
                                                    .child(Icon::new(IconName::Search)),
                                            )
                                            .bg(ui::field_fill(false, cx))
                                            .when_some(filter_hint, |input, kbd| {
                                                input.suffix(
                                                    div()
                                                        .id("sidebar-filter-kbd")
                                                        .test_support()
                                                        .child(kbd),
                                                )
                                            }),
                                        &self.filter,
                                    )),
                            )
                            .child(
                                // Mouse shortcuts: the palette commands are the keyboard path, and
                                // History stays the one Tab stop between the filter and the tree.
                                Button::new("sidebar-collapse-all")
                                    .lock(idle, cx)
                                    .tab_stop(false)
                                    .ghost()
                                    .compact()
                                    .icon(IconName::FoldVertical)
                                    .tooltip_with_action(
                                        label("collection.collapse_all"),
                                        &CollapseAll,
                                        Some(SHELL_CONTEXT),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.set_folders_open(None, false, window, cx)
                                    }))
                                    .accessibility_label(label("collection.collapse_all")),
                            )
                            .child(
                                Button::new("sidebar-expand-all")
                                    .lock(idle, cx)
                                    .tab_stop(false)
                                    .ghost()
                                    .compact()
                                    .icon(IconName::UnfoldVertical)
                                    .tooltip_with_action(
                                        label("collection.expand_all"),
                                        &ExpandAll,
                                        Some(SHELL_CONTEXT),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.set_folders_open(None, true, window, cx)
                                    }))
                                    .accessibility_label(label("collection.expand_all")),
                            )
                            .child(
                                Button::new("sidebar-history")
                                    .lock(idle, cx)
                                    .ghost()
                                    .compact()
                                    .icon(IconName::RotateCcwClock)
                                    .tooltip_with_action(
                                        "History",
                                        &OpenHistory,
                                        Some(SHELL_CONTEXT),
                                    )
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(OpenHistory), cx)
                                    })
                                    .accessibility_label("History"),
                            ),
                    ),
            )
            .child(
                div()
                    .id("sidebar-tree")
                    .test_support()
                    .track_focus(&self.tree_focus)
                    .flex_1()
                    .min_h_0()
                    .when(scratch_only, |d| {
                        d.child(
                            div()
                                .id("sidebar-scratch-hint")
                                .test_support()
                                .px(style::SPACE_2)
                                .py(style::SPACE_1)
                                .text_xs()
                                .text_color(muted)
                                .child(SCRATCH_HINT),
                        )
                    })
                    .when(no_match, |d| {
                        d.child(
                            div()
                                .id("sidebar-no-match")
                                .test_support()
                                .p(style::SPACE_2)
                                .text_color(cx.theme().muted_foreground)
                                .child("No requests match"),
                        )
                    })
                    .child(
                        tree(&self.tree, move |_, entry, selected, window, cx| {
                            let id = entry.item().id.clone();
                            // Selection is the kit's active fill plus an overlay on the row's own
                            // box: a left edge, accent while the tree has focus. Only
                            // the suffix slot is a direct child of that box, so the overlay takes
                            // it and the dirty dot moves into the content.
                            // The row's accessible id is its element id, so agents address it the same way.
                            let item = ListItem::new(id.clone())
                                .accessibility_id(id.clone())
                                .h(style::ROW_HEIGHT)
                                .pl(style::SPACE_2 + style::SPACE_3 * entry.depth() as f32)
                                .group(ROW_GROUP)
                                .selected(selected)
                                .when(selected, |i| {
                                    let id = id.clone();
                                    i.suffix(move |_, cx| selection_overlay(&id, tree_focused, cx))
                                });
                            // The kit overwrites `.selected(..)` after this closure, so a member
                            // that is not the focus row is filled and edged here.
                            let member = members.contains(&id);
                            let item = item.when(member && !selected, |i| {
                                let id = id.clone();
                                i.bg(list_active)
                                    .suffix(move |_, cx| selection_overlay(&id, false, cx))
                            });
                            // A row already lit as selected or in the set needs no second mark.
                            let target = menu_target.as_ref() == Some(&id);
                            let item = item.when(target && !selected && !member, |i| {
                                let id = id.clone();
                                i.bg(list_hover)
                                    .suffix(move |_, cx| menu_target_overlay(&id, cx))
                            });
                            // Only the set being active gives rows a role, so single selection
                            // leaves the accessibility tree as it was.
                            let item = item.when(
                                multi_on && rows.get(&id).is_some_and(|n| selectable(n.kind)),
                                |i| i.role(Role::ListItem).aria_selected(member),
                            );
                            let item = match key_menu.as_ref().filter(|(row, _)| *row == id) {
                                Some((_, menu)) => {
                                    item.child(key_menu_layer(menu.clone(), menu_margin))
                                }
                                None => item,
                            };
                            let Some(n) = rows.get(&id) else {
                                return item.child(entry.item().label.clone());
                            };
                            if let Some((Some(rid), input, error)) = &renaming
                                && *rid == id
                            {
                                let field = ui::invalid_input(
                                    Input::new(input).bg(ui::field_fill(false, cx)),
                                    div(),
                                    input,
                                    error.is_some(),
                                    (
                                        "sidebar-rename-danger-ring".into(),
                                        "sidebar-rename-invalid-icon".into(),
                                    ),
                                    window,
                                    cx,
                                );
                                return item.child(
                                    v_flex()
                                        .id(marker_id("rename", &id))
                                        .test_support()
                                        .child(field)
                                        .when_some(error.clone(), |d, message| {
                                            // An empty box after the field puts the anchor
                                            // at the field's bottom edge.
                                            d.child(div().child(ui::field_refusal(
                                                "sidebar-rename-error",
                                                message,
                                                cx,
                                            )))
                                        }),
                                );
                            }
                            let zone = hover.as_ref().filter(|(h, _)| *h == id).map(|(_, z)| *z);
                            let item = drag_and_drop(item, n, zone, drag_set.as_ref(), &view, cx);
                            match n.kind {
                                NodeKind::Collection => {
                                    // The buttons create in this collection, not the workspace's
                                    // current one, and swallow the mouse-down so the tree does not
                                    // toggle it. Off the Tab order: the keyboard selects the row,
                                    // which makes it current, then uses New request or New folder.
                                    let create = |id: &'static str, icon, tip, entry| {
                                        let (view, node) = (view.clone(), n.clone());
                                        ui::icon_button((id, n.collection), icon, tip, None)
                                            .tab_stop(false)
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                            .on_click(move |_, window, cx| {
                                                view.update(cx, |s, cx| {
                                                    s.choose(&node, entry, window, cx)
                                                })
                                                .ok();
                                            })
                                    };
                                    // Like a folder's run button, the actions show on hover or on the
                                    // selected row of a focused tree, and float at rest so the name
                                    // runs to the edge; beside a roll-up dot they keep their room.
                                    let rolled = rollup.contains(&n.id) && !n.expanded;
                                    let actions = h_flex()
                                        .items_center()
                                        .child(
                                            create(
                                                "new-request",
                                                IconName::FilePlus,
                                                label("collection.new_request"),
                                                MenuEntry::NewRequest,
                                            )
                                            .tooltip_with_action(
                                                label("collection.new_request"),
                                                &NewRequest,
                                                Some(SHELL_CONTEXT),
                                            ),
                                        )
                                        .child(create(
                                            "new-folder",
                                            IconName::FolderPlus,
                                            label("collection.new_folder"),
                                            MenuEntry::NewFolder,
                                        ))
                                        .child(
                                            run_button(
                                                cx,
                                                ("run-collection", n.collection),
                                                "collection.run",
                                                MenuEntry::RunCollection,
                                                n,
                                                &view,
                                            )
                                            .lock(
                                                scratch_dir.as_deref() == Some(n.root.as_path()),
                                                cx,
                                            ),
                                        );
                                    let over = hovered.as_ref() == Some(&id);
                                    let slot =
                                        run_slot(actions, selected && tree_focused).hovered(over);
                                    let actions = if rolled { slot } else { slot.overlay() };
                                    let hover_view = view.clone();
                                    let hover_id = id.clone();
                                    item.on_hover(move |&entered, _, cx| {
                                        hover_view
                                            .update(cx, |s, cx| {
                                                s.set_hovered(&hover_id, entered, cx)
                                            })
                                            .ok();
                                    })
                                    .child(
                                        h_flex()
                                            .relative()
                                            .w_full()
                                            .justify_between()
                                            .gap(style::SPACE_1)
                                            .child(
                                                h_flex()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .gap(style::SPACE_1)
                                                    .child(chevron_slot(n))
                                                    .child(collection_row(
                                                        n.collection,
                                                        n.name.clone().into(),
                                                        busy.get(n.collection)
                                                            .copied()
                                                            .unwrap_or(false),
                                                    ))
                                                    // Two same-named collections show where they
                                                    // differ; the name gives way before the hint.
                                                    .children(n.hint.clone().map(|hint| {
                                                        marker("hint", n)
                                                            .min_w_0()
                                                            .flex_shrink(HINT_SHRINK)
                                                            .truncate()
                                                            .text_xs()
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child(hint)
                                                    })),
                                            )
                                            .child(
                                                h_flex()
                                                    .when_some(
                                                        button_focus.get(n.collection),
                                                        |d, f| d.track_focus(f),
                                                    )
                                                    .items_center()
                                                    .children(rolled.then(|| {
                                                        ui::rollup_dot(
                                                            marker_id("rollup", &n.id).into(),
                                                        )
                                                    }))
                                                    .child(actions),
                                            ),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        |e, _, cx| {
                                            if click_kind(&e.modifiers).is_some() {
                                                cx.stop_propagation()
                                            }
                                        },
                                    )
                                }
                                NodeKind::Invalid => {
                                    // One line inside the fixed row height; the tooltip keeps
                                    // the whole message readable.
                                    let label = entry.item().label.clone();
                                    let tip = label.clone();
                                    item.child(
                                        h_flex().min_w_0().child(
                                            div()
                                                .id(marker_id("invalid", &n.id))
                                                .test_support()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(cx.theme().danger)
                                                .child(label)
                                                .tooltip(move |window, cx| {
                                                    Tooltip::new(tip.clone()).build(window, cx)
                                                }),
                                        ),
                                    )
                                }
                                NodeKind::Request => {
                                    // A request with examples is a tree folder, which the tree
                                    // toggles on mouse-down; the row swallows that so a click only
                                    // opens, and the chevron alone toggles. The second click of a
                                    // double click keeps the tab the first one previewed.
                                    let toggle = entry
                                        .is_folder()
                                        .then(|| examples_toggle(n, &view).into_any_element());
                                    let view = view.clone();
                                    let (root, path) = (n.root.clone(), n.path.clone());
                                    item.child(with_suffix(
                                        item_row(n, toggle, &paints, muted),
                                        n,
                                        &git,
                                        false,
                                        cx,
                                    ))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(
                                        move |e: &ClickEvent, window, cx| {
                                            if let Some(secondary) = click_kind(&e.modifiers()) {
                                                view.update(cx, |s, cx| {
                                                    s.modified_click(&id, secondary, window, cx)
                                                })
                                                .ok();
                                                return;
                                            }
                                            let event = SidebarEvent::OpenItem {
                                                root: root.clone(),
                                                path: path.clone(),
                                                permanent: e.click_count() >= 2,
                                                focus_url: false,
                                            };
                                            view.update(cx, |s, cx| {
                                                s.plain_click(cx);
                                                s.focus_row(&id, window, cx);
                                                cx.emit(event)
                                            })
                                            .ok();
                                        },
                                    )
                                }
                                NodeKind::Example => {
                                    let view = view.clone();
                                    let (root, path) = (n.root.clone(), n.path.clone());
                                    let index = n.example.unwrap_or_default();
                                    item.child(example_row(n))
                                        .on_mouse_down(MouseButton::Left, |e, _, cx| {
                                            if click_kind(&e.modifiers).is_some() {
                                                cx.stop_propagation()
                                            }
                                        })
                                        .on_click(move |e: &ClickEvent, _, cx| {
                                            if click_kind(&e.modifiers()).is_some() {
                                                return;
                                            }
                                            let event = SidebarEvent::OpenExample {
                                                root: root.clone(),
                                                path: path.clone(),
                                                index,
                                                permanent: e.click_count() >= 2,
                                            };
                                            view.update(cx, |_, cx| cx.emit(event)).ok();
                                        })
                                }
                                // The group only heads its drafts and an Empty row only says
                                // so: the row swallows the tree's mouse-down so a click neither
                                // collapses nor selects it. Both leave the chevron slot blank, so
                                // the word starts where a sibling's icon or method tag would.
                                NodeKind::UnsavedGroup | NodeKind::Empty => item
                                    .child(
                                        h_flex()
                                            .gap(style::SPACE_1)
                                            .child(div().w(style::SPACE_4).flex_none())
                                            .child(
                                                marker("name", n)
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(n.name.clone()),
                                            ),
                                    )
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    }),
                                // A click focuses the draft's open tab; it is never previewed.
                                NodeKind::Draft => {
                                    let view = view.clone();
                                    let (root, path) = (n.root.clone(), n.path.clone());
                                    item.child(with_suffix(
                                        item_row(n, None, &paints, muted)
                                            .italic()
                                            .text_color(cx.theme().muted_foreground),
                                        n,
                                        &git,
                                        false,
                                        cx,
                                    ))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(
                                        move |e: &ClickEvent, window, cx| {
                                            // Modifier clicks leave the set and the tabs alone.
                                            if click_kind(&e.modifiers()).is_some() {
                                                return;
                                            }
                                            let event = SidebarEvent::OpenItem {
                                                root: root.clone(),
                                                path: path.clone(),
                                                permanent: true,
                                                focus_url: false,
                                            };
                                            view.update(cx, |s, cx| {
                                                s.focus_row(&id, window, cx);
                                                cx.emit(event)
                                            })
                                            .ok();
                                        },
                                    )
                                }
                                NodeKind::Opaque => {
                                    let view = view.clone();
                                    let unsupported = n.unsupported;
                                    let content = with_suffix(
                                        item_row(n, None, &paints, muted)
                                            .text_color(cx.theme().muted_foreground),
                                        n,
                                        &git,
                                        false,
                                        cx,
                                    );
                                    // A kind the app cannot send yet says so on hover, not only
                                    // when opened.
                                    item.child(match unsupported {
                                        Some(kind) => content
                                            .id(marker_id("unsupported", &n.id))
                                            .tooltip(ui::text_tooltip(
                                                marker_id("unsupported-tip", &n.id),
                                                kind.notice(),
                                            ))
                                            .into_any_element(),
                                        None => content.into_any_element(),
                                    })
                                    .on_mouse_down(MouseButton::Left, |e, _, cx| {
                                        if click_kind(&e.modifiers).is_some() {
                                            cx.stop_propagation()
                                        }
                                    })
                                    .on_click(
                                        move |e: &ClickEvent, window, cx| {
                                            view.update(cx, |s, cx| {
                                                match click_kind(&e.modifiers()) {
                                                    Some(secondary) => {
                                                        s.modified_click(&id, secondary, window, cx)
                                                    }
                                                    None => {
                                                        s.plain_click(cx);
                                                        if let Some(kind) = unsupported {
                                                            show_unsupported(kind, window, cx);
                                                        }
                                                    }
                                                }
                                            })
                                            .ok();
                                        },
                                    )
                                }
                                NodeKind::Folder => {
                                    // The tree toggles on mouse-down, which a drag also starts,
                                    // so the row swallows it and its click selects and toggles.
                                    let run = run_button(
                                        cx,
                                        marker_id("run-folder", &n.id),
                                        "collection.run_folder",
                                        MenuEntry::RunFolder,
                                        n,
                                        &view,
                                    );
                                    let view = view.clone();
                                    // The slot floats over the row's edge only where nothing
                                    // else lives there; a git letter, dirty dot or disk marker
                                    // keeps its own room beside it.
                                    let bare = git_change(&git, n).is_none()
                                        && !(rollup.contains(&n.id) && !n.expanded)
                                        && !n.dirty
                                        && !matches!(
                                            n.disk,
                                            DiskState::Removed | DiskState::Broken
                                        );
                                    let over = hovered.as_ref() == Some(&id);
                                    let slot = run_slot(run, selected).hovered(over);
                                    let slot = if bare { slot.overlay() } else { slot };
                                    // An empty folder has nothing to run.
                                    let empty =
                                        rows.contains_key(&model::empty_id(n.collection, &n.path));
                                    let hover_view = view.clone();
                                    let hover_id = id.clone();
                                    item.on_hover(move |&entered, _, cx| {
                                        hover_view
                                            .update(cx, |s, cx| {
                                                s.set_hovered(&hover_id, entered, cx)
                                            })
                                            .ok();
                                    })
                                    .child(
                                        h_flex()
                                            .relative()
                                            .w_full()
                                            .gap(style::SPACE_1)
                                            .child(
                                                with_suffix(
                                                    item_row(n, None, &paints, muted),
                                                    n,
                                                    &git,
                                                    rollup.contains(&n.id) && !n.expanded,
                                                    cx,
                                                )
                                                .flex_1()
                                                .min_w_0(),
                                            )
                                            .when(!empty, |d| d.child(slot)),
                                    )
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(
                                        move |e: &ClickEvent, window, cx| {
                                            view.update(cx, |s, cx| {
                                                match click_kind(&e.modifiers()) {
                                                    Some(secondary) => {
                                                        s.modified_click(&id, secondary, window, cx)
                                                    }
                                                    None => {
                                                        s.plain_click(cx);
                                                        s.toggle_folder(&id, window, cx)
                                                    }
                                                }
                                            })
                                            .ok();
                                        },
                                    )
                                }
                            }
                        })
                        .context_menu(
                            move |_, entry, menu, window, cx| {
                                // The kit's dismissal can leave focus on the closed menu; put
                                // it back where it was, or on the tree if that is gone. A rename
                                // the item started owns focus through its input instead.
                                let before = menu_focus::held(window, cx);
                                let (idle, tree_owner) = (menu_view.clone(), menu_view.clone());
                                let marked = entry.item().id.clone();
                                {
                                    let (view, marked) = (menu_view.clone(), marked.clone());
                                    window.defer(cx, move |_, cx| {
                                        view.update(cx, |s, cx| {
                                            s.menu_target = Some(marked);
                                            cx.notify();
                                        })
                                        .ok();
                                    });
                                }
                                menu_focus::watch_built(
                                    &menu,
                                    cx,
                                    move |cx| {
                                        // A later menu may have marked its own row already.
                                        let idle = idle.update(cx, |s, cx| {
                                            if s.menu_target.take_if(|t| *t == marked).is_some() {
                                                cx.notify();
                                            }
                                            s.rename.is_none()
                                        });
                                        match (before, idle.unwrap_or(false)) {
                                            (None, _) => menu_focus::Before::Nothing,
                                            (Some(held), true) => menu_focus::Before::Held(held),
                                            (Some(_), false) => menu_focus::Before::Gone,
                                        }
                                    },
                                    move |window, cx| {
                                        tree_owner
                                            .update(cx, |s, cx| {
                                                if s.rename.is_none() {
                                                    s.tree().update(cx, |t, cx| t.focus(window, cx))
                                                }
                                            })
                                            .ok();
                                    },
                                );
                                fill_row_menu(&menu_view, &entry.item().id, menu, window, cx)
                            },
                        ),
                    ),
            )
    }
}

// The context menu the keyboard opened, and the row it acts on.
struct KeyMenu {
    row: SharedString,
    menu: Entity<PopupMenu>,
    // Not drawn yet, so its first entry is still to be highlighted.
    fresh: bool,
    _dismissed: Subscription,
}

// The keyboard's menu hangs below its row, drawn above everything as the kit draws a right
// click's menu, and kept `margin` inside the window.
fn key_menu_layer(menu: Entity<PopupMenu>, margin: Pixels) -> impl IntoElement {
    div().absolute().top_full().left_0().child(
        deferred(anchored().snap_to_window_with_margin(margin).child(menu))
            .with_priority(gpui_kit::base::POPUP_PRIORITY),
    )
}

// A window-sized layer under the keyboard's menu that swallows wheel scrolling, as the kit's
// own context menu does, so the menu's row cannot scroll out from under it.
fn key_menu_shield(window: &Window) -> impl IntoElement {
    let size = window.bounds().size;
    deferred(
        anchored().position(Default::default()).child(
            div()
                .w(size.width)
                .h(size.height)
                .on_scroll(|_, _, cx| cx.stop_propagation()),
        ),
    )
    .with_priority(gpui_kit::base::POPUP_PRIORITY - 1)
}

/// Fills `menu` with row `id`'s context menu, as the sidebar stands now: the bulk menu for a
/// row inside the active multi-selection, else the row's own entries. A right click and the
/// keyboard both build it here, so their menus cannot drift apart.
fn fill_row_menu(
    view: &WeakEntity<Sidebar>,
    id: &SharedString,
    mut menu: PopupMenu,
    window: &mut Window,
    cx: &mut App,
) -> PopupMenu {
    let Some(sidebar) = view.upgrade() else {
        return menu;
    };
    let s = sidebar.read(cx);
    let multi_on = s.multi.is_active();
    let member = multi_on
        && s.multi
            .keys()
            .iter()
            .any(|k| s.id_of(k).as_ref() == Some(id));
    // A row inside the active set opens the bulk menu; any other row ends the set (deferred,
    // the tree may be busy building this menu).
    if member {
        let count = s.bulk_tally(cx);
        let run = s
            .run_plan(cx)
            .and_then(|plan| (!plan.1.is_empty()).then_some(plan.2));
        let mut prev = None;
        for choice in bulk_entries(count, run) {
            let view = view.clone();
            let line = ui::MenuLine::new(choice.label())
                .action(choice.action())
                .danger(choice.is_destructive())
                .group_break(prev.is_some_and(|p: BulkEntry| p.group() != choice.group()));
            prev = Some(choice);
            menu = line.add_to(
                menu,
                ui::SIDEBAR_ROW_KEYS,
                move |_, window, cx| {
                    view.update(cx, |s, cx| match choice {
                        BulkEntry::Run(_) => s.run_selected(&RunSelected, window, cx),
                        BulkEntry::Copy(_) => s.copy_selection(window, cx),
                        BulkEntry::Cut(_) => s.cut_selection(window, cx),
                        BulkEntry::Delete(_) => s.confirm_delete_set(window, cx),
                    })
                    .ok();
                },
                cx,
            );
        }
        return menu;
    }
    let node = s.rows.get(id).cloned();
    let facts = node.as_ref().map(|node| {
        let changed = node.kind == NodeKind::Request && git_change(&s.git, node).is_some();
        let request_kind = (node.kind == NodeKind::Request)
            .then(|| s.request_kind(node, cx))
            .flatten();
        let scratch = s.workspace.read(cx).scratch_dir();
        (changed, request_kind, scratch, s.paste_blocked(node))
    });
    if multi_on {
        let view = view.clone();
        window.defer(cx, move |_, cx| {
            view.update(cx, |s, cx| s.clear_multi(cx)).ok();
        });
    }
    let (Some(node), Some((changed, request_kind, scratch, paste_blocked))) = (node, facts) else {
        return menu;
    };
    let extra = changed.then_some(MenuEntry::ShowChanges);
    let mut prev = None;
    for &choice in menu_entries(node.kind).iter().chain(&extra) {
        let choice = request_kind.map_or(choice, |k| menu::for_request_kind(choice, k));
        let (view, node) = (view.clone(), node.clone());
        // Paste greys out with nothing cut or copied, or when it would put a folder into
        // itself; the scratch's row greys out what needs collection files.
        let blocked = (choice == MenuEntry::PasteItem && paste_blocked)
            || (scratch.as_deref() == Some(node.root.as_path()) && menu::scratch_blocked(choice));
        let line = ui::MenuLine::new(choice.label());
        let line = match choice.action() {
            Some(action) => line.action(action),
            None => line,
        };
        let line = line
            .disabled(blocked)
            .danger(choice.is_destructive())
            .group_break(prev.is_some_and(|p: MenuEntry| p.group() != choice.group()));
        prev = Some(choice);
        menu = line.add_to(
            menu,
            ui::SIDEBAR_ROW_KEYS,
            move |_, window, cx| {
                view.update(cx, |s, cx| s.choose(&node, choice, window, cx))
                    .ok();
            },
            cx,
        );
    }
    menu
}

/// The note under a scratch-only sidebar.
pub const SCRATCH_HINT: &str = "Unsaved requests; save to keep them in a collection";

// What a modifier click means: Some(true) toggles (secondary wins), Some(false) extends a range.
fn click_kind(m: &Modifiers) -> Option<bool> {
    if crate::shortcuts::is_secondary(m) {
        Some(true)
    } else {
        m.shift.then_some(false)
    }
}

/// The active selection as the rows see it while rendering: its collection, its top-level items
/// in sidebar order, and every member path.
struct DragSet {
    root: PathBuf,
    items: Vec<ItemPath>,
    members: std::collections::HashSet<ItemPath>,
}

/// Makes `item` a drop target with `zone`'s indicator drawn, and a drag source when its row
/// can move; invalid and example rows take neither part, and an Empty row is a target only.
fn drag_and_drop(
    item: ListItem,
    n: &Node,
    zone: Option<DropZone>,
    set: Option<&std::rc::Rc<DragSet>>,
    view: &WeakEntity<Sidebar>,
    cx: &App,
) -> ListItem {
    if matches!(
        n.kind,
        NodeKind::Invalid | NodeKind::Example | NodeKind::UnsavedGroup | NodeKind::Draft
    ) {
        return item;
    }
    let line = || {
        div()
            .absolute()
            .left_0()
            .right_0()
            .h(px(2.0))
            .bg(cx.theme().drag_border)
    };
    let item = match zone {
        Some(DropZone::Before) => item.relative().child(line().top_0()),
        Some(DropZone::After) => item.relative().child(line().bottom_0()),
        Some(DropZone::Inside) => item.bg(cx.theme().drop_target),
        None => item,
    };
    let item = if matches!(
        n.kind,
        NodeKind::Folder | NodeKind::Request | NodeKind::Opaque
    ) {
        // A member grabs the whole set; any other row ends the set when its drag starts.
        let member = set.filter(|s| s.root == n.root && s.members.contains(&n.path));
        let dragged = match member {
            Some(s) => {
                // A member under a selected folder travels as that folder.
                let grab = s
                    .items
                    .iter()
                    .find(|i| crate::stale::is_within(&n.path, i))
                    .cloned()
                    .unwrap_or_else(|| n.path.clone());
                DraggedRow::for_set(n.root.clone(), grab, &s.items)
            }
            None => DraggedRow::new(n.root.clone(), n.path.clone(), n.name.clone().into()),
        };
        let ends_set = set.is_some() && member.is_none();
        let owner = view.clone();
        item.on_drag(dragged, move |d: &DraggedRow, _, _, cx| {
            if ends_set {
                owner.update(cx, |s, cx| s.clear_multi(cx)).ok();
            }
            cx.new(|_| DragPreview(d.name.clone()))
        })
    } else {
        item
    };
    let (over, dropped, target) = (view.clone(), view.clone(), n.clone());
    item.on_drag_move(move |e: &DragMoveEvent<DraggedRow>, _, cx| {
        over.update(cx, |s, cx| s.drag_over(&target, e, cx)).ok();
    })
    .on_drop({
        let node = n.clone();
        move |d: &DraggedRow, _, cx| {
            dropped.update(cx, |s, cx| s.drop_on(&node, d, cx)).ok();
        }
    })
}

/// The chevron a folder row shows: down when expanded, right otherwise.
pub fn chevron_icon(expanded: bool) -> IconName {
    if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    }
}

/// The folder icon a folder row shows: open when expanded.
pub fn folder_icon(expanded: bool) -> IconName {
    if expanded {
        IconName::FolderOpen
    } else {
        IconName::Folder
    }
}

/// The borders of a selected row's overlay: a 2 px left edge and nothing else.
pub fn selection_edges() -> Edges<Pixels> {
    Edges {
        top: px(0.),
        right: px(0.),
        bottom: px(0.),
        left: px(2.),
    }
}

/// The selected row's edge colour: the accent while the tree holds focus, otherwise the muted
/// text colour, which stays visible on the selected fill in both themes.
pub fn selection_edge_color(tree_focused: bool, cx: &App) -> Hsla {
    let theme = cx.theme();
    if tree_focused {
        theme.primary
    } else {
        theme.muted_foreground
    }
}

/// The selected row's left edge, drawn with `selection_edges` widths in `selection_edge_color`.
fn selection_overlay(id: &SharedString, tree_focused: bool, cx: &App) -> impl IntoElement + use<> {
    let kind = if tree_focused { "focus-edge" } else { "edge" };
    let edges = selection_edges();
    div()
        .id(marker_id(kind, id))
        .test_support()
        .absolute()
        .inset_0()
        .map(|mut d| {
            let widths = &mut d.style().border_widths;
            widths.top = Some(edges.top.into());
            widths.right = Some(edges.right.into());
            widths.bottom = Some(edges.bottom.into());
            widths.left = Some(edges.left.into());
            d
        })
        .border_color(selection_edge_color(tree_focused, cx))
}

/// The outline of the row a right-click menu acts on, in the focus accent so it reads apart
/// from the selection's left edge.
fn menu_target_overlay(id: &SharedString, cx: &App) -> impl IntoElement + use<> {
    div()
        .id(marker_id("menu-target", id))
        .test_support()
        .absolute()
        .inset_0()
        .border_1()
        .border_color(selection_edge_color(true, cx))
}

/// A row's marker element, found in tests by `marker_id(kind, node)`.
fn marker(kind: &str, n: &Node) -> impl ParentElement + Styled + IntoElement + use<> {
    div().id(marker_id(kind, &n.id)).test_support().flex_none()
}

/// The examples chevron of a request row: flips the request's expansion without selecting or
/// opening it.
fn examples_toggle(n: &Node, view: &WeakEntity<Sidebar>) -> impl IntoElement + use<> {
    let (view, id) = (view.clone(), n.id.clone());
    let tip = if n.expanded {
        "Hide examples"
    } else {
        "Show examples"
    };
    ui::icon_button(
        marker_id("examples-toggle", &n.id),
        chevron_icon(n.expanded),
        tip,
        None,
    )
    .xsmall()
    // Off the Tab order; Right and Left on the row open and close its examples.
    .tab_stop(false)
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    .on_click(move |_, window, cx| {
        cx.stop_propagation();
        view.update(cx, |s, cx| s.toggle_expanded(&id, window, cx))
            .ok();
    })
}

/// The run button's tooltip: the registry label of `command` and the Shift+Enter key, which
/// runs the selected row from the tree.
pub fn run_tooltip(command: &str, cx: &App) -> String {
    ui::label_with_key(label(command), &RunSelected, cx)
}

/// A ghost run button `id` for collection or folder row `n`, tooltip the registry label of
/// `command`; it swallows the press and the click so the row neither toggles nor selects.
fn run_button(
    cx: &App,
    id: impl Into<ElementId>,
    command: &str,
    entry: MenuEntry,
    n: &Node,
    view: &WeakEntity<Sidebar>,
) -> Button {
    let (view, node) = (view.clone(), n.clone());
    // Off the Tab order so a big tree is one stop; Shift+Enter runs the selected row instead.
    ui::icon_button(id, IconName::Play, &run_tooltip(command, cx), None)
        .tab_stop(false)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            view.update(cx, |s, cx| s.choose(&node, entry, window, cx))
                .ok();
        })
}

/// Holds a row's actions, drawn while the row is `selected`, hovered, or an action holds
/// focus. The opacity sits on the slot, so a click on a transparent button still lands.
fn run_slot(actions: impl IntoElement, selected: bool) -> RunSlot {
    RunSlot {
        button: actions.into_any_element(),
        selected,
        hovered: false,
        overlay: false,
    }
}

#[derive(IntoElement)]
struct RunSlot {
    button: AnyElement,
    selected: bool,
    hovered: bool,
    overlay: bool,
}

impl RunSlot {
    /// Whether the pointer is over the row, which the slot cannot see at layout time.
    fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    /// Float the slot over the row's right edge instead of reserving width, so the name can
    /// use the space at rest; the slot paints the row's fill to cover what lies under it.
    fn overlay(mut self) -> Self {
        self.overlay = true;
        self
    }
}

impl RenderOnce for RunSlot {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // A focus handle that is no tab stop, only to see focus on the button.
        let focus = window
            .use_keyed_state("run-focus", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let shown = self.selected || self.hovered || focus.contains_focused(window, cx);
        let fill = if self.selected {
            cx.theme().list_active
        } else {
            cx.theme().list_hover
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            // While the button shows it takes its own room in the row, so the name ends in its
            // ellipsis before it; hidden, it floats and the name may run underneath.
            .when(self.overlay && !shown, |d| {
                d.absolute().right_0().top_0().bottom_0().bg(fill)
            })
            .when(!shown, |d| {
                d.opacity(0.).group_hover(ROW_GROUP, |s| s.opacity(1.))
            })
            .child(div().track_focus(&focus).child(self.button))
    }
}

/// An example row: its icon in the method column, then its name.
fn example_row(n: &Node) -> gpui_kit::Div {
    h_flex()
        .gap(style::SPACE_1)
        .min_w_0()
        .child(div().w(style::SPACE_4).flex_none())
        .child(
            marker("icon", n)
                .w(style::METHOD_COLUMN)
                .flex()
                .justify_end()
                .child(Icon::new(IconName::Bookmark).small()),
        )
        .child(div().truncate().child(n.name.clone()))
}

/// The expand chevron of a collection or folder row, centred in the leading slot.
fn chevron_slot(n: &Node) -> impl IntoElement + use<> {
    let slot = div().w(style::SPACE_4).flex_none().flex().justify_center();
    marker("chevron", n).child(slot.child(Icon::new(chevron_icon(n.expanded)).small()))
}

/// Says a request of `kind` cannot be sent yet. One note per kind, so repeated opens do not
/// stack copies of it.
pub(crate) fn show_unsupported(kind: Unsupported, window: &mut Window, cx: &mut App) {
    let notice = kind.notice();
    let note = ui::info_toast(notice)
        .id1::<Unsupported>(kind.tag())
        .content(move |_, _, _| {
            div()
                .id("unsupported-note")
                .test_support()
                .aria_label(notice)
                .into_any_element()
        });
    window.push_notification(note, cx);
}

/// The left part of a folder, request or opaque row. Folders lead with a chevron and their
/// icon; a request with examples puts its `toggle` in the chevron's slot; other rows leave
/// the slot empty, with the same gap, so folder icons and method tags start in one column.
fn item_row(
    n: &Node,
    toggle: Option<gpui_kit::AnyElement>,
    paints: &ui::TagPaints,
    muted: Hsla,
) -> gpui_kit::Div {
    let slot = || div().w(style::SPACE_4).flex_none().flex().justify_center();
    let lead = if n.kind == NodeKind::Folder {
        h_flex()
            .gap(style::SPACE_1)
            .child(chevron_slot(n))
            .child(marker("icon", n).child(Icon::new(folder_icon(n.expanded)).small()))
    } else {
        let method = match (n.method_tag(), n.unsupported) {
            // The kind tag of a request the app cannot send yet is named for assistive tools.
            (Some(tag), Some(kind)) => div()
                .id(marker_id("method", &n.id))
                .test_support()
                .flex_none()
                .aria_label(kind.tag())
                .child(tag.record(paints, n.id.clone()))
                .into_any_element(),
            (Some(tag), None) => marker("method", n)
                .child(tag.record(paints, n.id.clone()))
                .into_any_element(),
            (None, _) => div().w(style::METHOD_COLUMN).flex_none().into_any_element(),
        };
        h_flex()
            .gap(style::SPACE_1)
            .child(slot().children(toggle))
            .child(method)
    };
    h_flex()
        .gap(style::SPACE_1)
        .min_w_0()
        .child(lead)
        .child(
            marker("name", n)
                .flex_shrink(1.)
                .min_w_0()
                .child(div().truncate().child(n.name.clone())),
        )
        // The file name tells apart two sibling requests that share a name.
        .children(n.hint.clone().map(|hint| {
            marker("file", n)
                .flex_none()
                .text_xs()
                .text_color(muted)
                .child(hint)
        }))
}

/// A row's content: `lead` on the left and `row_suffix` for `n` at the right edge.
fn with_suffix(
    lead: gpui_kit::Div,
    n: &Node,
    git: &HashMap<PathBuf, Arc<GitChanges>>,
    rolled: bool,
    cx: &App,
) -> gpui_kit::Div {
    h_flex()
        .w_full()
        .justify_between()
        .gap(style::SPACE_1)
        .child(lead)
        .child(row_suffix(n, git_change(git, n), rolled, cx))
}

/// The git change of a request row's file or a folder row's `folder.yml`; other rows have none.
fn git_change<'a>(git: &'a HashMap<PathBuf, Arc<GitChanges>>, n: &Node) -> Option<&'a GitChange> {
    let folder = match n.kind {
        NodeKind::Request => false,
        NodeKind::Folder => true,
        _ => return None,
    };
    git.get(&n.root)?
        .get(&git_status::item_file(&n.path, folder))
}

/// The ids of the folder and collection rows with a changed or unsaved request or folder
/// somewhere inside, expanded or not.
fn rolled_up(nodes: &[Node], git: &HashMap<PathBuf, Arc<GitChanges>>) -> HashSet<SharedString> {
    // Whether anything under `nodes` is changed or unsaved, adding each ancestor on the way
    // back up so one pass covers every depth.
    fn walk(
        nodes: &[Node],
        git: &HashMap<PathBuf, Arc<GitChanges>>,
        out: &mut HashSet<SharedString>,
    ) -> bool {
        let mut any = false;
        for n in nodes {
            if !matches!(
                n.kind,
                NodeKind::Request | NodeKind::Folder | NodeKind::Collection
            ) {
                continue;
            }
            let inside = walk(&n.children, git, out);
            if inside && n.kind != NodeKind::Request {
                out.insert(n.id.clone());
            }
            // A collection's own flag is its settings file, which has no row of its own.
            let own = n.kind != NodeKind::Collection && (n.dirty || git_change(git, n).is_some());
            any |= inside || own;
        }
        any
    }
    let mut out = HashSet::new();
    walk(nodes, git, &mut out);
    out
}

/// The right edge of a row: a deleted or invalid file's disk marker, the git letter (named in
/// words by its label and tooltip so colour never carries it alone), then the dirty dot, in
/// the tabs' order.
fn row_suffix(n: &Node, change: Option<&GitChange>, rolled: bool, cx: &App) -> gpui_kit::Div {
    let text = |kind, label: &'static str, color| marker(kind, n).text_color(color).child(label);
    let disk = match n.disk {
        DiskState::Removed => Some(text("removed", "deleted", cx.theme().muted_foreground)),
        DiskState::Broken => Some(text("broken", "invalid", cx.theme().danger)),
        _ => None,
    };
    h_flex()
        .flex_none()
        .gap(style::SPACE_2)
        .children(disk)
        .children(rolled.then(|| ui::rollup_dot(marker_id("rollup", &n.id).into())))
        .children(change.map(|change| ui::git_letter(marker_id("git", &n.id).into(), change, cx)))
        .children(
            n.dirty
                .then(|| ui::dirty_dot(marker_id("dirty", &n.id)).test_support()),
        )
}

/// The collection's label by the shared rule, from the in-memory snapshot.
fn collection_name(root: &Path, snapshot: Option<&CollectionSnapshot>) -> String {
    let name = snapshot.and_then(|s| s.content.collection.name.as_deref());
    crate::workspace::label_or_dir(name, root).to_string()
}

/// Every node by id, without its children, for the row renderer and handlers.
fn index(nodes: &[Node], out: &mut HashMap<SharedString, Node>) {
    for n in nodes {
        index(&n.children, out);
        out.insert(
            n.id.clone(),
            Node {
                children: Vec::new(),
                ..n.clone()
            },
        );
    }
}
