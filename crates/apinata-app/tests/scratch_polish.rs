//! First-run polish of the scratch pad: saving opens the request, the sidebar says what the
//! scratch is, the trust prompt is worded for it, a pasted curl clears the old response and
//! the app menu greys out what needs a collection.

use crate::support;

use apinata_app::app_menu::app_menu_disabled;
use apinata_app::script_gate::{trust_question, trust_question_for};
use apinata_app::send_model::SendState;
use apinata_app::tabs::TabKind;
use apinata_core::ItemPath;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

#[gpui_kit::test]
fn saving_a_scratch_draft_opens_the_saved_request_in_a_tab(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    let (from, draft) = active_draft(cx, &s.ws);
    let col = collection();
    let target = col.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.open(target.clone(), cx));
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| {
        w.save_draft_into(
            &from,
            draft,
            &target,
            ItemPath::root(),
            "Echo".into(),
            Vec::new(),
            cx,
        )
    });
    cx.run_until_parked();
    let active = s.ws.read_with(cx, |w, _| w.tabs().active().cloned());
    let active = active.expect("a tab is open after the save");
    assert_eq!(active.root, target);
    assert!(
        matches!(&active.kind, TabKind::Request(p) if !p.is_draft()),
        "the active tab shows the saved request, not a draft: {:?}",
        active.kind
    );
}

#[gpui_kit::test]
fn the_scratch_sidebar_carries_a_quiet_hint(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    assert!(shown(cx, s.win, "sidebar-scratch-hint"));
}

#[gpui_kit::test]
fn the_scratch_hint_shows_beside_a_real_collection(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    assert!(shown(cx, s.win, "sidebar-scratch-hint"));
}

#[gpui_kit::test]
fn a_real_collection_has_no_scratch_hint(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    assert!(!shown(cx, s.win, "sidebar-scratch-hint"));
}

#[test]
fn the_scratch_trust_question_does_not_say_collection() {
    let q = trust_question_for(true);
    let text = format!(
        "{} {} {}",
        q.title,
        q.detail.as_deref().unwrap_or(""),
        q.choices
            .iter()
            .map(|c| c.label.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    );
    assert!(!text.contains("collection"), "{text}");
    assert_eq!(q.choices.len(), 3);
    assert_eq!(q.default, 2);
}

#[test]
fn the_collection_trust_question_is_unchanged() {
    assert_eq!(
        trust_question_for(false).title.as_ref(),
        trust_question().title.as_ref()
    );
}

#[gpui_kit::test]
fn pasting_curl_into_a_draft_clears_the_previous_response(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    let tab = s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().id);
    let editor = editor_of(cx, &s, tab);
    give_result(cx, &editor);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    assert!(model.read_with(cx, |m, _| matches!(m.state(), SendState::Done(_))));
    cx.update_window(s.win, |_, w, cx| {
        editor.update(cx, |e, cx| {
            assert!(e.paste_url("curl http://127.0.0.1:9/p", w, cx))
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| matches!(m.state(), SendState::Idle)));
}

#[test]
fn the_scratch_menu_greys_out_collection_only_entries() {
    let off = app_menu_disabled(true, false);
    for want in [
        "Export collection to Postman",
        "Export collection to Postman (keep scripts as written)",
        "Export collection to OpenAPI",
        "Export collection to HTML documentation",
        "Collection settings",
        "Run collection",
        "Close collection",
    ] {
        assert!(off.contains(&want), "{want} is disabled: {off:?}");
    }
    assert!(!off.contains(&"Open collection"));
    assert!(app_menu_disabled(false, true).is_empty());
}

#[test]
fn outside_a_repository_the_menu_greys_out_the_repository_entries() {
    assert_eq!(
        app_menu_disabled(false, false),
        ["Switch branch…", "Show changes…"]
    );
}
