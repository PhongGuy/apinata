//! The command registry: stable ids, and the palette, app menu and key bindings derived from it.

use crate::support;

use std::collections::BTreeSet;

use apinata_app::app_menu::app_menu_items;
use apinata_app::commands::{Scope, places, registry};
use apinata_app::keymap::bindings;
use apinata_app::palette::palette_items;
use gpui_kit::{AsKeystroke as _, KeyBinding};
use support::secondary;

const CHORD: &str = if cfg!(target_os = "macos") {
    "cmd-y"
} else {
    "ctrl-h"
};

// The file format: an id is never renamed, so this list only grows.
const IDS: &[&str] = &[
    "collection.open",
    "collection.close",
    "collection.collapse_all",
    "collection.expand_all",
    "collection.new_request",
    "collection.new_graphql_request",
    "collection.new_folder",
    "collection.sort_by_name",
    "collection.import",
    "collection.import_curl",
    "collection.export",
    "collection.export_keep_scripts",
    "collection.export_openapi",
    "collection.export_html",
    "collection.new",
    "collection.clone",
    "collection.open_demo",
    "collection.switch_branch",
    "collection.settings",
    "collection.run",
    "collection.run_folder",
    "request.send",
    "request.stop",
    "request.copy_as_curl",
    "request.generate_code",
    "request.convert_to_graphql",
    "request.convert_to_http",
    "request.beautify_body",
    "request.minify_body",
    "request.beautify_query",
    "request.save_as_example",
    "request.export_har",
    "request.compare_response",
    "request.copy_json_value",
    "request.copy_json_path",
    "request.set_json_variable",
    "request.assert_json_value",
    "request.save",
    "request.save_all",
    "request.discard",
    "request.undo",
    "request.redo",
    "request.undo_delete",
    "request.move_row_up",
    "request.move_row_down",
    "request.rename",
    "request.duplicate",
    "request.copy",
    "request.cut",
    "request.paste",
    "request.show_in_file_manager",
    "request.open_in_terminal",
    "request.delete",
    "request.run_selected",
    "tabs.close",
    "tabs.close_all",
    "tabs.close_others",
    "tabs.close_to_right",
    "tabs.reveal_in_sidebar",
    "tabs.close_saved",
    "tabs.pin",
    "tabs.unpin",
    "tabs.reopen",
    "tabs.next",
    "tabs.previous",
    "tabs.last",
    "tabs.go_to_1",
    "tabs.go_to_2",
    "tabs.go_to_3",
    "tabs.go_to_4",
    "tabs.go_to_5",
    "tabs.go_to_6",
    "tabs.go_to_7",
    "tabs.go_to_8",
    "view.toggle_sidebar",
    "view.toggle_layout",
    "view.toggle_console",
    "view.toggle_theme",
    "view.zoom_in",
    "view.zoom_out",
    "view.reset_zoom",
    "view.filter_requests",
    "view.find_in_response",
    "view.find_next_in_response",
    "view.find_previous_in_response",
    "view.find_regex_in_response",
    "view.find_whole_word_in_response",
    "view.focus_url",
    "view.next_section",
    "view.previous_section",
    "view.focus_response",
    "view.focus_next_control",
    "view.focus_previous_control",
    "view.preferences",
    "view.history",
    "view.search_collection",
    "view.check_for_updates",
    "view.command_palette",
    "environment.open",
    "environment.open_selector",
    "environment.import",
    "environment.export",
    "environment.export_all",
    "environment.cookies",
    "environment.variables_overview",
    "environment.runtime_variables",
    "environment.manage_globals",
];

type Triple = (String, String, String);

fn triples(list: &[KeyBinding]) -> Vec<Triple> {
    let mut t: Vec<Triple> = list
        .iter()
        .map(|b| {
            let keys: Vec<String> = b
                .keystrokes()
                .iter()
                .map(|k| k.as_keystroke().unparse(apinata_app::shortcuts::HOST_OS))
                .collect();
            let context = b.predicate().map(|p| p.to_string()).unwrap_or_default();
            (keys.join(" "), b.action().name().to_string(), context)
        })
        .collect();
    t.sort();
    t
}

// Today's bindings (main after plan 5x merged), written out by hand.
fn today() -> Vec<Triple> {
    let s = "Shell";
    let mut e: Vec<(String, &str, &str)> = vec![
        (secondary("enter"), "apinata::SendActive", s),
        (secondary("enter"), "apinata::SendRequest", "RequestEditor"),
        (
            secondary("enter"),
            "apinata::SendRequest",
            "RequestEditor > Input",
        ),
        (
            secondary("enter"),
            "apinata::SendRequest",
            "RequestEditor > Select",
        ),
        (secondary("enter"), "apinata::SendActive", "Shell > Select"),
        (
            secondary("enter"),
            "apinata::CommitChanges",
            "ChangesMessage > Input",
        ),
        (secondary("s"), "apinata::SaveActive", s),
        (secondary("s"), "apinata::SaveRequest", "RequestEditor"),
        (secondary("s"), "apinata::SaveEnvironment", "EnvEditor"),
        (secondary("s"), "apinata::SaveConfig", "ConfigEditor"),
        (secondary("shift-s"), "apinata::SaveAll", s),
        (secondary("z"), "apinata::UndoActive", s),
        (
            secondary("z"),
            "apinata::UndoEdit",
            "RequestEditor && !Input",
        ),
        (
            secondary("z"),
            "apinata::UndoEdit",
            "ConfigEditor && !Input",
        ),
        (secondary("z"), "apinata::UndoEdit", "EnvEditor && !Input"),
        (
            secondary("z"),
            "apinata::UndoEdit",
            "StructuredRows > Input",
        ),
        (secondary("z"), "apinata::UndoEdit", "EnvTable > Input"),
        (secondary("shift-z"), "apinata::RedoActive", s),
        (
            secondary("shift-z"),
            "apinata::RedoEdit",
            "RequestEditor && !Input",
        ),
        (
            secondary("shift-z"),
            "apinata::RedoEdit",
            "ConfigEditor && !Input",
        ),
        (
            secondary("shift-z"),
            "apinata::RedoEdit",
            "EnvEditor && !Input",
        ),
        (
            secondary("shift-z"),
            "apinata::RedoEdit",
            "StructuredRows > Input",
        ),
        (
            secondary("shift-z"),
            "apinata::RedoEdit",
            "EnvTable > Input",
        ),
        ("alt-up".into(), "apinata::MoveRowUp", "StructuredRows"),
        ("alt-down".into(), "apinata::MoveRowDown", "StructuredRows"),
        ("alt-up".into(), "apinata::MoveRowUp", "EnvTable"),
        ("alt-down".into(), "apinata::MoveRowDown", "EnvTable"),
        (secondary("n"), "apinata::NewRequest", s),
        (secondary("w"), "apinata::CloseTab", s),
        (secondary("shift-w"), "apinata::CloseAllTabs", s),
        (secondary("shift-t"), "apinata::ReopenClosedTab", s),
        (secondary("9"), "apinata::ActivateLastTab", s),
        (secondary("{"), "apinata::ActivatePrevTab", s),
        (secondary("}"), "apinata::ActivateNextTab", s),
        ("ctrl-pageup".into(), "apinata::ActivatePrevTab", s),
        ("ctrl-pagedown".into(), "apinata::ActivateNextTab", s),
        (secondary("l"), "apinata::FocusUrl", s),
        (secondary("alt-right"), "apinata::NextSection", s),
        (secondary("alt-left"), "apinata::PreviousSection", s),
        (secondary("alt-r"), "apinata::FocusResponse", s),
        ("alt-shift-f".into(), "apinata::BeautifyQuery", s),
        ("ctrl-tab".into(), "apinata::FocusNextControl", "CodeEditor"),
        (
            "ctrl-shift-tab".into(),
            "apinata::FocusPreviousControl",
            "CodeEditor",
        ),
        (secondary("alt-e"), "apinata::OpenEnvironmentSelector", s),
        (secondary("b"), "apinata::ToggleSidebar", s),
        (secondary("j"), "apinata::ToggleOrientation", s),
        ("ctrl-@".into(), "apinata::ToggleConsole", s),
        ("ctrl-`".into(), "apinata::ToggleConsole", s),
        (secondary("e"), "apinata::OpenEnvironments", s),
        (secondary("o"), "apinata::OpenCollection", s),
        (secondary("shift-i"), "apinata::ImportCollection", s),
        (secondary("shift-n"), "apinata::NewCollection", s),
        (secondary(","), "apinata::OpenPreferences", s),
        (CHORD.into(), "apinata::OpenHistory", s),
        (secondary("shift-f"), "apinata::OpenCollectionSearch", s),
        (secondary("shift-p"), "apinata::OpenCommandPalette", s),
        (
            secondary("f"),
            "apinata::FocusPreferencesSearch",
            "Preferences",
        ),
        (
            "escape".into(),
            "apinata::ClearPreferencesSearch",
            "Preferences",
        ),
        (
            "escape".into(),
            "apinata::CloseFindInResponse",
            "ResponseBody",
        ),
        (secondary("f"), "apinata::FindInResponse", "ResponseBody"),
        (
            secondary("f"),
            "apinata::FindInResponse",
            "ResponseBody > Input",
        ),
        (
            if cfg!(target_os = "macos") {
                "cmd-g"
            } else {
                "f3"
            }
            .into(),
            "apinata::FindNextInResponse",
            "ResponseBody",
        ),
        (
            if cfg!(target_os = "macos") {
                "cmd-g"
            } else {
                "f3"
            }
            .into(),
            "apinata::FindNextInResponse",
            "ResponseBody > Input",
        ),
        (
            if cfg!(target_os = "macos") {
                "cmd-shift-g"
            } else {
                "shift-f3"
            }
            .into(),
            "apinata::FindPreviousInResponse",
            "ResponseBody",
        ),
        (
            if cfg!(target_os = "macos") {
                "cmd-shift-g"
            } else {
                "shift-f3"
            }
            .into(),
            "apinata::FindPreviousInResponse",
            "ResponseBody > Input",
        ),
        ("enter".into(), "zed::NoAction", "DialogFooter"),
        (
            "enter".into(),
            "dialog_select::OpenSelect",
            "DialogSelect > Select",
        ),
        ("space".into(), "ui::Confirm", "Select"),
        (
            "space".into(),
            "dialog_select::OpenSelect",
            "DialogSelect > Select",
        ),
        ("enter".into(), "segmented::ActivateSegment", "Segmented"),
        ("space".into(), "segmented::ActivateSegment", "Segmented"),
        ("right".into(), "segmented::NextSegment", "Segmented"),
        ("down".into(), "segmented::NextSegment", "Segmented"),
        ("left".into(), "segmented::PrevSegment", "Segmented"),
        ("up".into(), "segmented::PrevSegment", "Segmented"),
        ("up".into(), "apinata::RowUp", "ResponseBody"),
        ("down".into(), "apinata::RowDown", "ResponseBody"),
        ("pageup".into(), "apinata::RowPageUp", "ResponseBody"),
        ("pagedown".into(), "apinata::RowPageDown", "ResponseBody"),
        ("home".into(), "apinata::RowFirst", "ResponseBody"),
        ("ctrl-home".into(), "apinata::RowFirst", "ResponseBody"),
        ("end".into(), "apinata::RowLast", "ResponseBody"),
        ("ctrl-end".into(), "apinata::RowLast", "ResponseBody"),
        (secondary("enter"), "zed::NoAction", "SidebarFilter > Input"),
        (
            secondary("enter"),
            "zed::NoAction",
            "Sidebar > Tree > Input",
        ),
        ("enter".into(), "sidebar::OpenSelected", "Sidebar > Tree"),
        ("space".into(), "sidebar::PreviewSelected", "Sidebar > Tree"),
        ("f2".into(), "sidebar::RenameSelected", "Sidebar > Tree"),
        (
            "f12".into(),
            "input::GoToDefinition",
            "GraphqlQuery > Input",
        ),
        (
            "shift-enter".into(),
            "sidebar::RunSelected",
            "Sidebar > Tree",
        ),
        ("shift-tab".into(), "sidebar::LeaveTree", "Sidebar > Tree"),
        ("escape".into(), "sidebar::ClearSelection", "Sidebar > Tree"),
        (
            "escape".into(),
            "sidebar::ClearFilter",
            "SidebarFilter > Input",
        ),
        ("shift-up".into(), "sidebar::ExtendUp", "Sidebar > Tree"),
        ("shift-down".into(), "sidebar::ExtendDown", "Sidebar > Tree"),
        ("delete".into(), "sidebar::DeleteSelected", "Sidebar > Tree"),
        (
            "backspace".into(),
            "sidebar::DeleteSelected",
            "Sidebar > Tree",
        ),
    ];
    for ctx in [s, "Preferences", "Dialog"] {
        e.push((secondary("="), "apinata::ZoomIn", ctx));
        e.push((secondary("+"), "apinata::ZoomIn", ctx));
        e.push((secondary("-"), "apinata::ZoomOut", ctx));
        e.push((secondary("0"), "apinata::ResetZoom", ctx));
    }
    for n in 1..=8 {
        e.push((secondary(&n.to_string()), "apinata::ActivateTab", s));
    }
    for ctx in [s, "Sidebar"] {
        e.push((secondary("f"), "sidebar::FocusFilter", ctx));
        e.push((secondary("r"), "sidebar::RenameSelected", ctx));
        e.push((secondary("d"), "sidebar::CloneSelected", ctx));
    }
    // Item copy, cut and paste belong to the tree alone, so they are not Shell chords.
    for (k, a) in [
        ("c", "sidebar::CopySelected"),
        ("x", "sidebar::CutSelected"),
        ("v", "sidebar::PasteSelected"),
    ] {
        e.push((secondary(k), a, "Sidebar"));
    }
    e.push((secondary("z"), "sidebar::UndoDelete", "Sidebar"));
    // The kit's Redo gets Ctrl+Shift+Z off macOS, where it already has Cmd+Shift+Z.
    #[cfg(not(target_os = "macos"))]
    e.push(("ctrl-shift-z".into(), "input::Redo", "Input"));
    // The find toggles' chords, off macOS only.
    #[cfg(not(target_os = "macos"))]
    for (k, a) in [
        ("alt-r", "apinata::ToggleFindRegex"),
        ("alt-w", "apinata::ToggleFindWholeWord"),
    ] {
        for c in ["ResponseBody", "ResponseBody > Input"] {
            e.push((k.into(), a, c));
        }
    }
    // Redo's extra chord off macOS.
    #[cfg(not(target_os = "macos"))]
    for (a, c) in [
        ("apinata::RedoActive", "Shell"),
        ("apinata::RedoEdit", "RequestEditor && !Input"),
        ("apinata::RedoEdit", "ConfigEditor && !Input"),
        ("apinata::RedoEdit", "EnvEditor && !Input"),
        ("apinata::RedoEdit", "StructuredRows > Input"),
        ("apinata::RedoEdit", "EnvTable > Input"),
    ] {
        e.push(("ctrl-y".into(), a, c));
    }
    let mut t: Vec<Triple> = e
        .into_iter()
        .map(|(k, a, c)| (k, a.to_string(), c.to_string()))
        .collect();
    t.sort();
    t
}

#[gpui_kit::test]
fn the_generated_bindings_are_todays(cx: &mut gpui_kit::TestAppContext) {
    assert_eq!(triples(&cx.update(|cx| bindings(cx))), today());
}

#[test]
fn ids_are_stable_unique_and_grouped() {
    let cmds = registry();
    let ids: Vec<&str> = cmds.iter().map(|c| c.id).collect();
    assert_eq!(ids, IDS);
    for c in &cmds {
        let (group, name) = c.id.split_once('.').expect(c.id);
        assert_eq!(group, c.group.label().to_lowercase(), "{}", c.id);
        assert!(
            name.chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        );
        assert!(!c.targets.is_empty(), "{}", c.id);
        assert!(!c.label.is_empty() && !c.label.to_lowercase().contains("bruno"));
        let in_shell = c.targets.iter().any(|t| t.context == "Shell");
        assert_eq!(c.scope == Scope::Shell, in_shell, "{}", c.id);
        assert!(
            !in_shell || c.targets[0].context == "Shell",
            "{}: Shell target first",
            c.id
        );
    }
    let labels: BTreeSet<&str> = cmds.iter().map(|c| c.label).collect();
    assert_eq!(labels.len(), cmds.len(), "labels are unique");
}

#[test]
fn the_palette_and_the_app_menu_come_from_the_registry() {
    let cmds = registry();
    let listed: Vec<_> = cmds.iter().filter(|c| c.in_palette).collect();
    let items = palette_items();
    assert_eq!(items.len(), listed.len());
    for (e, c) in items.iter().zip(&listed) {
        assert_eq!(
            (e.label, e.group, e.context),
            (c.label, c.group, c.targets[0].context)
        );
        assert!(
            e.action.partial_eq(c.targets[0].action.as_ref()),
            "{}",
            c.id
        );
    }
    let hidden: BTreeSet<&str> = cmds
        .iter()
        .filter(|c| !c.in_palette)
        .map(|c| c.id)
        .collect();
    let mut expected: BTreeSet<&str> = IDS
        .iter()
        .copied()
        .filter(|id| id.starts_with("tabs.go_to_"))
        .collect();
    expected.insert("view.command_palette");
    expected.insert("request.move_row_up");
    expected.insert("request.move_row_down");
    expected.insert("view.find_in_response");
    expected.insert("view.find_next_in_response");
    expected.insert("view.find_previous_in_response");
    expected.insert("view.find_regex_in_response");
    expected.insert("view.find_whole_word_in_response");
    expected.insert("view.focus_next_control");
    expected.insert("view.focus_previous_control");
    assert_eq!(hidden, expected);
    let menu = app_menu_items();
    assert_eq!(menu.len(), 23);
    for (label, action) in menu {
        let c = cmds.iter().find(|c| c.label == label).expect(label);
        assert!(action.partial_eq(c.targets[0].action.as_ref()), "{label}");
    }
}

#[test]
fn find_in_response_is_a_pane_command_on_secondary_f() {
    let cmds = registry();
    let c = cmds
        .iter()
        .find(|c| c.id == "view.find_in_response")
        .expect("registered");
    assert_eq!(c.scope, Scope::Pane("ResponseBody"));
    assert_eq!(c.label, "Find in response");
    assert_eq!(c.default_chords, ["secondary-f"]);
    assert!(!c.in_palette);
    let contexts: Vec<&str> = c.targets.iter().map(|t| t.context).collect();
    assert_eq!(contexts, ["ResponseBody", "ResponseBody > Input"]);
}

#[test]
fn undo_delete_is_a_sidebar_pane_command_on_secondary_z() {
    let cmds = registry();
    let c = cmds
        .iter()
        .find(|c| c.id == "request.undo_delete")
        .expect("registered");
    assert_eq!(c.scope, Scope::Pane("Sidebar"));
    assert_eq!(c.label, "Undo delete");
    assert_eq!(c.default_chords, ["secondary-z"]);
    assert!(c.in_palette);
    let contexts: Vec<&str> = c.targets.iter().map(|t| t.context).collect();
    assert_eq!(contexts, ["Sidebar"]);
}

#[test]
fn environment_and_example_labels_read_as_elsewhere() {
    let cmds = registry();
    let label = |id: &str| cmds.iter().find(|c| c.id == id).expect(id).label;
    assert_eq!(label("environment.open"), "Manage environments");
    assert_eq!(label("environment.import"), "Import environment…");
    assert_eq!(label("request.save_as_example"), "Save response as example");
    assert_eq!(label("request.compare_response"), "Compare response");
    assert_eq!(apinata_app::env_selector::NO_ENVIRONMENT, "No environment");
    assert_eq!(
        apinata_app::env_selector::option_id(apinata_app::env_selector::NO_ENVIRONMENT),
        "env-none"
    );
}

#[test]
fn the_four_dialog_commands_end_in_an_ellipsis() {
    let cmds = registry();
    let label = |id: &str| cmds.iter().find(|c| c.id == id).expect(id).label;
    assert_eq!(label("collection.open"), "Open collection…");
    assert_eq!(label("collection.new"), "New collection…");
    assert_eq!(label("collection.import"), "Import collection…");
    assert_eq!(label("environment.export"), "Export environment…");
    assert_eq!(label("environment.export_all"), "Export environments…");
}

#[test]
fn the_branch_and_clone_commands_are_palette_only() {
    let cmds = registry();
    let label = |id: &str| cmds.iter().find(|c| c.id == id).expect(id).label;
    assert_eq!(label("collection.clone"), "Clone repository…");
    assert_eq!(label("collection.switch_branch"), "Switch branch…");
    assert!(
        app_menu_items()
            .iter()
            .all(|(l, _)| !l.contains("branch") && !l.contains("Clone"))
    );
}

#[test]
fn every_scope_context_has_a_place_and_nests_only_in_places() {
    let named: BTreeSet<&str> = places().iter().map(|p| p.context).collect();
    for c in registry() {
        if let Scope::Editor(ctx) | Scope::Pane(ctx) = c.scope {
            assert!(named.contains(ctx), "{}: {ctx} has a place", c.id);
        }
    }
    for p in places() {
        assert!(p.place.starts_with("in "), "{}: {}", p.context, p.place);
        for w in p.within {
            assert!(named.contains(w), "{} sits within the place {w}", p.context);
        }
    }
}
