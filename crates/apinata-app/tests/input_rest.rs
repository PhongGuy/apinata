//! A value longer than its input shows its start once the input loses focus: the caret moves
//! to the start, so returning by keyboard shows it and a click or key moves it as usual.

use crate::support;

use apinata_app::env_table::{EnvRow, EnvVarTable};
use gpui_kit::component::input::InputState;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, Focusable as _, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window,
    WindowOptions, div, px, size,
};
use support::*;

const LONG: &str = "https://jsonplaceholder.typicode.com/users/1/posts/2/comments/3/albums/4/photos/5/todos/6/users/7/posts/8/comments/9";

struct Host {
    table: Entity<EnvVarTable>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(480.)).h(px(300.)).child(self.table.clone())
    }
}

fn opened(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<EnvVarTable>) {
    support::init(cx);
    cx.update(apinata_app::input_rest::install);
    let row = EnvRow {
        name: "baseUrl".into(),
        value: String::new(),
        enabled: true,
        secret: false,
        source: Some(0),
        unsaved: false,
    };
    let (win, host) = cx
        .update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(px(0.), px(0.)),
                    size(px(480.), px(300.)),
                ))),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |w, cx| {
                cx.new(|cx| {
                    let table = cx.new(|cx| EnvVarTable::new(w, cx));
                    table.update(cx, |t, cx| t.load(vec![row], w, cx));
                    Host { table }
                })
            })
        })
        .unwrap();
    // Focus changes are only seen while the window is active.
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    let table = host.read_with(cx, |h, _| h.table.clone());
    (win, table)
}

fn value_input(cx: &mut TestAppContext, table: &Entity<EnvVarTable>) -> Entity<InputState> {
    table.read_with(cx, |t, _| t.value_input(0).unwrap().clone())
}

fn scroll_x(cx: &mut TestAppContext, input: &Entity<InputState>) -> f32 {
    input.read_with(cx, |s, _| f32::from(s.scroll_offset().x))
}

#[gpui_kit::test]
fn a_long_value_shows_its_start_after_blur(cx: &mut TestAppContext) {
    let (win, table) = opened(cx);
    let input = value_input(cx, &table);
    type_at_end(cx, win, None, ("env-value", 0usize), LONG);
    frame(cx, win);
    frame(cx, win);
    assert!(
        scroll_x(cx, &input) < 0.,
        "while typing, the caret at the end is in view"
    );
    cx.update_window(win, |_, w, cx| w.blur(cx)).unwrap();
    frame(cx, win);
    frame(cx, win);
    assert_eq!(scroll_x(cx, &input), 0., "at rest the start is in view");
    // A later redraw does not scroll back to the caret left at the end.
    cx.update_window(win, |_, w, _| w.refresh()).unwrap();
    frame(cx, win);
    assert_eq!(scroll_x(cx, &input), 0.);
}

#[gpui_kit::test]
fn typing_after_returning_follows_the_caret_again(cx: &mut TestAppContext) {
    let (win, table) = opened(cx);
    let input = value_input(cx, &table);
    type_at_end(cx, win, None, ("env-value", 0usize), LONG);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.blur(cx)).unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, ("env-value", 0usize), "x");
    frame(cx, win);
    frame(cx, win);
    assert!(scroll_x(cx, &input) < 0., "the caret at the end is in view");
}

#[gpui_kit::test]
fn returning_without_typing_shows_the_caret_at_the_start(cx: &mut TestAppContext) {
    let (win, table) = opened(cx);
    let input = value_input(cx, &table);
    type_at_end(cx, win, None, ("env-value", 0usize), LONG);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.blur(cx)).unwrap();
    frame(cx, win);
    // Focus comes back as Tab brings it, without a click placing the caret.
    let handle = input.read_with(cx, |s, cx| s.focus_handle(cx));
    cx.update_window(win, |_, w, cx| handle.focus(w, cx))
        .unwrap();
    frame(cx, win);
    frame(cx, win);
    assert_eq!(
        input.read_with(cx, |s, _| s.cursor()),
        0,
        "the caret is at the start"
    );
    assert_eq!(scroll_x(cx, &input), 0., "and the start is in view");
}

#[gpui_kit::test]
fn switching_windows_leaves_the_field_as_it_was(cx: &mut TestAppContext) {
    let (win, table) = opened(cx);
    let input = value_input(cx, &table);
    type_at_end(cx, win, None, ("env-value", 0usize), LONG);
    frame(cx, win);
    frame(cx, win);
    let before = scroll_x(cx, &input);
    VisualTestContext::from_window(win, cx).deactivate_window();
    frame(cx, win);
    cx.run_until_parked();
    assert_eq!(scroll_x(cx, &input), before, "the end stays in view");
    assert_eq!(input.read_with(cx, |s, _| s.cursor()), LONG.len());
}
