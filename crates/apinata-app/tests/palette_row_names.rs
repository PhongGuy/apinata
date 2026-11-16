//! Palette commands that act on the tree's selected row name that row, Rename shows one key on
//! both surfaces, and the settings entry goes by one name in menus, palette and heading.

use crate::support;

use apinata_app::palette::palette_items;
use apinata_app::shortcuts::HOST_OS;
use apinata_app::tabs::TabKind;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Keystroke, SharedString, TestAppContext};

fn collection() -> tempfile::TempDir {
    support::write_tree(&[
        ("opencollection.yml", support::ROOT),
        ("a.yml", &support::request("a", "http://x/a")),
        ("b.yml", &support::request("b", "http://x/b")),
        ("f/folder.yml", &support::folder("F")),
        ("f/c.yml", &support::request("c", "http://x/c")),
    ])
}

// Selects `id` in the shell's sidebar and gives the tree focus, as arrowing to it would.
fn select_in_tree(cx: &mut TestAppContext, s: &support::Started, id: SharedString) {
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&id);
        assert!(ix.is_some(), "{id} is a row");
        t.set_selected_index(ix, cx)
    });
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    support::frame(cx, s.win);
}

fn open_palette(cx: &mut TestAppContext, s: &support::Started) {
    cx.simulate_keystrokes(s.win, &support::key("shift-p"));
    support::frame(cx, s.win);
}

fn palette_ix(label: &str) -> usize {
    (palette_items().iter())
        .position(|e| e.label == label)
        .unwrap_or_else(|| panic!("{label} is a palette entry"))
}

// The row name a palette opened now shows beside `label`, found by typing the label.
fn target_of(cx: &mut TestAppContext, s: &support::Started, label: &str) -> Option<String> {
    open_palette(cx, s);
    cx.update_window(s.win, |_, w, cx| w.input(label, cx))
        .unwrap();
    support::frame(cx, s.win);
    let ix = palette_ix(label);
    let named = cx
        .update_window(s.win, |_, w, _| {
            w.within("command-palette")
                .try_find(("palette-target", ix))
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    cx.simulate_keystrokes(s.win, "escape");
    support::frame(cx, s.win);
    named
}

#[gpui_kit::test]
fn folder_commands_name_the_selected_folder(cx: &mut TestAppContext) {
    let col = collection();
    let s = support::started(cx, &col);
    support::open_tab(cx, &s.ws, &s.root, "a.yml");
    select_in_tree(cx, &s, support::row("f"));
    for label in ["Folder settings", "Run folder", "Rename"] {
        assert_eq!(target_of(cx, &s, label).as_deref(), Some("F"), "{label}");
    }
}

#[gpui_kit::test]
fn collection_commands_name_the_selected_collection(cx: &mut TestAppContext) {
    let col = collection();
    let s = support::started(cx, &col);
    select_in_tree(cx, &s, support::row(""));
    for label in ["Collection settings", "Run collection"] {
        assert_eq!(target_of(cx, &s, label).as_deref(), Some("c"), "{label}");
    }
}

#[gpui_kit::test]
fn draft_commands_name_the_selected_draft(cx: &mut TestAppContext) {
    let col = collection();
    let s = support::started(cx, &col);
    s.ws.update(cx, |w, cx| w.new_draft(&s.root, support::ip(""), cx));
    cx.run_until_parked();
    let draft = support::tab_names(cx, &s.ws)
        .into_iter()
        .find(|t| support::ip(t).is_draft())
        .expect("a draft tab");
    let name = support::view_of(cx, &s.ws, &s.root, &draft).name.clone();
    support::open_tab(cx, &s.ws, &s.root, "a.yml");
    select_in_tree(cx, &s, support::row(&draft));
    for label in ["Save selected draft", "Close selected draft"] {
        assert_eq!(target_of(cx, &s, label).as_deref(), Some(&*name), "{label}");
    }
}

#[gpui_kit::test]
fn a_command_with_no_fitting_row_keeps_its_plain_label(cx: &mut TestAppContext) {
    let col = collection();
    let s = support::started(cx, &col);
    support::open_tab(cx, &s.ws, &s.root, "f/c.yml");
    select_in_tree(cx, &s, support::row("b.yml"));
    for label in ["Run folder", "Collection settings", "Run collection"] {
        assert_eq!(target_of(cx, &s, label), None, "{label}");
    }
}

#[gpui_kit::test]
fn run_folder_from_the_tree_runs_the_selected_folder(cx: &mut TestAppContext) {
    let col = collection();
    let s = support::started(cx, &col);
    support::open_tab(cx, &s.ws, &s.root, "a.yml");
    select_in_tree(cx, &s, support::row("f"));
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(apinata_app::keymap::RunFolder), cx)
    })
    .unwrap();
    support::frame(cx, s.win);
    let kind =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()));
    assert_eq!(kind, Some(TabKind::Runner(support::ip("f"))));
}

// How a key hint reads for `chord`, with `secondary` spelled for the host.
fn chord(chord: &str) -> Keystroke {
    let secondary = if HOST_OS == gpui_kit::OperatingSystem::Mac {
        "cmd"
    } else {
        "ctrl"
    };
    Keystroke::parse_with_os(&chord.replace("secondary", secondary), HOST_OS).unwrap()
}

#[gpui_kit::test]
fn rename_shows_one_key_in_the_palette_and_the_row_menu(cx: &mut TestAppContext) {
    let col = collection();
    let s = support::started(cx, &col);
    select_in_tree(cx, &s, support::row("b.yml"));
    open_palette(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.input("Rename", cx))
        .unwrap();
    support::frame(cx, s.win);
    let ix = palette_ix("Rename");
    let palette_key = cx
        .update_window(s.win, |_, w, _| {
            w.within("command-palette")
                .try_find(("palette-kbd", ix))
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    // The keymap lists Ctrl+R (Cmd+R) for Rename before the tree's F2, so both show it.
    let primary = chord("secondary-r");
    assert_eq!(palette_key, Some(apinata_app::shortcuts::unparse(&primary)));
    cx.simulate_keystrokes(s.win, "escape");
    support::frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.right_click(support::row("b.yml"), cx)
    })
    .unwrap();
    support::frame(cx, s.win);
    let menu_keys = cx
        .update_window(s.win, |_, w, _| support::menu_row_keys(w, "Rename"))
        .unwrap();
    assert_eq!(menu_keys, [Kbd::format(&primary)]);
}

#[gpui_kit::test]
fn the_folder_menu_names_its_settings_folder_settings(cx: &mut TestAppContext) {
    let col = collection();
    let s = support::started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.right_click(support::row("f"), cx)
    })
    .unwrap();
    support::frame(cx, s.win);
    let labels = cx
        .update_window(s.win, |_, w, _| support::menu_labels(w))
        .unwrap();
    assert!(
        labels.contains(&Some("Folder settings".into())),
        "{labels:?}"
    );
    assert!(!labels.contains(&Some("Settings".into())), "{labels:?}");
}
