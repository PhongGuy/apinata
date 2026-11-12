//! Right-click menus: entries grouped by separators, Delete marked as destructive, and each
//! entry's bound key shown beside it, read from the live keymap.

use crate::support;

use apinata_app::commands::{Scope, registry};
use apinata_app::shortcuts::HOST_OS;
use apinata_app::sidebar::menu::{BulkEntry, MenuEntry};
use apinata_app::sidebar::multi::Tally;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Keystroke, TestAppContext};
use support::*;

// How the menus print `chord`, with `secondary` spelled for the host.
fn shown(chord: &str) -> String {
    let secondary = if HOST_OS == gpui_kit::OperatingSystem::Mac {
        "cmd"
    } else {
        "ctrl"
    };
    let chord = chord.replace("secondary", secondary);
    Kbd::format(&Keystroke::parse_with_os(&chord, HOST_OS).unwrap())
}

fn open_menu(cx: &mut TestAppContext, win: AnyWindowHandle, path: &str) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.right_click(row(path), cx)
    })
    .unwrap();
    frame(cx, win);
}

fn row_keys(cx: &mut TestAppContext, win: AnyWindowHandle, label: &str) -> Vec<String> {
    cx.update_window(win, |_, w, _| menu_row_keys(w, label))
        .unwrap()
}

fn labels(cx: &mut TestAppContext, win: AnyWindowHandle) -> Vec<Option<String>> {
    cx.update_window(win, |_, w, _| menu_labels(w)).unwrap()
}

fn some(labels: &[&str]) -> Vec<Option<String>> {
    labels.iter().map(|l| Some(l.to_string())).collect()
}

#[gpui_kit::test]
fn the_collection_menu_is_split_into_groups(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    open_menu(cx, h.win, "");
    let groups: Vec<Vec<Option<String>>> = labels(cx, h.win)
        .split(Option::is_none)
        .map(<[_]>::to_vec)
        .collect();
    assert_eq!(
        groups,
        [
            some(&["New request", "New GraphQL request", "New folder"]),
            some(&["Paste", "Sort by name"]),
            some(&["Run collection", "Collection settings"]),
            some(&[
                MenuEntry::ExportPostman.label(),
                MenuEntry::ExportPostmanKeepScripts.label(),
                MenuEntry::ExportOpenApi.label(),
                MenuEntry::ExportHtml.label(),
            ]),
            some(&["Show in file manager", "Open in terminal"]),
            some(&["Collapse all", "Expand all"]),
            some(&["Close collection"]),
        ]
    );
}

#[gpui_kit::test]
fn a_request_menu_ends_with_rename_and_delete_apart(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    open_menu(cx, h.win, "health.yml");
    let all = labels(cx, h.win);
    let groups: Vec<_> = all.split(Option::is_none).collect();
    assert_eq!(groups.last().unwrap().to_vec(), some(&["Rename", "Delete"]));
    assert_eq!(
        groups[0].to_vec(),
        some(&["Duplicate", "Copy", "Cut", "Paste"])
    );
}

#[test]
fn delete_entries_are_destructive_and_nothing_else_is() {
    use MenuEntry as E;
    assert!(E::Delete.is_destructive());
    for e in [E::Rename, E::CloneItem, E::Close, E::CloseDraft, E::CutItem] {
        assert!(!e.is_destructive(), "{e:?}");
    }
    assert!(BulkEntry::Delete(Tally::default()).is_destructive());
    assert!(!BulkEntry::Cut(2).is_destructive());
}

#[gpui_kit::test]
fn sidebar_entries_show_their_bound_keys(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    open_menu(cx, h.win, "health.yml");
    let delete = if HOST_OS == gpui_kit::OperatingSystem::Mac {
        "backspace"
    } else {
        "delete"
    };
    for (label, chord) in [
        ("Rename", "f2"),
        ("Delete", delete),
        ("Duplicate", "secondary-d"),
        ("Copy", "secondary-c"),
    ] {
        assert_eq!(row_keys(cx, h.win, label), [shown(chord)], "{label}");
    }
    // Nothing binds these out of the box, so no key is shown.
    for label in ["Show in file manager", "Copy as curl"] {
        assert!(row_keys(cx, h.win, label).is_empty(), "{label}");
    }
}

#[gpui_kit::test]
fn a_rebound_key_shows_in_the_menu(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update(|cx| {
        apinata_app::keymap::apply(
            &overrides(&[
                ("request.rename_in_sidebar", &["f3"]),
                ("request.delete_in_sidebar", &[]),
                ("request.show_in_file_manager", &["alt-f"]),
            ]),
            cx,
        )
    });
    open_menu(cx, h.win, "health.yml");
    assert_eq!(row_keys(cx, h.win, "Rename"), [shown("f3")]);
    assert!(row_keys(cx, h.win, "Delete").is_empty());
    // Show in file manager binds in the Shell, which a sidebar shown alone has not.
    assert!(row_keys(cx, h.win, "Show in file manager").is_empty());
}

#[gpui_kit::test]
fn the_bulk_menu_groups_and_shows_keys(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("health.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("status.yml"));
    frame(cx, h.win);
    open_menu(cx, h.win, "health.yml");
    let all = labels(cx, h.win);
    let groups: Vec<Vec<Option<String>>> = all.split(Option::is_none).map(<[_]>::to_vec).collect();
    assert_eq!(
        groups,
        [
            some(&["Run 2 requests"]),
            some(&["Copy 2 items", "Cut 2 items"]),
            some(&["Delete 2 requests"]),
        ]
    );
    assert_eq!(row_keys(cx, h.win, "Copy 2 items"), [shown("secondary-c")]);
}

#[test]
fn f2_and_delete_are_sidebar_commands() {
    let cmds = registry();
    let find = |id: &str| cmds.iter().find(|c| c.id == id).expect(id);
    let rename = find("request.rename_in_sidebar");
    assert_eq!(rename.scope, Scope::Pane("Sidebar"));
    assert_eq!(rename.default_chords, ["f2"]);
    let delete = find("request.delete_in_sidebar");
    assert_eq!(delete.scope, Scope::Pane("Sidebar"));
    let mut chords = delete.default_chords.clone();
    chords.sort_unstable();
    assert_eq!(chords, ["backspace", "delete"]);
    for c in [rename, delete] {
        let contexts: Vec<&str> = c.targets.iter().map(|t| t.context).collect();
        assert_eq!(contexts, ["Sidebar > Tree"], "{}", c.id);
        assert!(!c.in_palette, "{}", c.id);
    }
}

#[gpui_kit::test]
fn the_tab_menu_groups_and_shows_keys(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    for name in ["a.yml", "b.yml"] {
        open_tab(cx, &s.ws, &s.root, name);
    }
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.right_click(("tab", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    let all = labels(cx, s.win);
    let groups: Vec<Vec<Option<String>>> = all.split(Option::is_none).map(<[_]>::to_vec).collect();
    assert_eq!(groups.len(), 4, "{all:?}");
    assert_eq!(groups[1], some(&["Reveal in sidebar"]));
    assert_eq!(groups[3], some(&["Pin tab"]));
    assert_eq!(row_keys(cx, s.win, "Close"), [shown("secondary-w")]);
}

#[gpui_kit::test]
fn the_switcher_actions_show_their_keys(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, s.win);
    let texts = |cx: &mut TestAppContext, id: &str| {
        let id = gpui_kit::ElementId::Name(id.to_string().into());
        cx.update_window(s.win, |_, w, _| {
            gpui_kit::base::test_support::texts(w)
                .into_iter()
                .filter(|t| t.path().contains(&id))
                .map(|t| t.text().to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap()
    };
    assert!(
        texts(cx, "switcher-open-collection").contains(&shown("secondary-o")),
        "{:?}",
        texts(cx, "switcher-open-collection")
    );
    assert!(
        texts(cx, "switcher-import-collection").contains(&shown("secondary-shift-i")),
        "{:?}",
        texts(cx, "switcher-import-collection")
    );
}
