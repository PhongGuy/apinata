//! The tab bar over the open request and settings tabs and the editor of the active one.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use apinata_core::{ItemPath, NetworkEntry, RequestKind, RequestResult, SendSource};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, Anchor, AnyElement, AnyView, App, Bounds, DragMoveEvent, Empty, Entity, FocusHandle,
    Focusable, MouseButton, Pixels, Role, ScrollHandle, SharedString, StyleRefinement,
    Subscription, TestSupportExt, Window, actions, div, relative, rems,
};

use crate::cached_part::{Part, PartOwner, Slot};
use crate::changes_dialog::open_changes;
use crate::closing;
use crate::command::{Command, Target, TreeOp};
use crate::commands::label;
use crate::config_editor::{ConfigEditor, ConfigTarget};
use crate::confirm;
use crate::draft_store;
use crate::env_editor::EnvEditor;
use crate::example_view::ExampleView;
use crate::git_status::{GitChange, item_file};
use crate::har_export;
use crate::keymap::{
    DiscardChanges, FocusResponse, FocusUrl, NewRequest, NextSection, OpenEnvironments,
    PreviousSection, RedoActive, SHELL_CONTEXT, SaveActive, UndoActive,
};
use crate::menu_focus::{self, Before};
use crate::palette::OpenCommandPalette;
use crate::request_editor::{RequestEditor, SendRequest};
use crate::runner_view::RunnerView;
use crate::save_draft;
use crate::save_example;
use crate::send_model::SendState;
use crate::sidebar::FocusFilter;
use crate::style::{self, ROW_HEIGHT, SPACE_1, TAB_MAX_WIDTH};
use crate::tabs::{Side, TabEntry, TabId, TabKind, Tabs, tab_details, tab_place};
use crate::ui;
use crate::workspace::{Workspace, WorkspaceEvent, label_or_dir};
use crate::zoom;
use gpui_kit::component::WindowExt as _;

/// The toast when Compare response has no previous send or example to compare with.
pub const NOTHING_TO_COMPARE: &str =
    "Nothing to compare yet: send the request or save an example first";

actions!(
    apinata,
    [
        /// Closes the active tab, asking first when it is dirty.
        CloseTab,
        /// Closes every unpinned tab, asking once for the dirty ones.
        CloseAllTabs,
        /// Closes every tab but the active one and the pinned ones, asking once for the
        /// dirty ones.
        CloseOtherTabs,
        /// Closes the unpinned tabs right of the active one, asking once for the dirty ones.
        CloseTabsToTheRight,
        /// Selects the active tab's item in the sidebar, opening its folders.
        RevealInSidebar,
        /// Closes every unpinned tab without unsaved changes.
        CloseSavedTabs,
        /// Pins the active tab to the front of the strip.
        PinTab,
        /// Unpins the active tab.
        UnpinTab,
        /// Reopens the most recently closed tab.
        ReopenClosedTab,
        /// Activates the last tab.
        ActivateLastTab,
        /// Activates the tab to the left, wrapping around.
        ActivatePrevTab,
        /// Activates the tab to the right, wrapping around.
        ActivateNextTab,
        /// Saves every dirty request that has an open tab.
        SaveAll,
        /// Sends the active tab's request, wherever focus is in the window.
        SendActive,
        /// Copies the active tab's request as a curl command.
        CopyAsCurl,
        /// Stops the active tab's open response, keeping what arrived.
        StopActive,
        /// Opens the Generate Code dialog on the active tab's request.
        GenerateCode,
        /// Re-indents the active request's JSON or XML body.
        BeautifyBody,
        /// Strips the whitespace from the active request's JSON or XML body.
        MinifyBody,
        /// Formats the active GraphQL request's query.
        BeautifyQuery,
        /// Saves the active request tab's shown response as an example of that request.
        SaveAsExample,
        /// Exports the active request tab's shown response, with its request, as a HAR file.
        ExportHar,
        /// Opens the Compare tab on the active request's shown response.
        Compare,
        /// Rewrites the active saved HTTP request as a GraphQL request.
        ConvertToGraphql,
        /// Rewrites the active saved GraphQL request as an HTTP request.
        ConvertToHttp
    ]
);

/// Activates the tab at a position, counted from zero.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = apinata, no_json)]
pub struct ActivateTab(pub usize);

/// What a tab's context menu offers, in menu order.
#[derive(Clone, Copy)]
enum MenuChoice {
    Close,
    CloseOthers,
    CloseToRight,
    CloseSaved,
    CloseAll,
    Reveal,
}

impl MenuChoice {
    const ALL: [MenuChoice; 6] = [
        MenuChoice::Close,
        MenuChoice::CloseOthers,
        MenuChoice::CloseToRight,
        MenuChoice::CloseSaved,
        MenuChoice::CloseAll,
        MenuChoice::Reveal,
    ];

    fn label(self) -> &'static str {
        match self {
            MenuChoice::Close => "Close",
            MenuChoice::CloseOthers => label("tabs.close_others"),
            MenuChoice::CloseToRight => label("tabs.close_to_right"),
            MenuChoice::CloseSaved => label("tabs.close_saved"),
            MenuChoice::CloseAll => label("tabs.close_all"),
            MenuChoice::Reveal => label("tabs.reveal_in_sidebar"),
        }
    }
}

/// The labels of a tab's context menu, in menu order.
pub fn tab_menu_labels() -> [&'static str; 6] {
    MenuChoice::ALL.map(MenuChoice::label)
}

/// The sidebar item a tab reveals, if it has one: a request or example its request, a folder
/// or runner its folder, the collection (or a root runner) its root row.
fn reveal_path(kind: &TabKind) -> Option<ItemPath> {
    match kind {
        TabKind::Request(item) | TabKind::Example { item, .. } => Some(item.clone()),
        TabKind::Folder(path) | TabKind::Runner(path) => Some(path.clone()),
        TabKind::Collection => Some(ItemPath::root()),
        TabKind::Environments | TabKind::RunSelection(_) => None,
    }
}

/// The unpinned tabs after `from` in the strip; the tab menu and the palette both ask it
/// whether "Close to the right" has anything to close.
pub(crate) fn tabs_right_of(tabs: &Tabs, from: TabId) -> Vec<TabId> {
    let entries = tabs.entries();
    let Some(at) = entries.iter().position(|e| e.id == from) else {
        return Vec::new();
    };
    (entries[at + 1..].iter())
        .filter(|e| !e.pinned)
        .map(|e| e.id)
        .collect()
}

/// The palette commands the open tabs allow, beyond [`TabCommands`].
#[derive(Clone, Copy)]
pub(crate) struct TabPaletteFlags {
    pub close_others: bool,
    pub reopen_closed: bool,
    pub stop: bool,
    pub discard: bool,
    pub save_all: bool,
    pub beautify: bool,
    pub beautify_query: bool,
    pub has_tabs: bool,
    pub request_tab: bool,
    pub to_graphql: bool,
    pub to_http: bool,
    pub close_all: bool,
    pub pin: bool,
    pub unpin: bool,
    pub close_saved: bool,
    pub multi_tab: bool,
    pub not_last: bool,
}

/// Which tab commands apply to the active tab, as the tab menu decides for its own tab.
#[derive(Clone, Copy)]
pub(crate) struct TabCommands {
    pub close_to_right: bool,
    pub reveal: bool,
}

/// The commands the active tab allows; with no active tab neither applies.
pub(crate) fn active_tab_commands(tabs: &Tabs) -> TabCommands {
    let Some(active) = tabs.active() else {
        return TabCommands {
            close_to_right: false,
            reveal: false,
        };
    };
    TabCommands {
        close_to_right: !tabs_right_of(tabs, active.id).is_empty(),
        reveal: reveal_path(&active.kind).is_some(),
    }
}

/// A save sent on close: the tab closes once the snapshot acknowledging `command`, the last
/// of its saves, shows every target of the tab clean.
struct PendingSave {
    tab: TabId,
    root: PathBuf,
    command: u64,
}

/// The editor a tab shows, by tab kind.
enum TabView {
    Request(Entity<RequestEditor>),
    Config(Entity<ConfigEditor>),
    Env(Entity<EnvEditor>),
    Runner(Entity<RunnerView>),
    Example(Entity<ExampleView>),
}

impl TabView {
    // Where focus lands when the tab is handed focus: a request's URL input, which draws the
    // ring, else the view's own handle.
    fn stop(&self, cx: &App) -> FocusHandle {
        match self {
            TabView::Request(e) => e.read(cx).url_focus(cx),
            view => view.focus_handle(cx),
        }
    }

    fn element(&self) -> AnyView {
        match self {
            TabView::Request(e) => e.clone().into(),
            TabView::Config(e) => e.clone().into(),
            TabView::Env(e) => e.clone().into(),
            TabView::Runner(e) => e.clone().into(),
            TabView::Example(e) => e.clone().into(),
        }
    }

    // The view's own focus target.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            TabView::Request(e) => e.focus_handle(cx),
            TabView::Config(e) => e.focus_handle(cx),
            TabView::Env(e) => e.focus_handle(cx),
            TabView::Runner(e) => e.focus_handle(cx),
            TabView::Example(e) => e.focus_handle(cx),
        }
    }
}

/// The payload of a tab drag: the tab, its collection and the label the drag preview shows.
#[derive(Clone)]
pub struct DraggedTab {
    id: TabId,
    root: PathBuf,
    label: SharedString,
}

impl DraggedTab {
    /// A drag of tab `id` of the collection at `root`, previewed as `label`.
    pub fn new(id: TabId, root: PathBuf, label: SharedString) -> Self {
        DraggedTab { id, root, label }
    }
}

/// The side of a tab's `bounds` a pointer at `x` is on: a drop lands before it on the left half.
pub fn drop_side(x: Pixels, bounds: Bounds<Pixels>) -> Side {
    if x < bounds.center().x {
        Side::Before
    } else {
        Side::After
    }
}

/// The label that follows the pointer while a tab is dragged.
struct TabDragPreview(SharedString);

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(SPACE_1)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.0.clone())
    }
}

/// The workspace's tabs and one cached editor per tab that has been shown, with the kind
/// it was built for.
pub struct TabArea {
    workspace: Entity<Workspace>,
    editors: HashMap<TabId, (TabKind, TabView)>,
    pending: Vec<PendingSave>,
    // Tabs whose close waits for an actor, so a repeated close asks nothing a second time.
    settling: HashSet<TabId>,
    hovered: Option<TabId>,
    // Where a dragged tab would land: the tab under the pointer and the side of it.
    drop: Option<(TabId, Side)>,
    // What held focus when a tab's right press began, for the menu it opens to give back.
    // The tab whose view held it, so a restore never targets a tab closed meanwhile.
    before_menu: Option<(FocusHandle, Option<TabId>)>,
    scroll: ScrollHandle,
    reveal: ui::Reveal,
    revealed: Option<TabId>,
    fades: ui::StripFades,
    focus: FocusHandle,
    // Where focus goes once no tab is open: the sidebar's tree, set by the shell.
    empty_focus: Option<Entity<gpui_kit::component::tree::TreeState>>,
    // The active tab as of the last change, so a switch can close the Compare tab left behind.
    last_active: Option<TabId>,
    strip: Slot<TabArea, StripInputs>,
    _observer: Subscription,
    _focus_lost: Subscription,
    _drafts: Subscription,
}

impl TabArea {
    /// A tab area over the tabs of `workspace`.
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observer = cx.observe_in(&workspace, window, |this, _, window, cx| {
            this.changed(window, cx)
        });
        let focus_lost = cx.on_focus_lost(window, Self::focus_lost);
        let drafts = cx.subscribe_in(&workspace, window, Self::workspace_event);
        TabArea {
            workspace,
            editors: HashMap::new(),
            last_active: None,
            pending: Vec::new(),
            settling: HashSet::new(),
            hovered: None,
            drop: None,
            before_menu: None,
            scroll: ScrollHandle::new(),
            reveal: ui::Reveal::default(),
            revealed: None,
            fades: ui::StripFades::default(),
            focus: cx.focus_handle(),
            empty_focus: None,
            strip: Slot::new(cx.weak_entity(), (), false, cx),
            _observer: observer,
            _focus_lost: focus_lost,
            _drafts: drafts,
        }
    }

    /// The request editor of tab `id`, once that request tab has been shown.
    pub fn editor(&self, id: TabId) -> Option<&Entity<RequestEditor>> {
        match self.editors.get(&id).map(|(_, v)| v)? {
            TabView::Request(e) => Some(e),
            _ => None,
        }
    }

    /// The request editor of tab `tab`, building its view first when the tab has not been
    /// shown yet; `None` for a missing tab or one that is not a request.
    fn ensure_editor(
        &mut self,
        tab: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<&Entity<RequestEditor>> {
        if !self.editors.contains_key(&tab) {
            let entry = self.workspace.read(cx).tabs().get(tab).cloned()?;
            let view = self.view_for(&entry, window, cx);
            self.editors.insert(tab, (entry.kind, view));
        }
        self.editor(tab)
    }

    /// Shows `section` in request tab `tab`'s editor, building it now when the tab has not
    /// been shown yet; does nothing for a tab that is not a request.
    pub fn show_section(
        &mut self,
        tab: TabId,
        section: crate::request_editor::Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.ensure_editor(tab, window, cx).cloned() {
            editor.update(cx, |e, cx| e.show(section, cx));
        }
    }

    /// Shows kept send `seq` in request tab `tab`'s response pane, building its editor now
    /// when the tab has not been shown yet; does nothing for a tab that is not a request.
    pub fn show_kept_send(
        &mut self,
        tab: TabId,
        seq: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.ensure_editor(tab, window, cx) else {
            return;
        };
        let pane = editor.read(cx).response().clone();
        pane.update(cx, |p, cx| p.show_history(seq, cx));
    }

    /// Shows `result` in request tab `tab`'s response pane, building its editor now when the
    /// tab has not been shown yet; does nothing for a tab that is not a request.
    pub fn show_result(
        &mut self,
        tab: TabId,
        result: Arc<RequestResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = (self.workspace.read(cx).tabs().get(tab)).map(|e| e.root.clone()) else {
            return;
        };
        let Some(editor) = self.ensure_editor(tab, window, cx) else {
            return;
        };
        let send = editor.read(cx).send_model().clone();
        send.update(cx, |m, cx| m.show_at(&root, result, cx));
    }

    /// The settings editor of tab `id`, once that folder or collection tab has been shown.
    pub fn config_editor(&self, id: TabId) -> Option<&Entity<ConfigEditor>> {
        match self.editors.get(&id).map(|(_, v)| v)? {
            TabView::Config(e) => Some(e),
            _ => None,
        }
    }

    /// The environments editor of tab `id`, once that environments tab has been shown.
    pub fn env_editor(&self, id: TabId) -> Option<&Entity<EnvEditor>> {
        match self.editors.get(&id).map(|(_, v)| v)? {
            TabView::Env(e) => Some(e),
            _ => None,
        }
    }

    /// The models of the runner tabs whose run is planning or running.
    pub fn running_runs(&self, cx: &App) -> Vec<Entity<crate::runner_model::RunnerModel>> {
        (self.editors.values())
            .filter_map(|(_, v)| match v {
                TabView::Runner(e) => Some(e.read(cx).model().clone()),
                _ => None,
            })
            .filter(|m| m.read(cx).is_running())
            .collect()
    }

    /// The runner view of tab `id`; made as soon as the runner tab opens.
    pub fn runner(&self, id: TabId) -> Option<&Entity<RunnerView>> {
        match self.editors.get(&id).map(|(_, v)| v)? {
            TabView::Runner(e) => Some(e),
            _ => None,
        }
    }

    /// The example view of tab `id`, once that example tab has been shown.
    pub fn example_view(&self, id: TabId) -> Option<&Entity<ExampleView>> {
        match self.editors.get(&id).map(|(_, v)| v)? {
            TabView::Example(e) => Some(e),
            _ => None,
        }
    }

    /// Closes the tabs `ids`. When any of them is dirty, one prompt asks Save, Don't save or
    /// Cancel first; Save closes each dirty tab only once its save leaves it clean, and a
    /// dirty draft is first named in the Save dialog. A closed draft is discarded.
    pub fn close(&mut self, ids: Vec<TabId>, window: &mut Window, cx: &mut Context<Self>) {
        // Keystrokes the actor has not applied yet are not in any snapshot, so wait for it
        // as a window close does; a window close prompt already showing owns the wait.
        let roots = self.roots_of(&ids, cx);
        if ids.iter().any(|id| self.settling.contains(id)) {
            return;
        }
        if !closing::prompting(window, cx) && closing::roots_behind(&self.workspace, &roots, cx) {
            let (area, workspace) = (cx.entity(), self.workspace.clone());
            self.settling.extend(ids.iter().copied());
            closing::settle_roots(workspace, roots, window, cx, move |window, cx| {
                area.update(cx, |a, cx| {
                    ids.iter().for_each(|id| _ = a.settling.remove(id));
                    a.close_settled(ids, window, cx)
                })
            });
            return;
        }
        self.close_settled(ids, window, cx);
    }

    fn roots_of(&self, ids: &[TabId], cx: &App) -> Vec<PathBuf> {
        let tabs = self.workspace.read(cx).tabs();
        let mut roots: Vec<PathBuf> = ids
            .iter()
            .filter_map(|&id| tabs.get(id).map(|e| e.root.clone()))
            .collect();
        roots.sort();
        roots.dedup();
        roots
    }

    // The close flow on what the actors published. A collection still behind after the wait
    // counts as dirty, so the prompt covers the edit that has not landed.
    fn close_settled(&mut self, ids: Vec<TabId>, window: &mut Window, cx: &mut Context<Self>) {
        // Held typing goes through the Save check first, and the store's write settles again.
        let held = self.env_editors(&ids, cx);
        let held: Vec<_> = held
            .into_iter()
            .filter(|editor| editor.read(cx).holds_typing(cx))
            .collect();
        // Environment secret typing is stored now; an OAuth secret waits for the prompt's Save.
        let envs: Vec<TabId> = (ids.iter().copied())
            .filter(|id| matches!(self.editors.get(id), Some((_, TabView::Env(_)))))
            .collect();
        if !held.is_empty() {
            return self.check_replace(held, false, window, cx, move |go, area, window, cx| {
                if go {
                    area.store_pending_secrets(&envs, false, cx);
                    area.close(ids, window, cx);
                }
            });
        }
        self.store_pending_secrets(&envs, false, cx);
        // A typed OAuth secret counts as unsaved but stays in its field until Save.
        let dirty: Vec<TabEntry> = ids
            .iter()
            .filter_map(|&id| self.workspace.read(cx).tabs().get(id).cloned())
            .filter(|e| {
                self.holds_secret_typing(e.id, cx)
                    || self.is_dirty(e, cx)
                    || closing::roots_behind(&self.workspace, std::slice::from_ref(&e.root), cx)
            })
            .collect();
        if dirty.is_empty() {
            let active = self.workspace.read(cx).tabs().active().map(|e| e.id);
            self.close_now(&ids, cx);
            // Closing the active tab hands focus to the tab now in front, or to the tree, even
            // when a click left focus on the tree or the shell, where no focus-lost event follows.
            if active.is_some_and(|id| ids.contains(&id)) || window.focused(cx).is_none() {
                self.focus_fallback(window, cx);
            }
            return;
        }
        let message = match &dirty[..] {
            [one] => format!("Save changes to \"{}\"?", self.tab_label(one, cx)),
            many => format!("Save changes to {} tabs?", many.len()),
        };
        closing::hold(window, cx);
        let answer = confirm::ask(
            confirm::save_ask(message.into(), None, "Don't save", "Save"),
            window,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = answer.recv().await;
            this.update_in(cx, |area, window, cx| {
                match choice {
                    // Save ends the hold itself, once its Replace check has answered.
                    Ok(2) => return area.save_and_close(ids, dirty, window, cx),
                    Ok(1) => area.revert_and_close(ids, dirty, cx),
                    _ => {}
                }
                closing::unhold(window, cx);
            })
            .ok();
        })
        .detach();
    }

    // Clean tabs close now; each dirty one is saved and waits for its acknowledgement. Dirty
    // drafts are left to `save_drafts_and_close`, which names them one dialog at a time. The
    // close prompt's hold ends once the Replace check answers, whatever the answer.
    fn save_and_close(
        &mut self,
        ids: Vec<TabId>,
        dirty: Vec<TabEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let clean: Vec<TabId> = ids
            .into_iter()
            .filter(|id| dirty.iter().all(|d| d.id != *id))
            .collect();
        let typed: Vec<TabId> = dirty.iter().map(|e| e.id).collect();
        let (drafts, dirty): (Vec<TabEntry>, Vec<TabEntry>) =
            dirty.into_iter().partition(|e| draft_of(e).is_some());
        // The prompt held back typing in environment rows and OAuth secrets; Save now owns it,
        // and each secret's edit goes out ahead of its tab's save.
        let saving: Vec<TabId> = dirty.iter().map(|e| e.id).collect();
        self.save_check(&saving, window, cx, move |go, area, window, cx| {
            if go {
                let owed = area.store_pending_secrets(&typed, true, cx);
                area.send_and_close(clean, dirty, drafts, owed, window, cx);
            }
            closing::unhold(window, cx);
        });
    }

    fn send_and_close(
        &mut self,
        mut clean: Vec<TabId>,
        dirty: Vec<TabEntry>,
        drafts: Vec<TabEntry>,
        owed: HashSet<TabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for entry in dirty {
            match self.send_owed(&entry, Command::Save, owed.contains(&entry.id), cx) {
                Some(command) => self.pending.push(PendingSave {
                    tab: entry.id,
                    root: entry.root,
                    command,
                }),
                // Saved elsewhere while the prompt was open.
                None if !self.is_dirty(&entry, cx) => clean.push(entry.id),
                None => {}
            }
        }
        self.close_now(&clean, cx);
        self.save_drafts_and_close(drafts, window, cx);
    }

    // One dialog at a time, over the draft's own tab: each answer is awaited before the next
    // dialog opens. A named draft closes like a saved file, once its acknowledgement shows it
    // clean; a cancelled dialog keeps that tab open and the walk moves on.
    fn save_drafts_and_close(
        &mut self,
        drafts: Vec<TabEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if drafts.is_empty() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            for entry in drafts {
                let Some(draft) = draft_of(&entry).cloned() else {
                    continue;
                };
                let asked = this.update_in(cx, |area, window, cx| {
                    area.tabs_action(cx, |t| {
                        t.activate(entry.id);
                    });
                    save_draft::ask_for(&area.workspace, &entry.root, draft.clone(), window, cx)
                });
                let Ok(Some(answer)) = asked else {
                    continue;
                };
                let Ok(Some(save_as)) = answer.recv().await else {
                    continue;
                };
                this.update(cx, |area, cx| {
                    let texts = area.draft_texts(&entry.root, &draft, cx);
                    let sent = save_draft::send_with(
                        &area.workspace,
                        &entry.root,
                        draft,
                        save_as,
                        texts.clone(),
                        cx,
                    );
                    area.workspace.update(cx, |w, _| {
                        w.approve_when_saved(&entry.root, sent, None, texts)
                    });
                    if let Some(command) = sent {
                        area.pending.push(PendingSave {
                            tab: entry.id,
                            root: entry.root,
                            command,
                        });
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    // A draft has nothing on disk to revert to; `close_now` discards it.
    fn revert_and_close(&mut self, ids: Vec<TabId>, dirty: Vec<TabEntry>, cx: &mut Context<Self>) {
        for entry in dirty.iter().filter(|e| draft_of(e).is_none()) {
            self.send_each(entry, Command::Revert, cx);
        }
        self.close_now(&ids, cx);
    }

    // A tab that still shows a draft takes the draft with it when it closes.
    fn close_now(&mut self, ids: &[TabId], cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        // The history key is read before `Tabs::close` drops it; it names the draft's file.
        let tabs = self.workspace.read(cx).tabs();
        let drafts: Vec<(PathBuf, ItemPath, Option<ItemPath>)> = ids
            .iter()
            .filter_map(|&id| tabs.get(id))
            .filter_map(|e| {
                let key = tabs.history_key(e.id).cloned();
                Some((e.root.clone(), draft_of(e)?.clone(), key))
            })
            .collect();
        let mut keys = Vec::new();
        for (root, draft, key) in drafts {
            let op = TreeOp::DiscardDraft { draft };
            self.workspace
                .update(cx, |w, cx| w.send(&root, Command::Tree(op), cx));
            keys.extend(key);
        }
        self.workspace.update(cx, |w, cx| {
            w.update_tabs(cx, |t| {
                for &id in ids {
                    t.close(id);
                }
            })
        });
        // A key a dirty tab still open holds keeps its file, so the keys go once the tabs do.
        for key in keys {
            if !self.workspace.read(cx).draft_key_held_dirty(&key, cx) {
                draft_store::discard(&key, cx);
            }
        }
    }

    // Each workspace change settles the saves it acknowledged (a still dirty target is a
    // failed save, so its tab stays) and drops the editors of tabs that are gone. A settings
    // or runner view whose folder was renamed is dropped too and rebuilt on the new path;
    // request editors follow their tab by id and are kept. Runner views are made at once, so
    // a run can start before the tab is first drawn.
    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ws = self.workspace.read(cx);
        // Runs on every notify, so it is one id compare and acts only on a real switch: the
        // Compare tab of the request left behind closes.
        let active = ws.tabs().active().map(|e| e.id);
        if active != self.last_active {
            let left = std::mem::replace(&mut self.last_active, active);
            let pane = left
                .and_then(|id| self.editor(id))
                .map(|e| e.read(cx).response().clone());
            if let Some(pane) = pane {
                pane.update(cx, |p, cx| p.close_compare(cx));
            }
        }
        let ws = self.workspace.read(cx);
        let mut close = Vec::new();
        self.pending.retain(|p| {
            let Some(entry) = ws.tabs().get(p.tab) else {
                return false;
            };
            let acked = ws
                .snapshot(&p.root, cx)
                .is_some_and(|s| s.acked >= p.command);
            if acked && dirty_targets(ws, entry, cx).is_empty() {
                close.push(p.tab);
            }
            !acked
        });
        self.editors.retain(|id, (kind, view)| {
            ws.tabs().get(*id).is_some_and(|e| {
                !matches!(view, TabView::Config(_) | TabView::Runner(_)) || e.kind == *kind
            })
        });
        let runners: Vec<TabEntry> = ws
            .tabs()
            .entries()
            .iter()
            .filter(|e| {
                matches!(e.kind, TabKind::Runner(_) | TabKind::RunSelection(_))
                    && !self.editors.contains_key(&e.id)
            })
            .cloned()
            .collect();
        for entry in runners {
            let view = self.view_for(&entry, window, cx);
            self.editors.insert(entry.id, (entry.kind, view));
        }
        self.close_now(&close, cx);
        cx.notify();
    }

    /// Builds tab `id`'s view now if it is not yet, and focuses its first stop. False when no
    /// such tab is open.
    pub fn focus_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(entry) = self.workspace.read(cx).tabs().get(id).cloned() else {
            return false;
        };
        if !self.editors.contains_key(&id) {
            let view = self.view_for(&entry, window, cx);
            self.editors.insert(id, (entry.kind, view));
        }
        self.editors[&id].1.stop(cx).focus(window, cx);
        true
    }

    /// The focus for the active tab: a request's URL input, another view's own handle, or
    /// the tab area's when that view is not built yet; `None` when no tab is open.
    pub fn active_focus(&self, cx: &App) -> Option<FocusHandle> {
        let id = self.workspace.read(cx).tabs().active()?.id;
        let stop = self.editors.get(&id).map(|(_, v)| v.stop(cx));
        Some(stop.unwrap_or_else(|| self.focus.clone()))
    }

    // Sends each workspace event this area acts on to its handler.
    fn workspace_event(
        &mut self,
        _: &Entity<Workspace>,
        event: &WorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            WorkspaceEvent::DraftOpened(id) => self.focus_draft(*id, window, cx),
            WorkspaceEvent::SelectionChanged(id) => self.replan_selection(*id, cx),
            _ => {}
        }
    }

    // A new draft is meant to be typed into at once, so its editor is built now rather than
    // at the next render, with focus in its URL input.
    fn focus_draft(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_tab(id, window, cx) {
            return;
        }
        if let Some(seq) = self.workspace.update(cx, |w, _| w.take_kept_view(id)) {
            self.show_kept_send(id, seq, window, cx);
        }
    }

    // Asks the runner tab for a fresh plan, which lists only the members on disk now.
    fn replan_selection(&mut self, id: TabId, cx: &mut Context<Self>) {
        if let Some(runner) = self.runner(id) {
            let model = runner.read(cx).model().clone();
            model.update(cx, |m, cx| m.refresh_preview(cx));
        }
    }

    /// Names the tree that takes focus when the last tab closes.
    pub fn set_empty_focus(&mut self, tree: Entity<gpui_kit::component::tree::TreeState>) {
        self.empty_focus = Some(tree);
    }

    // A closed tab takes its focused editor out of the tree, and focus left nowhere sits
    // above the shell's keys. When the tab area is what remains of the lost focus, it moves
    // to the new active tab, or to the sidebar tree once no tab is open. A loss with no
    // owner at all is claimed too: a dialog may hand focus back to a tab that its answer
    // closes before a frame is drawn, and then the loss is seen along the dialog's path. A
    // control gone from inside the active tab's view hands focus to its nearest drawn holder.
    fn focus_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = window.focus_lost_restore_target(cx);
        if target.is_none() || target.as_ref() == Some(&self.focus) {
            self.focus_fallback(window, cx);
            crate::focus_redraw::after_draw_move(window, cx);
        } else if let Some(target) = target.filter(|t| self.holds_inside_view(t, window, cx)) {
            // Such as an editor a segment hid, so the next key stays in that section.
            target.focus(window, cx);
            crate::focus_redraw::after_draw_move(window, cx);
        }
    }

    // Whether `target` sits strictly inside the active tab's view, for a control that went
    // away rather than focus dropped on purpose, with no dialog open (a dialog repairs its own).
    fn holds_inside_view(&self, target: &FocusHandle, window: &mut Window, cx: &mut App) -> bool {
        if window.focused(cx).is_none() || window.has_active_dialog(cx) {
            return false;
        }
        let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) else {
            return false;
        };
        let Some((_, view)) = self.editors.get(&id) else {
            return false;
        };
        let root = view.focus_handle(cx);
        root != *target && root.contains(target, window)
    }

    /// Where focus goes once the element that held it is gone: the active tab's ringed stop.
    /// The tab area's own handle is no Tab stop, so a tab whose view is not built yet (or none
    /// open) sends focus to the tree.
    pub fn focus_fallback(&self, window: &mut Window, cx: &mut Context<Self>) {
        match (self.active_focus(cx), &self.empty_focus) {
            (Some(focus), Some(tree)) if focus == self.focus => {
                tree.update(cx, |t, cx| t.focus(window, cx))
            }
            (Some(focus), _) => {
                focus.focus(window, cx);
                // A key typed next extends the URL rather than landing before it.
                let id = self.workspace.read(cx).tabs().active().map(|e| e.id);
                if let Some(editor) = id.and_then(|id| self.editor(id))
                    && editor.read(cx).url_focus(cx) == focus
                {
                    editor.update(cx, |e, cx| e.url_caret_to_end(window, cx));
                }
            }
            (None, Some(tree)) => tree.update(cx, |t, cx| t.focus(window, cx)),
            (None, None) => self.focus.focus(window, cx),
        }
    }

    /// Closes the active tab, asking first when it is dirty.
    pub fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) {
            self.close(vec![id], window, cx);
        }
    }

    /// Closes every unpinned tab, asking once for the dirty ones. The label stays "Close all
    /// tabs"; pinned tabs are the ones a bulk close leaves alone.
    pub fn close_all_tabs(
        &mut self,
        _: &CloseAllTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self
            .workspace
            .read(cx)
            .tabs()
            .entries()
            .iter()
            .filter(|e| !e.pinned)
            .map(|e| e.id)
            .collect();
        self.close(ids, window, cx);
    }

    /// Pins the active tab; a no-op when it is pinned or none is open.
    pub fn pin_tab(&mut self, _: &PinTab, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) {
            self.set_pinned(id, true, cx);
        }
    }

    /// Unpins the active tab; a no-op when it is not pinned or none is open.
    pub fn unpin_tab(&mut self, _: &UnpinTab, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) {
            self.set_pinned(id, false, cx);
        }
    }

    fn set_pinned(&mut self, id: TabId, pinned: bool, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |w, cx| w.update_tabs(cx, |t| t.set_pinned(id, pinned)));
    }

    /// Sends the active tab's request through its editor; without an active request tab it
    /// does nothing.
    pub fn send_active(&mut self, _: &SendActive, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) else {
            return;
        };
        if let Some(editor) = self.editor(id).cloned() {
            editor.update(cx, |e, cx| e.send(&SendRequest, window, cx));
        }
    }

    /// Undoes the active request, settings or environment tab's last structured edit; other tabs do nothing.
    pub fn undo_active(&mut self, _: &UndoActive, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) else {
            return;
        };
        if let Some(editor) = self.editor(id).cloned() {
            editor.update(cx, |e, cx| e.undo_edit(window, cx));
        } else if let Some(config) = self.config_editor(id).cloned() {
            config.update(cx, |e, cx| e.undo_edit(window, cx));
        } else if let Some(env) = self.env_editor(id).cloned() {
            env.update(cx, |e, cx| e.undo_edit(window, cx));
        }
    }

    /// Redoes the active request tab's last undone structured edit.
    pub fn redo_active(&mut self, _: &RedoActive, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) else {
            return;
        };
        if let Some(editor) = self.editor(id).cloned() {
            editor.update(cx, |e, cx| e.redo_edit(window, cx));
        } else if let Some(config) = self.config_editor(id).cloned() {
            config.update(cx, |e, cx| e.redo_edit(window, cx));
        } else if let Some(env) = self.env_editor(id).cloned() {
            env.update(cx, |e, cx| e.redo_edit(window, cx));
        }
    }

    /// Which state-dependent palette commands have something to do for the open tabs.
    pub(crate) fn palette_flags(&self, cx: &App) -> TabPaletteFlags {
        let ws = self.workspace.read(cx);
        let tabs = ws.tabs();
        let active = tabs.active();
        let editor = active.and_then(|a| self.editor(a.id));
        TabPaletteFlags {
            close_others: active
                .is_some_and(|a| tabs.entries().iter().any(|e| e.id != a.id && !e.pinned)),
            reopen_closed: tabs.has_closed(),
            stop: editor.is_some_and(|e| e.read(cx).is_streaming(cx)),
            discard: active.is_some_and(|a| self.discardable(a, cx)),
            save_all: tabs.entries().iter().any(|e| self.is_dirty(e, cx)),
            beautify: editor.is_some_and(|e| e.read(cx).can_reformat_body(cx)),
            beautify_query: editor.is_some_and(|e| e.read(cx).can_beautify_query(cx)),
            has_tabs: active.is_some(),
            request_tab: editor.is_some(),
            to_graphql: self.active_converts_to(RequestKind::Graphql, cx),
            to_http: self.active_converts_to(RequestKind::Http, cx),
            close_all: tabs.entries().iter().any(|e| !e.pinned),
            pin: active.is_some_and(|a| !a.pinned),
            unpin: active.is_some_and(|a| a.pinned),
            close_saved: tabs
                .entries()
                .iter()
                .any(|e| !e.pinned && !self.is_dirty(e, cx)),
            multi_tab: tabs.entries().len() >= 2,
            not_last: active.is_some_and(|a| tabs.entries().last().is_some_and(|l| l.id != a.id)),
        }
    }

    /// Stops the active tab's open response; with none open it does nothing.
    pub fn stop_active(&mut self, _: &StopActive, _: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) else {
            return;
        };
        let Some(editor) = self.editor(id).cloned() else {
            return;
        };
        let model = editor.read(cx).send_model().clone();
        if model.read(cx).is_streaming() {
            model.update(cx, |m, cx| m.stop(cx));
        }
    }

    /// Copies the active tab's request as curl; without an active request tab it does nothing.
    pub fn copy_active_as_curl(
        &mut self,
        _: &CopyAsCurl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.workspace.read(cx).tabs().active().cloned() else {
            return;
        };
        if let Some(item) = entry.item().cloned() {
            crate::copy_curl::copy_as_curl(&self.workspace, &entry.root, item, window, cx);
        }
    }

    /// Converts the active saved HTTP request to GraphQL; any other tab does nothing.
    pub fn convert_active_to_graphql(
        &mut self,
        _: &ConvertToGraphql,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_active(RequestKind::Graphql, window, cx);
    }

    /// Converts the active saved GraphQL request to HTTP; any other tab does nothing.
    pub fn convert_active_to_http(
        &mut self,
        _: &ConvertToHttp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_active(RequestKind::Http, window, cx);
    }

    fn convert_active(&mut self, to: RequestKind, window: &mut Window, cx: &mut Context<Self>) {
        if !self.active_converts_to(to, cx) {
            return;
        }
        let Some(entry) = self.workspace.read(cx).tabs().active().cloned() else {
            return;
        };
        if let Some(item) = entry.item().cloned() {
            crate::convert_kind::convert(&self.workspace, &entry.root, item, to, window, cx);
        }
    }

    // Whether the active tab is a saved request of the other kind, which converts to `to`; a
    // draft has no file to rewrite.
    fn active_converts_to(&self, to: RequestKind, cx: &App) -> bool {
        let ws = self.workspace.read(cx);
        let Some(entry) = ws.tabs().active() else {
            return false;
        };
        let Some(item) = entry.item().filter(|i| !i.is_draft()) else {
            return false;
        };
        ws.request(&entry.root, item, cx)
            .is_some_and(|v| v.kind == crate::convert_kind::other_kind(to))
    }

    /// Copies the JSON value at the active request's response body caret.
    pub fn copy_caret_json_value(
        &mut self,
        _: &crate::body_node::CopyCaretJsonValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_caret_node("Copy value", window, cx);
    }

    /// Copies the assertion path of the JSON node at the response body's caret.
    pub fn copy_caret_json_path(
        &mut self,
        _: &crate::body_node::CopyCaretJsonPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_caret_node("Copy path", window, cx);
    }

    /// Opens the Set as variable dialog for the JSON node at the response body's caret.
    pub fn set_caret_json_variable(
        &mut self,
        _: &crate::body_node::SetCaretJsonVariable,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(body) = self.active_body(cx) else {
            return;
        };
        body.update(cx, |body, cx| body.set_caret_node_variable(window, cx));
    }

    /// Appends an assertion on the JSON node at the response body's caret to the request.
    pub fn assert_caret_json_value(
        &mut self,
        _: &crate::body_node::AssertCaretJsonValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(body) = self.active_body(cx) else {
            return;
        };
        body.update(cx, |body, cx| body.assert_caret_node(window, cx));
    }

    // The active request tab's response body, if a request tab is active.
    fn active_body(&self, cx: &Context<Self>) -> Option<Entity<crate::body_view::BodyView>> {
        let id = self.workspace.read(cx).tabs().active().map(|e| e.id)?;
        let editor = self.editor(id)?;
        Some(editor.read(cx).response().read(cx).body().clone())
    }

    // Hands the label to the active tab's response body; without a request tab nothing happens.
    fn copy_caret_node(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) else {
            return;
        };
        let Some(editor) = self.editor(id) else {
            return;
        };
        let body = editor.read(cx).response().read(cx).body().clone();
        body.update(cx, |body, cx| body.copy_caret_node(label, window, cx));
    }

    /// Beautifies the active request tab's JSON or XML body; with any other body or no
    /// request tab it does nothing.
    pub fn beautify_active_body(
        &mut self,
        _: &BeautifyBody,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) else {
            return;
        };
        if let Some(editor) = self.editor(id).cloned() {
            editor.update(cx, |e, cx| e.beautify_body(window, cx));
        }
    }

    /// Minifies the active request tab's JSON or XML body; with any other body or no
    /// request tab it does nothing.
    pub fn minify_active_body(
        &mut self,
        _: &MinifyBody,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) else {
            return;
        };
        if let Some(editor) = self.editor(id).cloned() {
            editor.update(cx, |e, cx| e.minify_body(window, cx));
        }
    }

    /// Formats the active GraphQL request's query; with any other request or no request tab it
    /// does nothing.
    pub fn beautify_active_query(
        &mut self,
        _: &BeautifyQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) else {
            return;
        };
        if let Some(editor) = self.editor(id).cloned() {
            editor.update(cx, |e, cx| e.beautify_query(window, cx));
        }
    }

    /// Opens the Generate Code dialog on the active tab's request; without an active request
    /// tab it does nothing.
    pub fn generate_active_code(
        &mut self,
        _: &GenerateCode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.workspace.read(cx).tabs().active().cloned() else {
            return;
        };
        if let Some(item) = entry.item().cloned() {
            let ws = &self.workspace;
            crate::generate_code::open_generate_code(ws, &entry.root, item, window, cx);
        }
    }

    /// Asks for a name and description, then saves the active request tab's shown response as
    /// an example of that request; where examples are not offered (a draft, a GraphQL request)
    /// it says why in a toast, and without a savable response it does nothing.
    pub fn save_as_example(
        &mut self,
        _: &SaveAsExample,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.workspace.read(cx).tabs().active().cloned() else {
            return;
        };
        let (Some(item), Some(editor)) = (entry.item().cloned(), self.editor(entry.id)) else {
            return;
        };
        let Some(view) = self.workspace.read(cx).request(&entry.root, &item, cx) else {
            return;
        };
        // Where examples are not offered, say why instead of doing nothing.
        if let Some(reason) = save_example::blocked(&view) {
            window.push_notification(ui::info_toast(reason), cx);
            return;
        }
        let result = match editor.read(cx).send_model().read(cx).state() {
            SendState::Done(result) => Arc::clone(result),
            _ => return,
        };
        let Some(response) = result
            .response
            .as_ref()
            .filter(|r| save_example::can_save(Some(r)))
        else {
            return;
        };
        let names: Vec<String> = view.examples.iter().map(|e| e.name.clone()).collect();
        let derived = response.derived_credentials().to_vec();
        let response = save_example::example_response(response);
        let initial = save_example::initial_name(&names);
        save_example::save_as_example(
            &self.workspace,
            &entry,
            response,
            initial,
            &derived,
            window,
            cx,
        );
    }

    /// Asks where to save, then writes the active request tab's shown request and response as
    /// a HAR file with the request's secrets replaced. Nothing is written if the secrets could
    /// not be checked; a response kept from an earlier session has no request and says so.
    pub fn export_har(&mut self, _: &ExportHar, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.workspace.read(cx).tabs().active().cloned() else {
            return;
        };
        let Some(editor) = self.editor(entry.id) else {
            return;
        };
        let send = editor.read(cx).send_model().read(cx);
        let SendState::Done(result) = send.state() else {
            return;
        };
        let Some(response) = result.response.as_ref() else {
            return;
        };
        if response.timeline().is_empty() {
            window.push_notification(ui::info_toast(har_export::NO_REQUEST), cx);
            return;
        }
        let (response, ended) = (response.clone(), send.ended_at());
        let derived = response.derived_credentials().to_vec();
        let secrets = save_example::request_secrets(&self.workspace, &entry, &derived, cx);
        har_export::save_sent(response, ended, secrets, window, cx);
    }

    /// Exports the open request tab whose shown response is the exchange `net` logged, through
    /// the same redacted path as [`TabArea::export_har`]. False when no tab shows it.
    pub fn export_logged(
        &mut self,
        net: &NetworkEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if net.source != SendSource::Request {
            return false;
        }
        let tabs: Vec<TabEntry> = self.workspace.read(cx).tabs().entries().to_vec();
        for entry in tabs {
            let Some(editor) = self.editor(entry.id) else {
                continue;
            };
            let send = editor.read(cx).send_model().read(cx);
            let SendState::Done(result) = send.state() else {
                continue;
            };
            let Some(response) = result.response.as_ref() else {
                continue;
            };
            let first = response.timeline().first();
            let same = first.is_some_and(|h| h.method == net.method && h.url == net.url)
                && Some(response.status()) == net.status
                && response.timeline().len() == net.hops.len()
                && response.response_time() == net.duration;
            if !same {
                continue;
            }
            if response.timeline().is_empty() {
                continue;
            }
            let (response, ended) = (response.clone(), send.ended_at());
            let derived = response.derived_credentials().to_vec();
            let secrets = save_example::request_secrets(&self.workspace, &entry, &derived, cx);
            har_export::save_sent(response, ended, secrets, window, cx);
            return true;
        }
        false
    }

    /// Opens the Compare tab on the active request's shown response, against the previous send
    /// or else the first saved example; with neither it says so in a toast.
    pub fn compare_response(&mut self, _: &Compare, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) else {
            return;
        };
        let Some(pane) = self.editor(id).map(|e| e.read(cx).response().clone()) else {
            return;
        };
        let choice = pane
            .read(cx)
            .compare_choices(cx)
            .into_iter()
            .find(|c| c.disabled.is_none());
        match choice {
            Some(choice) => pane.update(cx, |p, cx| p.compare_with(choice.source, cx)),
            None => window.push_notification(ui::info_toast(NOTHING_TO_COMPARE), cx),
        }
    }

    /// Saves every dirty target of the active tab; a draft asks for its name and folder first.
    pub fn save_active(&mut self, _: &SaveActive, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.workspace.read(cx).tabs().active().cloned() else {
            return;
        };
        if let Some(draft) = draft_of(&entry).cloned() {
            let Some(answer) =
                save_draft::ask_for(&self.workspace, &entry.root, draft.clone(), window, cx)
            else {
                return;
            };
            cx.spawn(async move |this, cx| {
                let Ok(Some(save_as)) = answer.recv().await else {
                    return;
                };
                this.update(cx, |area, cx| {
                    let texts = area.draft_texts(&entry.root, &draft, cx);
                    let sent = save_draft::send_with(
                        &area.workspace,
                        &entry.root,
                        draft,
                        save_as,
                        texts.clone(),
                        cx,
                    );
                    area.workspace.update(cx, |w, _| {
                        w.approve_when_saved(&entry.root, sent, None, texts)
                    });
                })
                .ok();
            })
            .detach();
            return;
        }
        self.after_replace_check(&[entry.id], window, cx, move |area, window, cx| {
            let owed = area.store_pending_secrets(&[entry.id], true, cx);
            area.warn_unsaved(&[entry.id], window, cx);
            area.send_owed(&entry, Command::Save, owed.contains(&entry.id), cx);
        });
    }

    /// Reverts the active tab's dirty targets once the user agrees; see [`Self::discard_tab`].
    pub fn discard_changes(
        &mut self,
        _: &DiscardChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.workspace.read(cx).tabs().active().map(|e| e.id) {
            self.discard_tab(tab, window, cx);
        }
    }

    /// Reverts tab `tab`'s dirty targets once the user agrees. A draft has nothing on disk to
    /// return to and a clean tab nothing to revert, so neither asks.
    fn discard_tab(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.workspace.read(cx).tabs().get(tab).cloned() else {
            return;
        };
        if !self.discardable(&entry, cx) {
            return;
        }
        let title = format!("Discard changes to \"{}\"?", self.tab_label(&entry, cx));
        let answer = confirm::ask(confirm::discard_ask(title), window, cx);
        cx.spawn(async move |this, cx| {
            if answer.recv().await == Ok(1) {
                this.update(cx, |area, cx| {
                    area.send_each(&entry, Command::Revert, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// Reopens the most recently closed tab.
    pub fn reopen_closed_tab(
        &mut self,
        _: &ReopenClosedTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.update(cx, |w, cx| w.reopen_tab(cx));
    }

    /// Activates the tab at the action's position.
    pub fn activate_tab(&mut self, a: &ActivateTab, _: &mut Window, cx: &mut Context<Self>) {
        self.tabs_action(cx, |t| {
            t.activate_index(a.0);
        })
    }

    /// Activates the last tab.
    pub fn activate_last_tab(
        &mut self,
        _: &ActivateLastTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs_action(cx, |t| {
            t.activate_last();
        })
    }

    /// Activates the tab to the left, wrapping around.
    pub fn activate_prev_tab(
        &mut self,
        _: &ActivatePrevTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs_action(cx, |t| t.activate_offset(-1))
    }

    /// Activates the tab to the right, wrapping around.
    pub fn activate_next_tab(
        &mut self,
        _: &ActivateNextTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs_action(cx, |t| t.activate_offset(1))
    }

    /// Puts the caret in the active request tab's URL input.
    pub fn focus_url(&mut self, _: &FocusUrl, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor(cx) {
            editor.update(cx, |e, cx| e.focus_url(window, cx));
        }
    }

    /// Shows the active request's next section, wrapping.
    pub fn next_section(&mut self, _: &NextSection, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor(cx) {
            editor.update(cx, |e, cx| e.step_section(1, window, cx));
        }
    }

    /// Shows the active request's previous section, wrapping.
    pub fn previous_section(
        &mut self,
        _: &PreviousSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.active_editor(cx) {
            editor.update(cx, |e, cx| e.step_section(-1, window, cx));
        }
    }

    /// Moves focus to the active request's response body.
    pub fn focus_response(
        &mut self,
        _: &FocusResponse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self.active_editor(cx) {
            editor.update(cx, |e, cx| e.focus_response(window, cx));
        }
    }

    fn active_editor(&self, cx: &App) -> Option<Entity<RequestEditor>> {
        let id = self.workspace.read(cx).tabs().active()?.id;
        self.editor(id).cloned()
    }

    /// Saves every dirty request that has an open tab.
    /// A no on the Replace check keeps only the environment tabs it asked about unsaved.
    pub fn save_all(&mut self, _: &SaveAll, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<TabId> = self.editors.keys().copied().collect();
        let asked: Vec<TabId> = (ids.iter().copied())
            .filter(|id| match self.editors.get(id) {
                Some((_, TabView::Env(editor))) => editor.read(cx).owes_replace_check(true, cx),
                _ => false,
            })
            .collect();
        self.save_check(&ids.clone(), window, cx, move |go, area, window, cx| {
            let kept = |id: &TabId| go || !asked.contains(id);
            let ids: Vec<TabId> = ids.into_iter().filter(kept).collect();
            let owed = area.store_pending_secrets(&ids, true, cx);
            area.warn_unsaved(&ids, window, cx);
            let dirty: Vec<TabEntry> = area
                .workspace
                .read(cx)
                .tabs()
                .entries()
                .iter()
                .filter(|e| kept(&e.id))
                .filter(|e| owed.contains(&e.id) || area.is_dirty(e, cx))
                .cloned()
                .collect();
            for entry in dirty {
                area.send_owed(&entry, Command::Save, owed.contains(&entry.id), cx);
            }
        });
    }

    // Runs `then` once no environment tab among `ids` would overwrite a stored value unasked,
    // all their names in one prompt; at once when none has a new secret row with typing.
    fn after_replace_check(
        &mut self,
        ids: &[TabId],
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.save_check(ids, window, cx, move |go, area, window, cx| {
            if go {
                then(area, window, cx);
            }
        });
    }

    // The Save check behind `after_replace_check`; `done` also hears a no.
    fn save_check(
        &mut self,
        ids: &[TabId],
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(bool, &mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let editors: Vec<Entity<EnvEditor>> = (self.env_editors(ids, cx).into_iter())
            .filter(|editor| editor.read(cx).owes_replace_check(true, cx))
            .collect();
        if editors.is_empty() {
            return done(true, self, window, cx);
        }
        self.check_replace(editors, true, window, cx, done);
    }

    // A failed store read is on its editor's error line; a hidden tab is brought to the front
    // so the person sees why the save or close stopped.
    fn show_failed_read(&mut self, editors: &[Entity<EnvEditor>], cx: &mut Context<Self>) {
        let failed = (self.workspace.read(cx).tabs().entries().iter()).find(|e| {
            (self.env_editor(e.id))
                .is_some_and(|ed| editors.contains(ed) && ed.read(cx).error().is_some())
        });
        let Some(id) = failed.map(|e| e.id) else {
            return;
        };
        self.tabs_action(cx, |tabs| {
            tabs.activate(id);
        });
    }

    // The environment editors among `ids` in tab strip order, which a shared prompt lists in.
    fn env_editors(&self, ids: &[TabId], cx: &App) -> Vec<Entity<EnvEditor>> {
        (self.workspace.read(cx).tabs().entries().iter())
            .filter(|e| ids.contains(&e.id))
            .filter_map(|e| self.env_editor(e.id).cloned())
            .collect()
    }

    // The check of a Save (`saving`) or a release of held typing over `editors`, one prompt for
    // all of them; `done` hears whether it agreed, and hears no at once when a check is already
    // under way.
    fn check_replace(
        &mut self,
        editors: Vec<Entity<EnvEditor>>,
        saving: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(bool, &mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if editors.iter().any(|editor| editor.read(cx).checking()) {
            return done(false, self, window, cx);
        }
        // Typing no Save can store is dropped by the close, so the person agrees to that first.
        let twins: Vec<String> = (editors.iter())
            .flat_map(|editor| editor.read(cx).twin_typing(cx))
            .collect();
        if !saving && !twins.is_empty() {
            cx.spawn_in(window, async move |this, cx| {
                let go = crate::secret_lock::twins_dropped(twins, cx).await;
                this.update_in(cx, |area, window, cx| match go {
                    true => {
                        for editor in &editors {
                            editor.update(cx, |e, cx| e.discard_twin_typing(window, cx));
                        }
                        area.check_replace(editors, saving, window, cx, done);
                    }
                    false => done(false, area, window, cx),
                })
                .ok();
            })
            .detach();
            return;
        }
        let reads = (editors.iter())
            .map(|editor| {
                let place = editor.read(cx).place(cx);
                (
                    place,
                    editor.update(cx, |e, cx| e.begin_replace_check(saving, cx)),
                )
            })
            .collect();
        cx.spawn_in(window, async move |this, cx| {
            let go = crate::secret_lock::replace_agreed(reads, cx).await;
            this.update_in(cx, |area, window, cx| {
                for editor in &editors {
                    editor.update(cx, |e, cx| e.end_replace_check(go, cx));
                }
                if !go {
                    area.show_failed_read(&editors, cx);
                }
                done(go, area, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The scripts typed in the draft's own editor, for the save that its answered dialog
    /// sends; the caller approves them once the write is confirmed.
    pub fn draft_texts(&self, root: &Path, draft: &ItemPath, cx: &App) -> Vec<String> {
        (self.workspace.read(cx).tabs().entries().iter())
            .find(|e| e.root == root && draft_of(e) == Some(draft))
            .and_then(|e| match &self.editors.get(&e.id)?.1 {
                TabView::Request(editor) => Some(editor.read(cx).typed_scripts(cx)),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The scripts typed in the editor of `target`'s tab in the collection at `root`; none
    /// for a draft or a target no editor shows.
    pub fn typed_for(&self, root: &Path, target: &Target, cx: &App) -> Vec<String> {
        (self.workspace.read(cx).tabs().entries().iter())
            .find(|e| e.root == root && own_target(&e.kind).as_ref() == Some(target))
            .and_then(|e| match &self.editors.get(&e.id)?.1 {
                TabView::Request(editor) => Some(editor.read(cx).typed_scripts(cx)),
                TabView::Config(editor) => Some(editor.read(cx).typed_scripts(cx)),
                _ => None,
            })
            .unwrap_or_default()
    }

    // A settings tab keeps a refused value as typed and saves the last valid one, so a save
    // names each refused value in a warning rather than dropping it silently.
    fn warn_unsaved(&self, ids: &[TabId], window: &mut Window, cx: &mut Context<Self>) {
        for id in ids {
            if let Some((_, TabView::Config(editor))) = self.editors.get(id) {
                editor.update(cx, |e, cx| e.warn_unsaved(window, cx));
            }
        }
    }

    // Secret typing lives only in its editor until it blurs, and the snapshot's dirty flag
    // cannot see it, so it is stored before a save; a close prompt holds an OAuth secret until
    // Save. An OAuth secret becomes an edit, and its tab is returned: it is owed a save before
    // the snapshot shows it dirty.
    fn store_pending_secrets(
        &self,
        ids: &[TabId],
        saving: bool,
        cx: &mut Context<Self>,
    ) -> HashSet<TabId> {
        let mut owed = HashSet::new();
        for id in ids {
            let sent = match self.editors.get(id) {
                Some((_, TabView::Env(editor))) => {
                    editor.update(cx, |e, cx| e.store_pending_secrets(saving, cx));
                    false
                }
                Some((_, TabView::Request(editor))) => {
                    editor.update(cx, |e, cx| e.commit_pending_secret(cx))
                }
                Some((_, TabView::Config(editor))) => {
                    editor.update(cx, |e, cx| e.commit_pending_secret(cx))
                }
                _ => false,
            };
            if sent {
                owed.insert(*id);
            }
        }
        owed
    }

    // Sends `command` for every dirty target of the tab, in target order, and returns the
    // last command's id: its acknowledgement covers them all.
    fn send_each(
        &self,
        entry: &TabEntry,
        command: fn(Target) -> Command,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        self.send_owed(entry, command, false, cx)
    }

    // `send_each`, plus the tab's own target when `owed`: an edit just sent for it that the
    // snapshot does not show yet.
    fn send_owed(
        &self,
        entry: &TabEntry,
        command: fn(Target) -> Command,
        owed: bool,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let mut targets = dirty_targets(self.workspace.read(cx), entry, cx);
        if owed
            && let Some(own) = own_target(&entry.kind)
            && !targets.contains(&own)
        {
            targets.push(own);
        }
        // A save vouches for the text typed in its own target's editor, once its write is
        // confirmed; nothing is approved before the ack, and never text read from disk.
        let saving = matches!(command(Target::Collection), Command::Save(_));
        let typed: Vec<Vec<String>> = (targets.iter())
            .map(|t| match saving {
                true => self.typed_for(&entry.root, t, cx),
                false => Vec::new(),
            })
            .collect();
        self.workspace.update(cx, |w, cx| {
            let mut last = None;
            for (t, texts) in targets.into_iter().zip(typed) {
                let id = w.send(&entry.root, command(t.clone()), cx);
                if saving {
                    w.approve_when_saved(&entry.root, id, Some(t), texts);
                }
                last = id.or(last);
            }
            last
        })
    }

    // Tab navigation edits only the workspace's tab list, so one helper serves every key.
    fn tabs_action(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut crate::tabs::Tabs)) {
        self.workspace
            .update(cx, |w, cx| w.update_tabs(cx, |t| f(t)));
    }

    /// Whether `entry`'s tab holds typing that only its variable table knows of.
    pub fn holds_typing(&self, entry: &TabEntry, cx: &App) -> bool {
        match self.editors.get(&entry.id) {
            Some((_, TabView::Env(editor))) => editor.read(cx).holds_typing(cx),
            _ => false,
        }
    }

    /// Whether any tab of the collection at `root`, or of any collection, holds such typing.
    pub fn holds_typing_in(&self, root: Option<&Path>, cx: &App) -> bool {
        !self.held_tabs(root, cx).is_empty()
    }

    fn held_tabs(&self, root: Option<&Path>, cx: &App) -> Vec<TabId> {
        (self.workspace.read(cx).tabs().entries().iter())
            .filter(|e| root.is_none_or(|root| e.root == root))
            .filter(|e| self.holds_typing(e, cx))
            .map(|e| e.id)
            .collect()
    }

    /// Stores the typing held in `root`'s tabs through the Save check; `done` hears whether it
    /// left the table, and runs after this update has ended.
    pub fn release_held(
        &mut self,
        root: Option<&Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(bool, &mut Window, &mut App) + 'static,
    ) {
        let ids = self.held_tabs(root, cx);
        let editors = self.env_editors(&ids, cx);
        self.check_replace(editors, false, window, cx, move |go, area, window, cx| {
            if go {
                area.store_pending_secrets(&ids, false, cx);
            }
            window.defer(cx, move |window, cx| done(go, window, cx));
        });
    }

    /// The targets of the request and settings tabs of `root`, or of every collection, whose
    /// OAuth secret is typed but not yet sent, as `(root, target)`.
    pub fn typed_secret_targets(&self, root: Option<&Path>, cx: &App) -> Vec<(PathBuf, Target)> {
        (self.workspace.read(cx).tabs().entries().iter())
            .filter(|e| root.is_none_or(|root| e.root == root))
            .filter(|e| self.holds_secret_typing(e.id, cx))
            .filter_map(|e| Some((e.root.clone(), own_target(&e.kind)?)))
            .collect()
    }

    // Whether tab `id`'s request or settings editor holds an OAuth secret not yet sent.
    fn holds_secret_typing(&self, id: TabId, cx: &App) -> bool {
        match self.editors.get(&id).map(|(_, view)| view) {
            Some(TabView::Request(editor)) => editor.read(cx).holds_secret_typing(cx),
            Some(TabView::Config(editor)) => editor.read(cx).holds_secret_typing(cx),
            _ => false,
        }
    }

    /// Sends the edits that OAuth secrets still focused in the request and settings tabs of
    /// `root`, or of every collection, owe, as a blur would.
    pub fn send_pending_secrets(&self, root: Option<&Path>, cx: &mut Context<Self>) {
        let ids: Vec<TabId> = (self.workspace.read(cx).tabs().entries().iter())
            .filter(|e| root.is_none_or(|root| e.root == root))
            .filter(|e| {
                let view = self.editors.get(&e.id).map(|(_, view)| view);
                matches!(view, Some(TabView::Request(_) | TabView::Config(_)))
            })
            .map(|e| e.id)
            .collect();
        self.store_pending_secrets(&ids, false, cx);
    }

    /// Readies the environment tabs of `root`, or of every collection, for a Save sent from
    /// outside the tab area: runs the Save check, then stores their secret typing. `done` hears
    /// whether it agreed, and runs after this update has ended.
    pub fn check_before_save(
        &mut self,
        root: Option<&Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(bool, &mut Window, &mut App) + 'static,
    ) {
        let ids: Vec<TabId> = (self.workspace.read(cx).tabs().entries().iter())
            .filter(|e| root.is_none_or(|root| e.root == root))
            .filter(|e| self.env_editor(e.id).is_some())
            .map(|e| e.id)
            .collect();
        self.save_check(&ids.clone(), window, cx, move |go, area, window, cx| {
            if go {
                area.store_pending_secrets(&ids, true, cx);
            }
            window.defer(cx, move |window, cx| done(go, window, cx));
        });
    }

    fn is_dirty(&self, entry: &TabEntry, cx: &App) -> bool {
        !dirty_targets(self.workspace.read(cx), entry, cx).is_empty()
            || self.holds_typing(entry, cx)
    }

    // Whether Discard changes has a saved file to revert to: the tab holds edits and is no draft.
    // The tab menu's greyed-out item and the discard itself share this one rule.
    fn discardable(&self, entry: &TabEntry, cx: &App) -> bool {
        draft_of(entry).is_none() && !dirty_targets(self.workspace.read(cx), entry, cx).is_empty()
    }

    // Each tab's muted hint, by the shared rule over every open tab's label and place. A tab
    // that stands for a whole collection also names its folder whenever that collection's
    // name is shared, since its title alone cannot say which one it is.
    fn details(&self, entries: &[TabEntry], labels: &[String], cx: &App) -> Vec<Option<String>> {
        let ws = self.workspace.read(cx);
        let tags = ws.collection_tags(cx);
        let places: Vec<Vec<String>> = (entries.iter())
            .map(|e| {
                let snapshot = ws.snapshot(&e.root, cx);
                let tag = tags.get(&e.root).map_or("", String::as_str);
                tab_place(e, tag, snapshot.as_ref().map(|s| &s.content))
            })
            .collect();
        let hints = ws.collection_hints(cx);
        (entries.iter().zip(tab_details(entries, labels, &places)))
            .map(|(e, detail)| {
                let whole = match &e.kind {
                    TabKind::Collection | TabKind::Environments => true,
                    TabKind::Runner(path) => path.is_root(),
                    _ => false,
                };
                detail.or_else(|| whole.then(|| hints.get(&e.root).cloned()).flatten())
            })
            .collect()
    }

    /// The title `entry`'s tab shows: a request's name, "<name> Settings" for a folder or the
    /// collection, "Environments", "Run <name>" for a runner, or an example's name.
    pub fn tab_label(&self, entry: &TabEntry, cx: &App) -> String {
        let snapshot = self.workspace.read(cx).snapshot(&entry.root, cx);
        let content = snapshot.as_ref().map(|s| &s.content);
        match &entry.kind {
            TabKind::Request(item) => content
                .and_then(|c| c.requests.get(item))
                .map(|v| v.name.clone())
                .unwrap_or_else(|| item.as_str().to_owned()),
            TabKind::Folder(path) => {
                let name = content
                    .and_then(|c| c.folders.get(path))
                    .and_then(|v| v.name.clone())
                    .unwrap_or_else(|| path.file_name().to_owned());
                format!("{name} Settings")
            }
            TabKind::Collection => {
                let name = content.and_then(|c| c.collection.name.as_deref());
                format!("{} Settings", label_or_dir(name, &entry.root))
            }
            TabKind::Environments => "Environments".into(),
            TabKind::Runner(path) if path.is_root() => {
                let name = content.and_then(|c| c.collection.name.as_deref());
                format!("Run {}", label_or_dir(name, &entry.root))
            }
            TabKind::Runner(path) => {
                let name = content
                    .and_then(|c| c.folders.get(path))
                    .and_then(|v| v.name.clone())
                    .unwrap_or_else(|| path.file_name().to_owned());
                format!("Run {name}")
            }
            // N is the requests the tab runs: those equal to or under a member, not the members.
            TabKind::RunSelection(items) => {
                let runs = content.map_or(0, |c| {
                    c.requests
                        .keys()
                        .filter(|r| items.iter().any(|i| crate::stale::is_within(r, i)))
                        .count()
                });
                format!("Selected ({runs})")
            }
            TabKind::Example { item, index } => content
                .and_then(|c| c.requests.get(item))
                .and_then(|v| v.examples.get(*index))
                .map(|e| e.name.clone())
                .unwrap_or_else(|| "Example".into()),
        }
    }

    // Each kind of tab gets its own editor, made on first show.
    fn view_for(&self, entry: &TabEntry, window: &mut Window, cx: &mut Context<Self>) -> TabView {
        let (ws, root, id) = (self.workspace.clone(), entry.root.clone(), entry.id);
        match &entry.kind {
            TabKind::Request(_) => {
                TabView::Request(cx.new(|cx| RequestEditor::new(ws, id, window, cx)))
            }
            TabKind::Folder(path) => {
                let target = ConfigTarget::Folder(path.clone());
                TabView::Config(cx.new(|cx| ConfigEditor::new(ws, root, target, window, cx)))
            }
            TabKind::Collection => TabView::Config(
                cx.new(|cx| ConfigEditor::new(ws, root, ConfigTarget::Collection, window, cx)),
            ),
            TabKind::Environments => {
                TabView::Env(cx.new(|cx| EnvEditor::new(ws, root, window, cx)))
            }
            TabKind::Runner(target) => {
                let target = target.clone();
                TabView::Runner(cx.new(|cx| RunnerView::new(ws, root, target, cx)))
            }
            TabKind::RunSelection(items) => {
                let items = items.clone();
                TabView::Runner(cx.new(|cx| RunnerView::for_items(ws, root, items, cx)))
            }
            TabKind::Example { .. } => {
                TabView::Example(cx.new(|cx| ExampleView::new(ws, id, window, cx)))
            }
        }
    }

    fn choose(
        &mut self,
        choice: MenuChoice,
        tab: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = self.workspace.read(cx).tabs().entries().to_vec();
        let ids = match choice {
            MenuChoice::Close => vec![tab],
            MenuChoice::CloseOthers => {
                self.close_others(tab, window, cx);
                return;
            }
            MenuChoice::CloseToRight => {
                self.close_to_right(tab, window, cx);
                return;
            }
            MenuChoice::Reveal => {
                self.reveal(tab, cx);
                return;
            }
            MenuChoice::CloseAll => entries.iter().filter(|e| !e.pinned).map(|e| e.id).collect(),
            MenuChoice::CloseSaved => {
                self.close_saved(cx);
                return;
            }
        };
        self.close(ids, window, cx);
    }

    // Closes every tab but `keep` and the pinned ones, asking once for the dirty ones.
    fn close_others(&mut self, keep: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let ids = (self.workspace.read(cx).tabs().entries().iter())
            .filter(|e| e.id != keep && !e.pinned)
            .map(|e| e.id)
            .collect();
        self.close(ids, window, cx);
    }

    // Closes the unpinned tabs after `from`, asking once for the dirty ones.
    fn close_to_right(&mut self, from: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let ids = tabs_right_of(self.workspace.read(cx).tabs(), from);
        if !ids.is_empty() {
            self.close(ids, window, cx);
        }
    }

    // Asks the shell to show tab `id`'s item in the sidebar; a tab with no sidebar row does nothing.
    fn reveal(&mut self, id: TabId, cx: &mut Context<Self>) {
        let ws = self.workspace.clone();
        let Some(entry) = ws.read(cx).tabs().get(id) else {
            return;
        };
        let Some(path) = reveal_path(&entry.kind) else {
            return;
        };
        let root = entry.root.clone();
        ws.update(cx, |w, cx| w.reveal_folder(&root, path, cx));
    }

    // Closes every unpinned tab without unsaved changes, without asking.
    fn close_saved(&mut self, cx: &mut Context<Self>) {
        let entries = self.workspace.read(cx).tabs().entries().to_vec();
        let clean: Vec<TabId> = entries
            .iter()
            .filter(|e| !e.pinned && !self.is_dirty(e, cx))
            .map(|e| e.id)
            .collect();
        // A tab whose pending OAuth secret just became an edit is no longer saved.
        let owed = self.store_pending_secrets(&clean, false, cx);
        let clean: Vec<TabId> = clean.into_iter().filter(|id| !owed.contains(id)).collect();
        self.close_now(&clean, cx);
    }

    /// Closes every tab but the active one and the pinned ones, asking once for the dirty
    /// ones; with no active tab it does nothing.
    pub fn close_other_tabs(
        &mut self,
        _: &CloseOtherTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) {
            self.close_others(id, window, cx);
        }
    }

    /// Closes the unpinned tabs right of the active one, asking once for the dirty ones; with
    /// no active tab, or nothing to its right, it does nothing.
    pub fn close_tabs_to_the_right(
        &mut self,
        _: &CloseTabsToTheRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) {
            self.close_to_right(id, window, cx);
        }
    }

    /// Shows the active tab's item in the sidebar, which opens if hidden; tabs with no row
    /// there (Environments, a run over a selection) do nothing.
    pub fn reveal_in_sidebar(
        &mut self,
        _: &RevealInSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.workspace.read(cx).tabs().active().map(|e| e.id) {
            self.reveal(id, cx);
        }
    }

    /// Closes every unpinned tab without unsaved changes.
    pub fn close_saved_tabs(&mut self, _: &CloseSavedTabs, _: &mut Window, cx: &mut Context<Self>) {
        self.close_saved(cx);
    }

    /// Records which tab the pointer is over. Leaving a tab clears the record only when it
    /// is still that tab, since entering the next one may be reported first.
    fn hover_tab(&mut self, id: TabId, hovered: bool, cx: &mut Context<Self>) {
        let next = match (hovered, self.hovered) {
            (true, _) => Some(id),
            (false, Some(current)) if current == id => None,
            (false, current) => current,
        };
        if next != self.hovered {
            self.hovered = next;
            cx.notify();
        }
    }

    /// Records where `dragged` would land with the pointer on `side` of `target`, or clears
    /// the record when `side` is `None`; a tab of another collection, or the dragged tab
    /// itself, never shows a landing place.
    pub fn hover_drop(
        &mut self,
        dragged: &DraggedTab,
        target: TabId,
        side: Option<Side>,
        cx: &mut Context<Self>,
    ) {
        let ws = self.workspace.read(cx);
        let valid = target != dragged.id
            && ws.tabs().get(target).is_some_and(|t| {
                // A drop never crosses the pinned boundary, so it shows no landing place.
                t.root == dragged.root
                    && ws
                        .tabs()
                        .get(dragged.id)
                        .is_none_or(|d| d.pinned == t.pinned)
            });
        let next = match side {
            Some(side) if valid => Some((target, side)),
            _ if self.drop.is_some_and(|(t, _)| t == target) => None,
            _ => self.drop,
        };
        if next != self.drop {
            self.drop = next;
            cx.notify();
        }
    }

    /// Moves the dragged tab to `side` of `target` and saves the new order; a no-op when
    /// the tabs are the same or belong to different collections.
    pub fn drop_tab(
        &mut self,
        dragged: &DraggedTab,
        target: TabId,
        side: Side,
        cx: &mut Context<Self>,
    ) {
        self.drop = None;
        cx.notify();
        self.workspace.update(cx, |w, cx| {
            w.update_tabs(cx, |t| t.move_to(dragged.id, target, side))
        });
    }

    // A tab is its method tag, name and any muted hint, then a fixed slot that
    // holds the close button, or the unsaved dot while a dirty tab is neither active nor
    // hovered.
    fn tab(
        &self,
        ix: usize,
        entry: &TabEntry,
        detail: Option<String>,
        active: bool,
        cx: &mut Context<Self>,
    ) -> Tab {
        let view = cx.entity();
        let id = entry.id;
        let pinned = entry.pinned;
        let menu_view = view.clone();
        let method = match &entry.kind {
            TabKind::Request(item) => self
                .workspace
                .read(cx)
                .request(&entry.root, item, cx)
                .map(|r| ui::method_tag(&r.method)),
            _ => None,
        };
        // A request tab's git change, looked up the way the sidebar keys its request rows.
        let change = entry.item().and_then(|item| {
            let changes = self.workspace.read(cx).git_changes(&entry.root)?;
            changes.get(&item_file(item, false)).cloned()
        });
        let show_changes = entry
            .item()
            .filter(|_| change.is_some())
            .map(|item| (self.workspace.clone(), entry.root.clone(), item.clone()));
        let discardable = self.discardable(entry, cx);
        let can_close_right = !tabs_right_of(self.workspace.read(cx).tabs(), id).is_empty();
        let can_reveal = reveal_path(&entry.kind).is_some();
        let name = self.tab_label(entry, cx);
        let name_for_drag: SharedString = name.clone().into();
        let name = if entry.preview {
            div()
                .id(("preview", ix))
                .test_support()
                .italic()
                .child(name)
                .into_any_element()
        } else {
            name.into_any_element()
        };
        // A long name is cut with an ellipsis inside the tab's cap; tag and close keep their size.
        let name = div()
            .id(("tab-name", ix))
            .test_support()
            .min_w_0()
            .truncate()
            .child(name);
        let label = h_flex()
            .id(("tab", ix))
            .test_support()
            .role(Role::Tab)
            // The press that opens the menu also moves focus to the tab area before the menu
            // looks, so the capture phase sees what the user had focused.
            .capture_any_mouse_down({
                let area = view.downgrade();
                move |e, window, cx| {
                    if e.button == MouseButton::Right {
                        let held = menu_focus::held(window, cx);
                        area.update(cx, |a, cx| {
                            let owner = a
                                .editors
                                .iter()
                                .find(|(_, (_, v))| v.focus_handle(cx).contains_focused(window, cx))
                                .map(|(id, _)| *id);
                            a.before_menu = held.map(|h| (h, owner));
                        })
                        .ok();
                    }
                }
            })
            .accessibility_id(format!("tab-{ix}"))
            .max_w(TAB_MAX_WIDTH)
            .min_w_0()
            .gap(SPACE_1)
            .when_some(method, |d, tag| {
                d.child(
                    div()
                        .id(("tab-method", ix))
                        .test_support()
                        .flex_none()
                        .child(tag),
                )
            })
            .child(name)
            .when_some(detail, |d, detail| {
                d.child(
                    div()
                        .id(("tab-detail", ix))
                        .test_support()
                        .min_w_0()
                        .text_color(cx.theme().muted_foreground)
                        .child(detail),
                )
            })
            .context_menu(move |mut menu, window, cx| {
                // The menu refocuses what was focused at right-click, which may be gone by
                // dismissal; the shared helper settles focus once it closes.
                let (area, fallback_area) = (menu_view.downgrade(), menu_view.downgrade());
                menu_focus::watch(
                    cx,
                    window,
                    move |cx| {
                        area.update(cx, |a, cx| {
                            let Some((handle, owner)) = a.before_menu.take() else {
                                return Before::Nothing;
                            };
                            let open = |id: &TabId| {
                                (a.workspace.read(cx).tabs().entries().iter()).any(|e| e.id == *id)
                            };
                            match owner.as_ref().is_none_or(open) {
                                true => Before::Held(handle),
                                false => Before::Gone,
                            }
                        })
                        .unwrap_or(Before::Gone)
                    },
                    move |window, cx| {
                        fallback_area
                            .update(cx, |a, cx| a.focus_fallback(window, cx))
                            .ok();
                    },
                );
                // No separators, so each item's index is its click index in tests.
                for choice in MenuChoice::ALL {
                    let view = menu_view.clone();
                    let off = match choice {
                        MenuChoice::CloseToRight => !can_close_right,
                        MenuChoice::Reveal => !can_reveal,
                        _ => false,
                    };
                    menu = menu.item(ui::menu_item(choice.label(), off, cx).on_click(
                        move |_, window, cx| {
                            view.update(cx, |a, cx| a.choose(choice, id, window, cx))
                        },
                    ));
                }
                // Discard changes and then Show changes follow the fixed choices, so their
                // indices hold; Discard is always listed, greyed out when there is nothing to revert.
                let view = menu_view.clone();
                menu = menu.item(ui::menu_item("Discard changes", !discardable, cx).on_click(
                    move |_, window, cx| view.update(cx, |a, cx| a.discard_tab(id, window, cx)),
                ));
                if let Some((workspace, root, item)) = show_changes.clone() {
                    menu = menu.item(PopupMenuItem::new("Show changes").on_click(
                        move |_, window, cx| {
                            let file = PathBuf::from(item_file(&item, false));
                            open_changes(&workspace, &root, Some(file), window, cx);
                        },
                    ));
                }
                // Last, so the indices of the fixed choices above hold.
                let view = menu_view.clone();
                menu.item(
                    PopupMenuItem::new(if pinned { "Unpin tab" } else { "Pin tab" }).on_click(
                        move |_, _, cx| view.update(cx, |a, cx| a.set_pinned(id, !pinned, cx)),
                    ),
                )
            });
        let dirty = self.is_dirty(entry, cx);
        let hovered = self.hovered == Some(id);
        let slot = h_flex()
            .size(ROW_HEIGHT)
            .flex_none()
            .justify_center()
            .map(|slot| {
                if dirty && !active && !hovered {
                    return slot.child(ui::dirty_dot(("dirty", ix)).test_support());
                }
                if pinned {
                    // A pinned tab trades its close button for the pin; clicking it unpins.
                    let pin_view = view.clone();
                    let icon = ui::icon_button(("tab-pin", ix), IconName::Pin, "Unpin tab", None)
                        .text_color(cx.theme().muted_foreground)
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            pin_view.update(cx, |a, cx| a.set_pinned(id, false, cx));
                        });
                    return slot.child(icon);
                }
                let close_view = view.clone();
                let close = ui::icon_button(("tab-close", ix), IconName::Close, "Close tab", None)
                    .tooltip_with_action("Close tab", &CloseTab, Some(SHELL_CONTEXT))
                    .when(!active && !hovered, |b| {
                        b.text_color(cx.theme().muted_foreground)
                    })
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        // A button cut by the strip's edge or under a fade selects its tab:
                        // the user cannot see what a press there would close.
                        close_view.update(cx, |a, cx| {
                            let slot = style::resolve(ROW_HEIGHT, window);
                            let fade = style::resolve(style::STRIP_FADE_WIDTH, window);
                            let count = a.workspace.read(cx).tabs().entries().len();
                            if ui::tail_clear_of_fades(&a.scroll, count, ix, slot, fade) {
                                a.close(vec![id], window, cx);
                            } else {
                                a.workspace.update(cx, |w, cx| {
                                    w.update_tabs(cx, |t| t.activate_index(ix))
                                });
                            }
                        });
                    });
                slot.child(close)
            });
        let weak = view.downgrade();
        let middle = weak.clone();
        let (over, dropped) = (weak.clone(), weak.clone());
        let payload = DraggedTab::new(id, entry.root.clone(), name_for_drag);
        let landing = self.drop.filter(|(t, _)| *t == id).map(|(_, side)| side);
        let workspace = self.workspace.clone();
        // A click activates through the tab's own element, not `Tab::on_click`: the bar wraps
        // that in its own reveal, which would fight the strip's least-scroll reveal.
        let tab = StatefulInteractiveElement::on_click(Tab::new(), move |_, _, cx| {
            workspace.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(ix)));
        });
        tab.relative()
            .on_drag(payload, |d: &DraggedTab, _, _, cx| {
                cx.new(|_| TabDragPreview(d.label.clone()))
            })
            // The pointer's half of this tab picks the landing side; leaving it clears the mark.
            .on_drag_move(move |e: &DragMoveEvent<DraggedTab>, _, cx| {
                let side = e
                    .bounds
                    .contains(&e.event.position)
                    .then(|| drop_side(e.event.position.x, e.bounds));
                let dragged = e.drag(cx).clone();
                over.update(cx, |a, cx| a.hover_drop(&dragged, id, side, cx))
                    .ok();
            })
            .on_drop(move |d: &DraggedTab, _, cx| {
                dropped
                    .update(cx, |a, cx| {
                        match a.drop.filter(|(t, _)| *t == id) {
                            Some((_, side)) => a.drop_tab(d, id, side, cx),
                            None => {
                                a.drop = None;
                                cx.notify();
                            }
                        };
                    })
                    .ok();
            })
            .when_some(landing, |t, side| {
                t.child(
                    div()
                        .id("tab-drop-indicator")
                        .test_support()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .w(rems(0.125))
                        .bg(cx.theme().primary)
                        .map(|d| match side {
                            Side::Before => d.left_0(),
                            Side::After => d.right_0(),
                        }),
                )
            })
            .on_hover(move |hovered, _, cx| {
                weak.update(cx, |a, cx| a.hover_tab(id, *hovered, cx)).ok();
            })
            // A middle click closes the tab the way its close button does, prompt included,
            // and its press leaves focus where it was rather than on the tab area.
            .on_mouse_down(MouseButton::Middle, |_, window, _| window.prevent_default())
            .on_mouse_up(MouseButton::Middle, move |_, window, cx| {
                cx.stop_propagation();
                middle
                    .update(cx, |a, cx| a.close(vec![id], window, cx))
                    .ok();
            })
            .child(
                h_flex()
                    .line_height(relative(style::TAB_LINE_HEIGHT))
                    .gap(SPACE_1)
                    .child(label)
                    // Letter then dot, the order an inactive tab shows with its dot in the slot.
                    .when_some(change, |d, change| {
                        d.child(ui::git_letter(("tab-git", ix).into(), &change, cx))
                    })
                    .when(dirty && active, |d| {
                        d.child(ui::dirty_dot(("dirty", ix)).test_support())
                    }),
            )
            .suffix(slot)
    }
}

impl TabArea {
    // `#tab-overflow`: a dropdown naming every tab in order, the active one checked, so a
    // tab scrolled out of the strip is one click away.
    fn overflow_menu(
        &self,
        labels: Vec<String>,
        details: Vec<Option<String>>,
        selected: usize,
    ) -> impl IntoElement + use<> {
        let workspace = self.workspace.clone();
        let reveal = self.reveal.clone();
        ui::strip_chevron(
            Button::new("tab-overflow")
                .ghost()
                .compact()
                .icon(IconName::ChevronDown)
                .tooltip("All tabs")
                .accessibility_label("All tabs")
                .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, window, cx| {
                    menu = menu.scrollable(true);
                    crate::ui::start_on(&menu, selected, window, cx);
                    for (ix, (label, detail)) in labels.iter().zip(&details).enumerate() {
                        let workspace = workspace.clone();
                        let reveal = reveal.clone();
                        menu = menu.item(
                            menu_tab_item(ix, label, detail.as_deref())
                                .checked(ix == selected)
                                .on_click(move |_, _, cx| {
                                    // A pick of the already active tab must still scroll it back
                                    // into view after a wheel moved it out.
                                    reveal.request();
                                    workspace.update(cx, |w, cx| {
                                        w.update_tabs(cx, |t| t.activate_index(ix))
                                    });
                                }),
                        );
                    }
                    menu
                }),
        )
    }
}

// An All tabs entry: the tab's label, then its muted hint as the tab shows it.
fn menu_tab_item(ix: usize, label: &str, detail: Option<&str>) -> PopupMenuItem {
    let Some(detail) = detail else {
        return PopupMenuItem::new(label.to_owned());
    };
    let (label, detail) = (SharedString::from(label.to_owned()), detail.to_owned());
    PopupMenuItem::element(move |_, cx| {
        h_flex()
            .id(("tab-menu-item", ix))
            .test_support()
            .gap(style::SPACE_2)
            .aria_label(format!("{label} {detail}"))
            .child(label.clone())
            .child(
                div()
                    .id(("tab-menu-detail", ix))
                    .test_support()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail.clone()),
            )
    })
}

/// The draft `entry` shows, if it is a draft's tab.
fn draft_of(entry: &TabEntry) -> Option<&ItemPath> {
    match &entry.kind {
        TabKind::Request(item) if item.is_draft() => Some(item),
        _ => None,
    }
}

// The target a request or settings tab edits, which an owed OAuth secret makes dirty.
fn own_target(kind: &TabKind) -> Option<Target> {
    match kind {
        TabKind::Request(item) => Some(Target::Request(item.clone())),
        TabKind::Folder(path) => Some(Target::Folder(path.clone())),
        TabKind::Collection => Some(Target::Collection),
        _ => None,
    }
}

/// The targets of `entry` with unsaved edits: its request, its folder's or the collection's
/// settings, or every dirty environment for an environments tab.
fn dirty_targets(ws: &Workspace, entry: &TabEntry, cx: &App) -> Vec<Target> {
    let Some(snapshot) = ws.snapshot(&entry.root, cx) else {
        return Vec::new();
    };
    let content = &snapshot.content;
    let one = |dirty: bool, target: Target| Vec::from_iter(dirty.then_some(target));
    match &entry.kind {
        TabKind::Request(item) => one(
            content.requests.get(item).is_some_and(|v| v.dirty),
            Target::Request(item.clone()),
        ),
        TabKind::Folder(path) => one(
            content.folders.get(path).is_some_and(|v| v.dirty),
            Target::Folder(path.clone()),
        ),
        TabKind::Collection => one(content.collection.dirty, Target::Collection),
        TabKind::Runner(_) | TabKind::RunSelection(_) | TabKind::Example { .. } => Vec::new(),
        TabKind::Environments => content
            .environments
            .iter()
            .filter(|e| e.dirty)
            .map(|e| Target::Environment(e.name.clone()))
            .collect(),
    }
}

impl Focusable for TabArea {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

// The tab strip is drawn by its own `Part` view, which calls back in here, and reused while
// its inputs hold.
impl TabArea {
    /// The view that draws the tab strip.
    pub fn strip(&self) -> &Entity<Part<TabArea>> {
        self.strip.view()
    }

    // Everything the strip reads that does not notify it: each tab as shown, the pointer's
    // hover and drag marks, the strip's gap and the zoom. Theme, focus and key bindings
    // refresh the window; the strip's own scroll, fades and menus notify its view.
    fn strip_inputs(&self, entries: &[TabEntry], active: TabId, cx: &App) -> StripInputs {
        let ws = self.workspace.read(cx);
        let labels: Vec<String> = entries.iter().map(|e| self.tab_label(e, cx)).collect();
        let details = self.details(entries, &labels, cx);
        let tabs = (entries.iter().zip(labels).zip(details))
            .map(|((entry, label), detail)| TabShown {
                method: match &entry.kind {
                    TabKind::Request(item) => {
                        (ws.request(&entry.root, item, cx)).map(|r| r.method.to_string())
                    }
                    _ => None,
                },
                change: entry.item().and_then(|item| {
                    let changes = ws.git_changes(&entry.root)?;
                    changes.get(&item_file(item, false)).cloned()
                }),
                dirty: self.is_dirty(entry, cx),
                entry: entry.clone(),
                label,
                detail,
            })
            .collect();
        StripInputs {
            tabs,
            active,
            hovered: self.hovered,
            drop: self.drop,
            gap: ui::strip_gap(&self.scroll, entries.len()),
            zoom: zoom::current(cx),
        }
    }

    // The strip: the kit's tab bar, its edge fades and the check that keeps the active tab
    // in view.
    fn tab_strip(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let ws = self.workspace.read(cx);
        let entries = ws.tabs().entries().to_vec();
        let Some(active) = ws.tabs().active().map(|e| e.id) else {
            return Empty.into_any_element();
        };
        let selected = entries.iter().position(|e| e.id == active).unwrap_or(0);
        let labels: Vec<String> = entries.iter().map(|e| self.tab_label(e, cx)).collect();
        let details = self.details(&entries, &labels, cx);
        let tabs: Vec<Tab> = entries
            .iter()
            .zip(details.iter().cloned())
            .enumerate()
            .map(|(ix, (e, detail))| self.tab(ix, e, detail, e.id == active, cx))
            .collect();
        let overflow = self.overflow_menu(labels, details, selected);
        // The kit's filler after the last tab keeps its close button off the chevron. The strip's
        // gap precedes it, so the filler pulls back by that gap and trails the tab by its width.
        let pad = style::resolve(style::ui(12.), window);
        let gap = ui::strip_gap(&self.scroll, entries.len());
        // The kit's underline tabs have no side padding; a prefix spacer insets the first
        // tab while the underline still spans the whole strip, where every tab's content starts.
        let bar = TabBar::new("tabs")
            .underline()
            .with_size(style::kit_size(zoom::current(cx)))
            .prefix(
                div()
                    .id("tab-inset")
                    .test_support()
                    .w(style::tab_content_inset(window))
                    .flex_none(),
            )
            .track_scroll(&self.scroll)
            .last_empty_space(div().flex_none().w(pad).ml(-gap))
            .selected_index(selected)
            .children(tabs)
            .suffix(overflow);
        // The kit strip turns a mouse wheel sideways itself; the wrapper does the same for a
        // vertical trackpad swipe, which the kit leaves to the page.
        let fades = self
            .fades
            .render("tab-strip", &self.scroll, entries.len(), cx);
        div()
            .id("tab-strip")
            .test_support()
            .relative()
            .on_scroll(ui::wheel_sideways(&self.scroll))
            .child(bar)
            .children(fades)
            .child(self.reveal.watch(&self.scroll, selected))
            .into_any_element()
    }
}

impl PartOwner for TabArea {
    type Part = ();

    fn render_part(&mut self, _: (), window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.tab_strip(window, cx)
    }
}

#[derive(PartialEq)]
struct StripInputs {
    tabs: Vec<TabShown>,
    active: TabId,
    hovered: Option<TabId>,
    drop: Option<(TabId, Side)>,
    gap: Pixels,
    zoom: f32,
}

// One tab as the strip draws it.
#[derive(PartialEq)]
struct TabShown {
    entry: TabEntry,
    label: String,
    detail: Option<String>,
    method: Option<String>,
    change: Option<GitChange>,
    dirty: bool,
}

impl Render for TabArea {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = v_flex()
            .size_full()
            .key_context("TabArea")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::close_all_tabs))
            .on_action(cx.listener(Self::close_other_tabs))
            .on_action(cx.listener(Self::close_tabs_to_the_right))
            .on_action(cx.listener(Self::reveal_in_sidebar))
            .on_action(cx.listener(Self::close_saved_tabs))
            .on_action(cx.listener(Self::pin_tab))
            .on_action(cx.listener(Self::unpin_tab))
            .on_action(cx.listener(Self::save_all))
            .on_action(cx.listener(Self::reopen_closed_tab))
            .on_action(cx.listener(Self::activate_tab))
            .on_action(cx.listener(Self::activate_last_tab))
            .on_action(cx.listener(Self::activate_prev_tab))
            .on_action(cx.listener(Self::activate_next_tab));
        let ws = self.workspace.read(cx);
        let entries = ws.tabs().entries().to_vec();
        let active = ws.tabs().active().map(|e| e.id);
        let Some(active) = active else {
            // A role-less div is accesskit's filtered GenericContainer and never reaches AT-SPI.
            return root.child(
                div()
                    .id("tab-empty")
                    .test_support()
                    .role(Role::Group)
                    .accessibility_id("tab-empty")
                    .size_full()
                    .child(crate::ui::empty_state(
                        "tab-empty-actions",
                        "No request open",
                        &[
                            ("New request", &NewRequest),
                            ("Filter requests", &FocusFilter),
                            ("Manage environments", &OpenEnvironments),
                            ("Command palette", &OpenCommandPalette),
                        ],
                    )),
            );
        };
        let selected = entries.iter().position(|e| e.id == active).unwrap_or(0);
        // A new active tab is checked even where it takes the old one's place and size.
        if self.revealed != Some(active) {
            self.reveal.request();
            self.revealed = Some(active);
        }
        // The check runs while the strip draws, so a pending one draws it afresh.
        if self.reveal.pending() {
            self.strip.invalidate();
        }
        let inputs = self.strip_inputs(&entries, active, cx);
        let style = StyleRefinement::default().w_full().flex_none();
        let strip = self.strip.element(inputs, style, window, cx);
        // Editors are made the first time their tab shows, and kept while it stays open.
        if !self.editors.contains_key(&active) {
            let entry = &entries[selected];
            let view = self.view_for(entry, window, cx);
            self.editors.insert(active, (entry.kind.clone(), view));
        }
        root.child(strip)
            // Views fill their parent with size_full, so they get the room under the strip
            // rather than the whole area. The clip keeps a view that cannot shrink from painting
            // into the dock below; popups and tooltips are deferred, so they still escape it.
            .child({
                let clip = div().flex_1().min_h_0().w_full().overflow_hidden();
                let view = &self.editors[&active].1;
                // A view with no scrolling of its own scrolls here once the area is shorter than
                // its minimum, instead of being cut at the bottom. The runner scrolls inside
                // itself, so Run never scrolls away.
                if matches!(view, TabView::Config(_)) {
                    clip.child(
                        div()
                            .id("tab-view-scroll")
                            .test_support()
                            .size_full()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .w_full()
                                    .h_full()
                                    .min_h(style::TAB_VIEW_MIN)
                                    .child(view.element()),
                            ),
                    )
                } else {
                    clip.child(view.element())
                }
            })
    }
}
