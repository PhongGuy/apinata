//! Percent-encode and Percent-decode act on the focused field's selection: one undoable edit
//! that leaves the rest of the text alone, and nothing at all on a masked field.

use crate::support;

use apinata_app::percent_codec::{PercentDecode, PercentEncode, offered};
use gpui_kit::component::input::{Input, InputState, Undo};
use gpui_kit::{
    Action, AnyWindowHandle, AppContext as _, Context, Entity, Focusable as _, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, Window, WindowOptions, div, px, size,
};
use support::*;

struct Host {
    input: Entity<InputState>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(480.)).h(px(100.)).child(Input::new(&self.input))
    }
}

// A window holding one field with `text`, focused, with `selected` selected.
fn opened(
    cx: &mut TestAppContext,
    text: &str,
    masked: bool,
    selected: std::ops::Range<usize>,
) -> (AnyWindowHandle, Entity<InputState>) {
    support::init(cx);
    cx.update(apinata_app::percent_codec::install);
    let text = text.to_owned();
    let (win, host) = cx
        .update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(px(0.), px(0.)),
                    size(px(480.), px(100.)),
                ))),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |w, cx| {
                cx.new(|cx| {
                    let input = cx.new(|cx| {
                        let mut s = InputState::new(w, cx).masked(masked);
                        s.set_value(text, w, cx);
                        s
                    });
                    Host { input }
                })
            })
        })
        .unwrap();
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    let input = host.read_with(cx, |h, _| h.input.clone());
    let handle = input.read_with(cx, |s, cx| s.focus_handle(cx));
    cx.update_window(win, |_, w, cx| handle.focus(w, cx))
        .unwrap();
    input.update(cx, |s, cx| s.set_selected_range(selected, cx));
    frame(cx, win);
    (win, input)
}

fn dispatch(cx: &mut TestAppContext, win: AnyWindowHandle, action: impl Action) {
    cx.update_window(win, |_, w, cx| w.dispatch_action(action.boxed_clone(), cx))
        .unwrap();
    frame(cx, win);
}

fn text(cx: &mut TestAppContext, input: &Entity<InputState>) -> String {
    input.read_with(cx, |s, _| s.value().to_string())
}

#[gpui_kit::test]
fn encode_replaces_only_the_selection_in_one_undo_step(cx: &mut TestAppContext) {
    let (win, input) = opened(cx, "q=a b&c {{x}}|tail", false, 2..13);
    assert!(input.read_with(cx, |s, _| offered(s)));
    dispatch(cx, win, PercentEncode);
    assert_eq!(text(cx, &input), "q=a%20b%26c%20{{x}}|tail");
    dispatch(cx, win, Undo);
    assert_eq!(
        text(cx, &input),
        "q=a b&c {{x}}|tail",
        "one undo restores it"
    );
}

#[gpui_kit::test]
fn decode_replaces_only_the_selection_in_one_undo_step(cx: &mut TestAppContext) {
    let (win, input) = opened(cx, "%41 a%20b%2Fc %42", false, 4..13);
    dispatch(cx, win, PercentDecode);
    assert_eq!(text(cx, &input), "%41 a b/c %42");
    dispatch(cx, win, Undo);
    assert_eq!(text(cx, &input), "%41 a%20b%2Fc %42");
}

#[gpui_kit::test]
fn decode_that_is_not_utf8_leaves_the_text_as_it_was(cx: &mut TestAppContext) {
    let (win, input) = opened(cx, "x %FF y", false, 0..7);
    dispatch(cx, win, PercentDecode);
    assert_eq!(text(cx, &input), "x %FF y");
}

#[gpui_kit::test]
fn a_masked_field_has_no_such_command(cx: &mut TestAppContext) {
    let (win, input) = opened(cx, "p@ss word", true, 0..9);
    assert!(!input.read_with(cx, |s, _| offered(s)));
    dispatch(cx, win, PercentEncode);
    assert_eq!(text(cx, &input), "p@ss word");
}
