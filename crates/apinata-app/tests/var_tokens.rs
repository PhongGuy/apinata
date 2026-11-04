use crate::support;

use std::ops::Range;
use std::sync::Arc;

use apinata_app::runtime_vars::overlay_note;
use apinata_app::theme::VarKind;
use apinata_app::var_tokens::{SECRET_DOTS, VarField, scope_label, tokens, tooltip};
use apinata_core::{Collection, ContextOptions, OverlayLayer, VarContext, VarScope, VariableSet};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Context, Entity, InputEvent as _, IntoElement,
    Modifiers, MouseMoveEvent, ParentElement as _, Pixels, Point, Render, Styled as _,
    TestAppContext, Window, WindowOptions, div, open_window, point, px,
};
use support::*;

const VARS_ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: host\n      value: c.test\n    - name: empty\n      value: \"\"\n";
const DEV: &str = "name: dev\nvariables:\n  - name: region\n    value: eu\n";
const TEXT: &str = "https://{{host}}/{{nope}}/{{$guid}}/{{token}}";
const HOST: Range<usize> = 8..16;
const NOPE: Range<usize> = 17..25;
const GUID: Range<usize> = 26..35;
const TOKEN: Range<usize> = 36..45;

/// The variables `a.yml` sees: collection `host` and `empty`, `dev`'s `region`, and a
/// secret `token` override; no process environment.
fn vars() -> (tempfile::TempDir, Arc<VariableSet>) {
    let dir = write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("environments/dev.yml", DEV),
        ("a.yml", &request("a", TEXT)),
    ]);
    let c = Collection::open(dir.path()).unwrap();
    let options = ContextOptions::new()
        .env("dev")
        .sensitive_env_var("token", "s3cret")
        .process_env(Vec::<(String, String)>::new());
    let ctx = VarContext::new(&c, options).unwrap();
    let set = ctx.variables(c.request(&ip("a.yml")).unwrap());
    (dir, Arc::new(set))
}

#[test]
fn tokens_are_classified_and_explained() {
    let (_dir, vars) = vars();
    let found: Vec<(Range<usize>, VarKind, String)> = tokens(TEXT, &vars)
        .into_iter()
        .map(|t| (t.range, t.kind, t.tip))
        .collect();
    assert_eq!(
        found,
        [
            (HOST, VarKind::Defined, "Collection · c.test".to_owned()),
            (NOPE, VarKind::Undefined, "Undefined variable".to_owned()),
            (
                GUID,
                VarKind::Dynamic,
                "Dynamic · new value on each send".to_owned()
            ),
            (
                TOKEN,
                VarKind::Defined,
                format!("Environment · {SECRET_DOTS}")
            ),
        ]
    );
    assert_eq!(tooltip(vars.get("empty").as_ref()), "Collection · (empty)");
    assert_eq!(tooltip(vars.get("region").as_ref()), "Environment · eu");
}

#[test]
fn a_long_value_is_cut() {
    let long = "x".repeat(300);
    let root = format!(
        "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: long\n      value: {long}\n"
    );
    let dir = write_tree(&[("opencollection.yml", &root), ("a.yml", &request("a", "x"))]);
    let c = Collection::open(dir.path()).unwrap();
    let ctx = VarContext::new(
        &c,
        ContextOptions::new().process_env(Vec::<(String, String)>::new()),
    )
    .unwrap();
    let set = ctx.variables(c.request(&ip("a.yml")).unwrap());
    let tip = tooltip(set.get("long").as_ref());
    assert_eq!(tip, format!("Collection · {}…", "x".repeat(120)));
}

struct Host {
    input: Entity<InputState>,
    vars: Option<Arc<VariableSet>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            VarField::new(
                "field-vars",
                &self.input,
                Input::new(&self.input).id("field").w_full(),
                self.vars.clone(),
            )
            .w(px(600.)),
        )
    }
}

fn host(
    cx: &mut TestAppContext,
    vars: Option<Arc<VariableSet>>,
) -> (AnyWindowHandle, Entity<InputState>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            open_window(WindowOptions::default(), cx, |w, cx| {
                let input = cx.new(|cx| InputState::new(w, cx).default_value(TEXT));
                cx.new(|_| Host { input, vars })
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let input = view.read_with(cx, |h, _| h.input.clone());
    (win, input)
}

/// Where `range` of `input` sits in the window, from the last frame.
fn token_bounds(
    cx: &mut TestAppContext,
    input: &Entity<InputState>,
    range: Range<usize>,
) -> Bounds<Pixels> {
    input
        .read_with(cx, |s, _| s.range_to_bounds(&range))
        .expect("laid out")
}

fn move_to(cx: &mut TestAppContext, win: AnyWindowHandle, at: Point<Pixels>) {
    cx.update_window(win, |_, w, cx| {
        let event = MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers: Modifiers::default(),
        };
        w.dispatch_event(event.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn tip(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("var-tooltip")
            .map(|t| t.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

#[gpui_kit::test]
fn hovering_a_placeholder_shows_its_tooltip(cx: &mut TestAppContext) {
    let (_dir, vars) = vars();
    let (win, input) = host(cx, Some(vars));
    assert_eq!(tip(cx, win), None);
    let cases = [
        (HOST, "Collection · c.test".to_owned()),
        (NOPE, "Undefined variable".to_owned()),
        (GUID, "Dynamic · new value on each send".to_owned()),
        (TOKEN, format!("Environment · {SECRET_DOTS}")),
    ];
    for (range, expected) in cases {
        let at = token_bounds(cx, &input, range).center();
        move_to(cx, win, at);
        assert_eq!(tip(cx, win), Some(expected));
    }
    let plain = token_bounds(cx, &input, 0..5).center();
    move_to(cx, win, plain);
    assert_eq!(tip(cx, win), None, "plain text has no tooltip");
}

#[gpui_kit::test]
fn a_placeholder_still_takes_clicks_and_drags(cx: &mut TestAppContext) {
    let (_dir, vars) = vars();
    let (win, input) = host(cx, Some(vars));
    let b = token_bounds(cx, &input, HOST);
    let origin = cx
        .update_window(win, |_, w, _| w.find("field").bounds().origin)
        .unwrap();
    move_to(cx, win, b.center());
    cx.update_window(win, |_, w, cx| w.click_at("field", b.center() - origin, cx))
        .unwrap();
    cx.run_until_parked();
    let caret = input.read_with(cx, |s, _| s.cursor());
    assert!(
        HOST.contains(&caret),
        "caret {caret} inside the placeholder"
    );
    let from = point(b.left() + px(1.), b.center().y);
    let to = point(b.right() - px(1.), b.center().y);
    cx.update_window(win, |_, w, cx| w.drag(from, to, cx))
        .unwrap();
    cx.run_until_parked();
    let selected = input.read_with(cx, |s, _| s.selected_range());
    assert!(
        !selected.is_empty() && selected.start >= HOST.start && selected.end <= HOST.end,
        "{selected:?}"
    );
}

#[gpui_kit::test]
fn without_variables_nothing_is_explained(cx: &mut TestAppContext) {
    let (win, input) = host(cx, None);
    let at = token_bounds(cx, &input, HOST).center();
    move_to(cx, win, at);
    assert_eq!(tip(cx, win), None);
}

#[test]
fn global_scopes_and_script_edits_are_labelled() {
    assert_eq!(
        scope_label(VarScope::GlobalEnvironment),
        "Global environment"
    );
    assert_eq!(
        overlay_note(OverlayLayer::Global),
        "Global, set by a script"
    );
    assert_eq!(
        overlay_note(OverlayLayer::Environment),
        "Environment, set by a script"
    );
    assert_eq!(
        overlay_note(OverlayLayer::Collection),
        "Collection, set by a script"
    );
}
