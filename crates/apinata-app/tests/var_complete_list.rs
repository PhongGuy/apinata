use crate::support;

use std::sync::Arc;

use apinata_app::var_tokens::{SECRET_DOTS, VarField};
use apinata_core::{Collection, ContextOptions, VarContext, VariableSet};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, TestAppContext, Window, WindowOptions, div, open_window, px,
};
use support::*;

const VARS_ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: host\n      value: c.test\n    - name: hostname\n      value: h.test\n    - name: empty\n      value: \"\"\n";
const DEV: &str = "name: dev\nvariables:\n  - name: region\n    value: eu\n";

/// Collection `host`, `hostname` and `empty`, `dev`'s `region` and `__name__`, a secret
/// `token`; no process environment.
fn vars() -> (tempfile::TempDir, Arc<VariableSet>) {
    let dir = write_tree(&[
        ("opencollection.yml", VARS_ROOT),
        ("environments/dev.yml", DEV),
        ("a.yml", &request("a", "x")),
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

struct Host {
    input: Entity<InputState>,
    vars: Option<Arc<VariableSet>>,
    enters: usize,
    _enter: Subscription,
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

/// A focused field holding `text` with the cursor at its end, and the window it is in.
fn field(
    cx: &mut TestAppContext,
    text: &'static str,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Host>) {
    support::init(cx);
    let (dir, vars) = vars();
    let (win, view) = cx
        .update(|cx| {
            open_window(WindowOptions::default(), cx, |w, cx| {
                let input = cx.new(|cx| InputState::new(w, cx).default_value(text));
                cx.new(|cx| {
                    let _enter = cx.subscribe(&input, |h: &mut Host, _, e: &InputEvent, _| {
                        if matches!(e, InputEvent::PressEnter { .. }) {
                            h.enters += 1;
                        }
                    });
                    Host {
                        input,
                        vars: Some(vars),
                        enters: 0,
                        _enter,
                    }
                })
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("field", cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
    (dir, win, view)
}

fn typing(cx: &mut TestAppContext, win: AnyWindowHandle, text: &str) {
    cx.update_window(win, |_, w, cx| w.input(text, cx)).unwrap();
    cx.run_until_parked();
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    cx.update_window(win, |_, w, cx| {
        w.press(key, cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The list's rows as (aria label, selected), or `None` when it is closed.
fn rows(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<Vec<(String, bool)>> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("var-complete")?;
        let mut found = Vec::new();
        while let Some(row) = w.try_find(("var-complete-row", found.len())) {
            let label = row.label().unwrap_or_default().to_owned();
            found.push((label, row.selected() == Some(true)));
        }
        Some(found)
    })
    .unwrap()
}

fn value(cx: &mut TestAppContext, view: &Entity<Host>) -> (String, usize) {
    let input = view.read_with(cx, |h, _| h.input.clone());
    input.read_with(cx, |s, _| (s.value().to_string(), s.cursor()))
}

#[gpui_kit::test]
fn typing_an_open_pair_lists_every_name_under_two_headings(cx: &mut TestAppContext) {
    let (_dir, win, _view) = field(cx, "https://");
    assert_eq!(rows(cx, win), None);
    typing(cx, win, "{{");
    let shown = rows(cx, win).expect("the list opens");
    assert_eq!(
        shown[..6],
        [
            ("__name__, Environment · dev".to_owned(), true),
            ("empty, Collection".to_owned(), false),
            ("host, Collection · c.test".to_owned(), false),
            ("hostname, Collection · h.test".to_owned(), false),
            ("region, Environment · eu".to_owned(), false),
            (format!("token, Environment · {SECRET_DOTS}"), false),
        ]
    );
    assert_eq!(shown[6], ("$guid".to_owned(), false));
    cx.update_window(win, |_, w, _| {
        assert_eq!(
            w.find(("var-complete-heading", 0usize)).label(),
            Some("Variables")
        );
        assert_eq!(
            w.find(("var-complete-heading", 1usize)).label(),
            Some("Dynamic")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_generated_data_names_are_offered(cx: &mut TestAppContext) {
    let (_dir, win, _view) = field(cx, "https://");
    typing(cx, win, "{{$randomFirst");
    assert_eq!(
        rows(cx, win),
        Some(vec![("$randomFirstName".to_owned(), true)])
    );
}

#[gpui_kit::test]
fn the_list_filters_as_you_type(cx: &mut TestAppContext) {
    let (_dir, win, _view) = field(cx, "https://");
    typing(cx, win, "{{host");
    assert_eq!(
        rows(cx, win),
        Some(vec![
            ("host, Collection · c.test".to_owned(), true),
            ("hostname, Collection · h.test".to_owned(), false),
        ])
    );
    typing(cx, win, "N");
    assert_eq!(
        rows(cx, win),
        Some(vec![("hostname, Collection · h.test".to_owned(), true)])
    );
    press(cx, win, "backspace");
    assert_eq!(rows(cx, win).map(|r| r.len()), Some(2));
    typing(cx, win, "zq");
    assert_eq!(rows(cx, win), None);
}

#[gpui_kit::test]
fn escape_closes_the_list_until_a_new_pair_is_typed(cx: &mut TestAppContext) {
    let (_dir, win, view) = field(cx, "");
    typing(cx, win, "{{h");
    press(cx, win, "escape");
    assert_eq!(rows(cx, win), None);
    typing(cx, win, "o");
    assert_eq!(rows(cx, win), None);
    typing(cx, win, "}}/{{");
    assert!(rows(cx, win).is_some());
    assert_eq!(value(cx, &view).0, "{{ho}}/{{");
}

#[gpui_kit::test]
fn moving_into_a_placeholder_does_not_open_the_list(cx: &mut TestAppContext) {
    let (_dir, win, _view) = field(cx, "{{host}}");
    press(cx, win, "left");
    press(cx, win, "left");
    press(cx, win, "left");
    assert_eq!(rows(cx, win), None);
}

#[gpui_kit::test]
fn nothing_matching_closes_the_list(cx: &mut TestAppContext) {
    let (_dir, win, view) = field(cx, "");
    typing(cx, win, "{{zz");
    assert_eq!(rows(cx, win), None);
    press(cx, win, "enter");
    assert_eq!(view.read_with(cx, |h, _| h.enters), 1);
}

#[gpui_kit::test]
fn tab_inserts_the_highlighted_name(cx: &mut TestAppContext) {
    let (_dir, win, view) = field(cx, "https://");
    typing(cx, win, "{{hostn");
    assert_eq!(
        rows(cx, win),
        Some(vec![("hostname, Collection · h.test".to_owned(), true)])
    );
    press(cx, win, "tab");
    assert_eq!(rows(cx, win), None);
    assert_eq!(value(cx, &view), ("https://{{hostname}}".to_owned(), 20));
}

#[gpui_kit::test]
fn arrows_move_the_highlight_and_enter_inserts_without_submitting(cx: &mut TestAppContext) {
    let (_dir, win, view) = field(cx, "");
    typing(cx, win, "{{ho");
    press(cx, win, "down");
    let shown = rows(cx, win).unwrap();
    assert_eq!(shown.iter().position(|r| r.1), Some(1));
    press(cx, win, "up");
    press(cx, win, "up");
    let shown = rows(cx, win).unwrap();
    assert_eq!(
        shown.iter().position(|r| r.1),
        Some(shown.len() - 1),
        "wraps"
    );
    press(cx, win, "down");
    press(cx, win, "down");
    press(cx, win, "enter");
    assert_eq!(value(cx, &view), ("{{hostname}}".to_owned(), 12));
    assert_eq!(view.read_with(cx, |h, _| h.enters), 0);
    press(cx, win, "enter");
    assert_eq!(
        view.read_with(cx, |h, _| h.enters),
        1,
        "closed, Enter is the field's again"
    );
}

#[gpui_kit::test]
fn closing_braces_after_the_cursor_are_not_doubled(cx: &mut TestAppContext) {
    let (_dir, win, view) = field(cx, "a/{{}}/b");
    press(cx, win, "left");
    press(cx, win, "left");
    press(cx, win, "left");
    press(cx, win, "left");
    typing(cx, win, "reg");
    press(cx, win, "enter");
    assert_eq!(value(cx, &view), ("a/{{region}}/b".to_owned(), 12));
}

#[gpui_kit::test]
fn clicking_a_row_inserts_it(cx: &mut TestAppContext) {
    let (_dir, win, view) = field(cx, "");
    typing(cx, win, "{{re");
    cx.update_window(win, |_, w, cx| w.click(("var-complete-row", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(value(cx, &view), ("{{region}}".to_owned(), 10));
    assert_eq!(rows(cx, win), None);
}

#[gpui_kit::test]
fn the_highlighted_row_scrolls_into_view(cx: &mut TestAppContext) {
    let (_dir, win, _view) = field(cx, "");
    typing(cx, win, "{{");
    let last = rows(cx, win).unwrap().len() - 1;
    let below = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let list = w.find("var-complete").bounds();
            let row = w.find(("var-complete-row", last)).bounds();
            row.bottom() > list.bottom()
        })
        .unwrap()
    };
    assert!(below(cx), "the list is taller than its box");
    press(cx, win, "up");
    assert!(!below(cx), "wrapping to the last row scrolls to it");
}

const LONG_PREFIX: &str = "https://example.test/a/fairly/long/path/to/push/it/right/";

/// The `{{` before the cursor, in window coordinates, and the open list's bounds.
fn anchor_and_list(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    view: &Entity<Host>,
) -> (
    gpui_kit::Bounds<gpui_kit::Pixels>,
    gpui_kit::Bounds<gpui_kit::Pixels>,
) {
    let input = view.read_with(cx, |h, _| h.input.clone());
    let at = LONG_PREFIX.len();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let anchor = input.read(cx).range_to_bounds(&(at..at + 2)).unwrap();
        (anchor, w.find("var-complete").bounds())
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_list_keeps_its_place_under_the_pair_mid_window(cx: &mut TestAppContext) {
    let (_dir, win, view) = field(cx, LONG_PREFIX);
    typing(cx, win, "{{");
    let (anchor, list) = anchor_and_list(cx, win, &view);
    assert!(
        (list.left() - anchor.left()).abs() < px(1.),
        "{list:?} {anchor:?}"
    );
    assert!(list.top() >= anchor.bottom(), "{list:?} {anchor:?}");
}

// Near the right edge the list ends where the pair ends rather than being pushed left
// by an arbitrary amount.
#[gpui_kit::test]
fn the_list_aligns_its_right_edge_to_the_pair_at_the_window_edge(cx: &mut TestAppContext) {
    let (_dir, win, view) = field(cx, LONG_PREFIX);
    typing(cx, win, "{{");
    let (anchor, list) = anchor_and_list(cx, win, &view);
    assert!(
        anchor.right() > list.size.width,
        "the pair sits too far left: {anchor:?}"
    );
    gpui_kit::VisualTestContext::from_window(win, cx)
        .simulate_resize(gpui_kit::size(anchor.right() + px(20.), px(600.)));
    cx.run_until_parked();
    let (anchor, list) = anchor_and_list(cx, win, &view);
    let width = cx
        .update_window(win, |_, w, _| w.viewport_size().width)
        .unwrap();
    assert!(list.right() <= width, "{list:?} past {width:?}");
    assert!(
        (list.right() - anchor.right()).abs() <= px(1.),
        "{list:?} {anchor:?}"
    );
    assert!(list.top() >= anchor.bottom(), "{list:?} {anchor:?}");
}
