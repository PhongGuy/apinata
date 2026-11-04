//! The "Input required" dialog: one pre-filled field per label, Continue and Cancel.

use crate::support;

use apinata_app::prompt_vars::{PromptForm, PromptMemory, ask};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

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

fn labels() -> Vec<String> {
    vec!["code".to_owned(), "Order id".to_owned()]
}

fn remembered() -> PromptMemory {
    let memory = PromptMemory::default();
    memory.remember(&[("code".to_owned(), "A1".to_owned())]);
    memory
}

type Answers = async_channel::Receiver<Option<Vec<(String, String)>>>;

fn opened(cx: &mut TestAppContext) -> (AnyWindowHandle, Answers) {
    support::init(cx);
    let (win, _) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Blank)
            })
        })
        .unwrap();
    let (labels, memory) = (labels(), remembered());
    let rx = cx
        .update_window(win, |_, w, cx| {
            let form = cx.new(|cx| PromptForm::new(&labels, &memory, w, cx));
            ask(form, w, cx)
        })
        .unwrap();
    frame(cx, win);
    (win, rx)
}

fn dialog_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

#[gpui_kit::test]
fn the_form_prefills_from_memory_and_answers_as_typed(cx: &mut TestAppContext) {
    support::init(cx);
    let (labels, memory) = (labels(), remembered());
    let (win, form) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| PromptForm::new(&labels, &memory, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    let values = form.read_with(cx, |f, cx| f.values(cx));
    assert_eq!(
        values,
        [
            ("code".to_owned(), "A1".to_owned()),
            ("Order id".to_owned(), String::new())
        ]
    );
    let second = form.read_with(cx, |f, _| f.inputs()[1].clone());
    cx.update_window(win, |_, w, cx| {
        second.update(cx, |i, cx| i.set_value(" 42 ", w, cx))
    })
    .unwrap();
    assert_eq!(form.read_with(cx, |f, cx| f.values(cx))[1].1, " 42 ");
}

#[gpui_kit::test]
fn continue_answers_every_field(cx: &mut TestAppContext) {
    let (win, rx) = opened(cx);
    assert!(present(cx, win, "prompt-vars-dialog"));
    assert!(present(cx, win, "prompt-var-0") && present(cx, win, "prompt-var-1"));
    cx.update_window(win, |_, w, cx| w.click("prompt-vars-continue", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        rx.try_recv(),
        Ok(Some(vec![
            ("code".to_owned(), "A1".to_owned()),
            ("Order id".to_owned(), String::new())
        ]))
    );
    assert!(!dialog_open(cx, win));
}

#[gpui_kit::test]
fn cancel_and_escape_answer_none(cx: &mut TestAppContext) {
    let (win, rx) = opened(cx);
    cx.update_window(win, |_, w, cx| w.click("prompt-vars-cancel", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(rx.try_recv(), Ok(None));
    assert!(!dialog_open(cx, win));
    let (win, rx) = opened(cx);
    cx.simulate_keystrokes(win, "escape");
    cx.run_until_parked();
    assert_eq!(rx.try_recv(), Ok(None));
}

#[gpui_kit::test]
fn enter_on_the_focused_cancel_button_answers_none(cx: &mut TestAppContext) {
    let (win, rx) = opened(cx);
    cx.update(apinata_app::keymap::init);
    enter_on(cx, win, "prompt-vars-cancel");
    assert_eq!(rx.try_recv(), Ok(None));
    assert!(!dialog_open(cx, win));
}

#[gpui_kit::test]
fn enter_continues(cx: &mut TestAppContext) {
    let (win, rx) = opened(cx);
    cx.simulate_keystrokes(win, "enter");
    cx.run_until_parked();
    assert!(matches!(rx.try_recv(), Ok(Some(values)) if values.len() == 2));
}
