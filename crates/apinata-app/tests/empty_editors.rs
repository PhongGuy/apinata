//! An empty editor shows a muted hint, and the dock's empty notes use the shared empty state.

use crate::support;

use std::path::PathBuf;

use apinata_app::body_section::{BodyMode, BodySection};
use apinata_app::console_dock::DockTab;
use apinata_app::keymap::ToggleConsole;
use apinata_app::script_section::ScriptSection;
use apinata_app::settings_section::DocsSection;
use apinata_core::BodySpec;
use apinata_core::ScriptPhase::{PostResponse, PreRequest, Tests};
use gpui_kit::component::input::EditorState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, px};
use support::*;

fn hint(cx: &mut TestAppContext, code: &Entity<EditorState>) -> String {
    code.read_with(cx, |c, _| c.presentation().placeholder().to_string())
}

#[gpui_kit::test]
fn script_editors_say_when_they_run(cx: &mut TestAppContext) {
    let (_win, host) = host_section(cx, |w, cx| {
        ScriptSection::new(&[PreRequest, PostResponse], w, cx)
    });
    let s = host.read_with(cx, |h, _| h.view.clone());
    let pre = s.read_with(cx, |s, _| s.code(PreRequest).cloned()).unwrap();
    let post = s
        .read_with(cx, |s, _| s.code(PostResponse).cloned())
        .unwrap();
    assert_eq!(hint(cx, &pre), "Runs before the request is sent.");
    assert_eq!(hint(cx, &post), "Runs after the response arrives.");
    let (_win, host) = host_section(cx, |w, cx| ScriptSection::new(&[Tests], w, cx));
    let t = host.read_with(cx, |h, _| h.view.clone());
    let tests = t.read_with(cx, |s, _| s.code(Tests).cloned()).unwrap();
    assert_eq!(hint(cx, &tests), "Add tests to check the response.");
}

#[gpui_kit::test]
fn the_body_hint_follows_the_mode(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&BodySpec::Json(String::new()), w, cx))
    })
    .unwrap();
    frame(cx, win);
    let code = s.read_with(cx, |s, _| s.code().clone());
    assert_eq!(hint(cx, &code), "Write the JSON body here.");
    for (mode, text) in [
        (BodyMode::Xml, "Write the XML body here."),
        (BodyMode::Text, "Write the body here."),
        (BodyMode::Sparql, "Write the SPARQL query here."),
        (BodyMode::Json, "Write the JSON body here."),
    ] {
        cx.update_window(win, |_, w, cx| {
            s.update(cx, |s, cx| s.set_mode(mode, w, cx))
        })
        .unwrap();
        frame(cx, win);
        assert_eq!(hint(cx, &code), text, "{mode:?}");
    }
}

#[gpui_kit::test]
fn the_docs_editor_invites_notes(cx: &mut TestAppContext) {
    let (_win, host) = host_section(cx, DocsSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let code = s.read_with(cx, |s, _| s.code().clone());
    assert_eq!(
        hint(cx, &code),
        "Write notes about this request. Markdown works."
    );
}

#[gpui_kit::test]
fn the_dock_empty_notes_centre_in_the_dock(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    for (tab, id) in [
        (DockTab::Console, "dock-console-none"),
        (DockTab::Network, "dock-network-none"),
    ] {
        dock.update(cx, |d, cx| d.set_tab(tab, cx));
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, _| {
            let area = w.find("console-dock").bounds();
            let title = w.within(id).find("empty-title").bounds();
            assert!(
                (title.center().x - area.center().x).abs() < px(2.),
                "{id} title {title:?} in {area:?}"
            );
        })
        .unwrap();
    }
}
