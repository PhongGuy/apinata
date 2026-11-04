//! The runtime variables dialog: lists a collection's runtime store and clears it.

use crate::support;

use apinata_app::palette::palette_items;
use apinata_app::runtime_vars::open_runtime_vars;
use apinata_core::{EnvOverlay, RuntimeVars};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext as _, Context, IntoElement, Render, TestAppContext, Window, WindowOptions, div,
};
use support::*;

struct Blank;
impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui_kit::test]
fn the_dialog_lists_runtime_variables_and_clears_them(cx: &mut TestAppContext) {
    support::init(cx);
    let vars = RuntimeVars::new();
    vars.set_text("token", "abc");
    vars.set_text("id", "42");
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let handle = vars.clone();
    let view = cx
        .update_window(win, |_, w, cx| {
            open_runtime_vars(handle, EnvOverlay::new(), Vec::new(), Vec::new(), w, cx)
        })
        .unwrap();
    frame(cx, win);
    assert!(
        cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
    assert!(present(cx, win, ("runtime-var", 0usize)) && present(cx, win, ("runtime-var", 1usize)));
    for ix in 0..2usize {
        assert!(
            present(cx, win, ("runtime-var-name", ix))
                && present(cx, win, ("runtime-var-value", ix))
        );
    }
    assert_eq!(
        view.read_with(cx, |v, _| v.rows()),
        [
            ("id".to_owned(), "42".to_owned()),
            ("token".to_owned(), "abc".to_owned())
        ]
    );
    cx.update_window(win, |_, w, cx| w.click("runtime-vars-clear", cx))
        .unwrap();
    frame(cx, win);
    assert!(vars.is_empty());
    assert!(!present(cx, win, ("runtime-var", 0usize)));
}

#[test]
fn the_palette_offers_runtime_variables() {
    assert!(
        palette_items()
            .iter()
            .any(|e| e.label == "Runtime variables")
    );
}

// Opens the dialog over a blank window for `vars`, `overlay` and the secret names `secrets`.
fn dialog(
    cx: &mut TestAppContext,
    vars: &RuntimeVars,
    overlay: &EnvOverlay,
    secrets: &[&str],
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<apinata_app::runtime_vars::RuntimeVarsView>,
) {
    dialog_with_globals(cx, vars, overlay, secrets, &[])
}

// `dialog`, with `global_secrets` as the active global environment's secret names.
fn dialog_with_globals(
    cx: &mut TestAppContext,
    vars: &RuntimeVars,
    overlay: &EnvOverlay,
    secrets: &[&str],
    global_secrets: &[&str],
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<apinata_app::runtime_vars::RuntimeVarsView>,
) {
    support::init(cx);
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let (v, o) = (vars.clone(), overlay.clone());
    let secrets = secrets.iter().map(|s| s.to_string()).collect();
    let globals = global_secrets.iter().map(|s| s.to_string()).collect();
    let view = cx
        .update_window(win, |_, w, cx| {
            open_runtime_vars(v, o, secrets, globals, w, cx)
        })
        .unwrap();
    frame(cx, win);
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    (win, view)
}

// Captured and script rows share one column layout, and a long value wraps inside the
// dialog instead of widening its row.
#[gpui_kit::test]
fn captured_and_script_rows_share_columns_and_long_values_wrap(cx: &mut TestAppContext) {
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    let long = "word ".repeat(80);
    vars.set_text("id", "42");
    vars.set_text("long_value", long.trim_end());
    overlay.set_text(apinata_core::OverlayLayer::Environment, "host", "h");
    let (win, _view) = dialog(cx, &vars, &overlay, &[]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let b = |w: &mut gpui_kit::Window, id: (&'static str, usize)| w.find(id).bounds();
        let (name, value) = (
            b(w, ("runtime-var-name", 0)),
            b(w, ("runtime-var-value", 0)),
        );
        let (s_name, s_value) = (b(w, ("script-var-name", 0)), b(w, ("script-var-value", 0)));
        assert_eq!(name.left(), s_name.left());
        assert_eq!(name.size.width, s_name.size.width, "a fixed name column");
        assert_eq!(value.left(), s_value.left(), "values start in one column");
        let long_value = b(w, ("runtime-var-value", 1));
        let frame = w.find("runtime-vars").bounds();
        assert!(
            long_value.right() <= frame.right() + gpui_kit::px(0.5),
            "{long_value:?} leaves {frame:?}"
        );
    })
    .unwrap();
}

// A captured value under a secret's name is masked until its own row is revealed.
#[gpui_kit::test]
fn a_captured_secret_is_masked_until_revealed(cx: &mut TestAppContext) {
    use apinata_app::env_table::SECRET_HIDDEN;
    let vars = RuntimeVars::new();
    vars.set_text("apikey", "k-123");
    vars.set_text("id", "42");
    let (win, view) = dialog(cx, &vars, &EnvOverlay::new(), &["apikey"]);
    let value = |cx: &mut TestAppContext, ix: usize| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            Some(
                w.find(("runtime-var-value", ix))
                    .value()
                    .unwrap_or("")
                    .to_owned(),
            )
        })
        .unwrap()
    };
    // Rows by name: apikey, id.
    assert_eq!(
        value(cx, 0).as_deref(),
        Some(""),
        "a hidden value is not in the field"
    );
    assert_eq!(value(cx, 1).as_deref(), Some("42"));
    assert_eq!(
        view.read_with(cx, |v, _| v.rows())[0],
        ("apikey".to_owned(), SECRET_HIDDEN.to_owned())
    );
    assert!(!present(cx, win, ("runtime-var-reveal", 1usize)));
    cx.update_window(win, |_, w, cx| w.click(("runtime-var-reveal", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(value(cx, 0).as_deref(), Some("k-123"));
    cx.update_window(win, |_, w, cx| w.click(("runtime-var-hide", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(value(cx, 0).as_deref(), Some(""));
}

// Clear forgets the captures and the script values of the environment shown, and leaves
// another environment's script values for when it is selected again.
#[gpui_kit::test]
fn clear_keeps_another_environments_script_values(cx: &mut TestAppContext) {
    use apinata_core::OverlayLayer;
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("token", "abc");
    overlay.select_environment(Some("prod"));
    overlay.set_text(OverlayLayer::Environment, "host", "prod-host");
    overlay.select_environment(Some("dev"));
    overlay.set_text(OverlayLayer::Environment, "host", "dev-host");
    overlay.set_text(OverlayLayer::Collection, "region", "eu");
    let (win, _view) = dialog(cx, &vars, &overlay, &[]);
    cx.update_window(win, |_, w, cx| w.click("runtime-vars-clear", cx))
        .unwrap();
    frame(cx, win);
    assert!(vars.is_empty() && overlay.is_empty());
    overlay.select_environment(Some("prod"));
    let left: Vec<_> = overlay
        .entries()
        .iter()
        .map(|v| (v.name().to_owned(), v.value().map(str::to_owned)))
        .collect();
    assert_eq!(left, [("host".to_owned(), Some("prod-host".to_owned()))]);
}

#[gpui_kit::test]
fn revealing_a_captured_row_leaves_the_script_row_of_that_name_masked(cx: &mut TestAppContext) {
    use apinata_core::OverlayLayer;
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("apikey", "k-1");
    overlay.set_text(OverlayLayer::Environment, "apikey", "k-2");
    let (win, _view) = dialog(cx, &vars, &overlay, &["apikey"]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("runtime-var-reveal", 0usize), cx);
    })
    .unwrap();
    frame(cx, win);
    let shown = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let l = |w: &mut gpui_kit::Window, id: (&'static str, usize)| {
                Some(w.find(id).value().unwrap_or("").to_owned())
            };
            (
                l(w, ("runtime-var-value", 0)),
                l(w, ("script-var-value", 0)),
            )
        })
        .unwrap();
    assert_eq!(shown, (Some("k-1".to_owned()), Some(String::new())));
}

// A long value takes the bulk of its row, beside a script row's note, and the wrapped
// row's name and Reveal slot sit at its top rather than centred on the wrapped text.
#[gpui_kit::test]
fn a_long_value_takes_the_row_width_and_its_row_top_aligns(cx: &mut TestAppContext) {
    use apinata_core::OverlayLayer;
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("longval", "abcdefghij ".repeat(40).trim_end());
    overlay.set_text(OverlayLayer::Environment, "host", "h");
    let (win, _view) = dialog(cx, &vars, &overlay, &["longval"]);
    cx.update_window(win, |_, w, cx| w.click(("runtime-var-reveal", 0usize), cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let row = w.find(("runtime-var", 0usize)).bounds();
        let name = w.find(("runtime-var-name", 0usize)).bounds();
        let value = w.find(("runtime-var-value", 0usize)).bounds();
        let slot = w.find(("runtime-var-action-slot", 0usize)).bounds();
        assert!(
            value.size.width * 2. >= row.size.width,
            "the value gets the bulk of the row: {value:?} in {row:?}"
        );
        assert_eq!(name.top(), row.top(), "the name sits at the row's top");
        assert_eq!(
            slot.top(),
            row.top(),
            "the Reveal slot sits at the row's top"
        );
        let s_value = w.find(("script-var-value", 0usize)).bounds();
        assert_eq!(value.left(), s_value.left(), "values start in one column");
        assert!(w.find(("script-var-note", 0usize)).bounds().size.width > gpui_kit::px(0.));
    })
    .unwrap();
}

// A masked row's Reveal and Hide are one square eye icon button, in captured and script rows.
#[gpui_kit::test]
fn reveal_and_hide_are_eye_icon_buttons(cx: &mut TestAppContext) {
    use apinata_core::OverlayLayer;
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("apikey", "k-1");
    overlay.set_text(OverlayLayer::Environment, "apikey", "k-2");
    let (win, _view) = dialog(cx, &vars, &overlay, &["apikey"]);
    let look = |cx: &mut TestAppContext, id: (&'static str, usize)| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let e = w.find(id);
            (e.bounds().size, e.label().map(str::to_owned))
        })
        .unwrap()
    };
    // The eye keeps one id through both faces; each face is named by a marker inside it.
    for (eye, reveal, hide) in [
        ("runtime-var-eye", "runtime-var-reveal", "runtime-var-hide"),
        ("script-var-eye", "script-var-reveal", "script-var-hide"),
    ] {
        assert!(present(cx, win, (reveal, 0usize)));
        let (size, label) = look(cx, (eye, 0));
        assert_eq!(
            size.width, size.height,
            "{reveal} is a square icon button: {size:?}"
        );
        assert_eq!(label.as_deref(), Some("Show value"));
        cx.update_window(win, |_, w, cx| w.click((reveal, 0usize), cx))
            .unwrap();
        frame(cx, win);
        assert!(present(cx, win, (hide, 0usize)));
        let (size, label) = look(cx, (eye, 0));
        assert_eq!(
            size.width, size.height,
            "{hide} is a square icon button: {size:?}"
        );
        assert_eq!(label.as_deref(), Some("Hide value"));
    }
}

// Two clicks on the eye button bring a masked row back to its hidden state and its
// original "Show value" button, with no Hide button left behind.
#[gpui_kit::test]
fn the_eye_button_toggles_twice_back_to_hidden(cx: &mut TestAppContext) {
    let vars = RuntimeVars::new();
    vars.set_text("apikey", "k-1");
    let (win, view) = dialog(cx, &vars, &EnvOverlay::new(), &["apikey"]);
    let label = |cx: &mut TestAppContext, id: (&'static str, usize)| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(id).label().map(str::to_owned)
        })
        .unwrap()
    };
    let hidden = view.read_with(cx, |v, _| v.rows());
    cx.update_window(win, |_, w, cx| w.click(("runtime-var-reveal", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert!(present(cx, win, ("runtime-var-hide", 0usize)));
    assert_eq!(
        label(cx, ("runtime-var-eye", 0)).as_deref(),
        Some("Hide value")
    );
    cx.update_window(win, |_, w, cx| w.click(("runtime-var-hide", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert!(present(cx, win, ("runtime-var-reveal", 0usize)));
    assert_eq!(
        label(cx, ("runtime-var-eye", 0)).as_deref(),
        Some("Show value")
    );
    assert!(!present(cx, win, ("runtime-var-hide", 0usize)));
    assert_eq!(view.read_with(cx, |v, _| v.rows()), hidden);
}

// Global script values sit under their own heading, apart from the other script rows, and
// Clear all forgets them too.
#[gpui_kit::test]
fn global_script_values_have_their_own_heading_and_are_cleared(cx: &mut TestAppContext) {
    use apinata_core::OverlayLayer;
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    overlay.set_text(OverlayLayer::Global, "g", "1");
    overlay.set_text(OverlayLayer::Collection, "c", "2");
    let (win, view) = dialog(cx, &vars, &overlay, &[]);
    assert!(present(cx, win, "runtime-vars-global-heading"));
    assert!(present(cx, win, ("global-var-name", 0usize)));
    assert_eq!(
        view.read_with(cx, |v, _| v.global_rows()),
        [("g".to_owned(), "1".to_owned())]
    );
    let names: Vec<String> = view.read_with(cx, |v, _| {
        v.script_rows().into_iter().map(|r| r.0).collect()
    });
    assert_eq!(
        names,
        ["c"],
        "a global value is not repeated among the script rows"
    );
    view.update(cx, |v, cx| v.clear(cx));
    assert!(overlay.is_empty());
    assert!(
        !present(cx, win, "runtime-vars-global-heading"),
        "no heading without global rows"
    );
}

// A global script value named like a secret of the active global environment is masked
// until its row is revealed; a collection row of that name is not a global secret.
#[gpui_kit::test]
fn a_global_script_value_under_a_global_secret_name_is_masked(cx: &mut TestAppContext) {
    use apinata_app::env_table::SECRET_HIDDEN;
    use apinata_core::OverlayLayer;
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    overlay.set_text(OverlayLayer::Global, "gtoken", "g-123");
    overlay.set_text(OverlayLayer::Collection, "gtoken", "c-456");
    let (win, view) = dialog_with_globals(cx, &vars, &overlay, &[], &["gtoken"]);
    let global = |cx: &mut TestAppContext| view.read_with(cx, |v, _| v.global_rows());
    assert_eq!(
        global(cx),
        [("gtoken".to_owned(), SECRET_HIDDEN.to_owned())]
    );
    assert_eq!(view.read_with(cx, |v, _| v.script_rows())[0].1, "c-456");
    cx.update_window(win, |_, w, cx| w.click(("global-var-reveal", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(global(cx), [("gtoken".to_owned(), "g-123".to_owned())]);
}

// What `vars` holds as `(name, value)` pairs, by name.
fn captured(vars: &RuntimeVars) -> Vec<(String, String)> {
    (vars.entries().iter())
        .map(|v| (v.name().to_owned(), v.value().unwrap_or("").to_owned()))
        .collect()
}

// What the overlay holds in `layer` as `(name, value)` pairs.
fn layered(overlay: &EnvOverlay, layer: apinata_core::OverlayLayer) -> Vec<(String, String)> {
    (overlay.entries().iter())
        .filter(|v| v.layer() == layer)
        .map(|v| (v.name().to_owned(), v.value().unwrap_or("").to_owned()))
        .collect()
}

fn pair(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}

fn click_id(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, id: (&'static str, usize)) {
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

fn press_enter(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, win);
}

// Deleting one row of each kind forgets that name only.
#[gpui_kit::test]
fn deleting_a_row_removes_only_that_variable(cx: &mut TestAppContext) {
    use apinata_core::OverlayLayer::{Environment, Global};
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("id", "42");
    vars.set_text("token", "abc");
    overlay.set_text(Environment, "e1", "x");
    overlay.set_text(Environment, "e2", "y");
    overlay.set_text(Global, "g1", "1");
    overlay.set_text(Global, "g2", "2");
    let (win, _view) = dialog(cx, &vars, &overlay, &[]);
    click_id(cx, win, ("runtime-var-delete", 0));
    assert_eq!(captured(&vars), [pair("token", "abc")]);
    click_id(cx, win, ("script-var-delete", 0));
    assert_eq!(layered(&overlay, Environment), [pair("e2", "y")]);
    click_id(cx, win, ("global-var-delete", 0));
    assert_eq!(layered(&overlay, Global), [pair("g2", "2")]);
    assert_eq!(captured(&vars), [pair("token", "abc")]);
    assert!(!present(cx, win, ("runtime-var", 1usize)));
}

// A value edit is saved on Enter in every kind of row.
#[gpui_kit::test]
fn an_edit_is_saved_on_enter(cx: &mut TestAppContext) {
    use apinata_core::OverlayLayer::{Collection, Global};
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("id", "42");
    overlay.set_text(Collection, "region", "eu");
    overlay.set_text(Global, "g", "1");
    let (win, _view) = dialog(cx, &vars, &overlay, &[]);
    for (id, text) in [
        ("runtime-var-value", "43"),
        ("script-var-value", "us"),
        ("global-var-value", "2"),
    ] {
        replace_input(cx, win, None, (id, 0usize), text);
        press_enter(cx, win);
    }
    assert_eq!(captured(&vars), [pair("id", "43")]);
    assert_eq!(layered(&overlay, Collection), [pair("region", "us")]);
    assert_eq!(layered(&overlay, Global), [pair("g", "2")]);
}

// Leaving the field saves too, and an untouched field writes nothing.
#[gpui_kit::test]
fn an_edit_is_saved_on_blur(cx: &mut TestAppContext) {
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("a", "1");
    vars.set_text("b", "2");
    let (win, _view) = dialog(cx, &vars, &overlay, &[]);
    replace_input(cx, win, None, ("runtime-var-value", 0usize), "10");
    // Blur is seen by comparing drawn frames, so the focus change gets a frame.
    frame(cx, win);
    assert_eq!(captured(&vars), [pair("a", "1"), pair("b", "2")]);
    click_id(cx, win, ("runtime-var-value", 1));
    assert_eq!(captured(&vars), [pair("a", "10"), pair("b", "2")]);
}

// The edited value is what the next send puts in the URL.
#[gpui_kit::test]
fn the_next_send_interpolates_the_edited_value(cx: &mut TestAppContext) {
    use wiremock::ResponseTemplate;
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = request("a", &format!("{}/item/{{{{id}}}}", server.uri()));
    let col = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        ),
        ("a.yml", &a),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let vars = ws
        .read_with(cx, |w, cx| w.runtime_vars(&root, cx))
        .expect("open");
    vars.set_text("id", "1");
    let (win, _view) = dialog(cx, &vars, &EnvOverlay::new(), &[]);
    replace_input(cx, win, None, ("runtime-var-value", 0usize), "99");
    press_enter(cx, win);
    let model = sender(cx, &ws);
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    finished(cx, &model);
    assert_eq!(served(cx, &server), ["/item/99"]);
}

// The Add row creates a captured variable on Enter and on its button, and ignores no name.
#[gpui_kit::test]
fn the_add_row_creates_a_captured_variable(cx: &mut TestAppContext) {
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    let (win, view) = dialog(cx, &vars, &overlay, &[]);
    assert!(present(cx, win, "runtime-vars-empty"));
    assert!(present(cx, win, "runtime-vars-add"));
    cx.update_window(win, |_, w, cx| w.click("runtime-vars-add", cx))
        .unwrap();
    frame(cx, win);
    assert!(vars.is_empty(), "an empty name adds nothing");
    replace_input(cx, win, None, "runtime-vars-add-name", "k");
    replace_input(cx, win, None, "runtime-vars-add-value", "v");
    press_enter(cx, win);
    assert_eq!(captured(&vars), [pair("k", "v")]);
    assert_eq!(view.read_with(cx, |v, _| v.rows()), [pair("k", "v")]);
    replace_input(cx, win, None, "runtime-vars-add-name", "k2");
    replace_input(cx, win, None, "runtime-vars-add-value", "v2");
    cx.update_window(win, |_, w, cx| w.click("runtime-vars-add", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(captured(&vars), [pair("k", "v"), pair("k2", "v2")]);
}

// After an add by Enter the add row's Name takes focus, ready for the next variable.
#[gpui_kit::test]
fn enter_in_the_add_value_adds_and_focuses_the_add_name(cx: &mut TestAppContext) {
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    let (win, _view) = dialog(cx, &vars, &overlay, &[]);
    replace_input(cx, win, None, "runtime-vars-add-name", "k");
    cx.update_window(win, |_, w, cx| {
        w.press("tab", cx);
        w.input("v", cx);
    })
    .unwrap();
    frame(cx, win);
    assert!(holds_focus(cx, win, "runtime-vars-add-value"), "Tab moved");
    press_enter(cx, win);
    assert_eq!(captured(&vars), [pair("k", "v")]);
    assert!(
        holds_focus(cx, win, "runtime-vars-add-name"),
        "the add row's name field takes focus"
    );
}

// A masked row edits through an empty field: the value is never put in it, an empty commit
// changes nothing, and typed text replaces the value.
#[gpui_kit::test]
fn a_masked_row_edits_through_an_empty_field(cx: &mut TestAppContext) {
    use apinata_app::env_table::SECRET_HIDDEN;
    let vars = RuntimeVars::new();
    vars.set_text("apikey", "k-123");
    let (win, view) = dialog(cx, &vars, &EnvOverlay::new(), &["apikey"]);
    let field = |cx: &mut TestAppContext| {
        view.read_with(cx, |v, cx| {
            v.captured_input(0).map(|i| i.read(cx).value().to_string())
        })
    };
    assert_eq!(field(cx).as_deref(), Some(""));
    click_id(cx, win, ("runtime-var-value", 0));
    press_enter(cx, win);
    assert_eq!(captured(&vars), [pair("apikey", "k-123")]);
    assert_eq!(
        view.read_with(cx, |v, _| v.rows()),
        [pair("apikey", SECRET_HIDDEN)]
    );
    replace_input(cx, win, None, ("runtime-var-value", 0usize), "k-456");
    press_enter(cx, win);
    assert_eq!(captured(&vars), [pair("apikey", "k-456")]);
    assert_eq!(
        field(cx).as_deref(),
        Some(""),
        "still no value in the field"
    );
    click_id(cx, win, ("runtime-var-reveal", 0));
    assert_eq!(field(cx).as_deref(), Some("k-456"));
}

// A capture made while the dialog is open shows in its row's field, and a field being
// edited keeps its text.
#[gpui_kit::test]
fn fields_follow_the_store_without_clobbering_an_edit(cx: &mut TestAppContext) {
    let vars = RuntimeVars::new();
    vars.set_text("a", "1");
    vars.set_text("b", "2");
    let (win, view) = dialog(cx, &vars, &EnvOverlay::new(), &[]);
    let field = |cx: &mut TestAppContext, ix: usize| {
        view.read_with(cx, |v, cx| {
            v.captured_input(ix).map(|i| i.read(cx).value().to_string())
        })
    };
    replace_input(cx, win, None, ("runtime-var-value", 0usize), "typing");
    vars.set_text("a", "from-store");
    vars.set_text("b", "also");
    view.update(cx, |_, cx| cx.notify());
    frame(cx, win);
    assert_eq!(field(cx, 0).as_deref(), Some("typing"));
    assert_eq!(field(cx, 1).as_deref(), Some("also"));
}

// A value typed into a masked captured row marks the variable sensitive, so the row stays
// masked with its eye, a reopened dialog that no longer knows the secret's name still masks
// it, and a short typed value does not mask other rows that happen to contain it.
#[gpui_kit::test]
fn a_value_typed_into_a_masked_row_stays_masked(cx: &mut TestAppContext) {
    use apinata_app::env_table::SECRET_HIDDEN;
    let vars = RuntimeVars::new();
    vars.set_text("echoed", "old-secret");
    vars.set_text("other", "a1b");
    let (win, view) = dialog(cx, &vars, &EnvOverlay::new(), &["echoed"]);
    replace_input(cx, win, None, ("runtime-var-value", 0usize), "1");
    press_enter(cx, win);
    assert_eq!(captured(&vars), [pair("echoed", "1"), pair("other", "a1b")]);
    assert!(vars.is_sensitive("echoed"));
    assert!(!vars.is_sensitive("other"));
    assert_eq!(
        view.read_with(cx, |v, _| v.rows()),
        [pair("echoed", SECRET_HIDDEN), pair("other", "a1b")],
        "the typed value is hidden and the other row stays plain"
    );
    assert!(present(cx, win, ("runtime-var-reveal", 0usize)));
    let (_, again) = dialog(cx, &vars, &EnvOverlay::new(), &[]);
    assert_eq!(
        again.read_with(cx, |v, _| v.rows()),
        [pair("echoed", SECRET_HIDDEN), pair("other", "a1b")]
    );
}

// The fields rest with a visible edge: the dialog's fill is the field fill, so the edge is
// the only cue in the light themes.
#[gpui_kit::test]
fn the_value_fields_show_their_edge_at_rest(cx: &mut TestAppContext) {
    let vars = RuntimeVars::new();
    vars.set_text("k", "v");
    let (win, _) = dialog(cx, &vars, &EnvOverlay::new(), &[]);
    let visible = cx
        .update_window(win, |_, w, _| {
            w.find(("runtime-var-edge", 0usize)).visible()
        })
        .unwrap();
    assert!(visible);
}

#[gpui_kit::test]
fn enter_on_a_focused_remove_button_deletes_that_variable(cx: &mut TestAppContext) {
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("id", "42");
    vars.set_text("token", "abc");
    let (win, _view) = dialog(cx, &vars, &overlay, &[]);
    cx.update(apinata_app::keymap::init);
    enter_on(cx, win, ("runtime-var-delete", 0usize));
    frame(cx, win);
    assert_eq!(captured(&vars), [pair("token", "abc")]);
}

#[gpui_kit::test]
fn enter_on_the_focused_clear_button_clears_the_store(cx: &mut TestAppContext) {
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("id", "42");
    let (win, _view) = dialog(cx, &vars, &overlay, &[]);
    cx.update(apinata_app::keymap::init);
    enter_on(cx, win, "runtime-vars-clear");
    frame(cx, win);
    assert!(vars.is_empty());
}

// Enter on a focused eye flips its face, and focus stays on the eye through both flips.
#[gpui_kit::test]
fn enter_on_a_focused_eye_keeps_focus_on_the_flipped_eye(cx: &mut TestAppContext) {
    use apinata_app::env_table::SECRET_HIDDEN;
    let vars = RuntimeVars::new();
    vars.set_text("apikey", "k-1");
    let (win, view) = dialog(cx, &vars, &EnvOverlay::new(), &["apikey"]);
    cx.update(apinata_app::keymap::init);
    assert!(present(cx, win, ("runtime-var-reveal", 0usize)));
    enter_on(cx, win, ("runtime-var-eye", 0usize));
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.rows()), [pair("apikey", "k-1")]);
    assert!(present(cx, win, ("runtime-var-hide", 0usize)));
    assert!(
        holds_focus(cx, win, ("runtime-var-eye", 0usize)),
        "the flipped eye holds focus"
    );
    press_enter(cx, win);
    assert_eq!(
        view.read_with(cx, |v, _| v.rows()),
        [pair("apikey", SECRET_HIDDEN)]
    );
    assert!(present(cx, win, ("runtime-var-reveal", 0usize)));
    assert!(
        holds_focus(cx, win, ("runtime-var-eye", 0usize)),
        "focus stays through the flip back"
    );
}

// With the app's keymap, whose button context unbinds Enter, a field's Enter still saves.
#[gpui_kit::test]
fn with_the_app_keymap_enter_in_a_field_still_saves(cx: &mut TestAppContext) {
    let (vars, overlay) = (RuntimeVars::new(), EnvOverlay::new());
    vars.set_text("id", "42");
    let (win, _view) = dialog(cx, &vars, &overlay, &[]);
    cx.update(apinata_app::keymap::init);
    replace_input(cx, win, None, ("runtime-var-value", 0usize), "43");
    press_enter(cx, win);
    assert_eq!(captured(&vars), [pair("id", "43")]);
    replace_input(cx, win, None, "runtime-vars-add-name", "k");
    replace_input(cx, win, None, "runtime-vars-add-value", "v");
    press_enter(cx, win);
    assert_eq!(captured(&vars), [pair("id", "43"), pair("k", "v")]);
}
