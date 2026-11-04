//! The chip input in the request Settings section: Enter, comma, Backspace and the remove
//! control each send the new tag list.

use crate::support;

use apinata_app::command::Edit;
use apinata_app::settings_section::SettingsSection;
use apinata_core::RequestSettings;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

fn section(
    cx: &mut TestAppContext,
    tags: &[&str],
) -> (AnyWindowHandle, Entity<SectionHost<SettingsSection>>) {
    let (win, host) = host_section(cx, SettingsSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let tags: Vec<String> = tags.iter().map(|t| (*t).to_owned()).collect();
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.load(&RequestSettings::new(), w, cx);
            s.load_tags(&tags, cx);
        })
    })
    .unwrap();
    frame(cx, win);
    (win, host)
}

fn last(cx: &mut TestAppContext, host: &Entity<SectionHost<SettingsSection>>) -> Vec<String> {
    let edit = host.read_with(cx, |h, _| {
        h.edits
            .iter()
            .rev()
            .find(|e| matches!(e, Edit::Tags(_)))
            .cloned()
    });
    match edit {
        Some(Edit::Tags(t)) => t,
        _ => panic!("no tags edit"),
    }
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, k: &str) {
    cx.update_window(win, |_, w, cx| w.press(k, cx)).unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn loading_tags_sends_no_edit(cx: &mut TestAppContext) {
    let (_win, host) = section(cx, &["a", "b"]);
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
}

#[gpui_kit::test]
fn enter_and_comma_commit_chips(cx: &mut TestAppContext) {
    let (win, host) = section(cx, &["a"]);
    type_at_end(cx, win, None, "tag-text", "b");
    press(cx, win, "enter");
    assert_eq!(last(cx, &host), ["a", "b"]);
    cx.update_window(win, |_, w, cx| w.input("c,", cx)).unwrap();
    frame(cx, win);
    assert_eq!(last(cx, &host), ["a", "b", "c"]);
}

#[gpui_kit::test]
fn backspace_on_an_empty_input_removes_the_last_chip(cx: &mut TestAppContext) {
    let (win, host) = section(cx, &["a", "b"]);
    cx.update_window(win, |_, w, cx| w.click("tag-text", cx))
        .unwrap();
    press(cx, win, "backspace");
    assert_eq!(last(cx, &host), ["a"]);
}

#[gpui_kit::test]
fn a_chips_remove_control_removes_it(cx: &mut TestAppContext) {
    let (win, host) = section(cx, &["a", "b"]);
    cx.update_window(win, |_, w, cx| w.click(("tag-remove", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(last(cx, &host), ["b"]);
}

#[gpui_kit::test]
fn blur_commits_pending_text(cx: &mut TestAppContext) {
    let (win, host) = section(cx, &[]);
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click("tag-text", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.input("late", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("set-max-redirects", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(last(cx, &host), ["late"]);
}
