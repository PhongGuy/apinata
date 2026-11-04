//! No code editor is a keyboard trap: Tab reaches every editor, an editable one keeps Tab for
//! indentation, Ctrl+Tab and Ctrl+Shift+Tab leave any of them, and a read-only one passes Tab on.

use crate::support;

use std::collections::BTreeSet;

use apinata_app::commands::{places, registry};
use apinata_app::shortcuts::nested_in;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::EditorState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, FocusHandle, Focusable as _};
use gpui_kit::{TestAppContext, WindowOptions};
use support::*;

fn focused(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<FocusHandle> {
    cx.update_window(win, |_, w, cx| w.focused(cx)).unwrap()
}

fn focus(cx: &mut TestAppContext, win: AnyWindowHandle, h: &FocusHandle) {
    cx.update_window(win, |_, w, cx| w.focus(h, cx)).unwrap();
    frame(cx, win);
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    frame(cx, win);
}

fn handle_of(cx: &mut TestAppContext, state: &Entity<EditorState>) -> FocusHandle {
    cx.update(|cx| state.read(cx).focus_handle(cx))
}

/// The controls window traversal puts before and after the editor, found by stepping from it.
fn neighbours(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ed: &FocusHandle,
) -> (FocusHandle, FocusHandle) {
    let step = |cx: &mut TestAppContext, forward: bool| {
        focus(cx, win, ed);
        cx.update_window(win, |_, w, cx| {
            if forward {
                w.focus_next(cx)
            } else {
                w.focus_prev(cx)
            }
        })
        .unwrap();
        frame(cx, win);
        focused(cx, win).expect("a control before and after the editor")
    };
    let (prev, next) = (step(cx, false), step(cx, true));
    assert!(&prev != ed && &next != ed, "the editor has neighbours");
    (prev, next)
}

/// The registry places around the focused editor, each of which the place table must declare
/// as one a code editor sits within; panics when it does not.
fn places_around(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ed: &FocusHandle,
) -> BTreeSet<&'static str> {
    focus(cx, win, ed);
    let stack = cx.update_window(win, |_, w, _| w.context_stack()).unwrap();
    let code = apinata_app::ui::CODE_EDITOR_CONTEXT;
    assert!(stack.iter().any(|k| k.contains(code)), "{stack:?}");
    let around: BTreeSet<&str> = places()
        .iter()
        .map(|p| p.context)
        .filter(|c| *c != code && stack.iter().any(|k| k.contains(c)))
        .collect();
    for c in &around {
        assert!(nested_in(places(), code, c), "{code} sits within {c}");
    }
    around
}

/// `enter` (Tab, or Ctrl+Tab after another code editor) from the control before the editor
/// lands in it, and Ctrl+Tab and Ctrl+Shift+Tab leave it for the controls after and before it.
fn assert_entered_and_left(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ed: &FocusHandle,
    enter: &str,
) {
    places_around(cx, win, ed);
    let (prev, next) = neighbours(cx, win, ed);
    focus(cx, win, &prev);
    press(cx, win, enter);
    assert_eq!(
        focused(cx, win).as_ref(),
        Some(ed),
        "{enter} reaches the editor"
    );
    press(cx, win, "ctrl-tab");
    assert_eq!(focused(cx, win), Some(next), "Ctrl+Tab leaves forward");
    focus(cx, win, ed);
    press(cx, win, "ctrl-shift-tab");
    assert_eq!(focused(cx, win), Some(prev), "Ctrl+Shift+Tab leaves back");
}

/// Plain Tab indents an editable editor and Shift+Tab outdents it, both keeping focus.
fn assert_tab_indents(cx: &mut TestAppContext, win: AnyWindowHandle, state: &Entity<EditorState>) {
    let ed = handle_of(cx, state);
    cx.update_window(win, |_, w, cx| {
        state.update(cx, |s, cx| s.set_value("a", w, cx))
    })
    .unwrap();
    focus(cx, win, &ed);
    press(cx, win, "tab");
    let text = cx.update(|cx| state.read(cx).value().to_string());
    assert!(
        text.len() > 1 && text.contains('a'),
        "Tab indents: {text:?}"
    );
    assert_eq!(focused(cx, win), Some(ed.clone()), "Tab keeps focus");
    press(cx, win, "shift-tab");
    assert_eq!(cx.update(|cx| state.read(cx).value().to_string()), "a");
    assert_eq!(focused(cx, win), Some(ed), "Shift+Tab keeps focus");
}

/// A read-only editor passes plain Tab and Shift+Tab on to window traversal.
fn assert_tab_passes(cx: &mut TestAppContext, win: AnyWindowHandle, ed: &FocusHandle) {
    let (prev, next) = neighbours(cx, win, ed);
    focus(cx, win, ed);
    press(cx, win, "tab");
    assert_eq!(focused(cx, win), Some(next), "Tab leaves forward");
    focus(cx, win, ed);
    press(cx, win, "shift-tab");
    assert_eq!(focused(cx, win), Some(prev), "Shift+Tab leaves back");
}

fn assert_reached_and_left(cx: &mut TestAppContext, win: AnyWindowHandle, ed: &FocusHandle) {
    assert_entered_and_left(cx, win, ed, "tab");
}

fn editable(cx: &mut TestAppContext, win: AnyWindowHandle, state: &Entity<EditorState>) {
    editable_after(cx, win, state, "tab");
}

fn editable_after(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    state: &Entity<EditorState>,
    enter: &str,
) {
    cx.update(apinata_app::keymap::init);
    let ed = handle_of(cx, state);
    assert_entered_and_left(cx, win, &ed, enter);
    assert_tab_indents(cx, win, state);
}

#[gpui_kit::test]
fn the_body_editor_is_reached_indents_and_is_left(cx: &mut TestAppContext) {
    use apinata_app::body_section::BodySection;
    let (win, host) = host_section(cx, |w, cx| {
        BodySection::new(std::path::PathBuf::from("/c"), w, cx)
    });
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.load(&apinata_core::BodySpec::Text("a".into()), w, cx)
        })
    })
    .unwrap();
    frame(cx, win);
    let state = s.read_with(cx, |s, _| s.code().clone());
    editable(cx, win, &state);
}

#[gpui_kit::test]
fn the_graphql_editors_are_reached_indent_and_are_left(cx: &mut TestAppContext) {
    use apinata_app::graphql_section::GraphqlSection;
    let (win, host) = host_section(cx, GraphqlSection::new);
    frame(cx, win);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let query = s.read_with(cx, |s, _| s.query().clone());
    let variables = s.read_with(cx, |s, _| s.variables().clone());
    editable(cx, win, &query);
    // The query editor comes right before, and Tab indents there, so Ctrl+Tab steps over.
    editable_after(cx, win, &variables, "ctrl-tab");
}

#[gpui_kit::test]
fn the_script_editor_is_reached_indents_and_is_left(cx: &mut TestAppContext) {
    use apinata_app::script_section::ScriptSection;
    use apinata_core::ScriptPhase::{PostResponse, PreRequest};
    let (win, host) = host_section(cx, |w, cx| {
        ScriptSection::new(&[PreRequest, PostResponse], w, cx)
    });
    frame(cx, win);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let state = s.read_with(cx, |s, _| s.code(PreRequest).unwrap().clone());
    editable(cx, win, &state);
}

#[gpui_kit::test]
fn the_docs_editor_is_reached_indents_and_is_left(cx: &mut TestAppContext) {
    use apinata_app::settings_section::DocsSection;
    let (win, host) = host_section(cx, DocsSection::new);
    frame(cx, win);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let state = s.read_with(cx, |s, _| s.code().clone());
    editable(cx, win, &state);
}

// The reported case: choosing Edit and then tabbing on must land in the editor.
#[gpui_kit::test]
fn tab_from_the_docs_edit_segment_reaches_the_editor(cx: &mut TestAppContext) {
    use apinata_app::settings_section::{DocsMode, DocsSection};
    let (win, host) = host_section(cx, DocsSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load("notes", w, cx)))
        .unwrap();
    frame(cx, win);
    assert_eq!(s.read_with(cx, |s, _| s.mode()), DocsMode::Preview);
    cx.update_window(win, |_, w, cx| w.within("docs-mode").click(1usize, cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(s.read_with(cx, |s, _| s.mode()), DocsMode::Edit);
    let ed = cx.update(|cx| s.read(cx).code().read(cx).focus_handle(cx));
    let segment = cx
        .update_window(win, |_, w, cx| {
            w.within("docs-mode").click(1usize, cx);
            w.focus_prev(cx);
            w.focused(cx)
        })
        .unwrap()
        .expect("the Edit segment takes focus");
    focus(cx, win, &segment);
    press(cx, win, "tab");
    assert_eq!(
        focused(cx, win),
        Some(ed),
        "Tab from Edit reaches the editor"
    );
}

// The same in the real app: the request's Docs section, Edit chosen, then Tab from Edit.
#[gpui_kit::test]
fn in_a_request_tab_from_the_docs_edit_segment_reaches_the_editor(cx: &mut TestAppContext) {
    use apinata_app::request_editor::Section;
    let body = format!("{}docs: notes\n", request("a", "http://127.0.0.1:9/a"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &body)]);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Docs as usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("docs-mode").click(1usize, cx))
        .unwrap();
    frame(cx, s.win);
    let docs = editor_of(cx, &s, tab).read_with(cx, |e, _| e.docs().clone());
    let state = docs.read_with(cx, |d, _| d.code().clone());
    let ed = handle_of(cx, &state);
    assert_eq!(
        focused(cx, s.win),
        Some(ed.clone()),
        "Edit focuses the editor"
    );
    // Shift+Tab outdents in place, so step back with traversal to reach the Edit segment.
    cx.update_window(s.win, |_, w, cx| w.focus_prev(cx))
        .unwrap();
    frame(cx, s.win);
    assert_ne!(focused(cx, s.win), Some(ed.clone()));
    press(cx, s.win, "tab");
    assert_eq!(focused(cx, s.win), Some(ed), "Tab reaches the docs editor");
}

// Collection settings host the same docs section; Tab from the control before it lands in it.
#[gpui_kit::test]
fn tab_reaches_the_collection_docs_editor(cx: &mut TestAppContext) {
    use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let s = started(cx, &col);
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, root, ConfigTarget::Collection, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    view.update(cx, |v, cx| v.set_section(ConfigSection::Docs, cx));
    frame(cx, win);
    let docs = view.read_with(cx, |v, _| v.docs().clone());
    let mode = docs.read_with(cx, |d, _| d.mode());
    let state = docs.read_with(cx, |d, _| d.code().clone());
    let ed = handle_of(cx, &state);
    assert_eq!(mode, apinata_app::settings_section::DocsMode::Edit);
    let (prev, _) = neighbours(cx, win, &ed);
    focus(cx, win, &prev);
    press(cx, win, "tab");
    assert_eq!(
        focused(cx, win),
        Some(ed.clone()),
        "Tab reaches the docs editor"
    );
    places_around(cx, win, &ed);
    std::mem::forget(col);
}

#[gpui_kit::test]
fn the_kv_bulk_editor_is_reached_indents_and_is_left(cx: &mut TestAppContext) {
    use apinata_app::kv_table::{KvRow, KvTable};
    use gpui_kit::{ParentElement as _, Styled as _};
    struct H(Entity<KvTable>);
    impl gpui_kit::Render for H {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            // Buttons on both sides give the editor neighbours to leave for.
            use gpui_kit::component::button::Button;
            gpui_kit::div()
                .size_full()
                .child(Button::new("before").label("Before"))
                .child(self.0.clone())
                .child(Button::new("after").label("After"))
        }
    }
    support::init(cx);
    let (win, h) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                let t = cx.new(|cx| KvTable::new(w, cx));
                let row = KvRow {
                    name: "a".into(),
                    value: "1".into(),
                    enabled: true,
                    ..KvRow::default()
                };
                t.update(cx, |t, cx| t.load(vec![row], w, cx));
                cx.new(|_| H(t))
            })
        })
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("kv-bulk-toggle", cx))
        .unwrap();
    frame(cx, win);
    let table = h.read_with(cx, |h, _| h.0.clone());
    let state = table.read_with(cx, |t, _| t.bulk_editor().unwrap().clone());
    editable(cx, win, &state);
}

// A response body showing JSON in its editor, in a window of its own.
fn response_editor(cx: &mut TestAppContext) -> (AnyWindowHandle, FocusHandle, Sent) {
    use apinata_app::body_view::{BodyView, Shown};
    let sent = send_once(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_raw("{\"a\":[1,2]}", "application/json"),
        "",
    );
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = sent.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    let state = view.read_with(cx, |v, _| v.editor().clone());
    let ed = handle_of(cx, &state);
    (win, ed, sent)
}

#[gpui_kit::test]
fn the_response_body_editor_passes_tab_and_is_left(cx: &mut TestAppContext) {
    let (win, ed, _sent) = response_editor(cx);
    assert_reached_and_left(cx, win, &ed);
    assert_tab_passes(cx, win, &ed);
}

// Every place the table says a code editor sits within is one a real host puts it in.
#[gpui_kit::test]
fn the_code_editor_places_are_the_ones_its_hosts_show(cx: &mut TestAppContext) {
    let code = apinata_app::ui::CODE_EDITOR_CONTEXT;
    let declared: BTreeSet<&str> = places()
        .iter()
        .filter(|p| p.context == code)
        .flat_map(|p| p.within.iter().copied())
        .collect();
    let (win, ed, _sent) = response_editor(cx);
    assert_eq!(places_around(cx, win, &ed), declared);
}

#[gpui_kit::test]
fn the_generated_code_editor_passes_tab_and_is_left(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x.test/ok")),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let shell = s.shell.clone();
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
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click_at("generate-code-preview", code_at(), cx)
    })
    .unwrap();
    frame(cx, s.win);
    let ed = focused(cx, s.win).expect("the preview takes focus on a click");
    assert_reached_and_left(cx, s.win, &ed);
    assert_tab_passes(cx, s.win, &ed);
}

fn curl_dialog(cx: &mut TestAppContext) -> (Started, FocusHandle) {
    use apinata_app::shell::ImportCurl;
    let col = two_requests();
    let s = started(cx, &col);
    std::mem::forget(col);
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(ImportCurl), cx);
    })
    .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "import-curl-dialog"));
    let opened = focused(cx, s.win).expect("the dialog opens with focus");
    // A click in the editor focuses it; the same handle shows the editor had focus already.
    cx.update_window(s.win, |_, w, cx| {
        w.click_at("import-curl-input", code_at(), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(
        focused(cx, s.win),
        Some(opened.clone()),
        "editor autofocused"
    );
    (s, opened)
}

fn on(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).focused()
    })
    .unwrap()
        == Some(true)
}

#[gpui_kit::test]
fn the_curl_editor_indents_and_ctrl_tab_reaches_the_buttons(cx: &mut TestAppContext) {
    let (s, ed) = curl_dialog(cx);
    places_around(cx, s.win, &ed);
    press(cx, s.win, "tab");
    assert_eq!(focused(cx, s.win), Some(ed.clone()), "Tab indents in place");
    press(cx, s.win, "ctrl-tab");
    assert!(
        on(cx, s.win, "import-curl-import"),
        "Ctrl+Tab reaches Import"
    );
    press(cx, s.win, "shift-tab");
    assert!(
        on(cx, s.win, "import-curl-cancel"),
        "Shift+Tab reaches Cancel"
    );
    press(cx, s.win, "tab");
    assert!(on(cx, s.win, "import-curl-import"), "Tab reaches Import");
    // Tab keeps cycling inside the dialog and back to the editor.
    for _ in 0..6 {
        if focused(cx, s.win).as_ref() == Some(&ed) {
            break;
        }
        press(cx, s.win, "tab");
        let inside = cx
            .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap();
        assert!(inside, "the dialog stays open");
    }
    assert_eq!(focused(cx, s.win), Some(ed.clone()), "Tab cycles back");
    press(cx, s.win, "ctrl-shift-tab");
    assert!(
        on(cx, s.win, "import-curl-import"),
        "Ctrl+Shift+Tab wraps to Import"
    );
}

#[gpui_kit::test]
fn escape_in_the_curl_editor_closes_the_dialog(cx: &mut TestAppContext) {
    let (s, _) = curl_dialog(cx);
    press(cx, s.win, "escape");
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(!open, "Esc closes the dialog");
}

#[gpui_kit::test]
fn both_exit_commands_are_listed_with_their_keys(cx: &mut TestAppContext) {
    let ids = ["view.focus_next_control", "view.focus_previous_control"];
    for id in ids {
        assert!(registry().iter().any(|c| c.id == id), "{id} is a command");
    }
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert_eq!(key_label(cx, &s, ids[0]), Some("ctrl-tab".into()));
    assert_eq!(key_label(cx, &s, ids[1]), Some("ctrl-shift-tab".into()));
}

// A click in the curl editor, then `key`: focus leaves it and the keyboard ring shows at once.
fn assert_exit_key_shows_the_ring(cx: &mut TestAppContext, key: &str) {
    use apinata_app::focus_mode;
    let (s, ed) = curl_dialog(cx);
    assert!(
        !cx.update(|cx| focus_mode::is_keyboard(cx)),
        "pointer first"
    );
    press(cx, s.win, key);
    assert_ne!(focused(cx, s.win), Some(ed), "{key} moves focus");
    assert!(
        cx.update(|cx| focus_mode::is_keyboard(cx)),
        "{key} counts as keyboard navigation"
    );
}

#[gpui_kit::test]
fn ctrl_tab_after_a_click_shows_the_focus_ring(cx: &mut TestAppContext) {
    assert_exit_key_shows_the_ring(cx, "ctrl-tab");
}

#[gpui_kit::test]
fn ctrl_shift_tab_after_a_click_shows_the_focus_ring(cx: &mut TestAppContext) {
    assert_exit_key_shows_the_ring(cx, "ctrl-shift-tab");
}

// A rebound exit key leaves the editor and the default one no longer does.
#[gpui_kit::test]
fn a_rebound_exit_key_leaves_and_ctrl_tab_stays(cx: &mut TestAppContext) {
    let (s, ed) = curl_dialog(cx);
    edit_keys(cx, |o| {
        o.insert("view.focus_next_control".into(), vec!["ctrl-alt-j".into()]);
    });
    focus(cx, s.win, &ed);
    press(cx, s.win, "ctrl-alt-j");
    assert!(
        on(cx, s.win, "import-curl-import"),
        "the new key reaches Import"
    );
    focus(cx, s.win, &ed);
    press(cx, s.win, "ctrl-tab");
    assert_eq!(focused(cx, s.win), Some(ed), "Ctrl+Tab stays in the editor");
}
