//! The collection's Script trust settings: one labelled row, explained.

use crate::support;

use apinata_app::script_settings::ScriptSettingsView;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

fn view(cx: &mut TestAppContext, files: &tempfile::TempDir) -> AnyWindowHandle {
    support::init(cx);
    let (file, root) = (
        files.path().join("script-trust.json"),
        files.path().join("c"),
    );
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| ScriptSettingsView::new(file, root, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    win
}

#[gpui_kit::test]
fn the_choice_has_its_label_on_the_left(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let win = view(cx, &files);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let label = w.within("script-decision-row").find("form-label").bounds();
        let group = w.find("script-decision").bounds();
        assert!(label.right() <= group.left(), "{label:?} {group:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_note_says_when_ask_asks(cx: &mut TestAppContext) {
    let files = tempfile::tempdir().unwrap();
    let win = view(cx, &files);
    let note = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("script-decision-note").label().map(str::to_owned)
        })
        .unwrap()
        .unwrap_or_default();
    assert!(note.starts_with("Ask:"), "{note}");
    assert!(note.contains("before a send"), "{note}");
}
