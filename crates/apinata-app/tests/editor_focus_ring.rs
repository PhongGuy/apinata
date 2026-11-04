//! A focused code editor shows the 2px accent ring by pointer and by key, in all four themes,
//! on every editor kind; an unfocused or disabled one shows none.

use crate::support;

use apinata_app::focus_mode::{Modality, set};
use apinata_app::theme;
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Theme, WindowExt as _};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Hsla, TestAppContext};
use support::*;

const THEMES: [&str; 4] = [theme::DARK, theme::LIGHT, theme::HC_DARK, theme::HC_LIGHT];

/// The accent ring token of the active mode, read from the theme file rather than the live
/// `ring`, which a pointer press has swapped for the input border.
fn ring_token(t: &Theme) -> Hsla {
    let config = if t.mode.is_dark() {
        &t.dark_theme
    } else {
        &t.light_theme
    };
    Hsla::parse_hex(config.colors.ring.as_deref().unwrap()).unwrap()
}

/// (colour, widest border) of quads outlining element `id`, within 6px of its edge.
fn ring_quads(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Vec<(Hsla, f32)> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.find(id).bounds();
        let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
        let (r, bt) = (b.right().as_f32() * scale, b.bottom().as_f32() * scale);
        w.painted_quads()
            .into_iter()
            .filter(|q| {
                let (x, y) = (q.bounds.origin.x.0, q.bounds.origin.y.0);
                q.border_widths.left.0 > 0.
                    && (x - l).abs() < 6. * scale
                    && (y - t).abs() < 6. * scale
                    && (x + q.bounds.size.width.0 - r).abs() < 6. * scale
                    && (y + q.bounds.size.height.0 - bt).abs() < 6. * scale
            })
            .map(|q| (q.border_color, q.border_widths.left.0 / scale))
            .collect()
    })
    .unwrap()
}

/// For every theme and modality: clicking the editor in `id` rings it with the 2px accent
/// token, and blurring it leaves no accent edge.
fn assert_editor_rings(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    for name in THEMES {
        use_theme(cx, name);
        for modality in [Modality::Pointer, Modality::Keyboard] {
            cx.update_window(win, |_, w, cx| w.click_at(id, code_at(), cx))
                .unwrap();
            // The click is a press, so it sets Pointer; restore the case under test.
            cx.update(|cx| set(modality, cx));
            frame(cx, win);
            let ring = cx.update(|cx| ring_token(cx.theme()));
            let seen = ring_quads(cx, win, id);
            assert!(
                seen.iter().any(|(c, w)| *c == ring && *w >= 2.0),
                "{id} {name} {modality:?}: no 2px accent ring in {seen:?}"
            );
            // A blur inside an open dialog comes back to the dialog's first stop, which may be
            // this editor, so there focus moves on to the next stop instead.
            cx.update_window(win, |_, w, cx| {
                if w.has_active_dialog(cx) {
                    w.focus_next(cx);
                } else {
                    w.blur(cx);
                }
            })
            .unwrap();
            frame(cx, win);
            let seen = ring_quads(cx, win, id);
            assert!(
                !seen.iter().any(|(c, _)| *c == ring),
                "{id} {name} {modality:?}: ringed while unfocused in {seen:?}"
            );
        }
    }
}

#[gpui_kit::test]
fn the_body_code_editor_rings_when_focused(cx: &mut TestAppContext) {
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
    assert_editor_rings(cx, win, "body-code");
}

#[gpui_kit::test]
fn the_graphql_editors_ring_when_focused(cx: &mut TestAppContext) {
    use apinata_app::graphql_section::GraphqlSection;
    let (win, _host) = host_section(cx, GraphqlSection::new);
    frame(cx, win);
    assert_editor_rings(cx, win, "graphql-query");
    assert_editor_rings(cx, win, "graphql-variables");
}

#[gpui_kit::test]
fn the_script_editor_rings_when_focused(cx: &mut TestAppContext) {
    use apinata_app::script_section::ScriptSection;
    use apinata_core::ScriptPhase::{PostResponse, PreRequest};
    let (win, _host) = host_section(cx, |w, cx| {
        ScriptSection::new(&[PreRequest, PostResponse], w, cx)
    });
    frame(cx, win);
    assert_editor_rings(cx, win, "script-pre-request-code");
}

#[gpui_kit::test]
fn the_kv_bulk_editor_rings_when_focused(cx: &mut TestAppContext) {
    use apinata_app::kv_table::{KvRow, KvTable};
    use gpui_kit::{ParentElement as _, Styled as _};
    struct H(gpui_kit::Entity<KvTable>);
    impl gpui_kit::Render for H {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            gpui_kit::div().size_full().child(self.0.clone())
        }
    }
    support::init(cx);
    let (win, _h) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
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
    assert_editor_rings(cx, win, "kv-bulk-text");
}

#[gpui_kit::test]
fn the_docs_editor_rings_when_focused(cx: &mut TestAppContext) {
    use apinata_app::settings_section::DocsSection;
    let (win, _host) = host_section(cx, DocsSection::new);
    frame(cx, win);
    assert_editor_rings(cx, win, "docs-code");
}

#[gpui_kit::test]
fn the_curl_import_editor_rings_when_focused(cx: &mut TestAppContext) {
    use apinata_app::shell::ImportCurl;
    use gpui_kit::Focusable as _;
    let col = two_requests();
    let s = started(cx, &col);
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(ImportCurl), cx);
    })
    .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "import-curl-dialog"));
    assert_editor_rings(cx, s.win, "import-curl-input");
}

#[gpui_kit::test]
fn the_generated_code_editor_rings_when_focused(cx: &mut TestAppContext) {
    use gpui_kit::Focusable as _;
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
    assert!(present(cx, s.win, "generate-code-preview"));
    assert_editor_rings(cx, s.win, "generate-code-preview");
}

#[gpui_kit::test]
fn the_response_body_editor_rings_when_focused(cx: &mut TestAppContext) {
    use apinata_app::body_view::{BodyView, Shown};
    let sent = send_once(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_raw("{\"a\":[1,2]}", "application/json"),
        "",
    );
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
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
    assert_editor_rings(cx, win, "body-editor");
}

struct Probe {
    state: gpui_kit::Entity<gpui_kit::component::input::EditorState>,
    disabled: bool,
}

impl gpui_kit::Render for Probe {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::component::input::Editor;
        use gpui_kit::{
            InteractiveElement as _, ParentElement as _, Styled as _, TestSupportExt as _,
        };
        gpui_kit::div().size_full().p_4().child(
            gpui_kit::div()
                .id("probe")
                .test_support()
                .size_full()
                .child(apinata_app::ui::ringed_editor(
                    Editor::new(&self.state).h_full().disabled(self.disabled),
                    &self.state,
                )),
        )
    }
}

// A disabled editor takes no ring even when its state holds focus; an enabled one does.
#[gpui_kit::test]
fn a_disabled_editor_gets_no_ring_and_an_enabled_one_does(cx: &mut TestAppContext) {
    use gpui_kit::Focusable as _;
    use gpui_kit::component::input::EditorState;
    for disabled in [false, true] {
        for modality in [Modality::Pointer, Modality::Keyboard] {
            support::init(cx);
            let (win, view) = cx
                .update(|cx| {
                    gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                        let state = cx.new(|cx| EditorState::new(w, cx));
                        cx.new(|_| Probe { state, disabled })
                    })
                })
                .unwrap();
            frame(cx, win);
            let state = view.read_with(cx, |p, _| p.state.clone());
            cx.update_window(win, |_, w, cx| {
                let handle = state.read(cx).focus_handle(cx);
                w.focus(&handle, cx);
            })
            .unwrap();
            cx.update(|cx| set(modality, cx));
            frame(cx, win);
            let ring = cx.update(|cx| ring_token(cx.theme()));
            let drawn = ring_quads(cx, win, "probe")
                .iter()
                .any(|(c, w)| *c == ring && *w >= 2.0);
            assert_eq!(drawn, !disabled, "disabled = {disabled} {modality:?}");
        }
    }
}

// Tab is the real key path: it moves focus and sets Keyboard modality on its own, so no
// modality is forced once the Tabs start.
#[gpui_kit::test]
fn tabbing_onto_the_body_editor_rings_it(cx: &mut TestAppContext) {
    use apinata_app::request_editor::Section;
    use gpui_kit::Focusable as _;
    let body = "info:\n  name: j\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://127.0.0.1:9/j\"\n  body:\n    type: text\n    data: 'x'\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("j.yml", body)]);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "j.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    let ed = editor_of(cx, &s, tab);
    let code = ed.read_with(cx, |e, cx| e.body().read(cx).code().clone());
    let focused = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            code.read(cx).focus_handle(cx).is_focused(w)
        })
        .unwrap()
    };
    for name in THEMES {
        use_theme(cx, name);
        // Start on the URL, before the editor in tab order, then Tab until focus lands; the
        // bound stops a Tab order that never reaches the editor from looping.
        cx.update_window(s.win, |_, w, cx| w.click("url", cx))
            .unwrap();
        frame(cx, s.win);
        assert!(
            !focused(cx),
            "{name}: the case needs focus before the editor"
        );
        cx.update(|cx| set(Modality::Pointer, cx));
        for _ in 0..40 {
            if focused(cx) {
                break;
            }
            cx.simulate_keystrokes(s.win, "tab");
            frame(cx, s.win);
        }
        assert!(focused(cx), "{name}: Tab never reached the body editor");
        assert!(
            cx.update(|cx| apinata_app::focus_mode::is_keyboard(cx)),
            "{name}: Tab left pointer modality"
        );
        let ring = cx.update(|cx| ring_token(cx.theme()));
        let seen = ring_quads(cx, s.win, "body-code");
        assert!(
            seen.iter().any(|(c, w)| *c == ring && *w >= 2.0),
            "{name}: no 2px accent ring after Tab in {seen:?}"
        );
    }
}
