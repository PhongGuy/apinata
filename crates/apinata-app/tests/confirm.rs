//! The confirm dialog: Enter answers the default, Escape cancels, Tab moves focus between
//! the buttons and Space presses the focused one.

use crate::support;

use apinata_app::confirm::{Ask, Choice, ask};
use apinata_app::style::{SPACE_3, at_zoom};
use gpui_kit::component::WindowExt;
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

fn choice(id: &'static str, label: &'static str, danger: bool) -> Choice {
    Choice {
        id,
        label: label.into(),
        danger,
    }
}

fn save_prompt() -> Ask {
    Ask {
        title: "Save changes to \"a\"?".into(),
        detail: Some("2 requests have unsaved changes.".into()),
        choices: vec![
            choice("confirm-cancel", "Cancel", false),
            choice("confirm-discard", "Don't Save", false),
            choice("confirm-save", "Save", false),
        ],
        default: 2,
        cancel: 0,
    }
}

fn opened(cx: &mut TestAppContext, spec: Ask) -> (AnyWindowHandle, async_channel::Receiver<usize>) {
    support::init(cx);
    let (win, _) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Blank)
            })
        })
        .unwrap();
    let rx = cx.update_window(win, |_, w, cx| ask(spec, w, cx)).unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, rx)
}

fn focused(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<bool> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).focused()
    })
    .unwrap()
}

fn dialog_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

// A full key press, down then up, as the kit's own button tests send it.
fn press(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    let mut v = gpui_kit::VisualTestContext::from_window(win, cx);
    let keystroke =
        gpui_kit::Keystroke::parse_with_os(key, apinata_app::shortcuts::HOST_OS).unwrap();
    v.simulate_event(gpui_kit::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    v.simulate_event(gpui_kit::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn enter_answers_the_default_and_closes(cx: &mut TestAppContext) {
    let (win, rx) = opened(cx, save_prompt());
    assert!(confirm_open(cx));
    cx.simulate_keystrokes(win, "enter");
    cx.run_until_parked();
    assert_eq!(rx.try_recv(), Ok(2));
    assert!(!dialog_open(cx, win));
}

#[gpui_kit::test]
fn escape_answers_cancel_and_closes(cx: &mut TestAppContext) {
    let (win, rx) = opened(cx, save_prompt());
    cx.simulate_keystrokes(win, "escape");
    cx.run_until_parked();
    assert_eq!(rx.try_recv(), Ok(0));
    assert!(!dialog_open(cx, win));
}

#[gpui_kit::test]
fn tab_walks_the_buttons_and_space_presses_the_focused_one(cx: &mut TestAppContext) {
    let (win, rx) = opened(cx, save_prompt());
    cx.simulate_keystrokes(win, "tab");
    assert_eq!(focused(cx, win, "confirm-cancel"), Some(true));
    cx.simulate_keystrokes(win, "tab");
    assert_eq!(focused(cx, win, "confirm-cancel"), Some(false));
    assert_eq!(focused(cx, win, "confirm-discard"), Some(true));
    press(cx, win, "space");
    assert_eq!(rx.try_recv(), Ok(1));
    assert!(!dialog_open(cx, win));
}

#[gpui_kit::test]
fn enter_presses_the_focused_button_not_the_default(cx: &mut TestAppContext) {
    let (win, rx) = opened(cx, save_prompt());
    cx.update(apinata_app::keymap::init);
    cx.simulate_keystrokes(win, "tab");
    assert_eq!(focused(cx, win, "confirm-cancel"), Some(true));
    press(cx, win, "enter");
    assert_eq!(rx.try_recv(), Ok(0));
    assert!(!dialog_open(cx, win));
}

#[gpui_kit::test]
fn a_click_answers_its_button_and_every_part_has_an_id(cx: &mut TestAppContext) {
    let (win, rx) = opened(cx, save_prompt());
    cx.update_window(win, |_, w, _| {
        for id in [
            "confirm-dialog",
            "confirm-detail",
            "confirm-cancel",
            "confirm-discard",
            "confirm-save",
        ] {
            assert!(w.try_find(id).is_some(), "{id} missing");
        }
    })
    .unwrap();
    answer_confirm(cx, "confirm-discard");
    assert_eq!(rx.try_recv(), Ok(1));
    assert!(!confirm_open(cx));
}

#[gpui_kit::test]
fn the_buttons_sit_in_the_shared_footer_in_order(cx: &mut TestAppContext) {
    let (win, _rx) = opened(cx, save_prompt());
    cx.update_window(win, |_, w, _| {
        let footer = w.within("confirm-dialog").find("dialog-footer").bounds();
        let [cancel, discard, save] =
            ["confirm-cancel", "confirm-discard", "confirm-save"].map(|id| w.find(id).bounds());
        assert!(cancel.left() < discard.left() && discard.left() < save.left());
        assert_eq!(save.right(), footer.right());
        let detail = w.find("confirm-detail").bounds();
        assert_eq!(
            save.top() - detail.bottom(),
            at_zoom(SPACE_3, 1.0),
            "spacing unchanged"
        );
    })
    .unwrap();
}

#[test]
fn the_extended_prompt_names_the_extenders_and_offers_only_cancel() {
    let by = ["a".to_string(), "b".to_string()];
    let spec = apinata_app::confirm::extended_ask("Environment", "dev", &by);
    assert_eq!(
        spec.title.as_ref(),
        "Environment \"dev\" is extended by \"a\", \"b\""
    );
    assert_eq!(
        spec.detail.as_ref().map(|d| d.as_ref()),
        Some("Change their extends first.")
    );
    let ids: Vec<&str> = spec.choices.iter().map(|c| c.id).collect();
    assert_eq!(ids, ["confirm-cancel"]);
    assert_eq!((spec.default, spec.cancel), (0, 0));
}

#[test]
fn the_drop_prompt_defaults_to_cancel_and_words_the_count() {
    let one = apinata_app::confirm::drop_staged_ask(1, "Unstage");
    assert_eq!(one.title.as_ref(), "Drop 1 staged copy?");
    assert_eq!(
        one.detail.as_deref(),
        Some("It is gone from its path, so the index may hold the only copy of the edit.")
    );
    assert_eq!((one.default, one.cancel), (0, 0));
    assert_eq!(one.choices[1].label.as_ref(), "Unstage");
    assert!(one.choices[1].danger);
    let two = apinata_app::confirm::drop_staged_ask(2, "Stage all");
    assert_eq!(two.title.as_ref(), "Drop 2 staged copies?");
    assert_eq!(
        two.detail.as_deref(),
        Some("They are gone from their paths, so the index may hold the only copies of the edits.")
    );
    assert_eq!(two.choices[1].label.as_ref(), "Stage all");
}
