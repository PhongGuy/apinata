//! The command palette: one registry naming every app action with its group, label and the
//! key context its binding lives in, and the mapping between registry entries and the kit's
//! grouped index paths, and the dialog that shows them. A "Requests" group after the
//! registry lists the open collections' requests.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use apinata_core::ItemPath;

use gpui_kit::base::actions::Cancel;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{ActiveTheme as _, IndexPath, WindowExt as _, h_flex};
use gpui_kit::{
    Action, AnyElement, App, AppContext as _, AsKeystroke as _, Entity, FocusHandle,
    Focusable as _, InteractiveElement, IntoElement as _, KeyContext, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, actions, div,
};

use crate::body_node::{
    AssertCaretJsonValue, CopyCaretJsonPath, CopyCaretJsonValue, SetCaretJsonVariable,
};
use crate::commands::registry;
use crate::keymap::{DiscardChanges, RedoActive, SaveActive, UndoActive};
use crate::sidebar::menu::MenuEntry;
use crate::sidebar::{
    CloneSelected, RenameSelected, RunSelected, ShowInFileManager, Sidebar, UndoDelete,
};
use crate::snapshot::RowKind;
use crate::style;
use crate::tab_area::{
    ActivateLastTab, ActivateNextTab, ActivatePrevTab, BeautifyBody, BeautifyQuery, CloseAllTabs,
    CloseOtherTabs, CloseSavedTabs, CloseTab, CloseTabsToTheRight, Compare, ConvertToGraphql,
    ConvertToHttp, CopyAsCurl, ExportHar, GenerateCode, MinifyBody, PinTab, ReopenClosedTab,
    RevealInSidebar, SaveAll, SaveAsExample, SendActive, StopActive, TabArea, UnpinTab,
};
use crate::workspace::Workspace;
use crate::zoom;

actions!(
    apinata,
    [
        /// Opens the command palette.
        OpenCommandPalette
    ]
);

/// A request the palette offers.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestEntry {
    /// The root of the collection the request belongs to.
    pub root: PathBuf,
    /// The request's path in its collection.
    pub path: ItemPath,
    /// The row's text, the request's name; also what the filter matches.
    pub label: String,
    /// The collection name and the folders holding the request, joined by " / ".
    pub detail: String,
}

/// Opens a request as a kept tab; the Shell handles it.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = apinata, no_json)]
pub struct OpenRequest {
    /// The root of the request's collection.
    pub root: PathBuf,
    /// The request's path in its collection.
    pub path: ItemPath,
}

/// Every request row of every open collection, collections in open order and rows in
/// snapshot order; folders and request types the app does not model are left out.
pub fn request_entries(ws: &Workspace, cx: &App) -> Vec<RequestEntry> {
    let mut entries = Vec::new();
    for collection in ws.collections() {
        let root = collection.root();
        let Some(snapshot) = ws.snapshot(root, cx) else {
            continue;
        };
        let name = ws.collection_name(root, cx);
        for row in snapshot.content.rows.iter() {
            if row.kind != RowKind::Request {
                continue;
            }
            // The detail names the folders by their directory segments, below the collection.
            let path = row.path.as_str();
            let folders = path.rsplit_once('/').map(|(dirs, _)| dirs);
            let detail = std::iter::once(name.as_ref())
                .chain(folders.into_iter().flat_map(|d| d.split('/')))
                .collect::<Vec<_>>()
                .join(" / ");
            entries.push(RequestEntry {
                root: root.to_path_buf(),
                path: row.path.clone(),
                label: row.name.clone(),
                detail,
            });
        }
    }
    entries
}

// The kit section of the Requests group: it follows every registry group.
fn requests_section() -> usize {
    PaletteGroup::ALL.len()
}

// The list's height cap, the kit list's row sizes and its padding at the 13 px UI base; the
// kit rows are rem-sized, so `list_height` scales them by the rem.
const PALETTE_LIST_HEIGHT: f32 = 315.;
// The query input's row above the list, its measured height at the base size rounded up.
const INPUT_ROW: f32 = 27.;
const HEADING_ROW: f32 = 26.;
const ITEM_ROW: f32 = 28.5;
const LIST_PADDING: f32 = 3.25;

/// A palette section; the variants are in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteGroup {
    /// Opening, creating and importing collections and requests.
    Collection,
    /// Acting on the open request or the sidebar selection.
    Request,
    /// Moving between and closing tabs.
    Tabs,
    /// Layout and window-level views.
    View,
    /// Environments of the current collection.
    Environment,
}

impl PaletteGroup {
    /// Every group, in display order.
    pub const ALL: [PaletteGroup; 5] = [
        PaletteGroup::Collection,
        PaletteGroup::Request,
        PaletteGroup::Tabs,
        PaletteGroup::View,
        PaletteGroup::Environment,
    ];

    /// The group's heading.
    pub fn label(self) -> &'static str {
        match self {
            PaletteGroup::Collection => "Collection",
            PaletteGroup::Request => "Request",
            PaletteGroup::Tabs => "Tabs",
            PaletteGroup::View => "View",
            PaletteGroup::Environment => "Environment",
        }
    }
}

/// One palette row: the action it runs and where its key binding lives.
pub struct PaletteEntry {
    /// The section the row appears in.
    pub group: PaletteGroup,
    /// The row's text, also what the filter matches.
    pub label: &'static str,
    /// The action dispatched when the row is chosen.
    pub action: Box<dyn Action>,
    /// The key context the action is bound in, for looking up its key hint.
    pub context: &'static str,
    /// The one pane the command works in, named beside its key; None when it works anywhere.
    pub area: Option<&'static str>,
}

/// Which state-dependent commands currently have something to do; the palette lists a
/// command only when its flag is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Availability {
    /// "Run selected requests": several rows, or a folder or collection row, are selected.
    pub run_selected: bool,
    /// "Close to the right": the active tab has an unpinned tab after it.
    pub close_to_right: bool,
    /// "Reveal in sidebar": the active tab has a sidebar row.
    pub reveal: bool,
    /// "Close other tabs": an unpinned tab other than the active one is open.
    pub close_others: bool,
    /// "Reopen closed tab": a closed tab is waiting.
    pub reopen_closed: bool,
    /// "Stop": the active tab's response is streaming.
    pub stop: bool,
    /// "Discard changes": the active tab holds edits to revert to a saved file.
    pub discard: bool,
    /// "Save all": some tab holds unsaved edits.
    pub save_all: bool,
    /// "Beautify" and "Minify": the active body is non-empty JSON or XML.
    pub beautify: bool,
    /// "Beautify query": the active request is GraphQL with a non-blank query.
    pub beautify_query: bool,
    /// "Close tab", Save, Undo and Redo: some tab is open.
    pub has_tabs: bool,
    /// Send, the copy and export commands and the JSON caret commands: the active tab is a
    /// request tab, a draft included.
    pub request_tab: bool,
    /// "Convert to GraphQL": the active tab is a saved HTTP request.
    pub to_graphql: bool,
    /// "Convert to HTTP": the active tab is a saved GraphQL request.
    pub to_http: bool,
    /// "Rename selected": no sidebar rename is already under way.
    pub rename_idle: bool,
    /// "Close all tabs": an unpinned tab is open.
    pub close_all: bool,
    /// "Pin tab": the active tab is not pinned.
    pub pin: bool,
    /// "Unpin tab": the active tab is pinned.
    pub unpin: bool,
    /// The collection-scoped commands: a collection, or scratch work, is open.
    pub collection_open: bool,
    /// "Rename selected": one row is selected and its menu offers Rename.
    pub rename: bool,
    /// "Duplicate selected": one row is selected and its menu offers Duplicate.
    pub duplicate: bool,
    /// "Show in file manager": one row is selected and its menu offers it.
    pub show_in_file_manager: bool,
    /// "Close saved tabs": an unpinned tab without unsaved edits is open.
    pub close_saved: bool,
    /// "Next tab" and "Previous tab": at least two tabs are open.
    pub multi_tab: bool,
    /// "Last tab": the active tab is not already the last one.
    pub not_last: bool,
    /// "Undo delete": a delete into the trash is still undoable.
    pub undo_delete: bool,
}

impl Availability {
    /// Every command available.
    pub fn all() -> Self {
        Availability {
            run_selected: true,
            close_to_right: true,
            reveal: true,
            close_others: true,
            reopen_closed: true,
            stop: true,
            discard: true,
            save_all: true,
            beautify: true,
            beautify_query: true,
            has_tabs: true,
            request_tab: true,
            to_graphql: true,
            to_http: true,
            rename_idle: true,
            close_all: true,
            pin: true,
            unpin: true,
            collection_open: true,
            rename: true,
            duplicate: true,
            show_in_file_manager: true,
            close_saved: true,
            multi_tab: true,
            not_last: true,
            undo_delete: true,
        }
    }
}

/// Commands beyond the app menu's COLLECTION_ONLY that also do nothing without a collection.
const ALSO_COLLECTION_SCOPED: [&str; 7] = [
    "collection.close",
    "collection.collapse_all",
    "collection.expand_all",
    "collection.new_folder",
    "collection.sort_by_name",
    "collection.run_folder",
    "view.search_collection",
];

/// The ids of the commands hidden while no collection is open: the app menu's list plus the
/// ones that only act on a collection's tree or environments.
pub fn collection_scoped_ids() -> Vec<&'static str> {
    (crate::app_menu::COLLECTION_ONLY.iter())
        .chain(&ALSO_COLLECTION_SCOPED)
        .chain(&[
            "environment.open",
            "environment.runtime_variables",
            "environment.open_selector",
            "environment.import",
            "environment.export",
            "environment.export_all",
            "request.rename",
            "request.duplicate",
            "request.show_in_file_manager",
        ])
        .copied()
        .collect()
}

impl Default for Availability {
    fn default() -> Self {
        Self::all()
    }
}

/// Every palette entry, grouped in `PaletteGroup::ALL` order: the registry's palette commands.
pub fn palette_items() -> Vec<PaletteEntry> {
    palette_items_for(Availability::all())
}

/// `palette_items`, leaving out the commands `available` marks as having nothing to do.
pub fn palette_items_for(available: Availability) -> Vec<PaletteEntry> {
    let hidden: Vec<&str> = [
        (available.run_selected, RunSelected.name()),
        (available.close_to_right, CloseTabsToTheRight.name()),
        (available.reveal, RevealInSidebar.name()),
        (available.close_others, CloseOtherTabs.name()),
        (available.reopen_closed, ReopenClosedTab.name()),
        (available.stop, StopActive.name()),
        (available.discard, DiscardChanges.name()),
        (available.save_all, SaveAll.name()),
        (available.beautify, BeautifyBody.name()),
        (available.beautify, MinifyBody.name()),
        (available.beautify_query, BeautifyQuery.name()),
        (available.has_tabs, CloseTab.name()),
        (available.has_tabs, SaveActive.name()),
        (available.has_tabs, UndoActive.name()),
        (available.has_tabs, RedoActive.name()),
        (available.request_tab, SendActive.name()),
        (available.request_tab, CopyAsCurl.name()),
        (available.to_graphql, ConvertToGraphql.name()),
        (available.to_http, ConvertToHttp.name()),
        (available.request_tab, GenerateCode.name()),
        (available.request_tab, SaveAsExample.name()),
        (available.request_tab, ExportHar.name()),
        (available.request_tab, Compare.name()),
        (available.request_tab, CopyCaretJsonValue.name()),
        (available.request_tab, CopyCaretJsonPath.name()),
        (available.request_tab, SetCaretJsonVariable.name()),
        (available.request_tab, AssertCaretJsonValue.name()),
        (available.close_all, CloseAllTabs.name()),
        (available.pin, PinTab.name()),
        (available.unpin, UnpinTab.name()),
        (
            available.rename && available.rename_idle,
            RenameSelected.name(),
        ),
        (available.duplicate, CloneSelected.name()),
        (available.show_in_file_manager, ShowInFileManager.name()),
        (available.close_saved, CloseSavedTabs.name()),
        (available.multi_tab, ActivateNextTab.name()),
        (available.multi_tab, ActivatePrevTab.name()),
        (available.not_last, ActivateLastTab.name()),
        (available.undo_delete, UndoDelete.name()),
    ]
    .into_iter()
    .filter(|(on, _)| !on)
    .map(|(_, name)| name)
    .collect();
    let no_collection = if available.collection_open {
        Vec::new()
    } else {
        collection_scoped_ids()
    };
    registry()
        .into_iter()
        .filter(|c| c.in_palette && !no_collection.contains(&c.id))
        .map(|c| {
            // Every registry command has at least one target.
            #[allow(clippy::expect_used)]
            let target = c
                .targets
                .into_iter()
                .next()
                .expect("a command has a target");
            PaletteEntry {
                group: c.group,
                label: c.label,
                action: target.action,
                context: target.context,
                area: crate::shortcuts::scope_label(c.scope),
            }
        })
        .filter(|e| !hidden.contains(&e.action.name()))
        .collect()
}

/// The kit index path of entry `ix`. The section is the group's place in `PaletteGroup::ALL`,
/// since the kit numbers a group with every command hidden too. Panics when `ix` is out of range.
pub fn index_path(items: &[PaletteEntry], ix: usize) -> IndexPath {
    let group = items[ix].group;
    // `PaletteGroup::ALL` lists every group.
    #[allow(clippy::expect_used)]
    let section = PaletteGroup::ALL
        .iter()
        .position(|g| *g == group)
        .expect("every group is in ALL");
    let row = items[..ix]
        .iter()
        .rev()
        .take_while(|e| e.group == group)
        .count();
    IndexPath::new(row).section(section)
}

/// The entry at a kit index path, or `None` when the path names no entry.
pub fn entry_index(items: &[PaletteEntry], path: IndexPath) -> Option<usize> {
    (0..items.len()).find(|&ix| index_path(items, ix).eq_row(path))
}

/// Opens the palette over `window`, recording the focused handle (else `fallback`) to
/// restore and dispatch from on confirm.
pub fn open(fallback: FocusHandle, window: &mut Window, cx: &mut App) {
    open_with(fallback, Vec::new(), None, None, None, window, cx);
}

/// `open` with a "Requests" group listing `requests` after the registry's groups;
/// `sidebar` and `workspace` are read live for which commands apply, and the palette redraws
/// when either changes; with none, their commands are always listed.
pub fn open_with(
    fallback: FocusHandle,
    requests: Vec<RequestEntry>,
    sidebar: Option<Entity<Sidebar>>,
    workspace: Option<Entity<Workspace>>,
    tab_area: Option<Entity<TabArea>>,
    window: &mut Window,
    cx: &mut App,
) {
    let target = window.focused(cx).unwrap_or(fallback);
    // The dialog builder runs every frame, so the requests are shared, not rebuilt.
    let requests: Rc<[RequestEntry]> = requests.into();
    // The state outlives the dialog's per-render rebuild, so the query and highlight persist.
    let state = cx.new(|cx| CommandState::new(window, cx));
    let dialog_state = state.clone();
    let rows_key = Rc::new(RowsKey::new());
    // The dialog layer redraws only with its host, so a selection change must refresh the window.
    if let Some(sidebar) = &sidebar {
        state.update(cx, |_, cx| {
            cx.observe_in(sidebar, window, |_, _, window, _| window.refresh())
                .detach();
        });
    }
    if let Some(workspace) = &workspace {
        state.update(cx, |_, cx| {
            cx.observe_in(workspace, window, |_, _, window, _| window.refresh())
                .detach();
        });
    }
    window.open_dialog(cx, move |dialog, window, cx| {
        // The key holds everything the rows are built from but the keymap's hint widths, so
        // an unchanged frame skips building them, and `palette` with them.
        let available =
            availability_now(sidebar.as_ref(), workspace.as_ref(), tab_area.as_ref(), cx);
        let (rem, cap) = (window.rem_size(), list_cap(window));
        let key = (dialog_state.read(cx).query(cx), available, rem, cap);
        let command = rows_key.render(&dialog_state, key, || {
            palette(&dialog_state, &requests, available, rem, cap, cx).on_confirm({
                let (target, requests) = (target.clone(), Rc::clone(&requests));
                let (sidebar, workspace) = (sidebar.clone(), workspace.clone());
                let tab_area = tab_area.clone();
                move |path, window, cx| {
                    let available = availability_now(
                        sidebar.as_ref(),
                        workspace.as_ref(),
                        tab_area.as_ref(),
                        cx,
                    );
                    confirm(path, &requests, available, &target, window, cx)
                }
            })
        });
        crate::dialog_frame::place(dialog, window)
            .w(style::resolve(style::PALETTE_WIDTH, window))
            .close_button(false)
            .child(
                div()
                    .id("command-palette")
                    .test_support()
                    .capture_action(escape_closes(&dialog_state))
                    .child(command),
            )
    });
    // The dialog took focus for itself; the query input takes it over so typing filters.
    state.read(cx).focus_handle(cx).focus(window, cx);
}

// Which commands have something to do right now; a missing sidebar or workspace means all of theirs.
fn availability_now(
    sidebar: Option<&Entity<Sidebar>>,
    workspace: Option<&Entity<Workspace>>,
    tab_area: Option<&Entity<TabArea>>,
    cx: &App,
) -> Availability {
    let (close_to_right, reveal) = workspace.map_or((true, true), |w| {
        let tab = crate::tab_area::active_tab_commands(w.read(cx).tabs());
        (tab.close_to_right, tab.reveal)
    });
    let tabs = tab_area.map(|t| t.read(cx).palette_flags(cx));
    let offers = |entry| sidebar.is_none_or(|s| s.read(cx).selection_offers(entry, cx));
    Availability {
        run_selected: sidebar.is_none_or(|s| s.read(cx).run_selected_available(cx)),
        close_to_right,
        reveal,
        close_others: tabs.is_none_or(|t| t.close_others),
        reopen_closed: tabs.is_none_or(|t| t.reopen_closed),
        stop: tabs.is_none_or(|t| t.stop),
        discard: tabs.is_none_or(|t| t.discard),
        save_all: tabs.is_none_or(|t| t.save_all),
        beautify: tabs.is_none_or(|t| t.beautify),
        beautify_query: tabs.is_none_or(|t| t.beautify_query),
        has_tabs: tabs.is_none_or(|t| t.has_tabs),
        request_tab: tabs.is_none_or(|t| t.request_tab),
        to_graphql: tabs.is_none_or(|t| t.to_graphql),
        to_http: tabs.is_none_or(|t| t.to_http),
        rename_idle: sidebar.is_none_or(|s| s.read(cx).renaming().is_none()),
        close_all: tabs.is_none_or(|t| t.close_all),
        pin: tabs.is_none_or(|t| t.pin),
        unpin: tabs.is_none_or(|t| t.unpin),
        rename: offers(MenuEntry::Rename),
        duplicate: offers(MenuEntry::CloneItem),
        show_in_file_manager: offers(MenuEntry::ShowInFileManager),
        close_saved: tabs.is_none_or(|t| t.close_saved),
        multi_tab: tabs.is_none_or(|t| t.multi_tab),
        not_last: tabs.is_none_or(|t| t.not_last),
        undo_delete: sidebar.is_none_or(|s| s.read(cx).can_undo_delete(cx)),
        collection_open: workspace.is_none_or(|w| !w.read(cx).shows_empty_state(cx)),
    }
}

/// A capture-phase Cancel handler that empties `state`'s query, so the kit's own Cancel then
/// propagates to the hosting dialog and one Escape closes it whatever was typed.
pub(crate) fn escape_closes(
    state: &gpui_kit::Entity<CommandState>,
) -> impl Fn(&Cancel, &mut Window, &mut App) + 'static {
    let state = state.clone();
    move |_, window, cx| state.update(cx, |s, cx| s.set_query("", window, cx))
}

// The kit calls this deferred, once its own confirm is done with the palette's state.
// Closing first lets the dialog restore its record, then focusing `target` makes the
// fallback win when nothing was focused; the dispatch waits a defer so it runs from there.
fn confirm(
    path: IndexPath,
    requests: &[RequestEntry],
    available: Availability,
    target: &FocusHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let items = palette_items_for(available);
    let action: Box<dyn Action> = if path.section == requests_section() {
        let Some(r) = requests.get(path.row) else {
            return;
        };
        Box::new(OpenRequest {
            root: r.root.clone(),
            path: r.path.clone(),
        })
    } else {
        let Some(ix) = entry_index(&items, path) else {
            return;
        };
        items[ix].action.boxed_clone()
    };
    window.close_dialog(cx);
    target.focus(window, cx);
    window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
}

// One kit group per palette group, then one for the requests; items carry no kit action,
// so confirming runs nothing of the kit's own.
fn palette(
    state: &gpui_kit::Entity<CommandState>,
    requests: &[RequestEntry],
    available: Availability,
    rem: Pixels,
    cap: f32,
    cx: &App,
) -> Command {
    let items = palette_items_for(available);
    // Row ids keep their place in the full list, so a hidden entry shifts none of them.
    let all = palette_items();
    let query = state.read(cx).query(cx);
    let mut command = Command::new(state)
        .bordered(false)
        .max_h(list_height(&items, requests, &query, rem, cap))
        .placeholder("Type a command");
    for group in PaletteGroup::ALL {
        let rows = items.iter().filter(|e| e.group == group).map(|e| {
            let id = all
                .iter()
                .position(|a| a.action.partial_eq(e.action.as_ref()));
            palette_row(id.unwrap_or_default(), e)
        });
        command = command.group(CommandGroup::new().label(group.label()).items(rows));
    }
    if !requests.is_empty() {
        let rows = requests.iter().enumerate().map(|(r, e)| request_row(r, e));
        command = command.group(CommandGroup::new().label("Requests").items(rows));
    }
    command
}

/// What a palette's rows were last built from. The kit re-measures every custom row each
/// time a new `Command` is installed, so a frame whose key is unchanged renders the state
/// alone, which keeps its installed model and measured rows.
pub(crate) struct RowsKey<K: PartialEq> {
    last: RefCell<Option<K>>,
}

impl<K: PartialEq> RowsKey<K> {
    pub(crate) fn new() -> Self {
        RowsKey {
            last: RefCell::new(None),
        }
    }

    /// `build()` when `key` differs from the last one rendered, else `state` as it stands.
    pub(crate) fn render(
        &self,
        state: &Entity<CommandState>,
        key: K,
        build: impl FnOnce() -> Command,
    ) -> AnyElement {
        // The borrow ends before `build` runs, so a builder can never meet it still held.
        {
            let mut last = self.last.borrow_mut();
            if last.as_ref() == Some(&key) {
                return state.clone().into_any_element();
            }
            *last = Some(key);
        }
        build().into_any_element()
    }
}

thread_local! {
    static ROW_BUILDS: Cell<usize> = const { Cell::new(0) };
}

/// How many palette rows have been built on this thread, counted by every palette row
/// builder; tests read its change to tell a re-measure from a reused layout.
#[doc(hidden)]
pub fn row_builds() -> usize {
    ROW_BUILDS.with(Cell::get)
}

// Called by each row builder, which the kit runs once per row it lays out or paints.
pub(crate) fn count_row_build() {
    ROW_BUILDS.with(|n| n.set(n.get() + 1));
}

/// The palette list's height cap in the window's pixels, for other palette-shaped dialogs.
pub(crate) fn list_max_height(window: &Window) -> Pixels {
    window.rem_size() * list_cap(window) / zoom::UI_FONT_SIZE
}

// The list's height cap at the base size: the usual cap, or less where the input and list
// would not fit the room above the status bar at the window's zoom.
fn list_cap(window: &Window) -> f32 {
    let room = f32::from(crate::dialog_frame::body_room(window));
    let base = room * zoom::UI_FONT_SIZE / f32::from(window.rem_size());
    PALETTE_LIST_HEIGHT.min(base - INPUT_ROW).max(0.)
}

// The list's height for `query` at `rem`: the top padding plus the longest run of whole rows
// that fits `cap`. Rows paint into the bottom padding, so it is left out unless all fit. The
// filter mirrors the kit's crate-private matcher; with no match the cap stays, for the kit's
// empty state.
// The fit is zoom-invariant, so it runs at the base size and only the result is scaled.
fn list_height(
    items: &[PaletteEntry],
    requests: &[RequestEntry],
    query: &str,
    rem: Pixels,
    cap: f32,
) -> Pixels {
    let query = query.trim().to_lowercase();
    let hits = |label: &str| label.to_lowercase().contains(&query);
    let counts = PaletteGroup::ALL
        .into_iter()
        .map(|group| {
            (items.iter())
                .filter(|e| e.group == group && hits(e.label))
                .count()
        })
        .chain([requests.iter().filter(|e| hits(&e.label)).count()]);
    let rows = counts.flat_map(|matches| {
        let heading = (matches > 0).then_some(HEADING_ROW);
        heading
            .into_iter()
            .chain(std::iter::repeat_n(ITEM_ROW, matches))
    });
    let room = cap - LIST_PADDING;
    let mut used = 0.;
    let mut cut = false;
    for row in rows {
        if used + row > room {
            cut = true;
            break;
        }
        used += row;
    }
    // When every row fits, the bottom padding is kept too so nothing is left to scroll.
    let base = match (used == 0., cut) {
        (true, _) => cap,
        (false, false) => (2. * LIST_PADDING + used).min(cap),
        (false, true) => LIST_PADDING + used,
    };
    rem * base / zoom::UI_FONT_SIZE
}

// A row shows its label and the key bound in the entry's own context, so a key that only
// works inside the request editor still shows while the palette has focus.
fn palette_row(ix: usize, entry: &PaletteEntry) -> CommandItem {
    let (label, context, area) = (entry.label, entry.context, entry.area);
    let action = entry.action.boxed_clone();
    CommandItem::new().label(label).child(move |window, cx| {
        count_row_build();
        let key = KeyContext::parse(context)
            .ok()
            .and_then(|ctx| {
                window.highest_precedence_binding_for_action_in_context(action.as_ref(), ctx)
            })
            .and_then(|b| b.keystrokes().first().map(|k| k.as_keystroke().clone()));
        h_flex().flex_1().justify_between().child(label).child(
            h_flex()
                .gap(style::SPACE_1)
                .children(area.map(|area| {
                    div()
                        .id(("palette-scope", ix))
                        .aria_label(area)
                        .test_support()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(area)
                }))
                .children(key.map(|k| {
                    div()
                        .id(("palette-kbd", ix))
                        .aria_label(crate::shortcuts::unparse(&k))
                        .test_support()
                        .child(Kbd::new(k))
                })),
        )
    })
}

// A request row shows its name, with its collection and folders muted on the right where a
// command row shows its key.
fn request_row(r: usize, entry: &RequestEntry) -> CommandItem {
    let (label, detail) = (entry.label.clone(), entry.detail.clone());
    CommandItem::new().label(label.clone()).child(move |_, cx| {
        count_row_build();
        h_flex()
            .flex_1()
            .justify_between()
            .child(label.clone())
            .child(
                div()
                    .id(("palette-request-detail", r))
                    .text_color(cx.theme().muted_foreground)
                    .child(detail.clone()),
            )
    })
}
