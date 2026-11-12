//! The close button of every dialog that shows one: Enter, Space and a click close it, from
//! the kit's place, under the name "Close", as the last Tab stop.

use crate::support;

use std::cell::Cell;
use std::rc::Rc;

use apinata_app::app::AppState;
use apinata_app::shell::OpenRuntimeVariables;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Focusable as _, ParentElement as _,
    TestAppContext, point, px, size,
};
use support::*;

fn dialog_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

// The close button's element path, from a fresh frame.
fn close_path(cx: &mut TestAppContext, win: AnyWindowHandle) -> Vec<ElementId> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find("close").path().to_vec()
    })
    .unwrap()
}

// The x is the app's own, not the kit's: Enter on the kit's never reaches it.
fn assert_app_x(cx: &mut TestAppContext, win: AnyWindowHandle) {
    let path = close_path(cx, win);
    let kit: ElementId = "dialog-close".into();
    let app: ElementId = "dialog-x".into();
    assert!(!path.contains(&kit), "the kit's close button: {path:?}");
    assert!(path.contains(&app), "the app's close button: {path:?}");
}

// Enter on the focused x closes the dialog `open` shows.
fn enter_closes(cx: &mut TestAppContext, s: &Started, open: impl FnOnce(&mut TestAppContext)) {
    open(cx);
    frame(cx, s.win);
    assert!(dialog_open(cx, s.win));
    assert_app_x(cx, s.win);
    enter_on(cx, s.win, "close");
    frame(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Enter on the close button closes");
}

fn open_prefs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
}

fn zoom(cx: &mut TestAppContext) -> f32 {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().zoom)
}

#[gpui_kit::test]
fn enter_on_the_close_button_closes_preferences_and_keeps_a_typed_field(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert_app_x(cx, s.win);
    // Typed but never confirmed: closing commits it, as the close button always has.
    replace_input(cx, s.win, None, "zoom", "150");
    enter_on(cx, s.win, "close");
    frame(cx, s.win);
    assert!(!dialog_open(cx, s.win), "Enter on the close button closes");
    assert_eq!(zoom(cx), 1.5);
}

#[gpui_kit::test]
fn space_on_the_close_button_closes_preferences(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    for _ in 0..64 {
        if holds_focus(cx, s.win, "close") {
            break;
        }
        press_key(cx, s.win, "tab");
    }
    assert!(holds_focus(cx, s.win, "close"), "Tab reaches close");
    press_key(cx, s.win, "space");
    assert!(!dialog_open(cx, s.win), "Space on the close button closes");
}

#[gpui_kit::test]
fn a_click_on_the_close_button_closes_the_cookie_jar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::cookie_jar::open_cookie_jar(w, cx);
    })
    .unwrap();
    frame(cx, s.win);
    assert_app_x(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("close", cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(
        !dialog_open(cx, s.win),
        "a click on the close button closes"
    );
}

#[gpui_kit::test]
fn enter_on_the_close_button_closes_the_cookie_jar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    enter_closes(cx, &s, |cx| {
        cx.update_window(s.win, |_, w, cx| {
            apinata_app::cookie_jar::open_cookie_jar(w, cx);
        })
        .unwrap();
    });
}

#[gpui_kit::test]
fn enter_on_the_close_button_closes_the_runtime_variables(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    enter_closes(cx, &s, |cx| {
        gpui_kit::VisualTestContext::from_window(s.win, cx).dispatch_action(OpenRuntimeVariables);
    });
}

#[gpui_kit::test]
fn enter_on_the_close_button_closes_the_global_environments(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let ws = s.ws.clone();
    enter_closes(cx, &s, |cx| {
        cx.update_window(s.win, |_, w, cx| {
            apinata_app::global_env_editor::open_global_env_editor(ws, w, cx);
        })
        .unwrap();
    });
}

#[gpui_kit::test]
fn enter_on_the_close_button_closes_the_variables_overview(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    enter_closes(cx, &s, |cx| {
        cx.update_window(s.win, |_, w, cx| {
            apinata_app::variables_overview::open_failed(w, cx);
        })
        .unwrap();
    });
}

#[gpui_kit::test]
fn enter_on_the_close_button_closes_generate_code(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let first = std::fs::read_dir(&s.root)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .find(|n| n.ends_with(".yml") && n != "opencollection.yml")
        .unwrap();
    open_tab(cx, &s.ws, &s.root, &first);
    frame(cx, s.win);
    let shell = s.shell.clone();
    enter_closes(cx, &s, |cx| {
        cx.update_window(s.win, |_, w, cx| {
            shell.read(cx).focus_handle(cx).focus(w, cx)
        })
        .unwrap();
        cx.simulate_keystrokes(s.win, &key("shift-p"));
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, cx| w.input("generate code", cx))
            .unwrap();
        frame(cx, s.win);
        cx.simulate_keystrokes(s.win, "enter");
        cx.run_until_parked();
        assert!(cx.update(|cx| apinata_app::generate_code::current(cx).is_some()));
    });
}

// The x sits where the kit drew its own and is announced as "Close", at the default zoom.
#[gpui_kit::test]
fn the_close_button_keeps_the_kit_place_and_name(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let x = w.find("close");
        assert_eq!(x.label(), Some("Close"));
        // Measured on the kit's own x before it was replaced.
        let kit = Bounds::new(point(px(1107.5), px(117.)), size(px(19.5), px(19.5)));
        assert_eq!(x.bounds(), kit);
    })
    .unwrap();
    assert_app_x(cx, s.win);
}

// At 150% the x still sits where the kit drew its own, at the kit's zoomed size.
#[gpui_kit::test]
fn the_close_button_keeps_the_kit_place_when_zoomed(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |s| s.zoom = 1.5));
    });
    cx.run_until_parked();
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let x = w.find("close");
        // Measured on the kit's own x at 150% before it was replaced.
        let kit = Bounds::new(point(px(1282.), px(117.)), size(px(29.), px(29.)));
        assert_eq!(x.bounds(), kit);
    })
    .unwrap();
    assert_app_x(cx, s.win);
}

// The kit clips the panel's content flush with the title's top, so the x needs ring room
// above it inside the title, at 100% and at 150%.
#[gpui_kit::test]
fn the_close_button_keeps_its_ring_inside_the_title(cx: &mut TestAppContext) {
    use apinata_app::style::FOCUS_RING_ROOM;
    let col = two_requests();
    let s = started(cx, &col);
    for zoom in [1.0, 1.5] {
        cx.update(|cx| {
            let settings = cx.global::<AppState>().settings.clone();
            settings.update(cx, |p, cx| p.update(cx, |s| s.zoom = zoom));
        });
        cx.run_until_parked();
        open_prefs(cx, &s);
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let x = w.find("close").bounds();
            let title = w.find("dialog-title").bounds();
            assert!(
                x.top() - FOCUS_RING_ROOM >= title.top(),
                "{zoom}: {x:?} in {title:?}"
            );
            w.close_dialog(cx);
        })
        .unwrap();
        frame(cx, s.win);
    }
}

// Tab reaches the x last, after every control, and the next Tab wraps to the search box.
#[gpui_kit::test]
fn the_close_button_is_the_last_tab_stop(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let mut tabs = None;
    for n in 1..80 {
        press_key(cx, s.win, "tab");
        if holds_focus(cx, s.win, "close") {
            tabs = Some(n);
            break;
        }
    }
    // Measured with the kit's own x before it was replaced, plus the shortcut list's one stop.
    assert_eq!(tabs, Some(41));
    press_key(cx, s.win, "tab");
    assert!(holds_focus(cx, s.win, "prefs-search"));
    assert_app_x(cx, s.win);
}

// A dialog whose cancel refuses to close keeps refusing when Enter lands on its x.
#[gpui_kit::test]
fn enter_on_the_close_button_runs_the_cancel_veto(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let asked = Rc::new(Cell::new(0));
    let seen = asked.clone();
    cx.update_window(s.win, |_, w, cx| {
        w.open_dialog(cx, move |d, _, _| {
            let seen = seen.clone();
            apinata_app::ui::with_close(d, "Veto")
                .on_ok(|_, _, _| true)
                .on_cancel(move |_, _, _| {
                    seen.set(seen.get() + 1);
                    false
                })
                .child(gpui_kit::div().child("body"))
        });
    })
    .unwrap();
    frame(cx, s.win);
    assert_app_x(cx, s.win);
    enter_on(cx, s.win, "close");
    frame(cx, s.win);
    assert_eq!(asked.get(), 1, "Enter on the x asks the dialog's cancel");
    assert!(dialog_open(cx, s.win), "the veto keeps it open");
}

// Every dialog in the app turns the kit's close button off: one that wants an x takes the
// app's, which answers Enter, through `ui::with_close`. Each `open_dialog` call is checked on
// its own, with comments stripped, so a second dialog in a file cannot cover for a first.
#[test]
fn no_dialog_shows_the_kit_close_button() {
    let mut bad = Vec::new();
    walk_sources(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut bad,
    );
    assert!(bad.is_empty(), "dialogs that keep the kit's x: {bad:#?}");
}

fn walk_sources(dir: &std::path::Path, bad: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk_sources(&path, bad);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for line in kit_x_sites(&text) {
                bad.push(format!("{}:{line}", path.display()));
            }
        }
    }
}

// The 1-based lines of the `.open_dialog(` calls in `text` whose argument list, comments
// removed, neither turns the kit's x off nor goes through `with_close`.
fn kit_x_sites(text: &str) -> Vec<usize> {
    let code: String = text
        .lines()
        .map(|l| l.split_once("//").map_or(l, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n");
    let mut sites = Vec::new();
    let mut from = 0;
    while let Some(at) = code[from..].find(".open_dialog(") {
        let open = from + at + ".open_dialog".len();
        // The call runs to the parenthesis that balances its opening one.
        let mut depth = 0usize;
        let mut end = code.len();
        for (i, c) in code[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let call = &code[open..end];
        if !call.contains(".close_button(false)") && !call.contains("with_close(") {
            sites.push(code[..open].matches('\n').count() + 1);
        }
        from = end;
    }
    sites
}

// The scan itself: a forgotten site is caught even when another dialog in the file is fine
// and a comment names the cure.
#[test]
fn the_scan_finds_a_forgotten_site_beside_a_good_one() {
    let text = "fn a() {\n    w.open_dialog(cx, |d, _, _| d.close_button(false));\n}\n\
                fn b() {\n    // .close_button(false) would hide it\n    \
                w.open_dialog(cx, |d, _, _| d.title(\"x\"));\n}\n";
    assert_eq!(kit_x_sites(text), [6]);
}
