//! The Check collection dialog: running the core check, listing its findings, opening the
//! item a finding names, the empty and failed states, re-running, and the ways in.

use crate::support;

use apinata_app::check_dialog::{CheckCollection, FindingTarget, finding_target};
use apinata_app::commands::registry;
use apinata_app::palette::{PaletteGroup, palette_items};
use apinata_app::sidebar::menu::{MenuEntry, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use apinata_app::tabs::TabKind;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

// One request with an undefined variable and one file that does not parse.
fn broken() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/{{missing}}")),
        ("bad.yml", "info: [unclosed\n"),
    ])
}

fn clean() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
    ])
}

fn label(
    cx: &mut TestAppContext,
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
) -> Option<String> {
    let id = id.into();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

// Runs the command and waits for the check's answer to replace the running note.
fn check(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(CheckCollection), cx)
    })
    .unwrap();
    wait_answered(cx, s);
}

fn wait_answered(cx: &mut TestAppContext, s: &Started) {
    wait_real(cx, |cx| {
        frame(cx, s.win);
        present(cx, s.win, "check-summary") || present(cx, s.win, "check-error")
    });
}

#[gpui_kit::test]
fn a_parse_error_and_an_undefined_variable_are_both_listed(cx: &mut TestAppContext) {
    let col = broken();
    let s = started(cx, &col);
    check(cx, &s);
    assert!(dialog_open(cx, &s));
    assert_eq!(
        label(cx, &s, "check-summary").as_deref(),
        Some("2 errors, 0 warnings, 1 request checked")
    );
    assert_eq!(
        label(cx, &s, ("check-finding", 0usize)).as_deref(),
        Some("error: a.yml: variable `missing` is not defined")
    );
    let second = label(cx, &s, ("check-finding", 1usize)).unwrap_or_default();
    assert!(second.starts_with("error: bad.yml: "), "{second}");
    assert!(!present(cx, s.win, ("check-finding", 2usize)));
}

#[gpui_kit::test]
fn clicking_a_finding_opens_its_request_and_closes_the_dialog(cx: &mut TestAppContext) {
    let col = broken();
    let s = started(cx, &col);
    check(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click(("check-finding", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    assert!(!dialog_open(cx, &s));
    assert_eq!(tab_names(cx, &s.ws), ["a.yml"]);
}

#[gpui_kit::test]
fn enter_opens_the_selected_finding(cx: &mut TestAppContext) {
    let col = broken();
    let s = started(cx, &col);
    check(cx, &s);
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    assert!(!dialog_open(cx, &s));
    assert_eq!(tab_names(cx, &s.ws), ["a.yml"]);
}

#[gpui_kit::test]
fn down_moves_the_selection(cx: &mut TestAppContext) {
    let col = broken();
    let s = started(cx, &col);
    check(cx, &s);
    let selected = |cx: &mut TestAppContext, ix: usize| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("check-finding", ix)).selected()
        })
        .unwrap()
    };
    assert_eq!(selected(cx, 0), Some(true));
    cx.simulate_keystrokes(s.win, "down");
    frame(cx, s.win);
    assert_eq!(selected(cx, 0), Some(false));
    assert_eq!(selected(cx, 1), Some(true));
}

#[gpui_kit::test]
fn a_clean_collection_says_no_problems_were_found(cx: &mut TestAppContext) {
    let col = clean();
    let s = started(cx, &col);
    check(cx, &s);
    assert_eq!(
        label(cx, &s, "check-empty").as_deref(),
        Some("No problems found")
    );
    assert!(!present(cx, s.win, ("check-finding", 0usize)));
}

#[gpui_kit::test]
fn rerun_checks_the_files_again(cx: &mut TestAppContext) {
    let col = broken();
    let s = started(cx, &col);
    check(cx, &s);
    std::fs::remove_file(s.root.join("bad.yml")).unwrap();
    std::fs::write(s.root.join("a.yml"), request("a", "http://x/a")).unwrap();
    cx.update_window(s.win, |_, w, cx| w.click("check-rerun", cx))
        .unwrap();
    wait_real(cx, |cx| {
        frame(cx, s.win);
        present(cx, s.win, "check-empty")
    });
    assert!(dialog_open(cx, &s));
}

#[gpui_kit::test]
fn a_check_that_cannot_run_shows_why(cx: &mut TestAppContext) {
    let col = clean();
    let s = started(cx, &col);
    std::fs::remove_file(s.root.join("opencollection.yml")).unwrap();
    check(cx, &s);
    let text = label(cx, &s, "check-error").unwrap_or_default();
    assert!(!text.is_empty());
    assert!(!present(cx, s.win, "check-empty"));
}

#[test]
fn the_command_is_in_the_registry_and_the_palette() {
    let command = registry()
        .into_iter()
        .find(|c| c.id == "collection.check")
        .expect("registered");
    assert_eq!(command.label, "Check collection");
    let entry = palette_items()
        .into_iter()
        .find(|e| e.label == "Check collection")
        .expect("in the palette");
    assert_eq!(entry.group, PaletteGroup::Collection);
}

#[gpui_kit::test]
fn the_collection_menu_checks_that_collection(cx: &mut TestAppContext) {
    assert!(menu_entries(NodeKind::Collection).contains(&MenuEntry::CheckCollection));
    let col = broken();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.right_click(row(""), cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        let ix = menu_index(w, "Check collection");
        w.within("popup-menu").click(ix, cx)
    })
    .unwrap();
    wait_answered(cx, &s);
    assert_eq!(
        label(cx, &s, "check-summary").as_deref(),
        Some("2 errors, 0 warnings, 1 request checked")
    );
}

#[test]
fn a_finding_names_the_tab_or_row_it_opens() {
    let ip = support::ip;
    assert_eq!(
        finding_target("users/list.yml", None),
        FindingTarget::Tab(TabKind::Request(ip("users/list.yml")))
    );
    assert_eq!(
        finding_target("opencollection.yml", None),
        FindingTarget::Tab(TabKind::Collection)
    );
    assert_eq!(
        finding_target("users/folder.yml", None),
        FindingTarget::Tab(TabKind::Folder(ip("users")))
    );
    assert_eq!(
        finding_target("environments/dev.yml", Some("dev")),
        FindingTarget::Tab(TabKind::Environments)
    );
    // A valueless secret is reported on the environment by name.
    assert_eq!(
        finding_target("dev", Some("dev")),
        FindingTarget::Tab(TabKind::Environments)
    );
    assert_eq!(
        finding_target(".env", None),
        FindingTarget::Reveal(apinata_core::ItemPath::root())
    );
}
