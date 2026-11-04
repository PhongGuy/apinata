//! A press a few pixels inside the edge of any text input the app draws must reach the text:
//! the caret lands, a drag selects. The kit's input takes mouse-downs only on its text
//! element, so the padding needs the shared wrapper in `ui::padded_input`.
//!
//! Every probe is black box: it finds the input by its element id, seeds text, drops focus,
//! presses 4px in and reads the text back after typing or deleting.

use crate::support;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, InputEvent as _, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, TestAppContext, point, px,
};
use support::*;

const SEED: &str = "1234567890123456";

fn value(cx: &mut TestAppContext, win: AnyWindowHandle, id: &ElementId) -> String {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        // An id without a value is a wrong target, not an empty input.
        w.find(id.clone())
            .value()
            .unwrap_or_else(|| panic!("{id:?}: element has no value"))
            .to_owned()
    })
    .unwrap()
}

/// Types `SEED` into the input and takes focus away, so only a press can bring it back.
fn seed(cx: &mut TestAppContext, win: AnyWindowHandle, id: &ElementId) {
    let id = id.clone();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx);
        w.press(&key("a"), cx);
        w.press("backspace", cx);
        w.input(SEED, cx);
        w.blur(cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn move_to(
    p: gpui_kit::Point<gpui_kit::Pixels>,
    held: Option<MouseButton>,
) -> gpui_kit::PlatformInput {
    MouseMoveEvent {
        position: p,
        pressed_button: held,
        modifiers: Default::default(),
    }
    .to_platform_input()
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, id: &ElementId, dx: f32, drag: bool) {
    let id2 = id.clone();
    let start = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let b = w.find(id2).bounds();
            let p = point(b.left() + px(dx), b.center().y);
            w.dispatch_event(move_to(p, None), cx);
            let down = MouseDownEvent {
                button: MouseButton::Left,
                position: p,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            };
            w.dispatch_event(down.to_platform_input(), cx);
            p
        })
        .unwrap();
    // Each platform event is its own turn, so deferred work from the press runs first.
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        if drag {
            for step in 1..=8 {
                let p = point(start.x + px(12. * step as f32), start.y);
                w.dispatch_event(move_to(p, Some(MouseButton::Left)), cx);
                w.render_frame(cx);
            }
        }
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: start,
            modifiers: Default::default(),
            click_count: 1,
        };
        w.dispatch_event(up.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// A 4px press places the caret at the start: typing then lands in front of the text.
fn assert_press_puts_the_caret_in(cx: &mut TestAppContext, win: AnyWindowHandle, id: ElementId) {
    seed(cx, win, &id);
    assert_eq!(value(cx, win, &id), SEED, "{id:?}: seeding failed");
    press(cx, win, &id, 4., false);
    cx.update_window(win, |_, w, cx| w.input("Z", cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(
        value(cx, win, &id),
        format!("Z{SEED}"),
        "{id:?}: a 4px press did not put the caret in the text"
    );
}

/// A drag from 4px in selects: deleting then removes text.
fn assert_drag_selects_in(cx: &mut TestAppContext, win: AnyWindowHandle, id: ElementId) {
    seed(cx, win, &id);
    press(cx, win, &id, 4., true);
    cx.update_window(win, |_, w, cx| w.press("backspace", cx))
        .unwrap();
    cx.run_until_parked();
    let after = value(cx, win, &id);
    assert!(
        after.len() < SEED.len(),
        "{id:?}: a drag from 4px in selected nothing (text {after:?})"
    );
}

fn both(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    assert_press_puts_the_caret_in(cx, win, id.clone());
    assert_drag_selects_in(cx, win, id);
}

fn shell_with_tab(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    (col, s)
}

fn focus_shell(cx: &mut TestAppContext, s: &Started) {
    use gpui_kit::Focusable as _;
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    cx.update_window(s.win, |_, w, cx| handle.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn the_url_bar_takes_edge_presses(cx: &mut TestAppContext) {
    let (_col, s) = shell_with_tab(cx);
    both(cx, s.win, "url");
}

#[gpui_kit::test]
fn the_search_dialog_find_field_takes_edge_presses(cx: &mut TestAppContext) {
    let (_col, s) = shell_with_tab(cx);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("shift-f"));
    frame(cx, s.win);
    both(cx, s.win, "search-input");
}

#[gpui_kit::test]
fn the_search_dialog_replacement_field_takes_edge_presses(cx: &mut TestAppContext) {
    let (_col, s) = shell_with_tab(cx);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("shift-f"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("search-replace-toggle", cx))
        .unwrap();
    frame(cx, s.win);
    both(cx, s.win, "search-replace-input");
}

#[gpui_kit::test]
fn the_save_dialog_name_field_takes_edge_presses(cx: &mut TestAppContext) {
    let (_col, s) = shell_with_tab(cx);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("s"));
    frame(cx, s.win);
    both(cx, s.win, "save-draft-name");
}

#[gpui_kit::test]
fn the_preferences_inputs_take_edge_presses(cx: &mut TestAppContext) {
    let (_col, s) = shell_with_tab(cx);
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    both(cx, s.win, "zoom");
    both(cx, s.win, "editor-font-size");
}

#[gpui_kit::test]
fn the_settings_timeout_field_takes_edge_presses(cx: &mut TestAppContext) {
    use apinata_app::settings_section::{SettingsSection, TimeoutMode};
    let (win, host) = host_section(cx, SettingsSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.load(&apinata_core::RequestSettings::new(), w, cx);
            s.set_timeout_mode(TimeoutMode::Custom, w, cx)
        })
    })
    .unwrap();
    frame(cx, win);
    both(cx, win, "set-timeout");
}

#[gpui_kit::test]
fn the_folder_tag_field_takes_edge_presses(cx: &mut TestAppContext) {
    use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("users")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (win, ed) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, root, ConfigTarget::Folder(ip("users")), w, cx))
            })
        })
        .unwrap();
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Tags, cx));
    frame(cx, win);
    both(cx, win, "tag-text");
}

#[gpui_kit::test]
fn the_trusted_hosts_field_takes_edge_presses(cx: &mut TestAppContext) {
    use apinata_app::trusted_hosts::TrustedHostsView;
    support::init(cx);
    let (state, coll) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (file, root) = (
        state.path().join("trusted-hosts.json"),
        coll.path().to_path_buf(),
    );
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| TrustedHostsView::new(file, root, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    both(cx, win, "hosts-add");
}

#[gpui_kit::test]
fn the_prompt_dialog_fields_take_edge_presses(cx: &mut TestAppContext) {
    use apinata_app::prompt_vars::{PromptForm, PromptMemory, ask};
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
    support::init(cx);
    let (win, _) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Blank)
            })
        })
        .unwrap();
    let (labels, memory) = (vec!["code".to_owned()], PromptMemory::default());
    let _rx = cx
        .update_window(win, |_, w, cx| {
            let form = cx.new(|cx| PromptForm::new(&labels, &memory, w, cx));
            ask(form, w, cx)
        })
        .unwrap();
    frame(cx, win);
    both(cx, win, "prompt-var-0");
}

#[gpui_kit::test]
fn the_certificate_row_fields_take_edge_presses(cx: &mut TestAppContext) {
    use apinata_app::network_section::NetworkSection;
    use apinata_core::{ClientCertificate, CollectionNetwork};
    let (win, host) = host_section(cx, |w, cx| {
        NetworkSection::new(std::path::PathBuf::from("/c"), w, cx)
    });
    let section = host.read_with(cx, |h, _| h.view.clone());
    let mut network = CollectionNetwork::default();
    network.certificates = vec![ClientCertificate::pem("", "", "")];
    cx.update_window(win, |_, w, cx| {
        section.update(cx, |s, cx| s.load(&network, false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    // The passphrase is masked and exposes no value to read back; it shares the same helper.
    for id in ["cert-domain", "cert-file", "cert-key"] {
        both(cx, win, (id, 0usize));
    }
}

/// The source text with comments and all whitespace removed, so a call split across lines
/// (`Input::\nnew(`) or preceded by a comment still reads as one token run.
fn squashed(text: &str) -> String {
    let code: Vec<&str> = text
        .lines()
        .map(|l| {
            // Only a `//` at the line start or after whitespace opens a comment, so a
            // string like "http://x" keeps the rest of its line.
            let cut = (l.starts_with("//")).then_some(0).or_else(|| l.find(" //"));
            &l[..cut.unwrap_or(l.len())]
        })
        .collect();
    let joined: String = code.concat().split_whitespace().collect();
    joined.replace("crate::ui::", "").replace("ui::", "")
}

/// Every `Input::new(` outside `ui.rs` must sit directly in `padded_input(`, optionally through
/// the lock helpers; every `table_input(` must too, or live in a file whose cells go through
/// `kv_cell` (which forwards padding presses). No input bypasses the padding fix.
#[test]
fn no_source_file_draws_an_unwrapped_input() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    let mut bad = Vec::new();
    for f in files {
        if f.file_name().is_some_and(|n| n == "ui.rs") {
            continue;
        }
        let code = squashed(&std::fs::read_to_string(&f).unwrap());
        let wrapped = |at: usize| {
            let mut before = &code[..at];
            // The lock helpers take the input as their first argument.
            while let Some(rest) = before
                .strip_suffix("lock_input(")
                .or_else(|| before.strip_suffix("readonly_input("))
            {
                before = rest;
            }
            before.ends_with("padded_input(") || before.ends_with("invalid_input(")
        };
        let cells = code.contains("kv_cell(") || code.contains("kv_cell_resting(");
        for (at, _) in code.match_indices("Input::new(") {
            if !code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') && !wrapped(at) {
                bad.push(format!("{}: Input::new at {at}", f.display()));
            }
        }
        for (at, _) in code.match_indices("table_input(") {
            let defined = code[..at].ends_with("fn");
            if !defined && !cells && !wrapped(at) {
                bad.push(format!("{}: table_input at {at}", f.display()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "inputs not wrapped by ui::padded_input: {bad:#?}"
    );
}

#[gpui_kit::test]
fn the_custom_method_field_takes_edge_presses(cx: &mut TestAppContext) {
    let req = request("p", "http://x/p").replace("method: GET", "method: PURGE");
    let col = write_tree(&[("opencollection.yml", ROOT), ("p.yml", &req)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "p.yml");
    frame(cx, s.win);
    // The narrow box scrolls its seeded text, so only the drag is position independent.
    assert_drag_selects_in(cx, s.win, "custom-method".into());
}

/// Presses at `start` with the left button, moves in `step` increments with it held, then
/// releases where the drag ended. Unlike `press`, the start and direction are free.
fn drag_from(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    start: gpui_kit::Point<gpui_kit::Pixels>,
    step: f32,
) {
    cx.update_window(win, |_, w, cx| {
        w.dispatch_event(move_to(start, None), cx);
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: start,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        w.dispatch_event(down.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        let mut end = start;
        for n in 1..=8 {
            end = point(start.x + px(step * n as f32), start.y);
            w.dispatch_event(move_to(end, Some(MouseButton::Left)), cx);
            w.render_frame(cx);
        }
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: end,
            modifiers: Default::default(),
            click_count: 1,
        };
        w.dispatch_event(up.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_drag_from_the_sidebar_filter_icon_selects(cx: &mut TestAppContext) {
    for dx in [4., 8.] {
        let h = open_sidebar(cx);
        let id: ElementId = "sidebar-filter-input".into();
        seed(cx, h.win, &id);
        let seeded = h
            .view
            .read_with(cx, |v, cx| v.filter().read(cx).value().to_string());
        assert_eq!(seeded, SEED, "seeding the filter failed");
        let start = cx
            .update_window(h.win, |_, w, _| {
                let b = w.find("sidebar-filter-input").bounds();
                point(b.left() + px(dx), b.center().y)
            })
            .unwrap();
        drag_from(cx, h.win, start, 12.);
        cx.update_window(h.win, |_, w, cx| w.press("backspace", cx))
            .unwrap();
        cx.run_until_parked();
        let after = h
            .view
            .read_with(cx, |v, cx| v.filter().read(cx).value().to_string());
        assert!(
            after.len() < SEED.len(),
            "a drag from the icon {dx}px in selected nothing (text {after:?})"
        );
    }
}

/// A focused locked field with text, drawn as the app draws one, with its lock icon showing.
struct LockedHost {
    state: gpui_kit::Entity<gpui_kit::component::input::InputState>,
}

impl gpui_kit::Render for LockedHost {
    fn render(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{ParentElement as _, Styled as _};
        gpui_kit::div()
            .w(px(300.))
            .child(apinata_app::ui::padded_input(
                apinata_app::ui::lock_input(
                    gpui_kit::component::input::Input::new(&self.state),
                    &self.state,
                    true,
                    window,
                    cx,
                ),
                &self.state,
            ))
    }
}

#[gpui_kit::test]
fn a_drag_from_the_lock_icon_selects(cx: &mut TestAppContext) {
    use gpui_kit::Focusable as _;
    use gpui_kit::component::input::InputState;
    support::init(cx);
    let (win, host) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                let state = cx.new(|cx| InputState::new(w, cx).default_value(SEED));
                cx.new(|_| LockedHost { state })
            })
        })
        .unwrap();
    frame(cx, win);
    let state = host.read_with(cx, |h, _| h.state.clone());
    cx.update_window(win, |_, w, cx| {
        let handle = state.read(cx).focus_handle(cx);
        w.focus(&handle, cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    let id = state.entity_id();
    for dx in [-4., 0.] {
        let start = cx
            .update_window(win, |_, w, _| {
                let b = w.find(("lock-hint", id)).bounds();
                point(b.center().x + px(dx), b.center().y)
            })
            .unwrap();
        cx.update(|cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("old".into())));
        drag_from(cx, win, start, -40.);
        cx.update_window(win, |_, w, cx| w.press("ctrl-c", cx))
            .unwrap();
        let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
        assert!(
            copied
                .as_deref()
                .is_some_and(|t| t != "old" && !t.is_empty()),
            "a drag from the lock icon (offset {dx}) selected nothing (clipboard {copied:?})"
        );
    }
}

#[gpui_kit::test]
fn a_drag_from_the_shell_sidebar_filter_icon_selects(cx: &mut TestAppContext) {
    let (_col, s) = shell_with_tab(cx);
    let id: ElementId = "sidebar-filter-input".into();
    seed(cx, s.win, &id);
    let start = cx
        .update_window(s.win, |_, w, _| {
            let b = w.find("sidebar-filter-input").bounds();
            point(b.left() + px(4.), b.center().y)
        })
        .unwrap();
    cx.update(|cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("old".into())));
    drag_from(cx, s.win, start, 12.);
    cx.update_window(s.win, |_, w, cx| w.press("ctrl-c", cx))
        .unwrap();
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert!(
        copied
            .as_deref()
            .is_some_and(|t| t != "old" && !t.is_empty()),
        "a drag from the shell filter icon selected nothing (clipboard {copied:?})"
    );
}

/// Down, held moves and up inside one closure, so deferred work from the press has not run
/// when the first move arrives, as when a driver injects events back to back.
fn burst_drag(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    start: gpui_kit::Point<gpui_kit::Pixels>,
) {
    cx.update_window(win, |_, w, cx| {
        w.dispatch_event(move_to(start, None), cx);
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: start,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        w.dispatch_event(down.to_platform_input(), cx);
        let mut end = start;
        for n in 1..=8 {
            end = point(start.x + px(12. * n as f32), start.y);
            w.dispatch_event(move_to(end, Some(MouseButton::Left)), cx);
        }
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: end,
            modifiers: Default::default(),
            click_count: 1,
        };
        w.dispatch_event(up.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_burst_drag_from_the_edges_selects(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let id: ElementId = "sidebar-filter-input".into();
    seed(cx, h.win, &id);
    let start = cx
        .update_window(h.win, |_, w, _| {
            let b = w.find("sidebar-filter-input").bounds();
            point(b.left() + px(4.), b.center().y)
        })
        .unwrap();
    cx.update(|cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("old".into())));
    burst_drag(cx, h.win, start);
    cx.update_window(h.win, |_, w, cx| w.press("ctrl-c", cx))
        .unwrap();
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert!(
        copied
            .as_deref()
            .is_some_and(|t| t != "old" && !t.is_empty()),
        "burst drag from the filter icon selected nothing ({copied:?})"
    );

    let (_col, s) = shell_with_tab(cx);
    seed(cx, s.win, &"url".into());
    let start = cx
        .update_window(s.win, |_, w, _| {
            let b = w.find("url").bounds();
            point(b.left() + px(4.), b.center().y)
        })
        .unwrap();
    cx.update(|cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("old".into())));
    burst_drag(cx, s.win, start);
    cx.update_window(s.win, |_, w, cx| w.press("backspace", cx))
        .unwrap();
    cx.run_until_parked();
    let after = value(cx, s.win, &"url".into());
    assert!(
        after.len() < SEED.len(),
        "burst drag on the url bar selected nothing ({after:?})"
    );
}
