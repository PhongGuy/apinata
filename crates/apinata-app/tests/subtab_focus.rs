//! A click on a sub-tab or segment inside a request section keeps focus in that section, so
//! the next key never lands in the URL.

use crate::support;

use apinata_app::request_editor::Section;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

const URL: &str = "http://127.0.0.1:9/a";

// The real app over one request, its tab open and the window active.
fn one_request(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &request("a", URL))]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    (col, s)
}

fn drawn(cx: &mut TestAppContext, s: &Started) {
    next_frame(cx, s.win);
    next_frame(cx, s.win);
}

fn show_section(cx: &mut TestAppContext, s: &Started, section: Section) {
    platform_click(cx, s.win, ("section", section as usize));
    drawn(cx, s);
}

fn click_docs_mode(cx: &mut TestAppContext, s: &Started, ix: usize) {
    cx.update_window(s.win, |_, w, cx| w.within("docs-mode").click(ix, cx))
        .unwrap();
    drawn(cx, s);
}

#[gpui_kit::test]
fn a_key_after_clicking_docs_preview_stays_out_of_the_url(cx: &mut TestAppContext) {
    let (_col, s) = one_request(cx);
    show_section(cx, &s, Section::Docs);
    click_docs_mode(cx, &s, 1);
    // Preview takes the editor away while the clicked segment takes no focus.
    click_docs_mode(cx, &s, 0);
    assert!(!ring_on(cx, s.win, "url"), "the URL input took focus");
    assert!(
        ring_on(cx, s.win, "section-body"),
        "focus stays in the section"
    );
    platform_press(cx, s.win, "x");
    next_frame(cx, s.win);
    assert_eq!(drawn_text(cx, s.win, "url"), URL);
}

#[gpui_kit::test]
fn a_key_after_clicking_a_script_phase_tab_stays_out_of_the_url(cx: &mut TestAppContext) {
    use apinata_core::ScriptPhase::PreRequest;
    use gpui_kit::Focusable as _;
    let (_col, s) = one_request(cx);
    show_section(cx, &s, Section::Script);
    // The pre-request editor holds focus when the switch hides it.
    platform_click(cx, s.win, "script-pre-request-code");
    drawn(cx, &s);
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    let pre = area.read_with(cx, |a, cx| {
        let id = s.ws.read(cx).tabs().active().expect("a tab").id;
        let script = a
            .editor(id)
            .expect("a request editor")
            .read(cx)
            .script()
            .clone();
        let code = script.read(cx).code(PreRequest).cloned();
        code.expect("a pre-request editor")
            .read(cx)
            .focus_handle(cx)
    });
    let held = cx
        .update_window(s.win, |_, w, _| pre.is_focused(w))
        .unwrap();
    assert!(held, "the pre-request editor holds focus");
    platform_click(cx, s.win, ("script-tab", 1usize));
    drawn(cx, &s);
    assert!(!ring_on(cx, s.win, "url"), "the URL input took focus");
    assert!(
        ring_on(cx, s.win, "section-body"),
        "focus stays in the section"
    );
    platform_press(cx, s.win, "x");
    next_frame(cx, s.win);
    assert_eq!(drawn_text(cx, s.win, "url"), URL);
}
