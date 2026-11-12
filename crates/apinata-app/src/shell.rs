//! The window's root view: a title bar over a resizable sidebar and tab-area split, the
//! open-collection flow, and the dialogs and notifications for workspace and save errors.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use apinata_core::import::{
    CollectionOptions, EnvironmentFormat, FormatError, ImportBundle, ImportReport, ImportedRequest,
    parse_collections, write_collection,
};
use apinata_core::{DraftSnapshot, ItemPath};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Replace;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Icon, ResizablePanelEvent, ResizableState, Sizable as _, TitleBar, WindowExt,
    h_flex, h_resizable, resizable_panel, v_flex, v_resizable,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Action, AnyElement, App, AppContext, AsyncWindowContext, ClickEvent, Context, Decorations, Div,
    DragMoveEvent, Entity, ExternalPaths, FocusHandle, Focusable, FontWeight, Hsla, Image,
    ImageFormat, InteractiveElement, IntoElement, MouseButton, ParentElement, PathPromptOptions,
    Pixels, Rems, Render, Role, SharedString, StatefulInteractiveElement, StyleRefinement, Styled,
    Subscription, TestSupportExt, WeakEntity, Window, WindowControls, actions, canvas, div, img,
    px,
};

use crate::app::{AppState, window_decorations};
use crate::approvals::{Approvals, ApprovalsEvent, Prompt, ScriptPrompt};
use crate::autosave::Autosave;
use crate::cached_part::{Part, PartOwner, Slot};
use crate::changes_dialog::open_changes;
use crate::closing::CloseScope;
use crate::command::{Command, NoticeKind, Target};
use crate::commands;
use crate::console_dock::{ConsoleDock, ExpandDock};
use crate::cookie_jar::open_cookie_jar;
use crate::draft_file::MAX_DRAFTS;
use crate::draft_store::{DraftStoreEvent, Drafts, NotKeptReason};
use crate::env_selector::EnvSelector;
use crate::export::ExportFormat;
use crate::git_status::{GitChanges, branch_text};
use crate::global_env_editor::open_global_env_editor;
use crate::global_env_selector::GlobalEnvSelector;
use crate::history_palette::{OpenHistory, OpenHistoryEntry};
use crate::keymap::{
    NewRequest, OpenCollectionSettings, OpenEnvironmentSelector, OpenEnvironments, ResetZoom,
    RunCollection, RunFolder, SHELL_CONTEXT, SaveActive, ToggleConsole, ToggleOrientation,
    ToggleSidebar, ToggleTheme, UndoActive, ZoomIn, ZoomOut,
};
use crate::mock_server::{MOCK_TIP, MockState};
use crate::palette::{GoToRequest, OpenCommandPalette, OpenRequest};
use crate::path_display;
use crate::persisted::{Persisted, SaveFailed};
use crate::preferences::OpenPreferences;
use crate::runner_view::OpenRunResult;
use crate::search_dialog::{OpenCollectionSearch, OpenSearchHit};
use crate::session::{SavedBounds, Session};
use crate::settings::{LinuxTitleBar, Orientation, ThemeChoice};
use crate::sidebar::{FocusFilter, Sidebar, SidebarEvent, UndoDelete};
use crate::stand_in::StandIn;
use crate::style;
use crate::tab_area::TabArea;
use crate::tabs::{TabId, TabKind};
use crate::ui::{self, LockButton as _, icon_button, no_window_move};
use crate::update_install;
use crate::updates::{CheckForUpdates, Report, Updates, UpdatesEvent, report_text};
use crate::window_drop::{Dropped, classify};
use crate::workspace::{Workspace, WorkspaceEvent, collection_label, display_name};
use crate::zoom::{ZoomStep, ZoomTarget};

actions!(
    apinata,
    [
        /// Prompts for a collection directory and opens it.
        OpenCollection,
        /// Prompts for a collection file and a directory, imports the file there and opens it.
        ImportCollection,
        /// Asks for a curl command and opens it as a new draft in the current collection.
        ImportCurl,
        /// Opens the dialog that clones a repository and opens it as a collection.
        CloneRepository,
        /// Prompts for a folder, copies the bundled demo collection into it and opens it.
        OpenDemoCollection,
        /// Opens the remote branch picker for the current collection.
        SwitchBranch,
        /// Opens the Changes dialog, where the current collection's files are staged and committed.
        ShowChanges,
        /// Prompts for a file and exports the current collection there as Postman JSON.
        ExportCollection,
        /// Prompts for a file and exports the current collection there as Postman JSON with its
        /// scripts kept as written.
        ExportCollectionPostmanKeepScripts,
        /// Prompts for a file and exports the current collection there as OpenAPI JSON.
        ExportCollectionOpenApi,
        /// Prompts for a file and exports the current collection there as an HTML documentation
        /// page.
        ExportCollectionHtml,
        /// Prompts for a new directory, creates an empty collection there and opens it.
        NewCollection,
        /// Prompts for environment files and imports them into the current collection.
        ImportEnvironment,
        /// Prompts for a file and exports the current collection's active environment there as
        /// Postman environment JSON.
        ExportEnvironment,
        /// Prompts for a folder and exports every environment of the current collection there
        /// as Postman environment files.
        ExportEnvironments,
        /// Opens the cookie jar dialog.
        OpenCookies,
        /// Opens the current collection's runtime variables dialog.
        OpenRuntimeVariables,
        /// Opens the active request's variables overview.
        OpenVariablesOverview,
        /// Opens the global environments editor.
        OpenGlobalEnvironments,
        /// Closes the current collection, asking first about unsaved changes.
        CloseCollection,
        /// Starts the current collection's mock server, or stops it when it runs.
        ToggleMockServer
    ]
);

/// The icon of the title bar's environment editor button.
pub const ENV_EDITOR_ICON: IconName = IconName::SlidersHorizontal;

/// Tooltip and accessibility label of the environment editor button; one const, so they agree.
pub const MANAGE_ENVIRONMENTS_TIP: &str = "Manage environments";

/// Why the collection's environment select and the editor button are locked with nothing open.
pub const NO_COLLECTION_ENV_TIP: &str = "Open a collection to use its environments";

/// The icon of the title bar's global environments button.
pub const GLOBAL_ENV_ICON: IconName = IconName::Globe;

/// Tooltip and accessibility label of the global environments button; one const, so they agree.
pub const GLOBAL_ENVIRONMENTS_TIP: &str = "Global environments";

/// The icon of the title bar's variables button.
pub const VARIABLES_ICON: IconName = IconName::Braces;

/// Tooltip of the title bar's variables button.
pub const VARIABLES_TIP: &str = "Variables";

/// Tooltip and accessibility label of the app menu's slot.
pub const APP_MENU_TIP: &str = "App menu";

/// Element id of the app menu's tooltip text, so a test can see when it is drawn.
pub const APP_MENU_TIP_ID: &str = "app-menu-tip";

// The no-collection state's icon, decoded once rather than on every frame.
static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    let png = include_bytes!("../../../assets/icon/generated/png/64x64.png");
    Arc::new(Image::from_bytes(ImageFormat::Png, png.to_vec()))
});

/// The root view of the main window.
pub struct Shell {
    workspace: Entity<Workspace>,
    split: Entity<ResizableState>,
    // The shown sidebar width, viewport width and sidebar visibility the split was last
    // laid out with.
    sidebar_applied: (Pixels, Pixels, bool),
    sidebar: Entity<Sidebar>,
    tab_area: Entity<TabArea>,
    autosave: Entity<Autosave>,
    env_selector: Entity<EnvSelector>,
    global_env_selector: Entity<GlobalEnvSelector>,
    sidebar_visible: bool,
    dock: Entity<ConsoleDock>,
    // Cleared on every toggle and collapse, so the dock always opens at its initial height.
    dock_split: Entity<ResizableState>,
    dock_open: bool,
    // In a window too short for the open dock it shows only its header, until a click on that
    // header, a tab key or the chord; that holds until the window grows past the threshold.
    dock_expanded_short: bool,
    dock_collapsed: bool,
    // Whether the window was below the auto-hide width last frame, and whether the sidebar
    // is hidden because of it rather than by the user.
    narrow: bool,
    // Whether the title bar shows the global environments button in place of its select.
    global_button: bool,
    // Whether the environment select was last too squeezed to show.
    env_squeezed: bool,
    sidebar_auto_hidden: bool,
    focus: FocusHandle,
    // What held focus before the shell's own handle took it; see `refocus`.
    stand_in: Rc<StandIn>,
    // Whether the app menu's popup is open; its tooltip shows only while it is closed.
    app_menu_open: Rc<Cell<bool>>,
    approvals: Option<Entity<Approvals>>,
    showing: Option<Prompt>,
    showing_script: Option<ScriptPrompt>,
    updates: Option<Entity<Updates>>,
    last_import: Option<ImportReport>,
    // Whether files from the desktop hover over the window, so the drop overlay shows.
    drop_hover: bool,
    requested_title_bar: LinuxTitleBar,
    branch: Option<SharedString>,
    branch_root: Option<PathBuf>,
    // Finished git runs of `branch_root` when HEAD was last read.
    branch_runs: u64,
    // The home directory, found once at startup; paths under it show as "~".
    home: Option<PathBuf>,
    // Whether the last layout left the branch label no room, so its buttons are no Tab stops.
    branch_clipped: Rc<Cell<bool>>,
    // Whether the last layout left the label's change count no room, so it is hidden alone.
    changes_clipped: Rc<Cell<bool>>,
    // The change count's whole text width, from the last frame.
    count_natural: Rc<Cell<Rems>>,
    // The branch label's own handle, no Tab stop; the stops after it are the label's parts.
    label_focus: FocusHandle,
    // Pixel ctrl+scroll not yet stepped.
    scroll_pending: Rc<Cell<f32>>,
    pub(crate) legacy_offers: crate::legacy_convert::Offers,
    // Resolved recent-collection labels; render never reads opencollection.yml itself.
    recent_names: std::collections::HashMap<PathBuf, SharedString>,
    title: Slot<Shell, TitleInputs>,
    status: Slot<Shell, StatusInputs>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    /// Builds the shell over `workspace`, focuses it so its actions dispatch, and starts
    /// saving window bounds and the sidebar width into the workspace's session.
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let split = cx.new(|_| ResizableState::default());
        let sidebar = cx.new(|cx| Sidebar::new(workspace.clone(), window, cx));
        let tab_area = cx.new(|cx| TabArea::new(workspace.clone(), window, cx));
        let tree = sidebar.read(cx).tree().clone();
        tab_area.update(cx, |a, _| a.set_empty_focus(tree));
        let env_selector = cx.new(|cx| EnvSelector::new(workspace.clone(), window, cx));
        let global_env_selector =
            cx.new(|cx| GlobalEnvSelector::new(workspace.clone(), window, cx));
        let autosave = cx.new(|cx| Autosave::new(workspace.clone(), Some(tab_area.clone()), cx));
        let log = crate::log_feed::ensure(cx);
        let dock = cx.new(|cx| ConsoleDock::new(log, window, cx));
        dock.update(cx, |d, _| d.set_tab_area(tab_area.downgrade()));
        let dock_split = cx.new(|_| ResizableState::default());
        let session = workspace.read(cx).session().clone();
        let mut subscriptions = vec![
            // A single click previews a request and a double click keeps its tab open.
            cx.subscribe_in(
                &sidebar,
                window,
                |this, _, e: &SidebarEvent, window, cx| match e {
                    SidebarEvent::OpenItem {
                        root,
                        path,
                        permanent,
                    } => {
                        this.workspace
                            .update(cx, |w, cx| w.open_item(root, path.clone(), !permanent, cx));
                    }
                    SidebarEvent::OpenSettings { root, folder } => {
                        if folder.is_none()
                            && crate::scratch::refuse(&this.workspace, root, window, cx)
                        {
                            return;
                        }
                        let kind = folder.clone().map_or(TabKind::Collection, TabKind::Folder);
                        this.workspace
                            .update(cx, |w, cx| w.open_settings(root, kind, cx));
                    }
                    SidebarEvent::Run { root, folder } => {
                        if crate::scratch::refuse(&this.workspace, root, window, cx) {
                            return;
                        }
                        let target = folder.clone().unwrap_or_else(ItemPath::root);
                        this.workspace
                            .update(cx, |w, cx| w.open_runner(root, target, cx));
                    }
                    SidebarEvent::RunSelection { root, items } => {
                        if crate::scratch::refuse(&this.workspace, root, window, cx) {
                            return;
                        }
                        this.workspace
                            .update(cx, |w, cx| w.open_runner_items(root, items.clone(), cx));
                    }
                    SidebarEvent::OpenExample {
                        root,
                        path,
                        index,
                        permanent,
                    } => {
                        this.workspace.update(cx, |w, cx| {
                            w.open_example(root, path.clone(), *index, !permanent, cx)
                        });
                    }
                    // A draft row acts on its open tab: Save runs on it as the active tab, and
                    // Close goes through the tab area's unsaved prompt.
                    SidebarEvent::SaveDraft { root, path } => {
                        let Some(id) = this.draft_tab(root, path, cx) else {
                            return;
                        };
                        this.workspace
                            .update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(id)));
                        (this.tab_area).update(cx, |t, cx| t.save_active(&SaveActive, window, cx));
                    }
                    SidebarEvent::CloseDraft { root, path } => {
                        if let Some(id) = this.draft_tab(root, path, cx) {
                            (this.tab_area).update(cx, |t, cx| t.close(vec![id], window, cx));
                        }
                    }
                },
            ),
            // A loss inside an open dialog goes back into that dialog. Otherwise, with no
            // collection open nothing below the shell can hold focus, so a loss lands on the
            // shell itself, which keeps the app-wide keys reachable.
            cx.on_focus_lost(window, |this, window, cx| {
                if crate::dialog_focus::repair_lost(window, cx) {
                    return;
                }
                if this.workspace.read(cx).shows_empty_state(cx) {
                    this.focus.focus(window, cx);
                    crate::focus_redraw::after_draw_move(window, cx);
                    return;
                }
                // A closing dialog can hand focus back to a control gone from the window. Once
                // the tab area has had its say, the shell takes what is left. Focus dropped on
                // purpose, on a spot that takes none, stays dropped.
                cx.defer_in(window, |this, window, cx| {
                    if window.focused(cx).is_some() {
                        this.take_stranded_focus(window, cx);
                    }
                });
            }),
            cx.observe(&workspace, |this, _, cx| {
                // A finished git run is how the watcher reports a HEAD move, so HEAD is
                // re-read with the changed-file count and the two stay in step.
                let root = this.workspace.read(cx).current_root();
                let runs = root
                    .as_deref()
                    .map_or(0, |r| this.workspace.read(cx).git_runs(r));
                this.refresh_branch(runs != this.branch_runs, cx);
                this.cache_missing_recent_names(cx);
                cx.notify();
            }),
            // A squeezed environment select leaves the Tab order and takes its focus with it,
            // as the global select does when its button replaces it.
            cx.observe_in(&env_selector, window, |this, selector, window, cx| {
                let squeezed = selector.read(cx).squeezed();
                if squeezed != this.env_squeezed {
                    this.env_squeezed = squeezed;
                    cx.defer_in(window, Self::take_stranded_focus);
                }
            }),
            // HEAD can move while the window is in the background, so every activation
            // change re-reads it; the test window never reports itself active.
            // The changed-file count can go stale the same way. A finished status run only
            // notifies the workspace, never activation, so this cannot loop.
            cx.observe_window_activation(window, |this, _, cx| {
                this.refresh_branch(true, cx);
                this.workspace.update(cx, |w, cx| w.refresh_all_git(cx));
            }),
            cx.subscribe_in(&workspace, window, Self::workspace_event),
            cx.subscribe_in(&session, window, |_, _, e: &SaveFailed, window, cx| {
                let text = format!("Could not save the session: {}", e.0);
                window.push_notification(Notification::error(text), cx);
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                let bounds = Some(SavedBounds::from_window(window.window_bounds()));
                this.save_session(cx, move |s| s.window = bounds);
            }),
            cx.subscribe(&dock, |this, _, _: &ExpandDock, cx| {
                this.dock_expanded_short = true;
                cx.notify();
            }),
            // A hidden sidebar reports no real width, so the saved one is kept.
            cx.subscribe(&split, |this, split, _: &ResizablePanelEvent, cx| {
                if !this.sidebar_visible {
                    return;
                }
                let width = split.read(cx).sizes().first().map(|w| f32::from(*w));
                this.save_session(cx, move |s| s.sidebar_width = width);
            }),
        ];
        // The status bar reads the response layout; the shell then compares it in its render.
        subscriptions.extend(ui::redraw_on_settings(cx));
        // The settings exist only under `app::start`; a bare shell in a test has none.
        let mut requested_title_bar = LinuxTitleBar::default();
        if let Some(settings) = cx.try_global::<AppState>().map(|s| s.settings.clone()) {
            crate::theme::follow(settings.clone(), window, cx);
            // `start` requested the stored choice; later changes are requested at runtime.
            requested_title_bar = settings.read(cx).get().linux_title_bar;
            subscriptions.push(
                cx.observe_in(&settings, window, |this, settings, window, cx| {
                    let choice = settings.read(cx).get().linux_title_bar;
                    if choice == this.requested_title_bar {
                        return;
                    }
                    if let Some(decorations) = window_decorations(choice) {
                        window.request_decorations(decorations);
                        this.requested_title_bar = choice;
                    }
                }),
            );
            // Autosave follows the preference now and on every later change.
            let on = settings.read(cx).get().autosave;
            autosave.update(cx, |a, cx| a.set_enabled(on, cx));
            subscriptions.push(cx.observe(&settings, |this: &mut Shell, settings, cx| {
                let on = settings.read(cx).get().autosave;
                this.autosave.update(cx, |a, cx| a.set_enabled(on, cx));
            }));
            subscriptions.push(cx.subscribe_in(
                &settings,
                window,
                |_, _, e: &SaveFailed, window, cx| {
                    let text = format!("Could not save the settings: {}", e.0);
                    window.push_notification(Notification::error(text), cx);
                },
            ));
        }
        if let Some(drafts) = cx.try_global::<Drafts>().map(|d| d.0.clone()) {
            subscriptions.push(cx.subscribe_in(
                &drafts,
                window,
                |_, _, e: &DraftStoreEvent, window, cx| {
                    match e {
                        DraftStoreEvent::SaveFailed(error) => {
                            let text = format!("Could not keep a draft for after restart: {error}");
                            window.push_notification(Notification::error(text), cx);
                        }
                        DraftStoreEvent::NotKept { name, reason } => {
                            let text = match reason {
                                NotKeptReason::TooLarge => format!(
                                    "\"{name}\" is over 1 MiB and will not be kept after restart."
                                ),
                                NotKeptReason::Unserializable => format!(
                                    "\"{name}\" will not be kept after restart: its request cannot be written to a file."
                                ),
                                NotKeptReason::TooMany => format!(
                                    "\"{name}\" will not be kept after restart: {MAX_DRAFTS} drafts are kept already."
                                ),
                            };
                            window.push_notification(Notification::warning(text), cx);
                        }
                        DraftStoreEvent::CredentialsBlanked => {
                            let text = "Restored drafts had their credentials cleared; enter them again before sending.";
                            window.push_notification(Notification::warning(text), cx);
                        }
                    }
                },
            ));
        }
        if let Some(keymap) = cx.try_global::<AppState>().map(|s| s.keymap.clone()) {
            subscriptions.push(cx.subscribe_in(
                &keymap,
                window,
                |_, _, e: &SaveFailed, window, cx| {
                    let text = format!("Could not save the shortcuts: {}", e.0);
                    window.push_notification(Notification::error(text), cx);
                },
            ));
        }
        let approvals = cx.try_global::<AppState>().map(|s| s.approvals.clone());
        if let Some(approvals) = &approvals {
            subscriptions.push(cx.observe_in(approvals, window, |this, _, window, cx| {
                this.show_next_approval(window, cx);
            }));
            subscriptions.push(
                cx.observe_global_in::<crate::oauth2_signin::SignIn>(window, |this, window, cx| {
                    this.show_next_approval(window, cx)
                }),
            );
            subscriptions.push(cx.subscribe_in(
                approvals,
                window,
                |_, _, e: &ApprovalsEvent, window, cx| match e {
                    ApprovalsEvent::WatchFailed(message) => {
                        let text =
                            format!("Approval requests will not appear until restart: {message}");
                        window.push_notification(Notification::warning(text), cx);
                    }
                },
            ));
            // Closing the window is one more way out of a prompt, so it dismisses.
            subscriptions.push(cx.on_release(|this: &mut Shell, cx| {
                let Some(approvals) = &this.approvals else {
                    return;
                };
                if let Some(prompt) = this.showing.take() {
                    approvals.update(cx, |a, cx| a.dismiss(&prompt, cx));
                }
                if let Some(prompt) = this.showing_script.take() {
                    approvals.update(cx, |a, cx| a.dismiss_scripts(&prompt, cx));
                }
            }));
        }
        let updates = cx.try_global::<AppState>().map(|s| s.updates.clone());
        if let Some(updates) = &updates {
            subscriptions.push(cx.observe(updates, |_, _, cx| cx.notify()));
            subscriptions.push(cx.subscribe_in(
                updates,
                window,
                |_, _, e: &UpdatesEvent, window, cx| {
                    let UpdatesEvent::Reported(report) = e;
                    if let Some(note) = report_notification(report) {
                        window.push_notification(note, cx);
                    }
                },
            ));
        }
        crate::closing::install(workspace.clone(), tab_area.clone(), window, cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        // The shell's handle is no Tab stop. It takes focus only as a stand-in: on a press on a
        // bare spot, and from a dialog opened by the app menu, which dispatches from it. Focus
        // then goes back to what held it before a key or press, or to the active tab.
        let stand_in = Rc::new(StandIn::default());
        subscriptions.push(cx.on_focus(&focus, window, {
            let stand_in = Rc::clone(&stand_in);
            move |this, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                if let Some(held) = stand_in.take() {
                    this.refocus(&held, window, cx);
                }
            }
        }));
        let me = window.window_handle();
        subscriptions.push(cx.intercept_keystrokes({
            let (stand_in, shell) = (Rc::clone(&stand_in), focus.clone());
            move |_, window, cx| {
                if window.window_handle() == me {
                    stand_in.hold(&shell, window, cx);
                }
            }
        }));
        let mut shell = Shell {
            workspace,
            split,
            sidebar_applied: (px(0.), px(0.), false),
            sidebar,
            tab_area,
            autosave,
            env_selector,
            global_env_selector,
            sidebar_visible: true,
            dock,
            dock_split,
            dock_open: false,
            dock_expanded_short: false,
            dock_collapsed: false,
            narrow: false,
            global_button: false,
            env_squeezed: false,
            sidebar_auto_hidden: false,
            focus,
            stand_in,
            app_menu_open: Rc::default(),
            requested_title_bar,
            approvals,
            showing: None,
            showing_script: None,
            updates,
            last_import: None,
            drop_hover: false,
            branch: None,
            branch_root: None,
            branch_runs: 0,
            home: dirs::home_dir(),
            branch_clipped: Rc::default(),
            changes_clipped: Rc::default(),
            count_natural: Rc::new(Cell::new(Rems(0.))),
            label_focus: cx.focus_handle(),
            scroll_pending: Rc::default(),
            legacy_offers: Default::default(),
            recent_names: Default::default(),
            title: Slot::new(cx.weak_entity(), ShellPart::Title, false, cx),
            status: Slot::new(cx.weak_entity(), ShellPart::Status, false, cx),
            _subscriptions: subscriptions,
        };
        shell.refresh_branch(true, cx);
        shell.cache_missing_recent_names(cx);
        shell
    }

    /// The git branch label for the current collection, if it sits in a repository.
    pub fn branch(&self) -> Option<&SharedString> {
        self.branch.as_ref()
    }

    // Re-reads HEAD when the current collection changed, or always when `force` is set.
    fn refresh_branch(&mut self, force: bool, cx: &mut Context<Self>) {
        let root = self.workspace.read(cx).current_root();
        if !force && root == self.branch_root {
            return;
        }
        let branch = root
            .as_deref()
            .and_then(crate::git_head::branch_label)
            .map(SharedString::from);
        self.branch_runs = root
            .as_deref()
            .map_or(0, |r| self.workspace.read(cx).git_runs(r));
        self.branch_root = root;
        if branch != self.branch {
            self.branch = branch;
            cx.notify();
        }
    }

    /// The workspace this window shows.
    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspace
    }

    // The open tab showing the draft `path` of the collection at `root`.
    fn draft_tab(&self, root: &Path, path: &ItemPath, cx: &App) -> Option<TabId> {
        let tabs = self.workspace.read(cx).tabs();
        let tab = tabs
            .entries()
            .iter()
            .find(|e| e.root == root && e.item() == Some(path))?;
        Some(tab.id)
    }

    /// The sidebar view of the open collections.
    pub fn sidebar(&self) -> &Entity<Sidebar> {
        &self.sidebar
    }

    /// The tab area beside the sidebar.
    pub fn tab_area(&self) -> &Entity<TabArea> {
        &self.tab_area
    }

    /// The autosave that follows the preference.
    pub fn autosave(&self) -> &Entity<Autosave> {
        &self.autosave
    }

    /// The console dock under the sidebar and tab area.
    pub fn dock(&self) -> &Entity<ConsoleDock> {
        &self.dock
    }

    /// Whether the console dock is shown.
    pub fn dock_open(&self) -> bool {
        self.dock_open
    }

    /// The title bar's environment selector.
    pub fn env_selector(&self) -> &Entity<EnvSelector> {
        &self.env_selector
    }

    /// The colour of the stripe drawn along the window's top edge, if any.
    pub fn env_stripe(&self, cx: &App) -> Option<Hsla> {
        let workspace = self.workspace.read(cx);
        let root = workspace.current_root()?;
        let name = workspace.selected_environment(&root, cx)?;
        crate::env_color::env_fill(workspace.environment_color(&root, &name, cx).as_deref())
    }

    /// The title bar's global environment picker.
    pub fn global_env_selector(&self) -> &Entity<GlobalEnvSelector> {
        &self.global_env_selector
    }

    /// Whether the sidebar panel is shown; hiding it is not saved in the session.
    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    /// The sidebar and tab-area split.
    pub fn split(&self) -> &Entity<ResizableState> {
        &self.split
    }

    /// The split between the tab area and the console dock.
    pub fn dock_split(&self) -> &Entity<ResizableState> {
        &self.dock_split
    }

    /// The approval prompt on screen, if any.
    pub fn approval_shown(&self) -> Option<&Prompt> {
        self.showing.as_ref()
    }

    /// The script approval prompt on screen, if any.
    pub fn script_approval_shown(&self) -> Option<&ScriptPrompt> {
        self.showing_script.as_ref()
    }

    /// The report of the last collection imported in this window, if any.
    pub fn last_import(&self) -> Option<&ImportReport> {
        self.last_import.as_ref()
    }

    /// Forgets the prompt on screen; called by the prompt before it records its answer.
    pub(crate) fn approval_answered(&mut self) {
        self.showing = None;
    }

    /// Forgets the script prompt on screen; called by it before it records its answer.
    pub(crate) fn script_approval_answered(&mut self) {
        self.showing_script = None;
    }

    /// Replaces the home directory used to abbreviate paths; tests inject theirs.
    pub fn set_home(&mut self, home: Option<PathBuf>) {
        self.home = home;
    }

    /// The shown name of the current collection, if one is open.
    pub fn current_name(&self, cx: &App) -> Option<SharedString> {
        let workspace = self.workspace.read(cx);
        Some(workspace.collection_name(&workspace.current_root()?, cx))
    }

    // One prompt at a time: the head of a queue opens only once the last one is answered,
    // host prompts before script prompts.
    fn show_next_approval(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(approvals) = self.approvals.clone() else {
            return;
        };
        // The sign-in dialog closes itself with `close_dialog`, so nothing may stack on it.
        if self.showing.is_some()
            || self.showing_script.is_some()
            || crate::oauth2_signin::showing(cx).is_some()
        {
            return;
        }
        let shell = cx.entity().downgrade();
        if let Some(head) = approvals.read(cx).queue().first().cloned() {
            self.showing = Some(head.clone());
            let name = self
                .workspace
                .read(cx)
                .collection_name(&head.collection, cx);
            crate::approval_prompt::open(shell, approvals, head, name, window, cx);
        } else if let Some(head) = approvals.read(cx).script_queue().first().cloned() {
            self.showing_script = Some(head.clone());
            let name = self
                .workspace
                .read(cx)
                .collection_name(&head.collection, cx);
            crate::script_prompt::open(shell, approvals, head, name, window, cx);
        }
    }

    fn session(&self, cx: &App) -> Entity<Persisted<Session>> {
        self.workspace.read(cx).session().clone()
    }

    // Writes only a real change, so an unchanged resize or bounds report costs no save.
    fn save_session(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut Session)) {
        let session = self.session(cx);
        let mut next = session.read(cx).get().clone();
        f(&mut next);
        if &next != session.read(cx).get() {
            session.update(cx, |p, cx| p.update(cx, |s| *s = next));
        }
    }

    fn on_open_collection(
        &mut self,
        _: &OpenCollection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_open(window, cx);
    }

    // A hidden panel leaves the rendered tree, and focus inside it would drop to the window
    // root, above the shell's key context. Focus outside the tab area moves to the active tab,
    // else to the shell; the sidebar's tree keeps its focus handle private, so this is the test.
    fn toggle_sidebar(&mut self, _: &ToggleSidebar, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_auto_hidden = false;
        self.set_sidebar(!self.sidebar_visible, window, cx);
    }

    // The dock polls only while shown, and focus inside a closing dock moves to the active tab,
    // else to the shell, as for the sidebar. A collapsed dock expands rather than closes.
    fn toggle_console(&mut self, _: &ToggleConsole, window: &mut Window, cx: &mut Context<Self>) {
        if self.dock_open && self.dock_collapsed {
            self.dock_expanded_short = true;
            cx.notify();
            return;
        }
        self.dock_open = !self.dock_open;
        self.dock_split.update(cx, |s, _| s.clear());
        let open = self.dock_open;
        self.dock.update(cx, |d, cx| d.set_polling(open, cx));
        if !open && self.dock.focus_handle(cx).contains_focused(window, cx) {
            self.hand_back_focus(window, cx);
        }
        cx.notify();
    }

    // Focus leaving a dock part that is no longer drawn goes to the active tab, else the shell.
    fn hand_back_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.tab_area.read(cx).active_focus(cx);
        target
            .unwrap_or_else(|| self.focus.clone())
            .focus(window, cx);
    }

    /// Makes the collection current and shows the sidebar, even one the narrow window hid,
    /// with the collection's root row selected and scrolled into view.
    pub fn reveal_collection(&mut self, root: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |w, cx| w.set_current_root(root, cx));
        self.sidebar_auto_hidden = false;
        if !self.sidebar_visible {
            self.set_sidebar(true, window, cx);
        }
        self.sidebar
            .update(cx, |s, cx| s.reveal_root(root, window, cx));
    }

    // Focus on the filter of a hidden sidebar would sit outside the drawn tree, and actions
    // would then dispatch from the window root past the shell; so the sidebar comes back first.
    fn focus_filter(&mut self, a: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_auto_hidden = false;
        if !self.sidebar_visible {
            self.set_sidebar(true, window, cx);
        }
        self.sidebar
            .update(cx, |s, cx| s.focus_filter(a, window, cx));
    }

    fn set_sidebar(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_visible = visible;
        if !self.sidebar_visible && !self.tab_area.focus_handle(cx).contains_focused(window, cx) {
            let target = self.tab_area.read(cx).active_focus(cx);
            target
                .unwrap_or_else(|| self.focus.clone())
                .focus(window, cx);
        }
        cx.notify();
    }

    // Open request editors observe the settings, so they re-split on this write.
    fn toggle_orientation(
        &mut self,
        _: &ToggleOrientation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(settings) = cx.try_global::<AppState>().map(|s| s.settings.clone()) else {
            return;
        };
        settings.update(cx, |p, cx| {
            p.update(cx, |s| {
                s.response_orientation = match s.response_orientation {
                    Orientation::Vertical => Orientation::Horizontal,
                    Orientation::Horizontal => Orientation::Vertical,
                }
            })
        });
    }

    // The choice flips against the mode on screen, so a System choice becomes the opposite
    // fixed theme; `theme::follow` applies the write.
    fn toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        let Some(settings) = cx.try_global::<AppState>().map(|s| s.settings.clone()) else {
            return;
        };
        let dark = gpui_kit::component::Theme::global(cx).mode.is_dark();
        settings.update(cx, |p, cx| {
            p.update(cx, |s| {
                s.theme = if dark {
                    ThemeChoice::Light
                } else {
                    ThemeChoice::Dark
                }
            })
        });
    }

    // A zoom key with focus outside the response body sizes the body when the pointer is
    // over the active tab's shown body, and zooms the whole app otherwise.
    fn zoom(&mut self, step: ZoomStep, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.response_hovered(window, cx) {
            ZoomTarget::Response
        } else {
            ZoomTarget::App
        };
        crate::zoom::apply_step(target, step, cx);
    }

    // Only the active tab's body, and only while the pane shows it, is consulted, so a hidden
    // view's stale bounds never count.
    fn response_hovered(&self, window: &Window, cx: &App) -> bool {
        let Some(id) = self.workspace.read(cx).tabs().active().map(|t| t.id) else {
            return false;
        };
        let Some(editor) = self.tab_area.read(cx).editor(id) else {
            return false;
        };
        let pane = editor.read(cx).response().read(cx);
        pane.body_shown(cx) && pane.body().read(cx).pointer_inside(window)
    }

    /// The Linux title bar choice the shell last requested of the window.
    pub fn requested_title_bar(&self) -> LinuxTitleBar {
        self.requested_title_bar
    }

    /// The title bar's close: a clean window closes, otherwise the save prompt opens.
    pub fn request_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::closing::confirm_close(self.workspace.clone(), CloseScope::Window, window, cx);
    }

    // Asks for one directory and opens it; a cancelled or failed prompt does nothing.
    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open collection".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = reply.await else {
                return;
            };
            let Some(root) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |shell, window, cx| shell.open(root, window, cx));
        })
        .detach();
    }

    // The list is read through the store queue so sends saved just before are in it; a
    // closed channel (no queue) opens an empty list.
    fn open_history(&mut self, _: &OpenHistory, window: &mut Window, cx: &mut Context<Self>) {
        let ws = self.workspace.read(cx);
        let roots = ws
            .collections()
            .iter()
            .map(|c| c.root().to_path_buf())
            .collect();
        let (reply, list) = async_channel::bounded(1);
        let op = crate::last_response::StoreOp::Recent {
            roots,
            // Uncapped: the palette searches every send of a request still open.
            limit: usize::MAX,
            reply,
        };
        crate::last_response::enqueue(op, cx);
        cx.spawn_in(window, async move |this, cx| {
            let recent = list.recv().await.unwrap_or_default();
            let _ = this.update_in(cx, |this, window, cx| {
                let entries = crate::history_palette::all_history_entries(
                    &recent,
                    this.workspace.read(cx),
                    cx,
                );
                crate::history_palette::open_history(this.focus.clone(), entries, window, cx);
            });
        })
        .detach();
    }

    // A draft's file is gone, so its kept request snapshot seeds a new draft that continues
    // the same history; with no readable snapshot the user is told and nothing opens.
    fn reopen_draft(&mut self, a: &OpenHistoryEntry, window: &mut Window, cx: &mut Context<Self>) {
        let (root, path, seq) = (a.root.clone(), a.path.clone(), a.seq);
        let (reply, answer) = async_channel::bounded(1);
        let op = crate::last_response::StoreOp::PeekRequest {
            root: root.clone(),
            item: path.clone(),
            seq,
            reply,
        };
        crate::last_response::enqueue(op, cx);
        cx.spawn_in(window, async move |this, cx| {
            let json = answer.recv().await.ok().flatten();
            let snap = json.and_then(|json| DraftSnapshot::from_json(&json).ok());
            let _ = this.update_in(cx, |this, window, cx| match snap {
                Some(snap) => {
                    // A hidden credential would go out as its placeholder, so the user is told.
                    let masked = crate::scrub::holds_placeholder(&snap.to_json());
                    let opened = (this.workspace)
                        .update(cx, |w, cx| w.reopen_draft(&root, path, seq, snap, cx));
                    if masked && opened.is_some() {
                        let text = crate::history_palette::DRAFT_MASKED;
                        // An empty marker carries the text, since the kit cannot read a toast's.
                        let toast = Notification::warning(text).content(move |_, _, _| {
                            div()
                                .id("toast-draft-masked")
                                .test_support()
                                .aria_label(text)
                                .into_any_element()
                        });
                        window.push_notification(toast, cx);
                    }
                }
                None => window.push_notification(
                    Notification::warning(crate::history_palette::DRAFT_GONE),
                    cx,
                ),
            });
        })
        .detach();
    }

    // The current collection's root when it sits in a git repository; otherwise a notice says
    // why the git command cannot run.
    fn repository_root(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<PathBuf> {
        let ws = self.workspace.read(cx);
        let Some(root) = ws.current_root().filter(|r| !ws.is_scratch(r)) else {
            window.push_notification(ui::info_toast("Open a collection first"), cx);
            return None;
        };
        if self.branch().is_none() {
            window.push_notification(
                ui::info_toast("This collection is not in a git repository"),
                cx,
            );
            return None;
        }
        Some(root)
    }

    // The palette's twin of the status bar toggle, for the current user collection.
    fn toggle_mock_server(&mut self, _: &ToggleMockServer, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((root, _)) = self.current_mock(cx) {
            self.workspace.update(cx, |w, cx| w.toggle_mock(&root, cx));
        }
    }

    // Opens the Changes dialog, as the title bar's changed-file count does.
    fn show_changes(&mut self, _: &ShowChanges, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.repository_root(window, cx) else {
            return;
        };
        if self.workspace.read(cx).git_changes(&root).is_none() {
            window.push_notification(ui::info_toast("Changes are not available yet"), cx);
            return;
        }
        open_changes(&self.workspace, &root, None, window, cx);
    }

    // The picker needs a repository; its events refresh the label, since a switch keeps the root
    // and `refresh_branch` would otherwise leave the old name up.
    fn open_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.repository_root(window, cx) else {
            return;
        };
        let picker = crate::branch_dialog::open(&self.workspace, root, window, cx);
        cx.subscribe_in(&picker, window, Self::on_picker_event)
            .detach();
        // A switch can end with no event (timeout, failure after HEAD moved), yet always notifies.
        cx.observe(&picker, |this, _, cx| this.refresh_branch(true, cx))
            .detach();
    }

    fn on_picker_event(
        &mut self,
        picker: &Entity<crate::branch_picker::BranchPicker>,
        event: &crate::branch_picker::PickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::branch_picker::{Notice, PickerEvent};
        self.refresh_branch(true, cx);
        match event {
            PickerEvent::CollectionClosed { message, .. } => {
                window.push_notification(Notification::warning(message.clone()), cx);
            }
            // The dialog closes on landing, so a note about the branch's upstream becomes a toast.
            PickerEvent::Switched { .. } => {
                if let Some(Notice::OtherUpstream(text)) = picker.read(cx).notice() {
                    window.push_notification(ui::info_toast(text.clone()), cx);
                }
            }
        }
    }

    fn clone_repository(
        &mut self,
        _: &CloneRepository,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::clone_dialog::open(&self.workspace, window, cx);
    }

    // Copies the bundled demo into a folder the user picks, then opens it. Cancelling does
    // nothing; a failed copy raises an alert.
    fn open_demo(&mut self, _: &OpenDemoCollection, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = this.update(cx, |_, cx| {
                cx.prompt_for_paths(PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some("Copy the demo into".into()),
                })
            }) else {
                return;
            };
            let Some(parent) = first_path(reply.await) else {
                return;
            };
            let job = cx.background_spawn(async move { crate::demo_collection::install(&parent) });
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(root) => this.open(root, window, cx),
                Err(e) => {
                    let text: SharedString = crate::ui::plain_quotes(&e.to_string()).into();
                    window.open_alert_dialog(cx, move |dialog, _, _| {
                        dialog
                            .title("Could not copy the demo collection")
                            .description(text.clone())
                    });
                }
            });
        })
        .detach();
    }

    // With no collection the draft goes to the scratch; a failure to open it already raised a notice.
    fn import_curl(&mut self, _: &ImportCurl, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(root) = self.workspace.update(cx, |w, cx| w.draft_root(cx)) {
            crate::import_curl::open(&self.workspace, root, window, cx);
        }
    }

    // Import opens the link dialog; its Browse button runs the file prompt.
    fn prompt_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::import_link::open(cx.weak_entity(), window, cx);
    }

    /// Opens what an import wrote, each in the first environment its import produced so its
    /// first send has the variables, then its report and any failure behind it.
    pub(crate) fn landed(
        &mut self,
        roots: Written,
        report: Option<ImportReport>,
        error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (root, environment) in roots {
            if let Some(name) = environment {
                self.workspace
                    .update(cx, |w, _| w.select_on_open(&root, name));
            }
            self.open(root, window, cx);
        }
        if let Some(report) = report {
            if !report.notes.is_empty() {
                crate::import_report::open("Import report", &report, window, cx);
            }
            self.last_import = Some(report);
        }
        if let Some(message) = error {
            import_failed(message, window, cx);
        }
    }

    // Each dropped path is sniffed off the main thread, then handled in order: a folder opens,
    // a curl file becomes a draft, a collection file asks where it goes, anything else is
    // refused with a toast. One path failing never stops the rest.
    fn files_dropped(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_hover = false;
        cx.notify();
        let store = Arc::clone(self.workspace.read(cx).store());
        cx.spawn_in(window, async move |this, cx| {
            let sniffed = cx
                .background_spawn(
                    async move { paths.iter().map(|p| classify(p)).collect::<Vec<_>>() },
                )
                .await;
            for dropped in sniffed {
                match dropped {
                    Dropped::Folder(root) => {
                        let _ = this.update_in(cx, |this, window, cx| this.open(root, window, cx));
                    }
                    Dropped::Request(request) => {
                        let _ = this.update(cx, |this, cx| this.draft_from_drop(*request, cx));
                    }
                    Dropped::Collections(bundles) => {
                        place_bundles(&this, bundles, Arc::clone(&store), cx).await;
                    }
                    Dropped::Refused(message) => {
                        let _ = this.update_in(cx, |_, window, cx| {
                            window.push_notification(Notification::error(message), cx)
                        });
                    }
                }
            }
        })
        .detach();
    }

    // The dropped curl command lands at the current root or the scratch, as Import curl does.
    fn draft_from_drop(&mut self, request: ImportedRequest, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.update(cx, |w, cx| w.draft_root(cx)) else {
            return;
        };
        self.workspace
            .update(cx, |w, cx| w.new_draft_from_import(&root, request, cx));
    }

    // Asks where the new collection goes, beside the current one or else in the home directory;
    // the chosen directory's name is the collection's. A cancelled prompt does nothing.
    fn prompt_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .workspace
            .read(cx)
            .current_root()
            .and_then(|root| root.parent().map(Path::to_path_buf))
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let reply = cx.prompt_for_new_path(&start, Some("New Collection"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(dir))) = reply.await else {
                return;
            };
            let job =
                cx.background_spawn(async move { crate::save_draft::create_collection(&dir) });
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(root) => this.open(root, window, cx),
                Err(message) => {
                    let text: SharedString = message.into();
                    window.open_alert_dialog(cx, move |dialog, _, _| {
                        dialog
                            .title("Could not create collection")
                            .description(text.clone())
                    });
                }
            });
        })
        .detach();
    }

    /// `menu` in a div `app-menu-slot` that carries the menu's accessibility label, and its
    /// tooltip while the menu is closed (`open` false), since the app menu itself takes neither.
    fn app_menu_slot(menu: impl IntoElement, open: bool) -> impl IntoElement {
        no_window_move(div().id("app-menu-slot"))
            .test_support()
            .aria_label(APP_MENU_TIP)
            .when(!open, |slot| {
                slot.tooltip(|window, cx| {
                    let tip = |_: &mut Window, _: &mut App| {
                        div().id(APP_MENU_TIP_ID).test_support().child(APP_MENU_TIP)
                    };
                    Tooltip::element(tip).build(window, cx)
                })
            })
            .child(menu)
    }

    // Moves focus that is out of reach of the shell's keys back to the control the user was in,
    // or, once that is gone, on to the active tab or the tree. Taking the record also ends a
    // freeze left by an app menu removed while open. A dialog or a drawn control keeps focus.
    fn take_stranded_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) || window.is_action_available(&OpenCommandPalette, cx) {
            return;
        }
        match self.stand_in.take() {
            Some(held) => self.refocus(&held, window, cx),
            None => self.hand_on_focus(window, cx),
        }
    }

    // Focus for the active tab, or the tree when no tab view is built. The shell's own handle
    // takes it only while nothing else that could is drawn, since it shows no ring.
    fn hand_on_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.read(cx).shows_empty_state(cx) {
            self.focus.focus(window, cx);
            return;
        }
        let area = self.tab_area.read(cx);
        let to_tree = (area.active_focus(cx)).is_none_or(|f| f == area.focus_handle(cx));
        if to_tree && !self.sidebar_visible {
            self.focus.focus(window, cx);
        } else {
            (self.tab_area).update(cx, |t, cx| t.focus_fallback(window, cx));
        }
    }

    // Focuses `held` again; once it is no longer drawn (a closed menu, a closed tab), the
    // shell's keys are out of reach from it, and focus goes to the active tab or a drawn tree.
    fn refocus(&mut self, held: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        held.focus(window, cx);
        if !window.is_action_available(&OpenCommandPalette, cx)
            && !self.workspace.read(cx).shows_empty_state(cx)
        {
            self.hand_on_focus(window, cx);
        }
        crate::focus_redraw::after_draw_move(window, cx);
    }

    // Opens the environments tab of the collection the selector acts on, if any is open, with
    // focus in it, so the keys go on to the tab rather than staying where they were.
    fn open_environments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let opened = self.workspace.update(cx, |w, cx| {
            let root = w.current_root()?;
            w.open_settings(&root, TabKind::Environments, cx)
        });
        if let Some(id) = opened {
            (self.tab_area).update(cx, |t, cx| t.focus_tab(id, window, cx));
            crate::focus_redraw::after_draw_move(window, cx);
        }
    }

    // Exports the active environment of the current collection as saved; without one, says so
    // instead of opening a save dialog.
    fn export_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ws = self.workspace.read(cx);
        let Some(root) = ws.current_root() else {
            return;
        };
        let Some(name) = ws.selected_environment(&root, cx) else {
            let note = Notification::error("Select an environment to export");
            window.push_notification(note, cx);
            return;
        };
        let dirty = ws.snapshot(&root, cx).is_some_and(|s| {
            let env = s.content.environments.iter().find(|e| e.name == name);
            env.is_some_and(|e| e.dirty)
        });
        crate::export::export_environment(&root, &name, dirty, window, cx);
    }

    // Exports every environment of the current collection as saved to a chosen folder.
    fn export_environments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ws = self.workspace.read(cx);
        let Some(root) = ws.current_root() else {
            return;
        };
        let snapshot = ws.snapshot(&root, cx);
        let envs = snapshot
            .as_ref()
            .map_or(&[][..], |s| &s.content.environments[..]);
        let dirty = envs.iter().any(|e| e.dirty);
        let source = crate::export::EnvironmentSource::Collection(root.clone());
        crate::export::export_environments(source, envs.len(), dirty, window, cx);
    }

    // Opens the settings tab of the collection the selector acts on, if any is open.
    fn open_collection_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.workspace.read(cx).current_root() else {
            return;
        };
        if crate::scratch::refuse(&self.workspace, &root, window, cx) {
            return;
        }
        self.workspace
            .update(cx, |w, cx| w.open_settings(&root, TabKind::Collection, cx));
    }

    // Opens a runner tab without starting it. With `folder`, an active request or example
    // runs its own folder; anything else runs the whole current collection.
    fn run_current(&mut self, folder: bool, window: &mut Window, cx: &mut Context<Self>) {
        let w = self.workspace.read(cx);
        let item = w
            .tabs()
            .active()
            .filter(|_| folder)
            .and_then(|t| match &t.kind {
                TabKind::Request(p) | TabKind::Example { item: p, .. } => {
                    Some((t.root.clone(), p.parent().unwrap_or_else(ItemPath::root)))
                }
                _ => None,
            });
        let target = item.or_else(|| w.current_root().map(|r| (r, ItemPath::root())));
        let Some((root, path)) = target else {
            return;
        };
        if crate::scratch::refuse(&self.workspace, &root, window, cx) {
            return;
        }
        self.workspace
            .update(cx, |w, cx| w.open_runner(&root, path, cx));
    }

    /// Re-reads the names of the recent collections into the in-memory cache. Render only
    /// looks names up, so this is the one place they come off the disk.
    pub fn refresh_recent_names(&mut self, cx: &mut Context<Self>) {
        let recent = self.session(cx).read(cx).get().recent.clone();
        self.recent_names = (recent.into_iter())
            .map(|root| (root.clone(), collection_label(&root)))
            .collect();
        cx.notify();
    }

    // Reads the names of recents not cached yet, so a collection that just entered the list
    // gets its name without re-reading the ones already known.
    fn cache_missing_recent_names(&mut self, cx: &Context<Self>) {
        let recent = self.session(cx).read(cx).get().recent.clone();
        for root in recent {
            let label = || collection_label(&root);
            self.recent_names.entry(root.clone()).or_insert_with(label);
        }
    }

    // The cached name of a recent collection; a miss falls back to the directory name
    // without touching the disk.
    fn recent_name(&self, root: &Path) -> SharedString {
        (self.recent_names.get(root).cloned()).unwrap_or_else(|| display_name(root))
    }

    // A collection already open becomes current and is revealed at once; a new one once it has
    // loaded. A folder of collections only opens them.
    fn open(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let absolute = crate::workspace::absolute(&root);
        let is_open = |w: &Workspace| w.collections().iter().any(|c| c.root() == absolute);
        let was_open = is_open(self.workspace.read(cx));
        self.workspace.update(cx, |w, cx| w.open_revealed(root, cx));
        if was_open {
            self.reveal_collection(&absolute, window, cx);
        }
        cx.notify();
    }

    fn workspace_event(
        &mut self,
        _: &Entity<Workspace>,
        event: &WorkspaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            WorkspaceEvent::Opened(root) => {
                if self.workspace.update(cx, |w, _| w.take_reveal(root)) {
                    self.reveal_collection(root, window, cx);
                }
                self.refresh_recent_names(cx);
            }
            WorkspaceEvent::OpenFailed { root, message } => {
                let text: SharedString = crate::import_report::open_failure(root, message).into();
                window.open_alert_dialog(cx, move |dialog, _, _| {
                    dialog
                        .title("Could not open collection")
                        .description(text.clone())
                });
                cx.notify();
            }
            WorkspaceEvent::LegacyFormat { root } => {
                crate::legacy_convert::offer(self, root.clone(), window, cx);
            }
            WorkspaceEvent::Notices { root, notices, .. } => {
                for notice in notices {
                    // A batched command is reported by the sidebar's one summary.
                    if self
                        .workspace
                        .update(cx, |w, _| w.take_batched(root, notice.command))
                    {
                        continue;
                    }
                    let Some(text) = notice_message(&notice.kind) else {
                        continue;
                    };
                    let note = match notice.kind {
                        NoticeKind::Warnings(_) => Notification::warning(text),
                        NoticeKind::ExampleAdded { .. } | NoticeKind::Restored(_) => {
                            ui::info_toast(text)
                        }
                        _ => Notification::error(text),
                    };
                    window.push_notification(note, cx);
                }
            }
            WorkspaceEvent::TabsClosed { items, tabs, .. } => {
                if let Some(text) = crate::workspace::tabs_closed_message(items, *tabs) {
                    window.push_notification(ui::info_toast(text), cx);
                }
            }
            WorkspaceEvent::DraftOpened(_)
            | WorkspaceEvent::SelectionChanged(_)
            | WorkspaceEvent::SplitRatio
            | WorkspaceEvent::EnvReverted { .. }
            | WorkspaceEvent::Delivered { .. } => {}
            WorkspaceEvent::RevealFolder { root, folder } => {
                self.sidebar_auto_hidden = false;
                if !self.sidebar_visible {
                    self.set_sidebar(true, window, cx);
                }
                let (root, folder) = (root.clone(), folder.clone());
                self.sidebar
                    .update(cx, |s, cx| s.reveal_folder(root, folder, window, cx));
            }
            WorkspaceEvent::FolderNotice(text) => {
                window.push_notification(Notification::warning(text.clone()), cx);
            }
            WorkspaceEvent::MockFailed { root, message } => {
                let name = display_name(root);
                let text = format!("The mock server of {name} did not start: {message}");
                window.push_notification(ui::kind_toast(ui::ToastKind::Error, text), cx);
                cx.notify();
            }
            WorkspaceEvent::WatchFailed { root, message }
                if self.workspace.read(cx).global_dir() == Some(root.as_path()) =>
            {
                let text = watch_failed_text(None, message);
                window.push_notification(Notification::warning(text), cx);
                cx.notify();
            }
            WorkspaceEvent::WatchFailed { root, message } => {
                let text = watch_failed_text(Some(&display_name(root)), message);
                window.push_notification(Notification::warning(text), cx);
                cx.notify();
            }
        }
    }

    fn open_roots(&self, cx: &App) -> Vec<PathBuf> {
        // The scratch is not a collection the user can switch to or close.
        (self.workspace.read(cx).user_collections().into_iter())
            .map(|c| c.root().to_path_buf())
            .collect()
    }

    // Recent collections not open right now, in the order the session keeps them.
    fn closed_recents(&self, cx: &App) -> Vec<PathBuf> {
        let open = self.open_roots(cx);
        let recent = self.session(cx).read(cx).get().recent.clone();
        recent
            .into_iter()
            .filter(|root| !open.contains(root))
            .collect()
    }

    // The recents list of the empty state: a muted heading, then one row per closed recent with
    // the collection name over its muted, truncated path. Nothing renders without recents.
    fn recent_list(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let recents = self.closed_recents(cx);
        if recents.is_empty() {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let hover = cx.theme().list_hover;
        let room = path_room(window);
        let rows = recents.into_iter().enumerate().map(|(ix, root)| {
            let name = self.recent_name(&root);
            let full = root.display().to_string();
            let path = path_display::fitted(&root, self.home.as_deref(), room, window);
            let cut = path_display::is_shortened(&root, self.home.as_deref(), &path);
            v_flex()
                .id(("recent", ix))
                .test_support()
                .role(Role::Button)
                .aria_label(name.clone())
                .w_full()
                .px(style::SPACE_2)
                .py(style::SPACE_1)
                .rounded(cx.theme().radius)
                .cursor_pointer()
                .hover(move |d| d.bg(hover))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.open(root.clone(), window, cx)),
                )
                .child(div().truncate().child(name))
                .child(
                    div()
                        .id(("recent-path", ix))
                        .test_support()
                        .aria_label(full.clone())
                        .aria_value(path.clone())
                        .truncate()
                        .text_size(style::TEXT_SMALL)
                        .text_color(muted)
                        .when(cut, |d| {
                            d.tooltip(ui::text_tooltip("recent-path-tooltip", full))
                        })
                        .child(path),
                )
        });
        Some(
            v_flex()
                .w(style::RECENT_LIST_WIDTH)
                .max_w_full()
                .gap(style::SPACE_1)
                .child(
                    div()
                        .id("recent-heading")
                        .test_support()
                        .aria_label("Recent")
                        .px(style::SPACE_2)
                        .text_size(style::TEXT_SMALL)
                        .text_color(muted)
                        .child("Recent"),
                )
                .children(rows),
        )
    }

    // What the tab-area pane shows while no collection is open: the icon, the open and import
    // actions and the recent collections.
    fn no_collection(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The demo is offered until a recent collection still exists on disk.
        let offer_demo = !self.closed_recents(cx).iter().any(|p| p.is_dir());
        let mut actions: Vec<(&str, &dyn Action)> = vec![
            ("New request", &NewRequest),
            ("Open collection…", &OpenCollection),
            ("New collection…", &NewCollection),
            ("Import collection…", &ImportCollection),
            ("Clone repository…", &CloneRepository),
        ];
        if offer_demo {
            actions.push(("Open the demo collection…", &OpenDemoCollection));
        }
        let demo = (actions.iter()).position(|(_, a)| a.partial_eq(&OpenDemoCollection));
        div()
            .id("no-collection")
            .test_support()
            .role(Role::Group)
            .accessibility_id("no-collection")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("app-icon")
                    .test_support()
                    .size(style::APP_ICON_SIZE)
                    .child(img(APP_ICON.clone()).size_full()),
            )
            .child(
                div().flex_none().child(
                    ui::empty_state("no-collection-actions", "No collection open", &actions)
                        .when_some(demo, |e, ix| {
                            e.note(ix, "demo-hint", "Sample requests; sending needs internet")
                        }),
                ),
            )
            .children(self.recent_list(window, cx))
    }

    // Whether Check for updates has a source to ask; the menus grey it out or hide it otherwise.
    fn updates_configured(&self, cx: &App) -> bool {
        (self.updates.as_ref()).is_some_and(|u| u.read(cx).enabled())
    }

    // A non-modal row offering the newer release; it renders nothing while there is no offer.
    fn update_notice(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let updates = self.updates.clone()?;
        let (offer, notes_shown) = {
            let u = updates.read(cx);
            (u.offer()?.clone(), u.notes_shown())
        };
        let on = |f: fn(&mut Updates, &mut Context<Updates>)| {
            let updates = updates.clone();
            move |_: &_, _: &mut Window, cx: &mut App| updates.update(cx, f)
        };
        let page = offer.download_page.clone();
        let buttons = div()
            .flex()
            .flex_row()
            .gap_2()
            .child(format!("Apinata {} is available", offer.version))
            .when(offer.notes.is_some(), |el| {
                el.child(
                    Button::new("update-notes")
                        .label("View notes")
                        .on_click(on(Updates::toggle_notes)),
                )
            })
            .child(
                Button::new("update-download")
                    .label("Open download page")
                    .on_click(move |_, _, cx| cx.open_url(&page)),
            )
            .child(
                Button::new("update-later")
                    .label("Later")
                    .on_click(on(Updates::later)),
            )
            .child(
                Button::new("update-skip")
                    .label("Skip this version")
                    .on_click(on(Updates::skip)),
            )
            .children(update_install::install_controls(
                &updates,
                &self.workspace,
                cx,
            ));
        let notes = offer.notes.filter(|_| notes_shown);
        Some(
            div()
                .id("update-notice")
                .test_support()
                .flex()
                .flex_col()
                .child(buttons)
                .when_some(notes, |el, notes| {
                    el.child(div().id("update-notes-text").test_support().child(notes))
                }),
        )
    }

    // The secret names of `root`'s selected environment, so script rows under them mask, and
    // the active global environment's, so global script rows under them mask.
    fn secret_names(&self, root: Option<&Path>, cx: &App) -> (Vec<String>, Vec<String>) {
        let ws = self.workspace.read(cx);
        let secrets = root
            .and_then(|r| ws.snapshot(r, cx))
            .map(|s| {
                let selected = s.selected_environment.as_deref();
                (s.content.environments.iter())
                    .filter(|e| Some(e.name.as_str()) == selected)
                    .flat_map(|e| e.variables.iter().filter(|v| v.secret))
                    .map(|v| v.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let global_secrets = (ws.effective_global())
            .and_then(|(g, name)| g.environment(&name).map(|e| e.variables()))
            .map(|vars| {
                (vars.into_iter())
                    .filter(|v| v.secret)
                    .map(|v| v.name)
                    .collect()
            })
            .unwrap_or_default();
        (secrets, global_secrets)
    }

    // Asks the actor for the active request's variable set, then lists it off the render
    // path: the set is built and flattened on a worker, so the dialog opens with finished rows.
    fn open_variables_overview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = {
            let ws = self.workspace.read(cx);
            ws.tabs().active().and_then(|e| match &e.kind {
                crate::tabs::TabKind::Request(item) => Some((e.root.clone(), item.clone())),
                _ => None,
            })
        };
        let Some((root, item)) = target else {
            crate::variables_overview::open(None, None, window, cx);
            return;
        };
        // The dialog's masking rule, so the overview hides what the dialog hides.
        let (secrets, global_secrets) = self.secret_names(Some(&root), cx);
        let masking = (self.workspace.read(cx).runtime_vars(&root, cx))
            .map(|vars| crate::runtime_vars::Masking::new(vars, secrets, global_secrets));
        let (reply, answer) = async_channel::bounded(1);
        let command = Command::Variables { item, reply };
        self.workspace
            .update(cx, |w, cx| w.send(&root, command, cx));
        cx.spawn_in(window, async move |this, cx| {
            // A context that failed to build, or a stopped actor, still gets an answer.
            let rows = match answer.recv().await {
                Ok(Ok(set)) => Some(cx.background_spawn(async move { set.overview() }).await),
                _ => None,
            };
            let _ = this.update_in(cx, |_, window, cx| match rows {
                Some(rows) => drop(crate::variables_overview::open(
                    Some(rows),
                    masking,
                    window,
                    cx,
                )),
                None => drop(crate::variables_overview::open_failed(window, cx)),
            });
        })
        .detach();
    }

    // The title bar's collection switcher: the open collections, the current one checked, each
    // making itself current with a gear for its settings, then the recents not open. The popover
    // rebuilds its rows on every render, so the rows capture owned root lists and dismiss the
    // popover after acting.
    fn collection_switcher(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current_name(cx);
        let open: Vec<_> = (self.open_roots(cx).into_iter())
            .map(|root| (self.workspace.read(cx).collection_name(&root, cx), root))
            .collect();
        let recent = self.closed_recents(cx);
        let names: std::collections::HashMap<_, _> = (recent.iter())
            .map(|r| (r.clone(), self.recent_name(r)))
            .collect();
        // Every row the popover lists shares one hint set.
        let listed: Vec<(&Path, &str)> = (open.iter())
            .map(|(name, root)| (root.as_path(), name.as_ref()))
            .chain(recent.iter().map(|r| (r.as_path(), names[r].as_ref())))
            .collect();
        let mut hints = crate::name_hints::collection_hints(&listed);
        let recent_hints = hints.split_off(open.len());
        let open_hints = hints;
        let current_root = self.workspace.read(cx).current_root();
        // The trigger follows the open collections alone, as tabs and the sidebar do, so a
        // closed twin in the recents no longer marks the current name.
        let current_hint = (current_root.as_ref())
            .and_then(|root| self.workspace.read(cx).collection_hints(cx).remove(root));
        let muted = cx.theme().muted_foreground;
        let trigger = ui::solid_ring(Button::new("collection-switcher").ghost().small(), cx)
            .min_w_0()
            .dropdown_caret(true)
            .map(|b| match current {
                Some(name) => b.label(name).children(current_hint.map(|hint| {
                    div()
                        .id("collection-switcher-hint")
                        .test_support()
                        .min_w_0()
                        .truncate()
                        .text_color(muted)
                        .child(hint)
                })),
                None => b.label("No collection").text_color(muted),
            });
        let (workspace, shell) = (self.workspace.clone(), cx.entity());
        let (home, room) = (self.home.clone(), path_room(window));
        // The name may truncate too; the branch label's heavier shrink weight empties it first.
        let mut wrapper = gpui_kit::StyleRefinement::default();
        wrapper.min_size.width = Some(px(0.).into());
        Popover::new("switcher")
            .trigger(trigger)
            .trigger_style(wrapper)
            .content(move |_, window, cx| {
                let muted = cx.theme().muted_foreground;
                // The gear is the label's sibling, so its click opens settings without revealing.
                let open_rows = open.iter().enumerate().map(|(ix, (name, root))| {
                    let (workspace, shell, root, gear_root) =
                        (workspace.clone(), shell.clone(), root.clone(), root.clone());
                    let path_root = root.clone();
                    let current = current_root.as_ref() == Some(&root);
                    let hint = open_hints[ix].clone();
                    let reveal =
                        switcher_row(("switch-open", ix), name.clone(), hint, muted, current)
                            .on_click(cx.listener(move |state, _, window, cx| {
                                shell.update(cx, |s, cx| s.reveal_collection(&root, window, cx));
                                state.dismiss(window, cx);
                            }));
                    let gear = Button::new(("switcher-settings", ix))
                        .ghost()
                        .compact()
                        .icon(IconName::Settings)
                        .tooltip(commands::label("collection.settings"))
                        .accessibility_label(commands::label("collection.settings"))
                        .on_click(cx.listener(move |state, _, window, cx| {
                            workspace.update(cx, |w, cx| {
                                w.open_settings(&gear_root, TabKind::Collection, cx);
                            });
                            state.dismiss(window, cx);
                        }));
                    let path = root_path_text(
                        ("switcher-open-path", ix),
                        &path_root,
                        (home.as_deref(), room, window),
                        muted,
                    );
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(reveal)
                                .child(under_name(path)),
                        )
                        .child(gear)
                });
                let recent_rows = recent.iter().enumerate().map(|(ix, root)| {
                    let (shell, root) = (shell.clone(), root.clone());
                    let path = root_path_text(
                        ("recent-path", ix),
                        &root,
                        (home.as_deref(), room, window),
                        muted,
                    );
                    let hint = recent_hints[ix].clone();
                    let button =
                        switcher_row(("recent", ix), names[&root].clone(), hint, muted, false)
                            .on_click(cx.listener(move |state, _, window, cx| {
                                shell.update(cx, |s, cx| s.open(root.clone(), window, cx));
                                state.dismiss(window, cx);
                            }));
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .child(button)
                        .child(under_name(path))
                });
                // The collection actions head the list, so the title bar can drop their
                // buttons; each one dismisses the popover before its native prompt opens, and
                // carries its command so the row can show that command's key.
                let actions = [
                    (
                        "switcher-open-collection",
                        IconName::FolderOpen,
                        "Open collection…",
                        Box::new(OpenCollection) as Box<dyn Action>,
                    ),
                    (
                        "switcher-import-collection",
                        IconName::Import,
                        "Import collection…",
                        Box::new(ImportCollection) as Box<dyn Action>,
                    ),
                    (
                        "switcher-new-collection",
                        IconName::FolderPlus,
                        "New collection…",
                        Box::new(NewCollection) as Box<dyn Action>,
                    ),
                    (
                        "switcher-clone-repository",
                        IconName::GitBranch,
                        "Clone repository…",
                        Box::new(CloneRepository) as Box<dyn Action>,
                    ),
                ];
                // The kit centres a button's content, so each row carries one full-width
                // child that puts the icon and label at the row's left edge.
                let action_rows =
                    actions
                        .into_iter()
                        .enumerate()
                        .map(|(ix, (id, icon, label, action))| {
                            let shell = shell.clone();
                            // Each row shows the key its command has in the live keymap, if any.
                            let key = ui::key_in(action.as_ref(), &[SHELL_CONTEXT], cx);
                            Button::new(id)
                                .ghost()
                                .w_full()
                                .accessibility_label(label)
                                .child(
                                    h_flex()
                                        .w_full()
                                        .gap(style::SPACE_2)
                                        .child(
                                            div()
                                                .id(("switcher-icon", ix))
                                                .test_support()
                                                .child(Icon::new(icon).small()),
                                        )
                                        .child(label)
                                        .children(key.map(|k| Kbd::new(k).ml_auto())),
                                )
                                .on_click(cx.listener(move |state, _, window, cx| {
                                    state.dismiss(window, cx);
                                    shell.update(cx, |s, cx| match id {
                                        "switcher-open-collection" => s.prompt_open(window, cx),
                                        "switcher-import-collection" => s.prompt_import(window, cx),
                                        "switcher-clone-repository" => {
                                            s.clone_repository(&CloneRepository, window, cx)
                                        }
                                        _ => s.prompt_new(window, cx),
                                    });
                                }))
                        });
                let has_rows = !open.is_empty() || !recent.is_empty();
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .children(action_rows)
                    .when(has_rows, |d| {
                        d.child(
                            div()
                                .id("switcher-divider")
                                .test_support()
                                .h(px(1.))
                                .w_full()
                                .bg(cx.theme().border),
                        )
                    })
                    .children(open_rows)
                    .children(recent_rows)
            })
    }

    // Manage environments, locked with a tip saying why while no collection is open, as the
    // collection's environment select beside it is.
    fn environments_button(&self, cx: &App) -> Button {
        if self.workspace.read(cx).current_root().is_some() {
            return icon_button(
                "environments",
                ENV_EDITOR_ICON,
                MANAGE_ENVIRONMENTS_TIP,
                Some(&OpenEnvironments),
            );
        }
        icon_button(
            "environments",
            ENV_EDITOR_ICON,
            MANAGE_ENVIRONMENTS_TIP,
            None,
        )
        .tooltip(NO_COLLECTION_ENV_TIP)
        .lock(true, cx)
    }

    // A title-bar slot for an env select: it starts at the minimum and grows to the full width.
    // In a `cramped` bar it may shrink below the minimum with weight `shrink`, its select
    // truncating; the heavier slot empties first.
    fn env_slot(view: impl IntoElement, cramped: bool, shrink: f32) -> impl IntoElement {
        no_window_move(div())
            .w(style::ENV_CONTROL_MIN)
            .flex_grow_1()
            .when(cramped, |d| d.min_w_0().flex_shrink(shrink))
            .when(!cramped, |d| d.flex_shrink_0())
            .max_w(style::ENV_CONTROL_WIDTH)
            .overflow_hidden()
            .child(view)
    }

    // Narrowing order: branch label, collection name to its floor, global select (then its
    // button), project select.
    // The left group outweighs the selects' group, so they yield only once it is at its floor.
    const GLOBAL_ENV_SHRINK: f32 = 1000.;
    const PROJECT_ENV_SHRINK: f32 = 1.;
    const LEFT_GROUP_SHRINK: f32 = 1000.;

    // How much harder the branch label shrinks than the collection name beside it, so a
    // narrow bar empties the label before it truncates the name.
    const BRANCH_SHRINK: f32 = 1000.;
    // The change count shrinks harder still, so a narrowing bar reaches it before the branch
    // name; it hides as soon as its whole text no longer fits.
    const COUNT_SHRINK: f32 = 1_000_000.;
    // Room beyond the count's whole text that its probe must fill before the count comes back:
    // more than the count's padding, border and the comma, so a returned count stays.
    const COUNT_BACK_SLACK: Rems = Rems(style::SPACE_5.0 + style::SPACE_2.0);
    // Layout rounding the count's box may lose against its whole text and still show it whole.
    const COUNT_ROUNDING: Rems = style::ui(0.5);
    // Narrower than this the text box holds less than an ellipsis and a letter: the label is
    // hidden and its button skipped by Tab.
    const BRANCH_MIN_ROOM: Rems = style::SPACE_5;

    // Whether the change count fits `room`, given its whole text's width. A hidden count needs
    // the slack beyond that before it returns; layout snaps its probe's asked-for width to a
    // device pixel, so the return allows one device pixel less. A shown count stays whole.
    fn count_fits(room: Pixels, whole: Pixels, shown: bool, window: &Window) -> bool {
        let rem = window.rem_size();
        let rounding = Self::COUNT_ROUNDING.to_pixels(rem);
        let need = match shown {
            true => whole - rounding,
            false => {
                let snap = px(1.) / window.scale_factor();
                whole + Self::COUNT_BACK_SLACK.to_pixels(rem) - rounding - snap
            }
        };
        room >= need
    }

    // The muted branch label in the title bar's left group; it shrinks and clips first. The
    // branch name opens the branch dialog and the changed-file count, if any, the Changes dialog.
    fn branch_label(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let workspace = self.workspace.read(cx);
        let root = workspace.current_root();
        let changes = root.as_deref().and_then(|r| workspace.git_changes(r));
        let text = SharedString::from(branch_text(self.branch.as_deref(), changes.as_deref())?);
        let count = changes.as_deref().and_then(GitChanges::count_label);
        let muted = cx.theme().muted_foreground;
        let icon = || Icon::new(IconName::GitBranch).small().flex_none();
        let rem = |r: Rems, window: &Window| r.to_pixels(window.rem_size());
        // The first part's text box shows how much room the bar left the label, and the count's
        // own box how much it left the count. A sliver only shows a clipped ellipsis dot, so the
        // whole label hides when the first part has no room, and the count alone when it has none.
        let hidden = self.branch_clipped.get();
        let has_name = self.branch.is_some();
        let count_hidden = hidden || (has_name && self.changes_clipped.get());
        let measure_at = |cell: &Rc<Cell<bool>>, least: Rems| {
            let cell = Rc::clone(cell);
            canvas(
                move |bounds, window, _| {
                    let now = bounds.size.width < rem(least, window);
                    if cell.replace(now) != now {
                        window.request_animation_frame();
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full()
        };
        let measure = |cell: &Rc<Cell<bool>>| measure_at(cell, Self::BRANCH_MIN_ROOM);
        // The count hides rather than show a cut text. Its box records the room it got, and a
        // hidden unwrapped copy drawn after it measures the whole text and decides in the same
        // frame, so a changed text is never judged by the old text's width.
        let whole = self.count_natural.get();
        let box_room: Rc<Cell<Option<Pixels>>> = Rc::default();
        let measure_whole = || {
            let room = Rc::clone(&box_room);
            canvas(
                move |bounds, _, _| room.set(Some(bounds.size.width)),
                |_, _, _, _| {},
            )
            .absolute()
            .size_full()
        };
        let whole_copy = |text: String| {
            let (natural, clipped) = (
                Rc::clone(&self.count_natural),
                Rc::clone(&self.changes_clipped),
            );
            let room = Rc::clone(&box_room);
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_0()
                .whitespace_nowrap()
                .invisible()
                .child(text)
                .child(
                    canvas(
                        move |bounds, window, _| {
                            let now = Rems(bounds.size.width / window.rem_size());
                            let mut redraw = natural.replace(now) != now;
                            if let Some(room) = room.take() {
                                let hide = !Self::count_fits(room, bounds.size.width, true, window);
                                redraw |= clipped.replace(hide) != hide;
                            }
                            if redraw {
                                window.request_animation_frame();
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
        };
        let text_box = |shown: String, cell: Option<&Rc<Cell<bool>>>| {
            div()
                .relative()
                .min_w_0()
                .truncate()
                .child(shown)
                .children(cell.map(measure))
        };
        let shrink = |factor: f32| {
            move |mut b: gpui_kit::Stateful<gpui_kit::Div>| {
                b.style().flex_shrink = Some(factor);
                b
            }
        };
        // A hidden part leaves the layout and a blank, zero-height probe stays behind, so its
        // room can be measured and the part come back. The label's probe has the label's shape;
        // the count's asks for more than the count and its comma need, so it cannot flicker.
        let probe = |shown: String| {
            h_flex()
                .id("git-label-probe")
                .h_0()
                .min_w_0()
                .overflow_hidden()
                .gap(style::SPACE_1)
                .map(shrink(Self::BRANCH_SHRINK))
                .child(div().w(style::SPACE_4).flex_none())
                .child(text_box(shown, Some(&self.branch_clipped)))
        };
        let back_room = Rems(whole.0 + Self::COUNT_BACK_SLACK.0);
        // The probe asks for room by the width it was built from. It decides by the larger of that
        // and this frame's width, which the whole copy measured before it, so a grown text that
        // no longer fits is not shown for a frame.
        let count_probe = || {
            let (clipped, natural) = (
                Rc::clone(&self.changes_clipped),
                Rc::clone(&self.count_natural),
            );
            div()
                .id("git-count-probe")
                .relative()
                .h_0()
                .w(back_room)
                .min_w_0()
                .map(shrink(Self::COUNT_SHRINK))
                .child(
                    canvas(
                        move |bounds, window, _| {
                            let hide = !Self::count_fits(
                                bounds.size.width,
                                rem(whole, window).max(rem(natural.get(), window)),
                                false,
                                window,
                            );
                            if clipped.replace(hide) != hide {
                                window.request_animation_frame();
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
        };
        let name = self.branch.clone();
        let first = match (&name, &count) {
            (Some(name), _) => name.to_string(),
            (None, count) => count.clone().unwrap_or_default(),
        };
        let probes = [
            hidden.then(|| probe(first)),
            (!hidden && count_hidden)
                .then(|| count.clone().zip(name.as_ref()))
                .flatten()
                .map(|_| count_probe()),
        ];
        // Each part is a ghost Button, so Tab, Enter and Space reach it.
        let part = |id: &'static str, name: SharedString, factor: f32, gone: bool| {
            ui::solid_ring(
                Button::new(id)
                    .ghost()
                    .small()
                    .accessibility_label(name)
                    .tab_stop(!gone)
                    .map(|mut b| {
                        b.style().flex_shrink = Some(factor);
                        b
                    }),
                cx,
            )
            .min_w_0()
            .when(gone, |d| {
                d.absolute()
                    .w_0()
                    .h_0()
                    .p_0()
                    .border_0()
                    .overflow_hidden()
                    .invisible()
            })
            .text_color(muted)
        };
        let comma_shows = has_name && count.is_some() && !count_hidden;
        let branch = name.map(|name| {
            let cell = (!hidden).then_some(&self.branch_clipped);
            part("git-branch", name.clone(), Self::BRANCH_SHRINK, hidden)
                .tooltip(format!("Switch branch ({name})"))
                // No right padding, so only the ring's border parts the comma from the name:
                // any more reads as a space before the comma.
                .when(comma_shows, |b| b.pr_0())
                .on_click(cx.listener(|this, _, window, cx| this.open_branch_picker(window, cx)))
                .child(icon())
                .child(
                    text_box(name.to_string(), cell)
                        .id("git-branch-text")
                        .test_support(),
                )
        });
        // The separator sits between the two buttons, outside both focus rings, and only
        // while the count shows beside the name.
        let comma = comma_shows.then(|| {
            div()
                .id("git-label-comma")
                .test_support()
                .flex_none()
                .text_sm()
                .text_color(muted)
                .child(",")
        });
        let ws = self.workspace.clone();
        let label_focus = self.label_focus.clone();
        let changed = count.zip(root).map(|(count, root)| {
            // Without a name the count leads the label and is measured as its first part.
            let factor = match has_name {
                true => Self::COUNT_SHRINK,
                false => Self::BRANCH_SHRINK,
            };
            let lead = (!has_name && !hidden).then_some(&self.branch_clipped);
            // Behind a name the count never ellipsizes: it is whole or hidden.
            let text = match has_name {
                true => div()
                    .relative()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(count.clone())
                    .children((!count_hidden).then(measure_whole)),
                false => text_box(count.clone(), lead),
            };
            part("git-changes", count.clone().into(), factor, count_hidden)
                .tooltip("Show changes")
                .on_click(move |event, window, cx| {
                    focus_pressed_part(event, &label_focus, has_name as usize, window, cx);
                    open_changes(&ws, &root, None, window, cx)
                })
                .when(!has_name, |b| b.child(icon()))
                .child(text.child(whole_copy(count)))
        });
        let label = h_flex()
            .id("git-label")
            .test_support()
            .track_focus(&self.label_focus)
            // A press on the bare comma leaves focus where it was, as one on a part does.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .aria_label(text)
            .map(|mut b| {
                b.style().flex_shrink = Some(Self::BRANCH_SHRINK);
                b
            })
            .min_w_0()
            .overflow_hidden()
            .when(hidden, |d| d.absolute().w_0().h_0())
            .children(branch)
            .children(comma)
            .children(changed);
        // The chevron opens the branch picker; the wrapper carries the shrink so narrowing is
        // unchanged.
        let chevron = Button::new("git-branch-chevron")
            .ghost()
            .small()
            .icon(IconName::ChevronDown)
            .accessibility_label("Switch branch")
            .tooltip("Switch branch")
            .on_click(cx.listener(|this, _, window, cx| this.open_branch_picker(window, cx)));
        Some(
            h_flex()
                .id("git-group")
                .test_support()
                .map(|mut b| {
                    b.style().flex_shrink = Some(Self::BRANCH_SHRINK);
                    b
                })
                .min_w_0()
                .child(label)
                // The probes follow the chevron, so a hidden part leaves no gap before it.
                .child(div().flex_none().child(chevron))
                .children(probes.into_iter().flatten()),
        )
    }

    // The window's foot: a marker and a manual reload for each collection no longer watched,
    // and the current collection's mock toggle.
    fn status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mock = self.mock_controls(cx);
        let workspace = self.workspace.read(cx);
        let unwatched: Vec<_> = workspace
            .listed_collections(cx)
            .into_iter()
            .enumerate()
            .filter(|(_, c)| workspace.watch_failed(c.root()).is_some())
            .map(|(ix, c)| (ix, c.root().to_path_buf()))
            .collect();
        let markers = unwatched.into_iter().map(|(ix, root)| {
            let name = display_name(&root);
            div()
                .flex()
                .flex_row()
                .child(
                    div()
                        .id(("unwatched", ix))
                        .test_support()
                        .child(format!("Not watching {name}")),
                )
                .child(
                    Button::new(("reload-collection", ix))
                        .label("Reload collection")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.workspace
                                .update(cx, |w, cx| w.send(&root, Command::ReloadAll, cx));
                        })),
                )
        });
        // The button names the layout a click switches to.
        let (layout_icon, layout_tip) = match cx
            .try_global::<AppState>()
            .map(|s| s.settings.read(cx).get().response_orientation)
        {
            Some(Orientation::Horizontal) => (IconName::Columns2, "Switch to stacked layout"),
            _ => (IconName::Rows2, "Switch to side by side layout"),
        };
        // The observed wrapper carries the id and the height; the kit bar fills it without
        // vertical padding so the extra-small buttons fit inside the border.
        div()
            .id("status-bar")
            .test_support()
            .h(style::STATUS_BAR_HEIGHT)
            .child(
                StatusBar::new()
                    .size_full()
                    .py_0()
                    .items_center()
                    .left(ui::solid_ring(
                        icon_button(
                            "toggle-sidebar",
                            IconName::PanelLeft,
                            "Toggle sidebar",
                            Some(&ToggleSidebar),
                        )
                        .xsmall(),
                        cx,
                    ))
                    .children(markers)
                    .when_some(mock, |bar, mock| bar.right(mock))
                    .right(ui::solid_ring(
                        icon_button(
                            "toggle-console",
                            IconName::PanelBottom,
                            "Toggle console",
                            Some(&ToggleConsole),
                        )
                        .xsmall(),
                        cx,
                    ))
                    .right(ui::solid_ring(
                        icon_button(
                            "toggle-layout",
                            layout_icon,
                            layout_tip,
                            Some(&ToggleOrientation),
                        )
                        .xsmall(),
                        cx,
                    )),
            )
    }
}

// The title bar and the status bar: each is drawn by its own `Part` view, which calls back in
// here, and reused while its inputs hold.
impl Shell {
    /// The view that draws the title bar.
    pub fn title_part(&self) -> &Entity<Part<Shell>> {
        self.title.view()
    }

    /// The view that draws the status bar.
    pub fn status_part(&self) -> &Entity<Part<Shell>> {
        self.status.view()
    }

    // Everything the title bar reads that does not notify it. The env selects are views of
    // their own that notify; theme, focus, key bindings and resizes refresh the window.
    fn title_inputs(&self, window: &Window, cx: &App) -> TitleInputs {
        let viewport = window.viewport_size().width;
        let workspace = self.workspace.read(cx);
        let root = workspace.current_root();
        let changes = root.as_deref().and_then(|r| workspace.git_changes(r));
        let counted = (changes.as_deref())
            .and_then(GitChanges::count_label)
            .is_some();
        TitleInputs {
            widths: (
                viewport < style::resolve(style::TITLE_BAR_CRAMPED_BELOW, window),
                viewport < style::resolve(style::VARIABLES_BUTTON_FROM, window),
                viewport < style::resolve(style::TITLE_GLOBAL_BUTTON_BELOW, window),
            ),
            scratch_only: workspace.user_collections().is_empty(),
            current: self.current_name(cx),
            open: (self.open_roots(cx).into_iter())
                .map(|root| (workspace.collection_name(&root, cx), root))
                .collect(),
            recent: (self.closed_recents(cx).into_iter())
                .map(|r| {
                    let name = self.recent_name(&r);
                    (r, name)
                })
                .collect(),
            branch: branch_text(self.branch.as_deref(), changes.as_deref()),
            changes_root: root.filter(|_| counted),
            clipped: (self.branch_clipped.get(), self.changes_clipped.get()),
            window: (
                window.window_decorations(),
                window.window_controls(),
                window.is_maximized(),
                window.is_fullscreen(),
            ),
        }
    }

    // What the status bar reads: the unwatched collections, the response layout and the
    // current collection's mock server.
    fn status_inputs(&self, cx: &App) -> StatusInputs {
        let workspace = self.workspace.read(cx);
        let unwatched = (workspace.listed_collections(cx).into_iter())
            .enumerate()
            .filter(|(_, c)| workspace.watch_failed(c.root()).is_some())
            .map(|(ix, c)| (ix, c.root().to_path_buf()))
            .collect();
        let orientation = cx
            .try_global::<AppState>()
            .map(|s| s.settings.read(cx).get().response_orientation);
        let current = self.current_mock(cx);
        let examples = (current.as_ref()).and_then(|(root, _)| workspace.mock_examples(root));
        StatusInputs(unwatched, orientation, current, examples)
    }

    // The current user collection and where its mock server stands; the scratch has none.
    fn current_mock(&self, cx: &App) -> Option<(PathBuf, MockState)> {
        let workspace = self.workspace.read(cx);
        let root = workspace.current_user_root()?;
        let state = workspace.mock_state(&root);
        Some((root, state))
    }

    // The mock toggle, then while it runs the address it answers on with a copy button and a
    // line saying how many examples it serves and how to stop it.
    fn mock_controls(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (root, state) = self.current_mock(cx)?;
        let examples = self.workspace.read(cx).mock_examples(&root);
        use crate::theme::SelectedStyle;
        use crate::ui::ChooseButton as _;
        let toggle = Button::new("mock-toggle")
            .ghost()
            .xsmall()
            .label("Mock")
            .choose(state != MockState::Off, SelectedStyle::Ghost, cx)
            .toggled(state != MockState::Off)
            .accessibility_label("Mock server")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.workspace.update(cx, |w, cx| w.toggle_mock(&root, cx));
            }));
        let address = match state {
            MockState::On(addr) => Some(addr),
            _ => None,
        };
        let tip = match address {
            Some(addr) => crate::mock_server::running_tip(addr.port()),
            None => MOCK_TIP.to_owned(),
        };
        let shown = address.map(|addr| {
            let url = format!("http://{addr}");
            let copied = url.clone();
            let status = crate::mock_server::status(examples.unwrap_or(0), addr.port());
            h_flex()
                .gap(style::SPACE_1)
                .child(
                    div()
                        .id("mock-address")
                        .test_support()
                        .aria_label(url.clone())
                        .text_color(cx.theme().muted_foreground)
                        .child(url),
                )
                .child(ui::solid_ring(
                    icon_button("mock-copy", IconName::Copy, "Copy mock address", None)
                        .xsmall()
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                copied.clone(),
                            ));
                        }),
                    cx,
                ))
                .child(
                    div()
                        .id("mock-status")
                        .test_support()
                        .aria_label(status.clone())
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
        });
        Some(
            h_flex()
                .gap(style::SPACE_1)
                .child(
                    div()
                        .id("mock-toggle-slot")
                        .tooltip(ui::text_tooltip("mock-tooltip", tip))
                        .child(ui::solid_ring(toggle, cx)),
                )
                .children(shown)
                .into_any_element(),
        )
    }

    // The kit's bar owns drag, double click and the window controls; its close goes through
    // the save prompt instead of removing the window outright.
    fn title_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let viewport = window.viewport_size().width;
        // With no user collection open the collection-only menu entries have nothing to act on.
        let scratch_only = self.workspace.read(cx).user_collections().is_empty();
        // Below this width even the selects' minimum overflows the bar, so they give way too.
        let cramped = viewport < style::resolve(style::TITLE_BAR_CRAMPED_BELOW, window);
        let narrow = viewport < style::resolve(style::VARIABLES_BUTTON_FROM, window);
        let global_button = viewport < style::resolve(style::TITLE_GLOBAL_BUTTON_BELOW, window);
        let shell = cx.entity();
        let title_bar = TitleBar::new()
            .h(style::TITLE_BAR_HEIGHT)
            .on_close_window(move |_, window, cx| {
                shell.update(cx, |s, cx| s.request_window_close(window, cx));
            })
            // The kit's bar never shrinks below its children's full text; a row laid over it
            // adds no width of its own, so the groups inside shrink to the window.
            .child(
                h_flex()
                    .id("title-row")
                    .test_support()
                    .absolute()
                    .inset_0()
                    // The margin keeps a shrunk branch label off the environment control's
                    // globe. The clip is padded out and the padding taken back in margins, so
                    // the buttons' focus rings, drawn just outside them, fit inside it.
                    // Every control is wrapped in `no_window_move`, so a press on it never
                    // carries the window; only the empty spots between them drag it.
                    .child(no_window_move(
                        h_flex()
                            .min_w_0()
                            .when(cramped, |d| {
                                d.min_w(style::TITLE_NAME_FLOOR)
                                    .flex_shrink(Self::LEFT_GROUP_SHRINK)
                            })
                            .p(style::SPACE_1)
                            .m(-style::SPACE_1)
                            .mr(style::SPACE_2 - style::SPACE_1)
                            .overflow_hidden()
                            .child(self.collection_switcher(window, cx))
                            .children(self.branch_label(cx)),
                    ))
                    // The env selects claim only their minimum and grow to full width when
                    // there is room; above the cramped width the group keeps that minimum and
                    // the left group shrinks instead. The inset keeps the menu's ring off the edge.
                    .child(
                        h_flex()
                            .flex_grow_1()
                            .when(cramped, |d| d.min_w_0().flex_shrink(1.))
                            .when(!cramped, |d| d.flex_shrink_0())
                            .justify_end()
                            .pr(style::SPACE_1)
                            .gap(style::SPACE_1)
                            .child(Self::env_slot(
                                self.env_selector.clone(),
                                cramped,
                                Self::PROJECT_ENV_SHRINK,
                            ))
                            // Too little room for a select to read, but the global dialog keeps
                            // a way in from the bar, as an icon that never shrinks.
                            .map(|row| {
                                if global_button {
                                    row.child(no_window_move(div().flex_none().child(
                                        ui::solid_ring(
                                            icon_button(
                                                "global-environments",
                                                GLOBAL_ENV_ICON,
                                                GLOBAL_ENVIRONMENTS_TIP,
                                                Some(&OpenGlobalEnvironments),
                                            ),
                                            cx,
                                        ),
                                    )))
                                } else {
                                    row.child(Self::env_slot(
                                        self.global_env_selector.clone(),
                                        cramped,
                                        Self::GLOBAL_ENV_SHRINK,
                                    ))
                                }
                            })
                            // A narrow bar has no room for it; the palette still lists the command.
                            .children((!narrow).then(|| {
                                no_window_move(div().flex_none().child(ui::solid_ring(
                                    icon_button(
                                        "variables-button",
                                        VARIABLES_ICON,
                                        VARIABLES_TIP,
                                        Some(&OpenVariablesOverview),
                                    ),
                                    cx,
                                )))
                            }))
                            .child(no_window_move(
                                div()
                                    .flex_none()
                                    .child(ui::solid_ring(self.environments_button(cx), cx)),
                            ))
                            .child(Self::app_menu_slot(
                                crate::app_menu::app_menu(
                                    self.focus.clone(),
                                    Rc::clone(&self.stand_in),
                                    Rc::clone(&self.app_menu_open),
                                    scratch_only,
                                    self.branch.is_some(),
                                    self.updates_configured(cx),
                                    cx,
                                ),
                                self.app_menu_open.get(),
                            )),
                    ),
            );
        title_bar.into_any_element()
    }
}

impl PartOwner for Shell {
    type Part = ShellPart;

    fn render_part(
        &mut self,
        part: ShellPart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match part {
            ShellPart::Title => self.title_bar(window, cx),
            ShellPart::Status => self.status_bar(cx).into_any_element(),
        }
    }
}

/// A part of the shell drawn as a view of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellPart {
    /// The title bar.
    Title,
    /// The status bar.
    Status,
}

#[derive(PartialEq)]
struct TitleInputs {
    // Whether the bar is cramped, too narrow for the variables button, and too narrow for the
    // global select.
    widths: (bool, bool, bool),
    scratch_only: bool,
    current: Option<SharedString>,
    open: Vec<(SharedString, PathBuf)>,
    recent: Vec<(PathBuf, SharedString)>,
    branch: Option<String>,
    // The collection whose changes the branch label opens, while it counts any.
    changes_root: Option<PathBuf>,
    clipped: (bool, bool),
    window: (Decorations, WindowControls, bool, bool),
}

#[derive(PartialEq)]
struct StatusInputs(
    Vec<(usize, PathBuf)>,
    Option<Orientation>,
    Option<(PathBuf, MockState)>,
    Option<usize>,
);

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self.session(cx).read(cx).get().sidebar_width;
        // The sidebar shows its preferred width capped at two fifths of the window. The split
        // keeps a panel's first laid-out width, so it is laid out afresh whenever the shown
        // width (zoom, cap, drag), the window width or the sidebar's visibility changes; only
        // a drag saves a width.
        let viewport = window.viewport_size().width;
        let cap = viewport * style::SIDEBAR_MAX_SHARE;
        let preferred = width.map_or_else(|| style::resolve(style::SIDEBAR_WIDTH, window), px);
        // A width saved under an older, smaller minimum shows at today's.
        let sidebar_range = style::sidebar_min(window).min(cap)..cap;
        let shown = preferred.max(sidebar_range.start).min(cap);
        // Resizes and zoom changes both re-render, so a crossing of the auto-hide width is
        // caught here and applied to this very frame; only the focus move out of a hidden
        // sidebar waits until after the draw. Only an auto-hidden sidebar comes back.
        let narrow =
            window.viewport_size().width < style::resolve(style::SIDEBAR_AUTO_HIDE_BELOW, window);
        if narrow != self.narrow {
            self.narrow = narrow;
            if narrow && self.sidebar_visible {
                self.sidebar_auto_hidden = true;
                self.sidebar_visible = false;
                cx.defer_in(window, |this, window, cx| {
                    this.set_sidebar(false, window, cx)
                });
            } else if !narrow && self.sidebar_auto_hidden {
                self.sidebar_auto_hidden = false;
                self.sidebar_visible = true;
            }
        }
        // The bar swaps the global select and its button at this width, and whichever goes
        // takes focus with it; a released button's focus leaves no loss to catch.
        let global_button = viewport < style::resolve(style::TITLE_GLOBAL_BUTTON_BELOW, window);
        if global_button != self.global_button {
            self.global_button = global_button;
            cx.defer_in(window, Self::take_stranded_focus);
        }
        // A tab area measured beside a hidden sidebar would keep that width on reveal, so
        // visibility counts too, read after this frame's auto-hide crossing.
        let beside = if self.sidebar_visible { shown } else { px(0.) };
        let drawn = self.sidebar_visible;
        self.sidebar.update(cx, |s, _| s.set_drawn(drawn));
        cx.set_global(crate::request_editor::TabAreaRoom(viewport - beside));
        let applied = (shown, viewport, self.sidebar_visible);
        if applied != self.sidebar_applied {
            self.sidebar_applied = applied;
            self.split.update(cx, |s, _| s.clear());
        }
        let no_collection = self.workspace.read(cx).shows_empty_state(cx);
        let inputs = self.title_inputs(window, cx);
        let full_width = StyleRefinement::default().w_full().flex_none();
        let title_bar = self.title.element(inputs, full_width.clone(), window, cx);
        let inputs = self.status_inputs(cx);
        let status_bar = self.status.element(inputs, full_width, window, cx);
        // The dock opens at its set height, but never above 40% of the height between the
        // bars, and leaves the tab area its minimum unless the dock minimum needs the room.
        let between_bars = window.viewport_size().height
            - style::resolve(style::TITLE_BAR_HEIGHT, window)
            - style::resolve(style::STATUS_BAR_HEIGHT, window);
        let tab_min = style::resolve(style::TAB_AREA_MIN, window);
        let dock_min = style::resolve(style::DOCK_MIN, window);
        let dock_height = style::dock_height(
            style::resolve(style::DOCK_HEIGHT, window),
            between_bars,
            tab_min,
            dock_min,
        );
        // A drag keeps both minimums too; the dock one wins in a window too short for both.
        let mut tab_range = tab_min.min(between_bars - dock_min)..Pixels::MAX;
        let mut dock_range = dock_min..(between_bars - tab_min).max(dock_min);
        let mut dock_height = dock_height;
        // Collapsing is a view rule, so the open toggle is untouched; a change of layout
        // clears the split, as a toggle does, so the panel takes its new size.
        let header = style::dock_header(crate::zoom::current(cx));
        let short = style::dock_collapses(
            between_bars,
            style::resolve(style::TAB_VIEW_MIN, window),
            header,
            dock_min,
        );
        self.dock_expanded_short &= short;
        let collapsed = short && !self.dock_expanded_short;
        if collapsed != self.dock_collapsed {
            self.dock_collapsed = collapsed;
            self.dock_split.update(cx, |s, _| s.clear());
            // The body stops drawing, so focus in it would strand above the shell's keys.
            if collapsed && self.dock.read(cx).body_focused(window, cx) {
                cx.defer_in(window, Self::hand_back_focus);
            }
        }
        self.dock.update(cx, |d, cx| d.set_collapsed(collapsed, cx));
        if collapsed {
            dock_height = header;
            dock_range = header..header;
            tab_range = (between_bars - header).max(px(0.))..Pixels::MAX;
        }
        div()
            .id("shell")
            .key_context(SHELL_CONTEXT)
            .track_focus(&self.focus)
            .capture_any_mouse_down({
                let stand_in = Rc::clone(&self.stand_in);
                let shell = self.focus.clone();
                move |_, window, cx| stand_in.hold(&shell, window, cx)
            })
            .on_action(cx.listener(Self::on_open_collection))
            .on_action(
                cx.listener(|this, _: &ImportCollection, window, cx| {
                    this.prompt_import(window, cx)
                }),
            )
            .on_action(cx.listener(Self::import_curl))
            .on_action(cx.listener(Self::clone_repository))
            .on_action(cx.listener(Self::open_demo))
            .on_action(
                cx.listener(|this, _: &SwitchBranch, window, cx| {
                    this.open_branch_picker(window, cx)
                }),
            )
            .on_action(cx.listener(Self::show_changes))
            .on_action(cx.listener(Self::toggle_mock_server))
            .on_action(
                cx.listener(|this, _: &NewCollection, window, cx| this.prompt_new(window, cx)),
            )
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_console))
            .on_action(cx.listener(Self::toggle_orientation))
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(cx.listener(|this, _: &ZoomIn, w, cx| this.zoom(ZoomStep::In, w, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, w, cx| this.zoom(ZoomStep::Out, w, cx)))
            .on_action(cx.listener(|this, _: &ResetZoom, w, cx| this.zoom(ZoomStep::Reset, w, cx)))
            .on_action(cx.listener(|this, _: &OpenEnvironments, window, cx| {
                this.open_environments(window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenCollectionSettings, window, cx| {
                this.open_collection_settings(window, cx)
            }))
            .on_action(cx.listener(|this, _: &RunCollection, window, cx| {
                this.run_current(false, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &RunFolder, window, cx| this.run_current(true, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ExportCollection, window, cx| {
                if let Some(root) = this.workspace.read(cx).current_root() {
                    let format = ExportFormat::Postman {
                        keep_scripts: false,
                    };
                    crate::export::export_collection(&this.workspace, &root, format, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &ExportCollectionPostmanKeepScripts, window, cx| {
                    if let Some(root) = this.workspace.read(cx).current_root() {
                        let format = ExportFormat::Postman { keep_scripts: true };
                        crate::export::export_collection(
                            &this.workspace,
                            &root,
                            format,
                            window,
                            cx,
                        );
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &ExportCollectionOpenApi, window, cx| {
                    if let Some(root) = this.workspace.read(cx).current_root() {
                        let format = ExportFormat::OpenApi;
                        crate::export::export_collection(
                            &this.workspace,
                            &root,
                            format,
                            window,
                            cx,
                        );
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &ExportCollectionHtml, window, cx| {
                if let Some(root) = this.workspace.read(cx).current_root() {
                    let format = ExportFormat::Html;
                    crate::export::export_collection(&this.workspace, &root, format, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ImportEnvironment, window, cx| {
                this.env_selector.update(cx, |s, cx| s.import(window, cx))
            }))
            .on_action(cx.listener(|this, _: &ExportEnvironment, window, cx| {
                this.export_environment(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportEnvironments, window, cx| {
                this.export_environments(window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenGlobalEnvironments, window, cx| {
                open_global_env_editor(this.workspace.clone(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseCollection, window, cx| {
                if let Some(root) = this.workspace.read(cx).current_root() {
                    close_collection_at(&this.workspace, root, window, cx);
                }
            }))
            .on_action(|_: &OpenCookies, window, cx| {
                open_cookie_jar(window, cx);
            })
            .on_action(cx.listener(|this, _: &OpenVariablesOverview, window, cx| {
                this.open_variables_overview(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenRuntimeVariables, window, cx| {
                let ws = this.workspace.read(cx);
                let root = ws.current_root();
                let stores = (root.as_ref())
                    .and_then(|r| Some((ws.runtime_vars(r, cx)?, ws.env_overlay(r, cx)?)));
                let (secrets, global_secrets) = this.secret_names(root.as_deref(), cx);
                if let Some((vars, overlay)) = stores {
                    crate::runtime_vars::open_runtime_vars(
                        vars,
                        overlay,
                        secrets,
                        global_secrets,
                        window,
                        cx,
                    );
                }
            }))
            .on_action(|_: &OpenPreferences, window, cx| {
                crate::preferences::open(window, cx);
            })
            .on_action(cx.listener(|this, _: &OpenCommandPalette, window, cx| {
                let requests = crate::palette::request_entries(this.workspace.read(cx), cx);
                let sidebar = Some(this.sidebar.clone());
                crate::palette::open_with(
                    this.focus.clone(),
                    requests,
                    sidebar,
                    Some(this.workspace.clone()),
                    Some(this.tab_area.clone()),
                    this.branch.is_some(),
                    this.updates_configured(cx),
                    window,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &GoToRequest, window, cx| {
                let requests = crate::palette::request_entries(this.workspace.read(cx), cx);
                crate::palette::open_requests(this.focus.clone(), requests, window, cx)
            }))
            .on_action(cx.listener(|this, a: &OpenRequest, _, cx| {
                (this.workspace)
                    .update(cx, |w, cx| w.open_item(&a.root, a.path.clone(), false, cx));
            }))
            .on_action(cx.listener(Self::open_history))
            .on_action(cx.listener(|this, _: &OpenCollectionSearch, window, cx| {
                if let Some(root) = this.workspace.read(cx).current_root() {
                    crate::search_dialog::open(
                        &this.workspace,
                        root,
                        this.focus.clone(),
                        window,
                        cx,
                    );
                }
            }))
            .on_action(cx.listener(|this, a: &OpenSearchHit, window, cx| {
                let tab = (this.workspace)
                    .update(cx, |w, cx| w.open_item(&a.root, a.path.clone(), false, cx));
                if let (Some(tab), Some(section)) = (tab, a.section) {
                    (this.tab_area).update(cx, |t, cx| t.show_section(tab, section, window, cx));
                }
            }))
            // A text field without search lets the kit's ctrl-h Replace bubble; swallowing it
            // here keeps the shell's History binding from firing inside a field.
            .on_action(|_: &Replace, _, _| {})
            .on_action(cx.listener(|this, a: &OpenHistoryEntry, window, cx| {
                if a.path.is_draft() {
                    return this.reopen_draft(a, window, cx);
                }
                let tab = (this.workspace)
                    .update(cx, |w, cx| w.open_item(&a.root, a.path.clone(), false, cx));
                if let Some(tab) = tab {
                    (this.tab_area).update(cx, |t, cx| t.show_kept_send(tab, a.seq, window, cx));
                }
            }))
            .on_action(cx.listener(|this, a: &OpenRunResult, window, cx| {
                let tab = (this.workspace)
                    .update(cx, |w, cx| w.open_item(&a.root, a.path.clone(), false, cx));
                if let Some(tab) = tab {
                    let result = Arc::clone(&a.result);
                    (this.tab_area).update(cx, |t, cx| t.show_result(tab, result, window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &CheckForUpdates, _, cx| {
                if let Some(updates) = &this.updates {
                    updates.update(cx, |u, cx| u.check_now(cx));
                }
            }))
            .on_action(cx.listener(Self::focus_filter))
            .on_action(cx.listener(to_sidebar(Sidebar::rename_selected)))
            .on_action(cx.listener(to_sidebar(Sidebar::clone_selected)))
            .on_action(cx.listener(to_sidebar(Sidebar::show_in_file_manager)))
            .on_action(cx.listener(to_sidebar(Sidebar::open_in_terminal)))
            .on_action(cx.listener(to_sidebar(Sidebar::copy_action)))
            .on_action(cx.listener(to_sidebar(Sidebar::cut_action)))
            .on_action(cx.listener(to_sidebar(Sidebar::paste_action)))
            .on_action(cx.listener(to_sidebar(Sidebar::new_request)))
            .on_action(cx.listener(to_sidebar(Sidebar::new_graphql_request)))
            .on_action(cx.listener(to_sidebar(Sidebar::delete_selected)))
            .on_action(cx.listener(to_sidebar(Sidebar::undo_delete)))
            .on_action(cx.listener(to_sidebar(Sidebar::run_selected)))
            .on_action(cx.listener(to_sidebar(Sidebar::new_folder)))
            .on_action(cx.listener(to_sidebar(Sidebar::sort_by_name)))
            .on_action(cx.listener(to_sidebar(Sidebar::collapse_all)))
            .on_action(cx.listener(to_sidebar(Sidebar::expand_all)))
            .on_action(cx.listener(to_tabs(TabArea::discard_changes)))
            .on_action(cx.listener(to_tabs(TabArea::save_active)))
            .on_action(cx.listener(to_tabs(TabArea::send_active)))
            // With no tab open, focus falls back to the shell and Undo edit has nothing to
            // undo, so the chord goes to the newest delete.
            .on_action(cx.listener(|this, a: &UndoActive, window, cx| {
                if this.workspace.read(cx).tabs().active().is_none() {
                    this.sidebar
                        .update(cx, |s, cx| s.undo_delete(&UndoDelete, window, cx));
                } else {
                    this.tab_area
                        .update(cx, |t, cx| t.undo_active(a, window, cx));
                }
            }))
            .on_action(cx.listener(to_tabs(TabArea::redo_active)))
            .on_action(cx.listener(to_tabs(TabArea::stop_active)))
            .on_action(cx.listener(to_tabs(TabArea::copy_active_as_curl)))
            .on_action(cx.listener(to_tabs(TabArea::generate_active_code)))
            .on_action(cx.listener(to_tabs(TabArea::convert_active_to_graphql)))
            .on_action(cx.listener(to_tabs(TabArea::convert_active_to_http)))
            .on_action(cx.listener(to_tabs(TabArea::beautify_active_body)))
            .on_action(cx.listener(to_tabs(TabArea::minify_active_body)))
            .on_action(cx.listener(to_tabs(TabArea::beautify_active_query)))
            .on_action(cx.listener(to_tabs(TabArea::copy_caret_json_value)))
            .on_action(cx.listener(to_tabs(TabArea::copy_caret_json_path)))
            .on_action(cx.listener(to_tabs(TabArea::set_caret_json_variable)))
            .on_action(cx.listener(to_tabs(TabArea::assert_caret_json_value)))
            .on_action(cx.listener(to_tabs(TabArea::save_as_example)))
            .on_action(cx.listener(to_tabs(TabArea::export_har)))
            .on_action(cx.listener(to_tabs(TabArea::compare_response)))
            .on_action(cx.listener(to_tabs(TabArea::save_all)))
            .on_action(cx.listener(to_tabs(TabArea::close_tab)))
            .on_action(cx.listener(to_tabs(TabArea::close_all_tabs)))
            .on_action(cx.listener(to_tabs(TabArea::close_other_tabs)))
            .on_action(cx.listener(to_tabs(TabArea::close_tabs_to_the_right)))
            .on_action(cx.listener(to_tabs(TabArea::reveal_in_sidebar)))
            .on_action(cx.listener(to_tabs(TabArea::close_saved_tabs)))
            .on_action(cx.listener(to_tabs(TabArea::pin_tab)))
            .on_action(cx.listener(to_tabs(TabArea::unpin_tab)))
            .on_action(cx.listener(to_tabs(TabArea::reopen_closed_tab)))
            .on_action(cx.listener(to_tabs(TabArea::activate_tab)))
            .on_action(cx.listener(to_tabs(TabArea::activate_last_tab)))
            .on_action(cx.listener(to_tabs(TabArea::activate_prev_tab)))
            .on_action(cx.listener(to_tabs(TabArea::activate_next_tab)))
            .on_action(cx.listener(to_tabs(TabArea::focus_url)))
            .on_action(cx.listener(to_tabs(TabArea::next_section)))
            .on_action(cx.listener(to_tabs(TabArea::previous_section)))
            .on_action(cx.listener(to_tabs(TabArea::focus_response)))
            .on_action(
                // A squeezed select is hidden and takes no focus, so the command opens the
                // environments tab, as the bar's environments button beside it does.
                cx.listener(|this, _: &OpenEnvironmentSelector, window, cx| {
                    if this.env_selector.read(cx).squeezed() {
                        return this.open_environments(window, cx);
                    }
                    this.env_selector
                        .update(cx, |s, cx| s.open_menu(window, cx))
                }),
            )
            // Files from the desktop: the overlay shows while they hover. A drag of the
            // sidebar's own rows is another type and never reaches these.
            .on_drag_move(
                cx.listener(|this, _: &DragMoveEvent<ExternalPaths>, _, cx| {
                    if !this.drop_hover {
                        this.drop_hover = true;
                        cx.notify();
                    }
                }),
            )
            .on_file_drop_exit(cx.listener(|this, _, _, cx| {
                this.drop_hover = false;
                cx.notify();
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.files_dropped(paths.paths().to_vec(), window, cx)
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(title_bar)
            // A decorative stripe in the active environment's colour, laid over the bar's top
            // edge so the layout below never moves when it comes or goes.
            .children(self.env_stripe(cx).map(|fill| {
                div()
                    .id("env-stripe")
                    .test_support()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(style::ENV_STRIPE_HEIGHT)
                    .bg(fill)
            }))
            .when(self.drop_hover, |root| {
                root.child(crate::window_drop::overlay(cx))
            })
            .children(self.update_notice(cx))
            // The split takes the height left between the title bar and the status bar; the
            // dock spans its full width under the sidebar and tab area.
            .child(
                div().flex_1().min_h_0().child(
                    v_resizable("shell-dock")
                        .with_state(&self.dock_split)
                        .child(
                            resizable_panel().size_range(tab_range).child(
                                h_resizable("shell-split")
                                    .with_state(&self.split)
                                    .child(
                                        resizable_panel()
                                            .visible(self.sidebar_visible)
                                            .size(shown)
                                            .size_range(sidebar_range)
                                            .child(
                                                div()
                                                    .id("sidebar")
                                                    .test_support()
                                                    .size_full()
                                                    .child(ui::cached(&self.sidebar)),
                                            ),
                                    )
                                    .child(resizable_panel().child(
                                        div().id("tab-area").test_support().size_full().map(
                                            |pane| {
                                                if no_collection {
                                                    pane.child(self.no_collection(window, cx))
                                                } else {
                                                    pane.child(self.tab_area.clone())
                                                }
                                            },
                                        ),
                                    )),
                            ),
                        )
                        .child(
                            resizable_panel()
                                .visible(self.dock_open)
                                .size(dock_height)
                                .size_range(dock_range)
                                .child(ui::cached(&self.dock)),
                        ),
                ),
            )
            .child(status_bar)
            // Painted last, so its capture listener runs after the response body's: a
            // ctrl+scroll the body did not take zooms the app.
            .child({
                let pending = Rc::clone(&self.scroll_pending);
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        crate::zoom::paint_scroll_listener(
                            ZoomTarget::App,
                            pending,
                            |_| true,
                            window,
                        )
                    },
                )
                .absolute()
                .size_full()
            })
            .child(crate::focus_mode::pointer_watch())
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The one path a single-path prompt answered with, or `None` when it was cancelled or failed.
pub(crate) fn first_path<E, F>(
    reply: Result<Result<Option<Vec<PathBuf>>, F>, E>,
) -> Option<PathBuf> {
    reply.ok()?.ok()??.into_iter().next()
}

/// A view's action handler, run from the shell on that view.
type Handler<V, A> = fn(&mut V, &A, &mut Window, &mut Context<V>);

// Window-level keys land on the shell; these forward each to the view that owns it.
fn to_sidebar<A: Action>(
    f: Handler<Sidebar, A>,
) -> impl Fn(&mut Shell, &A, &mut Window, &mut Context<Shell>) {
    move |this, a, window, cx| this.sidebar.update(cx, |v, cx| f(v, a, window, cx))
}

fn to_tabs<A: Action>(
    f: Handler<TabArea, A>,
) -> impl Fn(&mut Shell, &A, &mut Window, &mut Context<Shell>) {
    move |this, a, window, cx| this.tab_area.update(cx, |v, cx| f(v, a, window, cx))
}

/// A full-width switcher row for a collection: a check in the slot the action rows give their
/// icons, drawn only when `checked` so every name starts in the actions' label column.
fn switcher_row(
    id: (&'static str, usize),
    name: SharedString,
    hint: Option<String>,
    muted: Hsla,
    checked: bool,
) -> Button {
    let slot = if checked {
        "switch-current"
    } else {
        "switch-unchecked"
    };
    let check = check_slot(checked).id((slot, id.1)).test_support();
    Button::new(id)
        .ghost()
        .w_full()
        .accessibility_label(name.clone())
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .gap(style::SPACE_2)
                .child(check)
                .child(
                    div()
                        .id((name_id(id.0), id.1))
                        .test_support()
                        .min_w_0()
                        .truncate()
                        .child(name),
                )
                // A collection sharing its name with another listed one shows where it differs.
                .children(hint.map(|hint| {
                    div()
                        .id((hint_id(id.0), id.1))
                        .test_support()
                        // A heavier shrink weight makes the hint give up more room than the name.
                        .min_w_0()
                        .flex_shrink(8.)
                        .truncate()
                        .text_color(muted)
                        .child(hint)
                })),
        )
}

// The leading slot of a switcher row: a check, kept invisible when unchecked so it still holds
// the column every name starts in.
fn check_slot(checked: bool) -> Div {
    div()
        .flex_none()
        .when(!checked, |d| d.invisible())
        .child(Icon::new(IconName::Check).small())
}

// A switcher row's path line, inset by the row button's padding and an empty check slot so the
// path starts at the same left edge as the name above it.
fn under_name(path: impl IntoElement) -> Div {
    h_flex()
        .w_full()
        .min_w_0()
        .px_2p5()
        .gap(style::SPACE_2)
        .child(check_slot(false))
        .child(path)
}

// The test id of a switcher row's name, by the row's own id.
fn name_id(row: &'static str) -> &'static str {
    if row == "recent" {
        "recent-name"
    } else {
        "switch-open-name"
    }
}

// The test id of a switcher row's hint, by the row's own id.
fn hint_id(row: &'static str) -> &'static str {
    if row == "recent" {
        "recent-hint"
    } else {
        "switch-open-hint"
    }
}

/// A sidebar row for the open collection `name` in medium weight, carrying a spinner while
/// its actor has stalled.
pub fn collection_row(ix: usize, name: SharedString, busy: bool) -> impl IntoElement {
    let row = h_flex()
        .id(("collection", ix))
        .test_support()
        .aria_label(name.clone())
        .min_w_0()
        .gap(style::SPACE_2)
        .font_weight(FontWeight::MEDIUM)
        .child(div().truncate().child(name));
    if busy {
        row.child(
            div()
                .id(("busy", ix))
                .test_support()
                .flex_none()
                .child(Spinner::new().small()),
        )
    } else {
        row
    }
}

// The kit's buttons take no focus on a press, so Changes opened by a click would record, and
// on close hand focus back to, whatever held it before. A press first focuses part `nth` of
// the branch label: the label's handle is no Tab stop, so the stops after it are its parts.
fn focus_pressed_part(
    event: &ClickEvent,
    label: &FocusHandle,
    nth: usize,
    window: &mut Window,
    cx: &mut App,
) {
    if matches!(event, ClickEvent::Keyboard(_)) {
        return;
    }
    let before = window.focused(cx);
    label.focus(window, cx);
    for _ in 0..=nth {
        window.focus_next(cx);
    }
    // A part that is not drawn as a stop leaves focus where it was.
    if !label.contains_focused(window, cx) || label.is_focused(window) {
        match before {
            Some(before) => before.focus(window, cx),
            None => window.blur(cx),
        }
    }
}

// The width a row's path has: the list width less the row's side padding.
fn path_room(window: &Window) -> Pixels {
    style::resolve(style::RECENT_LIST_WIDTH, window) - style::resolve(style::SPACE_2, window) * 2.
}

// A collection's path as muted secondary text, fitted by `path_display` so the folder name
// and its parent stay visible; the full path is the label, and the tooltip once shortened.
fn root_path_text(
    id: (&'static str, usize),
    root: &Path,
    fit: (Option<&Path>, Pixels, &Window),
    muted: gpui_kit::Hsla,
) -> impl IntoElement + use<> {
    let (home, room, window) = fit;
    let shown = path_display::fitted(root, home, room, window);
    let full = root.display().to_string();
    let cut = path_display::is_shortened(root, home, &shown);
    div()
        .id(id)
        .test_support()
        .aria_label(full.clone())
        .aria_value(shown.clone())
        .min_w_0()
        .max_w(style::RECENT_LIST_WIDTH)
        .truncate()
        .text_size(style::TEXT_SMALL)
        .text_color(muted)
        .when(cut, |d| {
            d.tooltip(ui::text_tooltip(format!("{}-tooltip", id.0), full))
        })
        .child(shown)
}

/// The warning for a watch that stopped: of the collection named `collection`, or of the
/// global environments when `None`.
pub fn watch_failed_text(collection: Option<&str>, message: &str) -> String {
    let (subject, advice) = match collection {
        None => (
            "global environments",
            "Reopen Global environments to pick up changes.",
        ),
        Some(name) => (name, "Use Reload collection to pick up changes."),
    };
    // The notify message may or may not end its sentence; the advice always starts a new one,
    // and an empty message leaves the stop alone.
    let message = message.trim();
    if message.is_empty() {
        return format!("Stopped watching {subject}. {advice}");
    }
    let stop = if message.ends_with(['.', '!', '?']) {
        ""
    } else {
        "."
    };
    format!("Stopped watching {subject}: {message}{stop} {advice}")
}

/// The notification for a manual update check's report; an available release shows the
/// notice instead, so it has none.
pub fn report_notification(report: &Report) -> Option<Notification> {
    let text = report_text(report);
    match report {
        Report::Available(_) => None,
        Report::Failed(_) => Some(Notification::error(text)),
        Report::UpToDate | Report::Disabled => Some(ui::info_toast(text)),
    }
}

/// The notification text for an actor notice that reports an error, warnings or a
/// confirmation, or `None` for the kinds other views own (stale edits, creations and moves).
pub fn notice_message(kind: &NoticeKind) -> Option<String> {
    let text = match kind {
        NoticeKind::NotFound(target) => Some(format!("{} no longer exists", describe(target))),
        NoticeKind::Unsupported { target, field } => {
            Some(format!("{} has no {field:?} field", describe(target)))
        }
        NoticeKind::Failed { message, .. } => Some(message.clone()),
        NoticeKind::Warnings(warnings) => Some(warnings.join("\n")),
        NoticeKind::ExampleAdded { item, name } => {
            Some(format!("Saved example \"{name}\" to {item}"))
        }
        NoticeKind::Restored(item) => {
            let name = item.file_name();
            Some(format!(
                "Restored \"{}\"",
                name.strip_suffix(".yml").unwrap_or(name)
            ))
        }
        NoticeKind::Stale(_)
        | NoticeKind::Created(_)
        | NoticeKind::EnvironmentCreated(_)
        | NoticeKind::Moved { .. }
        | NoticeKind::Deleted(_)
        | NoticeKind::Trashed { .. }
        | NoticeKind::ExampleDeleted { .. } => None,
    };
    text.map(|t| ui::plain_quotes(&t))
}

fn describe(target: &Target) -> String {
    match target {
        Target::Request(path) | Target::Folder(path) => path.to_string(),
        Target::Collection => "The collection file".into(),
        Target::Environment(name) => format!("Environment {name}"),
    }
}

fn import_failed(message: String, window: &mut Window, cx: &mut App) {
    let text: SharedString = crate::import_report::failure_text(&message).into();
    window.open_alert_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Could not import collection")
            .description(text.clone())
    });
}

// Asks where the imported collections go, writes them there off the main thread, and opens
// what landed with its report. A cancelled prompt does nothing; a failed write keeps and opens
// the collections before it.
async fn place_bundles(
    this: &WeakEntity<Shell>,
    bundles: Vec<ImportBundle>,
    store: Arc<dyn apinata_core::SecretStore>,
    cx: &mut AsyncWindowContext,
) {
    let Ok(reply) = this.update(cx, |_, cx| {
        cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Import into".into()),
        })
    }) else {
        return;
    };
    let Some(parent) = first_path(reply.await) else {
        return;
    };
    let job = cx.background_spawn(async move { write_bundles(&bundles, &parent, &*store) });
    let (roots, report, error) = job.await;
    let _ = this.update_in(cx, |this, window, cx| {
        this.landed(roots, report, error, window, cx)
    });
}

/// Reads and parses every collection `file` holds, each named after the file's stem where the
/// format takes a name. Answers the failure as alert text when the file cannot be read or parsed.
pub fn parse_bundles(file: &Path, formats_shown: bool) -> Result<Vec<ImportBundle>, String> {
    let input =
        std::fs::read(file).map_err(|e| crate::import_report::failure_at(file, &e.to_string()))?;
    parse_bundles_from(file, &input, formats_shown)
}

/// `parse_bundles` over bytes already read from `file`, which only names the collection. With
/// `formats_shown`, the caller already lists the formats, so an unknown input is named alone.
pub fn parse_bundles_from(
    file: &Path,
    input: &[u8],
    formats_shown: bool,
) -> Result<Vec<ImportBundle>, String> {
    let mut options = CollectionOptions::default();
    options.name = file.file_stem().map(|s| s.to_string_lossy().into_owned());
    parse_collections(None, input, &options).map_err(|e| match e {
        // An environment file is a near miss: point at the action that takes it.
        FormatError::Unrecognized { .. } if EnvironmentFormat::detect(input).is_some() => {
            ENVIRONMENT_FILE.to_owned()
        }
        FormatError::Unrecognized { kind, .. } if formats_shown => {
            format!("Not a recognized {kind} format.")
        }
        e => crate::ui::plain_quotes(&e.to_string()),
    })
}

const ENVIRONMENT_FILE: &str = "This is an environment file, not a collection. To add it, open a collection and choose Import environment.";

// Each written root with the first environment its import produced.
pub(crate) type Written = Vec<(PathBuf, Option<String>)>;

// Writes `bundles` in order into `parent`, stopping at the first failure. Answers the roots
// written, their merged report (none if nothing landed), and the failure naming what was not
// written.
pub(crate) fn write_bundles(
    bundles: &[ImportBundle],
    parent: &Path,
    store: &dyn apinata_core::SecretStore,
) -> (Written, Option<ImportReport>, Option<String>) {
    let mut roots = Vec::new();
    let mut merged = ImportReport::default();
    for (i, bundle) in bundles.iter().enumerate() {
        match write_collection(parent, &bundle.collection, &bundle.environments, store) {
            Ok((collection, report)) => {
                let first = report.environments.first().cloned();
                roots.push((collection.root().to_path_buf(), first));
                merged.notes.extend(report.notes);
                merged.items.extend(report.items);
                merged.environments.extend(report.environments);
                merged.secrets_to_enter.extend(report.secrets_to_enter);
            }
            Err(e) => {
                let rest: Vec<_> = bundles[i + 1..]
                    .iter()
                    .map(|b| b.collection.name.as_str())
                    .collect();
                let mut message = format!("{}: {e}", bundle.collection.name);
                if !rest.is_empty() {
                    message.push_str(&format!("\nNot written: {}", rest.join(", ")));
                }
                let report = (!roots.is_empty()).then_some(merged);
                return (roots, report, Some(message));
            }
        }
    }
    (roots, Some(merged), None)
}

/// Closes the collection at `root` through the confirm flow shared by the sidebar menu's
/// Close and the palette's Close collection.
pub fn close_collection_at(
    workspace: &Entity<Workspace>,
    root: PathBuf,
    window: &mut Window,
    cx: &mut App,
) {
    if crate::scratch::refuse(workspace, &root, window, cx) {
        return;
    }
    let scope = crate::closing::CloseScope::Collection(root);
    crate::closing::confirm_close(workspace.clone(), scope, window, cx);
}
