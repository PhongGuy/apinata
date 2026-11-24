//! The global editor's list: the colour bar, a row's menu, double-click rename and the "…"
//! menu's Duplicate, as the collection editor's list has them.

use crate::support;

use apinata_app::env_editor::{ENV_SWATCH_NAMES, ENV_SWATCHES};
use apinata_app::global_env_editor::{GlobalEnvEditor, open_global_env_editor};
use apinata_app::workspace::Workspace;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;

const RED: &str = "name: g\ncolor: \"#CE4F3B\"\nvariables:\n  - name: host\n    value: a\n";

struct Blank;
impl gpui_kit::Render for Blank {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

struct Opened {
    _col: tempfile::TempDir,
    dir: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<Workspace>,
    win: AnyWindowHandle,
    editor: Entity<GlobalEnvEditor>,
}

// The global editor over globals `g` (red) and `h`, with `g` selected.
fn opened(cx: &mut TestAppContext) -> Opened {
    support::init(cx);
    let col = collection();
    let dir = write_tree(&[("g.yml", RED), ("h.yml", "name: h\n")]);
    let (state, ws, _root) = open_dir(cx, &col);
    let path = dir.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(path, cx));
    cx.run_until_parked();
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let shared = ws.clone();
    let editor = cx
        .update_window(win, |_, w, cx| open_global_env_editor(shared, w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("g", w, cx))
    })
    .unwrap();
    frame(cx, win);
    Opened {
        _col: col,
        dir,
        _state: state,
        ws,
        win,
        editor,
    }
}

fn selected(cx: &mut TestAppContext, o: &Opened) -> Option<String> {
    o.editor
        .read_with(cx, |e, _| e.selected().map(str::to_owned))
}

// The open name field's value, or None while no name is being collected.
fn name_field(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("global-env-name-input")
            .map(|e| e.value().unwrap_or_default().to_owned())
    })
    .unwrap()
}

// Right-clicks list row `ix` and draws the menu it opens.
fn row_menu(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.right_click(("global-env-item", ix), cx)
    })
    .unwrap();
    frame(cx, win);
}

// Opens the header's "…" menu and draws it.
fn more_menu(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-more", cx)
    })
    .unwrap();
    frame(cx, win);
}

// A coloured entry shows a bar, taller than wide, so it never reads as the round unsaved mark.
#[gpui_kit::test]
fn a_coloured_entry_shows_a_bar_and_the_names_stay_in_line(cx: &mut TestAppContext) {
    let o = opened(cx);
    cx.update_window(o.win, |_, w, cx| {
        w.render_frame(cx);
        let bar = |w: &mut gpui_kit::Window, ix: usize| {
            w.within(("global-env-item", ix))
                .try_find("env-item-color")
                .map(|e| e.bounds())
        };
        let g = bar(w, 0).expect("g is coloured");
        assert!(g.size.height > g.size.width * 2., "a bar, not a dot");
        assert!(bar(w, 1).is_none(), "h has no colour");
        let left = |ix: usize| w.find(("global-env-item-name", ix)).bounds().left();
        assert_eq!(left(0), left(1));
    })
    .unwrap();
}

// Duplicate lives in the "…" menu, after Rename, not in the header.
#[gpui_kit::test]
fn the_menus_duplicate_copies_the_selected_environment(cx: &mut TestAppContext) {
    let o = opened(cx);
    assert!(!present(cx, o.win, "global-env-duplicate"));
    more_menu(cx, o.win);
    let labels = cx.update_window(o.win, |_, w, _| menu_labels(w)).unwrap();
    let want = [
        Some("Rename"),
        Some("Duplicate"),
        Some("Export all to Postman…"),
        None,
        Some("Delete"),
    ];
    assert_eq!(labels, want.map(|l| l.map(str::to_owned)));
    click_menu(cx, o.win, "Duplicate");
    wait_for(cx, "copy selected", |cx| {
        selected(cx, &o).as_deref() == Some("g copy")
    });
    assert!(o.dir.path().join("g copy.yml").is_file());
}

#[gpui_kit::test]
fn a_list_entry_offers_rename_duplicate_color_and_a_separated_delete(cx: &mut TestAppContext) {
    let o = opened(cx);
    row_menu(cx, o.win, 1);
    let labels = cx.update_window(o.win, |_, w, _| menu_labels(w)).unwrap();
    let want = [
        Some("Rename"),
        Some("Duplicate"),
        Some("Color"),
        None,
        Some("Delete"),
    ];
    assert_eq!(labels, want.map(|l| l.map(str::to_owned)));
}

// Each entry acts on the row it was opened on, which it selects first.
#[gpui_kit::test]
fn a_list_entrys_duplicate_copies_that_entry(cx: &mut TestAppContext) {
    let o = opened(cx);
    row_menu(cx, o.win, 1);
    click_menu(cx, o.win, "Duplicate");
    wait_for(cx, "copy selected", |cx| {
        selected(cx, &o).as_deref() == Some("h copy")
    });
    assert!(o.dir.path().join("h copy.yml").is_file());
}

#[gpui_kit::test]
fn a_list_entrys_rename_opens_the_field_on_that_entry(cx: &mut TestAppContext) {
    let o = opened(cx);
    row_menu(cx, o.win, 1);
    click_menu(cx, o.win, "Rename");
    assert_eq!(selected(cx, &o).as_deref(), Some("h"));
    assert_eq!(name_field(cx, o.win).as_deref(), Some("h"));
    cx.simulate_input(o.win, "qa");
    cx.simulate_keystrokes(o.win, "enter");
    wait_for(cx, "renamed", |cx| {
        selected(cx, &o).as_deref() == Some("qa")
    });
    assert!(o.dir.path().join("qa.yml").is_file());
}

#[gpui_kit::test]
fn a_list_entrys_delete_asks_about_that_entry(cx: &mut TestAppContext) {
    let o = opened(cx);
    row_menu(cx, o.win, 1);
    click_menu(cx, o.win, "Delete");
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-delete");
    assert!(!o.dir.path().join("h.yml").exists());
    assert!(o.dir.path().join("g.yml").is_file());
}

// Color opens the swatches as a submenu; a pick colours that entry and saves it.
#[gpui_kit::test]
fn a_list_entrys_color_picks_a_preset_for_that_entry(cx: &mut TestAppContext) {
    let o = opened(cx);
    row_menu(cx, o.win, 1);
    click_menu(cx, o.win, "Color");
    frame(cx, o.win);
    cx.update_window(o.win, |_, w, cx| {
        w.render_frame(cx);
        let sub = w.within("submenu");
        let names: Vec<_> = (0..8usize)
            .filter_map(|ix| sub.try_find(ix).and_then(|e| e.label().map(str::to_owned)))
            .collect();
        let mut want = vec!["No color".to_owned()];
        want.extend(ENV_SWATCH_NAMES.iter().map(|n| n.to_string()));
        assert_eq!(names, want);
        w.within("submenu").click(1usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    let color = |cx: &mut TestAppContext, name: &str| {
        o.ws.read_with(cx, |w, _| {
            w.globals()
                .as_deref()
                .and_then(|g| g.environment(name).and_then(|e| e.color()))
        })
    };
    assert_eq!(color(cx, "h").as_deref(), Some(ENV_SWATCHES[0]));
    assert_eq!(color(cx, "g").as_deref(), Some("#CE4F3B"));
}

// An entry with unsaved edits cannot be renamed or copied, as in the "…" menu.
#[gpui_kit::test]
fn a_dirty_list_entry_offers_rename_and_duplicate_disabled(cx: &mut TestAppContext) {
    let o = opened(cx);
    replace_input(cx, o.win, None, ("env-value", 0usize), "b");
    assert!(o.editor.read_with(cx, |e, cx| e.has_edits(cx)));
    row_menu(cx, o.win, 0);
    assert!(present(cx, o.win, "env-rename-locked"));
    assert!(present(cx, o.win, "env-duplicate-locked"));
    cx.update_window(o.win, |_, w, cx| w.within("popup-menu").click(0usize, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(name_field(cx, o.win), None);
}

#[gpui_kit::test]
fn double_clicking_a_list_entry_renames_it(cx: &mut TestAppContext) {
    let o = opened(cx);
    cx.update_window(o.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("global-env-item", 1usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &o).as_deref(), Some("h"));
    assert_eq!(name_field(cx, o.win), None, "one click selects");
    cx.update_window(o.win, |_, w, cx| {
        w.render_frame(cx);
        w.double_click(("global-env-item", 1usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(name_field(cx, o.win).as_deref(), Some("h"));
}
