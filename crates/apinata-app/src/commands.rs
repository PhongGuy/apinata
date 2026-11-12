//! The command registry: every user-facing command with a stable id, its label, palette
//! group, default chords and the action targets those chords bind. The palette, the app
//! menu and `keymap::bindings` all derive from it.

use gpui_kit::Action;

use crate::body_find::{
    FindInResponse, FindNextInResponse, FindPreviousInResponse, KEY_CONTEXT as RESPONSE_BODY,
    ToggleFindRegex, ToggleFindWholeWord,
};
use crate::body_node::{
    AssertCaretJsonValue, CopyCaretJsonPath, CopyCaretJsonValue, SetCaretJsonVariable,
};
use crate::config_editor::SaveConfig;
use crate::env_editor::SaveEnvironment;
use crate::env_table::ENV_TABLE_CONTEXT as ENV_TABLE;
use crate::history_palette::OpenHistory;
use crate::keymap::{
    DIALOG_CONTEXT, DiscardChanges, FocusResponse, FocusUrl, NewGraphqlRequest, NewRequest,
    NextSection, OpenCollectionSettings, OpenEnvironmentSelector, OpenEnvironments,
    PreviousSection, RedoActive, ResetZoom, RunCollection, RunFolder, SHELL_CONTEXT, SaveActive,
    ToggleConsole, ToggleOrientation, ToggleSidebar, ToggleTheme, UndoActive, ZoomIn, ZoomOut,
};
use crate::palette::{GoToRequest, OpenCommandPalette, PaletteGroup};
use crate::preferences::{KEY_CONTEXT as PREFERENCES, OpenPreferences};
use crate::request_editor::{
    KEY_CONTEXT as REQUEST_EDITOR, RedoEdit, SaveRequest, SendRequest, UndoEdit,
};
use crate::row_move::{MoveRowDown, MoveRowUp};
use crate::search_dialog::OpenCollectionSearch;
use crate::shell::{
    CloneRepository, CloseCollection, ExportCollection, ExportCollectionHtml,
    ExportCollectionOpenApi, ExportCollectionPostmanKeepScripts, ExportEnvironment,
    ExportEnvironments, ImportCollection, ImportCurl, ImportEnvironment, NewCollection,
    OpenCollection, OpenCookies, OpenDemoCollection, OpenGlobalEnvironments, OpenRuntimeVariables,
    OpenVariablesOverview, ShowChanges, SwitchBranch,
};
use crate::sidebar::{
    CloneSelected, CollapseAll, CopySelected, CutSelected, DeleteSelected, ExpandAll, FocusFilter,
    KEY_CONTEXT as SIDEBAR, NewFolder, OpenInTerminal, PasteSelected, RenameSelected, RunSelected,
    ShowInFileManager, SortByName, UndoDelete,
};
use crate::tab_area::{
    ActivateLastTab, ActivateNextTab, ActivatePrevTab, ActivateTab, BeautifyBody, BeautifyQuery,
    CloseAllTabs, CloseOtherTabs, CloseSavedTabs, CloseTab, CloseTabsToTheRight, Compare,
    ConvertToGraphql, ConvertToHttp, CopyAsCurl, ExportHar, GenerateCode, MinifyBody, PinTab,
    ReopenClosedTab, RevealInSidebar, SaveAll, SaveAsExample, SendActive, StopActive, UnpinTab,
};
use crate::ui::{CODE_EDITOR_CONTEXT as CODE_EDITOR, FocusNextControl, FocusPreviousControl};
use crate::updates::CheckForUpdates;

/// The key context a user thinks of a command in, for conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Window-level: the command works with focus anywhere.
    Shell,
    /// Only inside the named editor's key context.
    Editor(&'static str),
    /// Only while focus is inside the named pane, where it deliberately takes the chord from shell commands.
    Pane(&'static str),
}

/// A key context a scope can name, how the Keyboard section speaks of it, and the contexts
/// it can sit inside.
pub struct Place {
    /// The key context, as a `Scope` names it.
    pub context: &'static str,
    /// The phrase shown after a command's label, such as "in code editors".
    pub place: &'static str,
    /// The area's short name, shown beside the keys of a command scoped to this pane.
    pub label: &'static str,
    /// The contexts in this table that this context can be rendered inside.
    pub within: &'static [&'static str],
}

/// Every non-shell key context a command's scope names. Inside a nested context both bind
/// and only one wins, so the conflict check counts the two as overlapping; a test
/// renders the editor hosts to keep `within` true.
pub fn places() -> &'static [Place] {
    &[
        Place {
            context: CODE_EDITOR,
            place: "in code editors",
            label: "Code editor",
            within: &[RESPONSE_BODY],
        },
        Place {
            context: RESPONSE_BODY,
            place: "in the response body",
            label: "Response",
            within: &[],
        },
        Place {
            context: STRUCTURED_ROWS,
            place: "in tables",
            label: "Tables",
            within: &[],
        },
        Place {
            context: SIDEBAR,
            place: "in the sidebar",
            label: "Sidebar",
            within: &[],
        },
    ]
}

/// One place a command's chords bind: an action in a key context.
pub struct Target {
    /// The action the chord dispatches.
    pub action: Box<dyn Action>,
    /// The key context the binding lives in.
    pub context: &'static str,
}

/// One command of the app.
pub struct Command {
    /// `<group>.<name>`, the keymap.json key; never renamed.
    pub id: &'static str,
    /// The text shown in the palette, the app menu and the Keyboard section.
    pub label: &'static str,
    /// The palette section.
    pub group: PaletteGroup,
    /// `Shell` when any target binds in the shell context.
    pub scope: Scope,
    /// Where the chords bind; the first target is what the palette runs.
    pub targets: Vec<Target>,
    /// The chords bound out of the box, as `secondary-x` strings.
    pub default_chords: Vec<&'static str>,
    /// Whether the palette lists the command.
    pub in_palette: bool,
    /// Words a palette search also finds the command by, for names its label lacks.
    pub keywords: &'static [&'static str],
}

// The rows of a structured table; a focused cell or checkbox inside it counts.
const STRUCTURED_ROWS: &str = "StructuredRows";
// A table cell's input inside `STRUCTURED_ROWS_CONTEXT`; a test pins the two together.
const STRUCTURED_ROWS_INPUT: &str = "StructuredRows > Input";
// The sidebar's tree, where row keys such as F2 and Delete act on the selected row.
const SIDEBAR_TREE: &str = "Sidebar > Tree";
// The environment table's cells, which edit through the same history.
const ENV_TABLE_INPUT: &str = "EnvTable > Input";

const GO_TO_IDS: [&str; 8] = [
    "tabs.go_to_1",
    "tabs.go_to_2",
    "tabs.go_to_3",
    "tabs.go_to_4",
    "tabs.go_to_5",
    "tabs.go_to_6",
    "tabs.go_to_7",
    "tabs.go_to_8",
];
const GO_TO_LABELS: [&str; 8] = [
    "Go to tab 1",
    "Go to tab 2",
    "Go to tab 3",
    "Go to tab 4",
    "Go to tab 5",
    "Go to tab 6",
    "Go to tab 7",
    "Go to tab 8",
];
const GO_TO_CHORDS: [&str; 8] = [
    "secondary-1",
    "secondary-2",
    "secondary-3",
    "secondary-4",
    "secondary-5",
    "secondary-6",
    "secondary-7",
    "secondary-8",
];

// A command's scope follows its targets, so the registry cannot disagree with itself.
fn command(
    id: &'static str,
    label: &'static str,
    group: PaletteGroup,
    targets: Vec<Target>,
    default_chords: &[&'static str],
) -> Command {
    let scope = if targets.iter().any(|t| t.context == SHELL_CONTEXT) {
        Scope::Shell
    } else {
        Scope::Editor(targets[0].context)
    };
    Command {
        id,
        label,
        group,
        scope,
        targets,
        default_chords: default_chords.to_vec(),
        in_palette: true,
        keywords: &[],
    }
}

fn at(action: impl Action, context: &'static str) -> Target {
    Target {
        action: Box::new(action),
        context,
    }
}

fn shell(action: impl Action) -> Vec<Target> {
    vec![at(action, SHELL_CONTEXT)]
}

// One action bound in the shell and in a second context that sits outside it, such as the
// Preferences dialog or a sidebar hosted without a shell.
fn shell_and(action: impl Action + Clone, context: &'static str) -> Vec<Target> {
    vec![at(action.clone(), SHELL_CONTEXT), at(action, context)]
}

// Zoom works from Preferences and every other dialog too; the Shell target stays first, since a
// palette's chord hint reads the first one.
fn zoom_targets(action: impl Action + Clone) -> Vec<Target> {
    let mut targets = shell_and(action.clone(), PREFERENCES);
    targets.push(at(action, DIALOG_CONTEXT));
    targets
}

/// The registry label of the command `id`; panics on an unknown id.
pub fn label(id: &str) -> &'static str {
    registry()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("unknown command {id}"))
        .label
}

/// Every command, in palette order.
pub fn registry() -> Vec<Command> {
    use PaletteGroup::*;
    let hidden = |mut c: Command| {
        c.in_palette = false;
        c
    };
    let found_by = |keywords: &'static [&'static str], mut c: Command| {
        c.keywords = keywords;
        c
    };
    let mut cmds = vec![
        command(
            "collection.open",
            "Open collection…",
            Collection,
            shell(OpenCollection),
            &["secondary-o"],
        ),
        command(
            "collection.close",
            "Close collection",
            Collection,
            shell(CloseCollection),
            &[],
        ),
        command(
            "collection.collapse_all",
            "Collapse all",
            Collection,
            shell(CollapseAll),
            &[],
        ),
        command(
            "collection.expand_all",
            "Expand all",
            Collection,
            shell(ExpandAll),
            &[],
        ),
        command(
            "collection.new_request",
            "New request",
            Collection,
            shell(NewRequest),
            &["secondary-n"],
        ),
        command(
            "collection.new_graphql_request",
            "New GraphQL request",
            Collection,
            shell(NewGraphqlRequest),
            &[],
        ),
        command(
            "collection.new_folder",
            "New folder",
            Collection,
            shell(NewFolder),
            &[],
        ),
        command(
            "collection.sort_by_name",
            "Sort by name",
            Collection,
            shell(SortByName),
            &[],
        ),
        command(
            "collection.import",
            "Import collection…",
            Collection,
            shell(ImportCollection),
            &["secondary-shift-i"],
        ),
        command(
            "collection.import_curl",
            "Import curl…",
            Collection,
            shell(ImportCurl),
            &[],
        ),
        command(
            "collection.export",
            "Export collection to Postman, converting scripts",
            Collection,
            shell(ExportCollection),
            &[],
        ),
        command(
            "collection.export_keep_scripts",
            "Export collection to Postman, keeping scripts as written",
            Collection,
            shell(ExportCollectionPostmanKeepScripts),
            &[],
        ),
        command(
            "collection.export_openapi",
            "Export collection to OpenAPI",
            Collection,
            shell(ExportCollectionOpenApi),
            &[],
        ),
        command(
            "collection.export_html",
            "Export collection to HTML documentation",
            Collection,
            shell(ExportCollectionHtml),
            &[],
        ),
        command(
            "collection.new",
            "New collection…",
            Collection,
            shell(NewCollection),
            &["secondary-shift-n"],
        ),
        found_by(
            &["git"],
            command(
                "collection.clone",
                "Clone repository…",
                Collection,
                shell(CloneRepository),
                &[],
            ),
        ),
        command(
            "collection.open_demo",
            "Open the demo collection…",
            Collection,
            shell(OpenDemoCollection),
            &[],
        ),
        found_by(
            &["git", "checkout"],
            command(
                "collection.switch_branch",
                "Switch branch…",
                Collection,
                shell(SwitchBranch),
                &[],
            ),
        ),
        found_by(
            &["git", "commit", "stage"],
            command(
                "collection.show_changes",
                "Show changes…",
                Collection,
                shell(ShowChanges),
                &[],
            ),
        ),
        command(
            "collection.settings",
            "Collection settings",
            Collection,
            shell(OpenCollectionSettings),
            &[],
        ),
        command(
            "collection.run",
            "Run collection",
            Collection,
            shell(RunCollection),
            &[],
        ),
        command(
            "collection.run_folder",
            "Run folder",
            Collection,
            shell(RunFolder),
            &[],
        ),
        command(
            "request.send",
            "Send request",
            Request,
            vec![
                at(SendActive, SHELL_CONTEXT),
                at(SendRequest, REQUEST_EDITOR),
                // Input depth too, so it beats the URL input's own Enter.
                at(SendRequest, "RequestEditor > Input"),
                // A closed dropdown too, over the kit's own Ctrl+Enter there; an open list
                // sits deeper and keeps it.
                at(SendRequest, "RequestEditor > Select"),
                at(SendActive, "Shell > Select"),
            ],
            &["secondary-enter"],
        ),
        command(
            "request.stop",
            "Stop response",
            Request,
            shell(StopActive),
            &[],
        ),
        command(
            "request.copy_as_curl",
            "Copy as curl",
            Request,
            shell(CopyAsCurl),
            &[],
        ),
        command(
            "request.generate_code",
            "Generate code",
            Request,
            shell(GenerateCode),
            &[],
        ),
        command(
            "request.convert_to_graphql",
            "Convert to GraphQL",
            Request,
            shell(ConvertToGraphql),
            &[],
        ),
        command(
            "request.convert_to_http",
            "Convert to HTTP",
            Request,
            shell(ConvertToHttp),
            &[],
        ),
        command(
            "request.beautify_body",
            "Beautify body",
            Request,
            shell(BeautifyBody),
            &[],
        ),
        command(
            "request.minify_body",
            "Minify body",
            Request,
            shell(MinifyBody),
            &[],
        ),
        command(
            "request.beautify_query",
            "Beautify query",
            Request,
            shell(BeautifyQuery),
            &["shift-alt-f"],
        ),
        command(
            "request.save_as_example",
            "Save response as example",
            Request,
            shell(SaveAsExample),
            &[],
        ),
        command(
            "request.export_har",
            "Export response as HAR",
            Request,
            shell(ExportHar),
            &[],
        ),
        command(
            "request.compare_response",
            "Compare response",
            Request,
            shell(Compare),
            &[],
        ),
        command(
            "request.copy_json_value",
            "Copy JSON value at caret",
            Request,
            shell(CopyCaretJsonValue),
            &[],
        ),
        command(
            "request.copy_json_path",
            "Copy JSON path at caret",
            Request,
            shell(CopyCaretJsonPath),
            &[],
        ),
        command(
            "request.set_json_variable",
            "Set JSON value at caret as variable…",
            Request,
            shell(SetCaretJsonVariable),
            &[],
        ),
        command(
            "request.assert_json_value",
            "Assert JSON value at caret",
            Request,
            shell(AssertCaretJsonValue),
            &[],
        ),
        command(
            "request.save",
            "Save",
            Request,
            vec![
                at(SaveActive, SHELL_CONTEXT),
                at(SaveRequest, REQUEST_EDITOR),
                at(SaveEnvironment, "EnvEditor"),
                at(SaveConfig, "ConfigEditor"),
            ],
            &["secondary-s"],
        ),
        command(
            "request.save_all",
            "Save all",
            Request,
            shell(SaveAll),
            &["secondary-shift-s"],
        ),
        command(
            "request.discard",
            "Discard changes",
            Request,
            shell(DiscardChanges),
            &[],
        ),
        // Outside a text input the tab's structured history; inside a table cell too, since a
        // cell's own undo would diverge from the table. Other inputs keep the kit's undo.
        command(
            "request.undo",
            "Undo edit",
            Request,
            vec![
                at(UndoActive, SHELL_CONTEXT),
                at(UndoEdit, "RequestEditor && !Input"),
                at(UndoEdit, "ConfigEditor && !Input"),
                at(UndoEdit, "EnvEditor && !Input"),
                at(UndoEdit, STRUCTURED_ROWS_INPUT),
                at(UndoEdit, ENV_TABLE_INPUT),
            ],
            &["secondary-z"],
        ),
        command(
            "request.redo",
            "Redo edit",
            Request,
            vec![
                at(RedoActive, SHELL_CONTEXT),
                at(RedoEdit, "RequestEditor && !Input"),
                at(RedoEdit, "ConfigEditor && !Input"),
                at(RedoEdit, "EnvEditor && !Input"),
                at(RedoEdit, STRUCTURED_ROWS_INPUT),
                at(RedoEdit, ENV_TABLE_INPUT),
            ],
            if cfg!(target_os = "macos") {
                &["secondary-shift-z"]
            } else {
                // The palette shows the last chord bound, so Ctrl+Shift+Z comes last.
                &["ctrl-y", "secondary-shift-z"]
            },
        ),
        {
            // Takes the shell's Undo edit chord inside the sidebar; with nothing to undo there
            // the handler passes the key on, and the sidebar's inputs keep their own undo. With
            // no tab open the shell's Undo edit falls back to it, so it works with the sidebar
            // hidden too.
            let mut c = command(
                "request.undo_delete",
                "Undo delete",
                Request,
                vec![at(UndoDelete, SIDEBAR)],
                &["secondary-z"],
            );
            c.scope = Scope::Pane(SIDEBAR);
            c
        },
        // Bound while focus is in a table row's cells or checkbox; the palette cannot say which row.
        {
            let mut c = command(
                "request.move_row_up",
                "Move row up",
                Request,
                vec![at(MoveRowUp, STRUCTURED_ROWS), at(MoveRowUp, ENV_TABLE)],
                &["alt-up"],
            );
            c.in_palette = false;
            c
        },
        {
            let mut c = command(
                "request.move_row_down",
                "Move row down",
                Request,
                vec![at(MoveRowDown, STRUCTURED_ROWS), at(MoveRowDown, ENV_TABLE)],
                &["alt-down"],
            );
            c.in_palette = false;
            c
        },
        command(
            "request.rename",
            "Rename selected",
            Request,
            shell_and(RenameSelected, SIDEBAR),
            &["secondary-r"],
        ),
        {
            // F2 renames in the tree alone; anywhere else it would rename a row out of sight.
            let mut c = command(
                "request.rename_in_sidebar",
                "Rename in sidebar",
                Request,
                vec![at(RenameSelected, SIDEBAR_TREE)],
                &["f2"],
            );
            c.scope = Scope::Pane(SIDEBAR);
            hidden(c)
        },
        command(
            "request.duplicate",
            "Duplicate selected",
            Request,
            shell_and(CloneSelected, SIDEBAR),
            &["secondary-d"],
        ),
        command(
            "request.copy",
            "Copy selected items",
            Request,
            vec![at(CopySelected, SIDEBAR)],
            &["secondary-c"],
        ),
        command(
            "request.cut",
            "Cut selected items",
            Request,
            vec![at(CutSelected, SIDEBAR)],
            &["secondary-x"],
        ),
        command(
            "request.paste",
            "Paste into selected folder",
            Request,
            vec![at(PasteSelected, SIDEBAR)],
            &["secondary-v"],
        ),
        command(
            "request.show_in_file_manager",
            "Show in file manager",
            Request,
            shell(ShowInFileManager),
            &[],
        ),
        command(
            "request.open_in_terminal",
            "Open in terminal",
            Request,
            shell(OpenInTerminal),
            &[],
        ),
        command(
            "request.delete",
            "Delete selected",
            Request,
            shell(DeleteSelected),
            &[],
        ),
        {
            // The last chord is the one menus show: Delete, or the Mac's Delete key (Backspace).
            let mut c = command(
                "request.delete_in_sidebar",
                "Delete in sidebar",
                Request,
                vec![at(DeleteSelected, SIDEBAR_TREE)],
                if cfg!(target_os = "macos") {
                    &["delete", "backspace"]
                } else {
                    &["backspace", "delete"]
                },
            );
            c.scope = Scope::Pane(SIDEBAR);
            hidden(c)
        },
        command(
            "request.run_selected",
            "Run selected requests",
            Request,
            shell(RunSelected),
            &[],
        ),
        command(
            "tabs.close",
            "Close tab",
            Tabs,
            shell(CloseTab),
            &["secondary-w"],
        ),
        command(
            "tabs.close_all",
            "Close all tabs",
            Tabs,
            shell(CloseAllTabs),
            &["secondary-shift-w"],
        ),
        command(
            "tabs.close_others",
            "Close other tabs",
            Tabs,
            shell(CloseOtherTabs),
            &[],
        ),
        command(
            "tabs.close_to_right",
            "Close to the right",
            Tabs,
            shell(CloseTabsToTheRight),
            &[],
        ),
        command(
            "tabs.reveal_in_sidebar",
            "Reveal in sidebar",
            Tabs,
            shell(RevealInSidebar),
            &[],
        ),
        command(
            "tabs.close_saved",
            "Close saved tabs",
            Tabs,
            shell(CloseSavedTabs),
            &[],
        ),
        command("tabs.pin", "Pin tab", Tabs, shell(PinTab), &[]),
        command("tabs.unpin", "Unpin tab", Tabs, shell(UnpinTab), &[]),
        command(
            "tabs.reopen",
            "Reopen closed tab",
            Tabs,
            shell(ReopenClosedTab),
            &["secondary-shift-t"],
        ),
        command(
            "tabs.next",
            "Next tab",
            Tabs,
            shell(ActivateNextTab),
            // Linux reports ctrl+shift+] as the shifted symbol, so `shift-]` would never fire.
            // Ctrl+Tab comes last, so the palette and tooltip show the key browsers use.
            &["secondary-}", "ctrl-pagedown", "ctrl-tab"],
        ),
        command(
            "tabs.previous",
            "Previous tab",
            Tabs,
            shell(ActivatePrevTab),
            &["secondary-{", "ctrl-pageup", "ctrl-shift-tab"],
        ),
        command(
            "tabs.last",
            "Last tab",
            Tabs,
            shell(ActivateLastTab),
            &["secondary-9"],
        ),
    ];
    cmds.extend((0..8).map(|n| {
        hidden(command(
            GO_TO_IDS[n],
            GO_TO_LABELS[n],
            Tabs,
            shell(ActivateTab(n)),
            &[GO_TO_CHORDS[n]],
        ))
    }));
    cmds.extend([
        command(
            "view.toggle_sidebar",
            "Toggle sidebar",
            View,
            shell(ToggleSidebar),
            &["secondary-b"],
        ),
        command(
            "view.toggle_layout",
            "Toggle layout: stacked or side by side",
            View,
            shell(ToggleOrientation),
            &["secondary-j"],
        ),
        // A literal ctrl on every platform: no app or kit binding uses the backtick. Linux
        // reports ctrl and the backtick key as ctrl-@ on a US layout, so that is bound too,
        // first, since the last chord is the one the palette and tooltip show.
        command(
            "view.toggle_console",
            "Toggle console",
            View,
            shell(ToggleConsole),
            &["ctrl-@", "ctrl-`"],
        ),
        found_by(
            &["dark mode", "light mode"],
            command(
                "view.toggle_theme",
                "Toggle theme",
                View,
                shell(ToggleTheme),
                &[],
            ),
        ),
        command(
            "view.zoom_in",
            "Zoom in",
            View,
            zoom_targets(ZoomIn),
            // Linux drops shift on symbol keys, so ctrl+shift+= arrives as `secondary-+`. The
            // later binding wins a tie, so the plain chord comes last and key hints show it.
            &["secondary-+", "secondary-="],
        ),
        command(
            "view.zoom_out",
            "Zoom out",
            View,
            zoom_targets(ZoomOut),
            &["secondary--"],
        ),
        command(
            "view.reset_zoom",
            "Reset zoom",
            View,
            zoom_targets(ResetZoom),
            &["secondary-0"],
        ),
        command(
            "view.filter_requests",
            "Filter requests",
            View,
            shell_and(FocusFilter, SIDEBAR),
            &["secondary-f"],
        ),
        {
            // The chord is the shell's Filter requests chord, taken only inside the body.
            let mut c = command(
                "view.find_in_response",
                "Find in response",
                View,
                vec![
                    at(FindInResponse, RESPONSE_BODY),
                    at(FindInResponse, "ResponseBody > Input"),
                ],
                &["secondary-f"],
            );
            c.scope = Scope::Pane(RESPONSE_BODY);
            hidden(c)
        },
        {
            // Stepping needs an open find bar, so both keys live in the body's contexts only.
            let mut c = command(
                "view.find_next_in_response",
                "Find next in response",
                View,
                vec![
                    at(FindNextInResponse, RESPONSE_BODY),
                    at(FindNextInResponse, "ResponseBody > Input"),
                ],
                if cfg!(target_os = "macos") {
                    &["cmd-g"]
                } else {
                    &["f3"]
                },
            );
            c.scope = Scope::Pane(RESPONSE_BODY);
            hidden(c)
        },
        {
            let mut c = command(
                "view.find_previous_in_response",
                "Find previous in response",
                View,
                vec![
                    at(FindPreviousInResponse, RESPONSE_BODY),
                    at(FindPreviousInResponse, "ResponseBody > Input"),
                ],
                if cfg!(target_os = "macos") {
                    &["cmd-shift-g"]
                } else {
                    &["shift-f3"]
                },
            );
            c.scope = Scope::Pane(RESPONSE_BODY);
            hidden(c)
        },
        // Alt+R and Alt+W as in common editors, but not on macOS, where Option types a
        // character in the find field and Cmd+Option+R already focuses the response.
        {
            let mut c = command(
                "view.find_regex_in_response",
                "Toggle regex in response find",
                View,
                vec![
                    at(ToggleFindRegex, RESPONSE_BODY),
                    at(ToggleFindRegex, "ResponseBody > Input"),
                ],
                if cfg!(target_os = "macos") {
                    &[]
                } else {
                    &["alt-r"]
                },
            );
            c.scope = Scope::Pane(RESPONSE_BODY);
            hidden(c)
        },
        {
            let mut c = command(
                "view.find_whole_word_in_response",
                "Toggle whole word in response find",
                View,
                vec![
                    at(ToggleFindWholeWord, RESPONSE_BODY),
                    at(ToggleFindWholeWord, "ResponseBody > Input"),
                ],
                if cfg!(target_os = "macos") {
                    &[]
                } else {
                    &["alt-w"]
                },
            );
            c.scope = Scope::Pane(RESPONSE_BODY);
            hidden(c)
        },
        // The chords below take Alt with the secondary key: the kit binds plain Alt-arrows to
        // word movement and Ctrl-arrows likewise, so these stay clear of every input.
        command(
            "view.focus_url",
            "Focus URL bar",
            View,
            shell(FocusUrl),
            &["secondary-l"],
        ),
        command(
            "view.next_section",
            "Next request section",
            View,
            shell(NextSection),
            &["secondary-alt-right"],
        ),
        command(
            "view.previous_section",
            "Previous request section",
            View,
            shell(PreviousSection),
            &["secondary-alt-left"],
        ),
        command(
            "view.focus_response",
            "Focus response",
            View,
            shell(FocusResponse),
            &["secondary-alt-r"],
        ),
        // Tab indents inside a code editor, so these leave it; the palette has no editor to
        // leave. A literal Ctrl on every platform, as text views on macOS also use it. They
        // deliberately take Ctrl+Tab from the window's tab switching inside an editor.
        {
            let mut c = command(
                "view.focus_next_control",
                "Focus next control",
                View,
                vec![at(FocusNextControl, CODE_EDITOR)],
                &["ctrl-tab"],
            );
            c.scope = Scope::Pane(CODE_EDITOR);
            hidden(c)
        },
        {
            let mut c = command(
                "view.focus_previous_control",
                "Focus previous control",
                View,
                vec![at(FocusPreviousControl, CODE_EDITOR)],
                &["ctrl-shift-tab"],
            );
            c.scope = Scope::Pane(CODE_EDITOR);
            hidden(c)
        },
        found_by(
            &["settings"],
            command(
                "view.preferences",
                "Preferences",
                View,
                shell(OpenPreferences),
                &["secondary-,"],
            ),
        ),
        command(
            "view.history",
            "History",
            View,
            shell(OpenHistory),
            if cfg!(target_os = "macos") {
                &["cmd-y"]
            } else {
                &["ctrl-h"]
            },
        ),
        command(
            "view.search_collection",
            "Search collection",
            View,
            shell(OpenCollectionSearch),
            &["secondary-shift-f"],
        ),
        command(
            "view.check_for_updates",
            "Check for updates",
            View,
            shell(CheckForUpdates),
            &[],
        ),
        hidden(command(
            "view.command_palette",
            "Command palette",
            View,
            shell(OpenCommandPalette),
            &["secondary-shift-p"],
        )),
        command(
            "view.go_to_request",
            "Go to request",
            View,
            shell(GoToRequest),
            &["secondary-p"],
        ),
        command(
            "environment.open",
            "Manage environments",
            Environment,
            shell(OpenEnvironments),
            &["secondary-e"],
        ),
        command(
            "environment.open_selector",
            "Open environment selector",
            Environment,
            shell(OpenEnvironmentSelector),
            &["secondary-alt-e"],
        ),
        command(
            "environment.import",
            "Import environment…",
            Environment,
            shell(ImportEnvironment),
            &[],
        ),
        command(
            "environment.export",
            "Export current environment to Postman…",
            Environment,
            shell(ExportEnvironment),
            &[],
        ),
        command(
            "environment.export_all",
            "Export all environments to Postman…",
            Environment,
            shell(ExportEnvironments),
            &[],
        ),
        command(
            "environment.cookies",
            "Cookies",
            Environment,
            shell(OpenCookies),
            &[],
        ),
        found_by(
            &["environment"],
            command(
                "environment.variables_overview",
                "Show all variables",
                Environment,
                shell(OpenVariablesOverview),
                &[],
            ),
        ),
        found_by(
            &["environment"],
            command(
                "environment.runtime_variables",
                "Edit variables set by requests",
                Environment,
                shell(OpenRuntimeVariables),
                &[],
            ),
        ),
        found_by(
            &["environment"],
            command(
                "environment.manage_globals",
                "Manage globals…",
                Environment,
                shell(OpenGlobalEnvironments),
                &[],
            ),
        ),
    ]);
    cmds
}
