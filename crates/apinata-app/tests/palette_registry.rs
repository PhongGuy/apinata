//! The command palette's registry: every app action, grouped, with the key context its
//! binding lives in, and a stable mapping from the kit's index paths.

use crate::support;

use std::collections::BTreeSet;

use apinata_app::keymap::bindings;
use apinata_app::palette::{PaletteGroup, entry_index, index_path, palette_items};
use apinata_app::tabs::TabKind;
use apinata_core::ItemPath;
use gpui_kit::component::IndexPath;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::{AppContext as _, AsKeystroke as _, TestAppContext};
use support::secondary;

// The kit's editor Redo only gains a second chord here; it is no app command.
// Context-specific saves go through Save, positional tab keys through next, previous and
// last, and the palette does not list itself. `NoAction` only masks a key, so it is no command.
const NOT_LISTED: &[&str] = &[
    "apinata::SaveRequest",
    "apinata::SendRequest",
    "apinata::UndoEdit",
    "apinata::RedoEdit",
    "apinata::SaveEnvironment",
    "apinata::SaveConfig",
    "apinata::CommitChanges",
    "apinata::ActivateTab",
    "apinata::OpenCommandPalette",
    "apinata::FocusPreferencesSearch",
    "apinata::MoveRowUp",
    "apinata::MoveRowDown",
    "apinata::FindInResponse",
    "apinata::FindNextInResponse",
    "apinata::FindPreviousInResponse",
    "apinata::CloseFindInResponse",
    "apinata::ClearPreferencesSearch",
    "apinata::FocusNextControl",
    "apinata::FocusPreviousControl",
    "input::Redo",
    "dialog_select::OpenSelect",
    "segmented::ActivateSegment",
    "segmented::NextSegment",
    "segmented::PrevSegment",
    "sidebar::OpenSelected",
    "sidebar::PreviewSelected",
    "sidebar::LeaveTree",
    "sidebar::ClearFilter",
    "sidebar::ClearSelection",
    "sidebar::ExtendUp",
    "sidebar::ExtendDown",
    "apinata::RowUp",
    "apinata::RowDown",
    "apinata::RowPageUp",
    "apinata::RowPageDown",
    "apinata::RowFirst",
    "apinata::RowLast",
    "zed::NoAction",
];

// Bound only inside the sidebar tree, whose chain a single palette context cannot name, so the
// palette row shows no key for them.
const TREE_ONLY: &[&str] = &["sidebar::DeleteSelected", "sidebar::RunSelected"];

// App actions with no key binding, which the bindings cannot reveal.
const UNBOUND: [&str; 27] = [
    "apinata::CopyCaretJsonValue",
    "apinata::CopyCaretJsonPath",
    "apinata::SetCaretJsonVariable",
    "apinata::CheckForUpdates",
    "apinata::ImportCurl",
    "apinata::OpenCollectionSettings",
    "apinata::RunCollection",
    "apinata::RunFolder",
    "apinata::ImportEnvironment",
    "apinata::OpenHistory",
    "apinata::OpenCookies",
    "apinata::OpenRuntimeVariables",
    "apinata::OpenVariablesOverview",
    "apinata::NewGraphqlRequest",
    "apinata::CloseOtherTabs",
    "apinata::CloseTabsToTheRight",
    "apinata::RevealInSidebar",
    "apinata::CloseSavedTabs",
    "apinata::OpenGlobalEnvironments",
    "apinata::CloseCollection",
    "apinata::ToggleTheme",
    "apinata::DiscardChanges",
    "sidebar::DeleteSelected",
    "sidebar::NewFolder",
    "sidebar::RunSelected",
    "apinata::CloneRepository",
    "apinata::SwitchBranch",
];

fn listed() -> BTreeSet<&'static str> {
    palette_items().iter().map(|e| e.action.name()).collect()
}

#[test]
fn every_app_action_is_listed() {
    let bound: BTreeSet<&str> = bindings()
        .iter()
        .map(|b| b.action().name())
        .filter(|n| !NOT_LISTED.contains(n))
        .chain(UNBOUND)
        .collect();
    let listed = listed();
    let missing: Vec<_> = bound.difference(&listed).collect();
    assert!(missing.is_empty(), "not in the palette: {missing:?}");
    for name in ["apinata::ImportCollection", "apinata::NewCollection"] {
        assert!(listed.contains(name), "plan 4c's {name} is listed");
    }
    assert!(
        NOT_LISTED.iter().all(|n| !listed.contains(n)),
        "an excluded action is listed"
    );
}

#[test]
fn entries_come_group_by_group_with_unique_labels() {
    let items = palette_items();
    let groups: Vec<PaletteGroup> = items.iter().map(|e| e.group).collect();
    let mut sorted = groups.clone();
    sorted.sort_by_key(|g| PaletteGroup::ALL.iter().position(|x| x == g).unwrap());
    assert_eq!(groups, sorted, "entries come in group order");
    for g in PaletteGroup::ALL {
        assert!(groups.contains(&g), "{g:?} has an entry");
        assert!(!g.label().is_empty());
    }
    let labels: BTreeSet<&str> = items.iter().map(|e| e.label).collect();
    assert_eq!(labels.len(), items.len(), "labels are unique");
    assert!(
        items
            .iter()
            .all(|e| !e.label.is_empty() && !e.label.to_lowercase().contains("bruno"))
    );
    assert!(
        items
            .iter()
            .all(|e| ["Shell", "RequestEditor", "Sidebar"].contains(&e.context))
    );
}

#[test]
fn index_paths_round_trip() {
    let items = palette_items();
    for ix in 0..items.len() {
        assert_eq!(
            entry_index(&items, index_path(&items, ix)),
            Some(ix),
            "entry {ix}"
        );
    }
    assert_eq!(index_path(&items, 0), IndexPath::new(0).section(0));
    let first_request = items
        .iter()
        .position(|e| e.group == PaletteGroup::Request)
        .unwrap();
    assert_eq!(
        index_path(&items, first_request),
        IndexPath::new(0).section(1)
    );
    assert_eq!(entry_index(&items, IndexPath::new(999).section(0)), None);
    assert_eq!(entry_index(&items, IndexPath::new(0).section(99)), None);
}

#[gpui_kit::test]
fn each_bound_entry_has_a_key_in_its_context(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let bound: BTreeSet<&str> = bindings().iter().map(|b| b.action().name()).collect();
    assert!(
        bound.contains("apinata::OpenCommandPalette"),
        "secondary-shift-p is bound"
    );
    let vcx = cx.add_empty_window();
    vcx.update(|window, _| {
        for e in palette_items()
            .iter()
            .filter(|e| bound.contains(e.action.name()) && !TREE_ONLY.contains(&e.action.name()))
        {
            let kbd = Kbd::binding_for_action(e.action.as_ref(), Some(e.context), window);
            assert!(kbd.is_some(), "{} has no key in {}", e.label, e.context);
        }
    });
    let toggle = palette_items()
        .into_iter()
        .find(|e| e.action.partial_eq(&apinata_app::keymap::ToggleSidebar))
        .expect("Toggle sidebar is listed");
    assert_eq!(toggle.context, "Shell");
    assert!(bindings().iter().any(
        |b| b.action().partial_eq(&apinata_app::keymap::ToggleSidebar)
            && b.keystrokes().first().map(|k| k.as_keystroke().unparse()) == Some(secondary("b"))
    ));
}

#[test]
fn every_action_that_left_the_title_bar_is_in_the_palette() {
    let listed = listed();
    for name in [
        "apinata::OpenCollection",
        "apinata::ImportCollection",
        "apinata::NewCollection",
        "apinata::ImportEnvironment",
        "apinata::OpenHistory",
        "apinata::OpenCookies",
    ] {
        assert!(listed.contains(name), "{name} is in the palette");
    }
}

#[test]
fn the_palette_offers_discard_delete_and_new_folder() {
    let labels: Vec<&str> = palette_items().iter().map(|e| e.label).collect();
    for l in [
        "Discard changes",
        "Delete selected",
        "New folder",
        "Duplicate selected",
    ] {
        assert!(labels.contains(&l), "{l} in {labels:?}");
    }
    let mut seen = Vec::new();
    for e in palette_items() {
        if seen.last() != Some(&e.group) {
            assert!(!seen.contains(&e.group), "{:?} is split", e.group);
            seen.push(e.group);
        }
    }
}

#[gpui_kit::test]
fn collection_settings_opens_the_current_collections_settings(cx: &mut TestAppContext) {
    let entry = palette_items()
        .into_iter()
        .find(|e| e.label == "Collection settings")
        .expect("Collection settings is in the palette");
    assert_eq!(entry.group, PaletteGroup::Collection);
    assert_eq!(entry.action.name(), "apinata::OpenCollectionSettings");
    let col = support::two_requests();
    let s = support::started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(entry.action.boxed_clone(), cx)
    })
    .unwrap();
    support::frame(cx, s.win);
    let active = s.ws.read_with(cx, |w, _| {
        w.tabs().active().map(|t| (t.root.clone(), t.kind.clone()))
    });
    let root = std::fs::canonicalize(&s.root).unwrap();
    let (at, kind) = active.expect("a tab is active");
    assert_eq!(kind, TabKind::Collection);
    assert_eq!(std::fs::canonicalize(at).unwrap(), root);
}

// A request in `users/` with one saved example.
const SHOW_USER: &str = "info:\n  name: Show user\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: http://x/users/1\nexamples:\n  - name: ok\n    request:\n      url: http://x/users/1\n      method: GET\n    response:\n      status: 200\n      statusText: OK\n";

// A collection with `users/list.yml`, `users/show.yml` (one example) and the root-level `a.yml`.
fn with_folder() -> tempfile::TempDir {
    support::write_tree(&[
        ("opencollection.yml", support::ROOT),
        ("a.yml", &support::request("a", "http://x/a")),
        ("users/folder.yml", &support::folder("Users")),
        (
            "users/list.yml",
            &support::request("List users", "http://x/users"),
        ),
        ("users/show.yml", SHOW_USER),
    ])
}

// Dispatches the Collection group's palette entry `label` and returns the active tab's kind.
fn run_from_palette(cx: &mut TestAppContext, s: &support::Started, label: &str) -> Option<TabKind> {
    let entry = palette_items()
        .into_iter()
        .find(|e| e.label == label)
        .unwrap_or_else(|| panic!("{label} is in the palette"));
    assert_eq!(entry.group, PaletteGroup::Collection);
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(entry.action.boxed_clone(), cx)
    })
    .unwrap();
    support::frame(cx, s.win);
    s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()))
}

#[gpui_kit::test]
fn run_collection_opens_the_current_collections_runner(cx: &mut TestAppContext) {
    let col = with_folder();
    let s = support::started(cx, &col);
    support::open_tab(cx, &s.ws, &s.root, "users/list.yml");
    let kind = run_from_palette(cx, &s, "Run collection");
    assert_eq!(kind, Some(TabKind::Runner(ItemPath::root())));
}

#[gpui_kit::test]
fn run_folder_runs_the_active_requests_folder(cx: &mut TestAppContext) {
    let col = with_folder();
    let s = support::started(cx, &col);
    support::open_tab(cx, &s.ws, &s.root, "users/list.yml");
    let kind = run_from_palette(cx, &s, "Run folder");
    assert_eq!(kind, Some(TabKind::Runner(support::ip("users"))));
}

#[gpui_kit::test]
fn run_folder_without_an_active_request_runs_the_collection(cx: &mut TestAppContext) {
    let col = with_folder();
    let s = support::started(cx, &col);
    let kind = run_from_palette(cx, &s, "Run folder");
    assert_eq!(kind, Some(TabKind::Runner(ItemPath::root())));
}

#[gpui_kit::test]
fn run_folder_on_a_root_level_request_runs_the_collection(cx: &mut TestAppContext) {
    let col = with_folder();
    let s = support::started(cx, &col);
    support::open_tab(cx, &s.ws, &s.root, "a.yml");
    let kind = run_from_palette(cx, &s, "Run folder");
    assert_eq!(kind, Some(TabKind::Runner(ItemPath::root())));
}

#[gpui_kit::test]
fn run_folder_from_an_example_runs_its_requests_folder(cx: &mut TestAppContext) {
    let col = with_folder();
    let s = support::started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.open_example(&s.root, support::ip("users/show.yml"), 0, false, cx)
    })
    .expect("an example tab");
    support::frame(cx, s.win);
    let kind = run_from_palette(cx, &s, "Run folder");
    assert_eq!(kind, Some(TabKind::Runner(support::ip("users"))));
}
