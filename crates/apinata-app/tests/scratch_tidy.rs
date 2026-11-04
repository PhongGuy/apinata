//! Scratch pad checks that go through the drawn UI: the trust prompt a scratch send opens and
//! the app menu rows greyed out while only the scratch pad is open.

use crate::support;

use apinata_app::app_menu::app_menu_index;
use apinata_app::command::{Command, Edit, Target};
use apinata_app::keymap::{NewRequest, RunCollection};
use apinata_app::shell::ExportCollection;
use apinata_core::{ScriptDecision, ScriptPhase, ScriptTrust};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Action, AppContext as _, TestAppContext};
use support::*;

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

// A scratch send reaches `script_gate::prepare` with the scratch directory set, so the
// question it opens is the draft's, and a Never answer is stored under the scratch root.
#[gpui_kit::test]
fn a_scratch_send_with_a_script_asks_in_the_drafts_words(cx: &mut TestAppContext) {
    http_ready(cx);
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    let (root, item) = active_draft(cx, &s.ws);
    for edit in [
        Edit::Url("http://127.0.0.1:9/x".into()),
        Edit::Script(ScriptPhase::PreRequest, "let a = 1".into()),
    ] {
        s.ws.update(cx, |w, cx| {
            w.send(
                &root,
                Command::Edit {
                    target: Target::Request(item.clone()),
                    edit,
                },
                cx,
            )
        });
    }
    cx.run_until_parked();
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&root, item, cx));
    wait_real(cx, |cx| {
        cx.run_until_parked();
        confirm_open(cx)
    });
    let win = confirm_window(cx).expect("the trust question is open");
    let detail = cx
        .update_window(win, |_, w, _| {
            w.find("script-trust-never").label().map(str::to_owned)
        })
        .unwrap()
        .expect("the button has a label");
    assert_eq!(detail, "Never for the scratch pad");
    answer_confirm(cx, "script-trust-never");
    let store = ScriptTrust::load(s.files.path().join("script-trust.json")).unwrap();
    assert_eq!(store.decision(&root), Some(ScriptDecision::Blocked));
}

// The kit's menu does not expose the disabled flag, so a click on the drawn row is the check:
// a greyed row does nothing, while the same click on an enabled row still acts.
#[gpui_kit::test]
fn the_drawn_app_menu_greys_out_collection_entries_for_the_scratch(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("app-menu", cx)
    })
    .unwrap();
    frame(cx, s.win);
    let tabs = |cx: &mut TestAppContext| s.ws.read_with(cx, |w, _| w.tabs().entries().len());
    for action in [&ExportCollection as &dyn Action, &RunCollection] {
        let before = (toasts(cx, &s), tabs(cx));
        let ix = app_menu_index(action).unwrap();
        cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            before,
            (toasts(cx, &s), tabs(cx)),
            "{} does nothing",
            action.name()
        );
    }
    let ix = app_menu_index(&NewRequest).unwrap();
    let before = tabs(cx);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(tabs(cx), before + 1, "an enabled row still acts");
}
