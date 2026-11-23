//! Percent-encode and Percent-decode act on the focused field's selection: one undoable edit
//! that leaves the rest of the text alone, and nothing at all on a masked or locked field.

use crate::support;

use apinata_app::percent_codec::{PercentDecode, PercentEncode, codec_entries, offered};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputState, Undo};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    Action, AnyWindowHandle, AppContext as _, Context, Entity, Focusable as _, IntoElement,
    ParentElement as _, Render, Styled as _, TestAppContext, Window, WindowOptions, div, px, size,
};
use support::*;

struct Host {
    input: Entity<InputState>,
    locked: bool,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let input = Input::new(&self.input).readonly(self.locked);
        div().w(px(480.)).h(px(300.)).child(input)
    }
}

#[derive(Clone, Copy, Default)]
struct Field {
    masked: bool,
    locked: bool,
}

// A new window holding one field with `text`, focused, with `selected` selected.
fn window_with(
    cx: &mut TestAppContext,
    text: &str,
    field: Field,
    selected: std::ops::Range<usize>,
) -> (AnyWindowHandle, Entity<InputState>) {
    let text = text.to_owned();
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
                    let input = cx.new(|cx| {
                        let mut s = InputState::new(w, cx).masked(field.masked);
                        s.set_value(text, w, cx);
                        s
                    });
                    Host {
                        input,
                        locked: field.locked,
                    }
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

fn opened(
    cx: &mut TestAppContext,
    text: &str,
    field: Field,
    selected: std::ops::Range<usize>,
) -> (AnyWindowHandle, Entity<InputState>) {
    support::init(cx);
    cx.update(apinata_app::percent_codec::install);
    window_with(cx, text, field, selected)
}

fn plain() -> Field {
    Field::default()
}

fn dispatch(cx: &mut TestAppContext, win: AnyWindowHandle, action: impl Action) {
    cx.update_window(win, |_, w, cx| w.dispatch_action(action.boxed_clone(), cx))
        .unwrap();
    frame(cx, win);
}

fn text(cx: &mut TestAppContext, input: &Entity<InputState>) -> String {
    input.read_with(cx, |s, _| s.value().to_string())
}

fn toasts(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    cx.update_window(win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn encode_replaces_only_the_selection_in_one_undo_step(cx: &mut TestAppContext) {
    let (win, input) = opened(cx, "q=a b&c {{x}}|tail", plain(), 2..13);
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
    let (win, input) = opened(cx, "%41 a%20b%2Fc %42", plain(), 4..13);
    dispatch(cx, win, PercentDecode);
    assert_eq!(text(cx, &input), "%41 a b/c %42");
    dispatch(cx, win, Undo);
    assert_eq!(text(cx, &input), "%41 a%20b%2Fc %42");
}

#[gpui_kit::test]
fn a_selection_ending_inside_a_placeholder_grows_to_keep_it_whole(cx: &mut TestAppContext) {
    let (win, input) = opened(cx, "a {{host}}", plain(), 0..6);
    dispatch(cx, win, PercentEncode);
    assert_eq!(text(cx, &input), "a%20{{host}}");
    assert_eq!(input.read_with(cx, |s, _| s.selected_range()), 0..12);
}

#[gpui_kit::test]
fn a_selection_starting_inside_a_placeholder_grows_to_keep_it_whole(cx: &mut TestAppContext) {
    let (win, input) = opened(cx, "{{host}} b", plain(), 4..10);
    dispatch(cx, win, PercentEncode);
    assert_eq!(text(cx, &input), "{{host}}%20b");
    assert_eq!(input.read_with(cx, |s, _| s.selected_range()), 0..12);
}

#[gpui_kit::test]
fn decode_that_is_not_utf8_leaves_the_text_and_says_so(cx: &mut TestAppContext) {
    let (win, input) = opened(cx, "x %FF y", plain(), 0..7);
    let before = toasts(cx, win);
    dispatch(cx, win, PercentDecode);
    assert_eq!(text(cx, &input), "x %FF y");
    assert_eq!(toasts(cx, win), before + 1, "a toast says why");
}

#[gpui_kit::test]
fn decode_never_puts_control_characters_in_a_single_line_field(cx: &mut TestAppContext) {
    for escaped in ["a%0Ab", "a%0Db", "a%00b", "a%1Bb"] {
        let (win, input) = opened(cx, escaped, plain(), 0..escaped.len());
        let before = toasts(cx, win);
        dispatch(cx, win, PercentDecode);
        assert_eq!(text(cx, &input), escaped, "{escaped} is left as it was");
        assert_eq!(
            toasts(cx, win),
            before + 1,
            "a toast says why for {escaped}"
        );
    }
    // A tab is ordinary text in a field.
    let (win, input) = opened(cx, "a%09b", plain(), 0..5);
    dispatch(cx, win, PercentDecode);
    assert_eq!(text(cx, &input), "a\tb");
}

#[gpui_kit::test]
fn a_masked_field_has_no_such_command(cx: &mut TestAppContext) {
    let masked = Field {
        masked: true,
        ..Field::default()
    };
    let (win, input) = opened(cx, "p@ss word", masked, 0..9);
    assert!(!input.read_with(cx, |s, _| offered(s)));
    assert!(input.read_with(cx, |s, _| codec_entries(s)).is_empty());
    dispatch(cx, win, PercentEncode);
    assert_eq!(text(cx, &input), "p@ss word");
}

#[gpui_kit::test]
fn a_locked_field_is_never_changed(cx: &mut TestAppContext) {
    let locked = Field {
        locked: true,
        ..Field::default()
    };
    let (win, input) = opened(cx, "a b", locked, 0..3);
    assert!(input.read_with(cx, |s, _| s.context_menu_capabilities().is_readonly()));
    assert!(!input.read_with(cx, |s, _| offered(s)));
    assert!(input.read_with(cx, |s, _| codec_entries(s)).is_empty());
    // The palette and the menu both dispatch the action, so neither path can reach it.
    dispatch(cx, win, PercentEncode);
    assert_eq!(text(cx, &input), "a b");
}

#[gpui_kit::test]
fn the_menu_entries_are_disabled_until_text_is_selected(cx: &mut TestAppContext) {
    let (win, input) = opened(cx, "a b", plain(), 1..1);
    assert_eq!(
        input.read_with(cx, |s, _| codec_entries(s)),
        [("Percent-encode", true), ("Percent-decode", true)]
    );
    input.update(cx, |s, cx| s.set_selected_range(0..3, cx));
    frame(cx, win);
    assert_eq!(
        input.read_with(cx, |s, _| codec_entries(s)),
        [("Percent-encode", false), ("Percent-decode", false)]
    );
}

#[gpui_kit::test]
fn the_palette_returns_focus_to_the_field_and_encodes_it(cx: &mut TestAppContext) {
    let (win, input) = opened(cx, "a b c", plain(), 0..3);
    let handle = input.read_with(cx, |s, cx| s.focus_handle(cx));
    cx.update_window(win, |_, w, cx| {
        apinata_app::palette::open(handle.clone(), w, cx)
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.input("percent-encode selection", cx))
        .unwrap();
    frame(cx, win);
    cx.simulate_keystrokes(win, "enter");
    frame(cx, win);
    assert_eq!(text(cx, &input), "a%20b c");
    let focused = cx
        .update_window(win, |_, w, _| handle.is_focused(w))
        .unwrap();
    assert!(focused, "focus is back in the field");
}

#[gpui_kit::test]
fn the_command_acts_in_the_window_that_dispatched_it(cx: &mut TestAppContext) {
    let (first, near) = opened(cx, "a b", plain(), 0..3);
    // The second window opens last, so it is the active one.
    let (_second, far) = window_with(cx, "c d", plain(), 0..3);
    dispatch(cx, first, PercentEncode);
    assert_eq!(text(cx, &near), "a%20b");
    assert_eq!(
        text(cx, &far),
        "c d",
        "the other window's field is untouched"
    );
}
