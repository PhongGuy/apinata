//! Pinned tabs: the pin icon on the strip, the palette and menu commands, the bulk closes
//! that skip them, drags that stay inside a group, and the saved session.

use crate::support;

use apinata_app::commands::registry;
use apinata_app::palette::palette_items;
use apinata_app::session::{CollectionSession, SavedTab, Session};
use apinata_app::tab_area::DraggedTab;
use apinata_app::tabs::{Side, TabId};
use gpui_kit::AppContext as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, TestAppContext};
use support::*;

fn four() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
        ("c.yml", &request("c", "http://x/c")),
        ("d.yml", &request("d", "http://x/d")),
    ])
}

fn opened(cx: &mut TestAppContext, col: &tempfile::TempDir) -> (Started, [TabId; 4]) {
    let s = started(cx, col);
    let ids = ["a.yml", "b.yml", "c.yml", "d.yml"].map(|n| open_tab(cx, &s.ws, &s.root, n));
    cx.run_until_parked();
    (s, ids)
}

fn run(cx: &mut TestAppContext, win: AnyWindowHandle, id: &str) {
    let cmd = registry().into_iter().find(|c| c.id == id).expect(id);
    let entry = palette_items()
        .into_iter()
        .find(|e| e.label == cmd.label)
        .unwrap_or_else(|| panic!("{id} is in the palette"));
    cx.update_window(win, |_, w, cx| {
        w.dispatch_action(entry.action.boxed_clone(), cx)
    })
    .unwrap();
    frame(cx, win);
}

fn activate(cx: &mut TestAppContext, s: &Started, id: TabId) {
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(id)));
}

fn pin(cx: &mut TestAppContext, s: &Started, id: TabId) {
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.set_pinned(id, true)));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_pin_and_unpin_commands_act_on_the_active_tab(cx: &mut TestAppContext) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    activate(cx, &s, ids[2]);
    run(cx, s.win, "tabs.pin");
    assert_eq!(tab_names(cx, &s.ws), ["c.yml", "a.yml", "b.yml", "d.yml"]);
    run(cx, s.win, "tabs.unpin");
    assert_eq!(tab_names(cx, &s.ws), ["c.yml", "a.yml", "b.yml", "d.yml"]);
    let pinned =
        s.ws.read_with(cx, |w, _| w.tabs().get(ids[2]).unwrap().pinned);
    assert!(!pinned);
}

#[gpui_kit::test]
fn a_pinned_tab_shows_a_pin_icon_instead_of_the_close_button(cx: &mut TestAppContext) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    assert!(present(cx, s.win, ("tab-close", 1usize)));
    assert!(!present(cx, s.win, ("tab-pin", 1usize)));
    pin(cx, &s, ids[1]);
    assert!(present(cx, s.win, ("tab-pin", 0usize)));
    assert!(!present(cx, s.win, ("tab-close", 0usize)));
    assert!(present(cx, s.win, ("tab-close", 1usize)), "others keep it");
}

#[gpui_kit::test]
fn clicking_the_pin_icon_unpins_the_tab(cx: &mut TestAppContext) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    pin(cx, &s, ids[1]);
    pin(cx, &s, ids[2]);
    cx.update_window(s.win, |_, w, cx| w.click(("tab-pin", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws), ["c.yml", "b.yml", "a.yml", "d.yml"]);
    assert!(present(cx, s.win, ("tab-close", 1usize)));
}

#[gpui_kit::test]
fn the_context_menu_offers_pin_tab_and_then_unpin_tab(cx: &mut TestAppContext) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    let label_at_end = |cx: &mut TestAppContext, tab: usize| {
        cx.update_window(s.win, |_, w, cx| w.right_click(("tab", tab), cx))
            .unwrap();
        frame(cx, s.win);
        let mut last = None;
        for ix in 0..8usize {
            let label = cx
                .update_window(s.win, |_, w, _| {
                    w.within("popup-menu")
                        .try_find(ix)
                        .and_then(|n| n.label().map(str::to_owned))
                })
                .unwrap();
            match label {
                Some(l) => last = Some((ix, l)),
                None => break,
            }
        }
        last.unwrap()
    };
    let (ix, label) = label_at_end(cx, 1);
    assert_eq!(label, "Pin tab");
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &s.ws), ["b.yml", "a.yml", "c.yml", "d.yml"]);
    let (_, label) = label_at_end(cx, 0);
    assert_eq!(label, "Unpin tab");
    let _ = ids;
}

#[gpui_kit::test]
fn close_all_skips_pinned_tabs(cx: &mut TestAppContext) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    pin(cx, &s, ids[1]);
    run(cx, s.win, "tabs.close_all");
    assert_eq!(tab_names(cx, &s.ws), ["b.yml"]);
}

#[gpui_kit::test]
fn close_others_keeps_the_active_tab_and_every_pinned_one(cx: &mut TestAppContext) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    pin(cx, &s, ids[0]);
    activate(cx, &s, ids[3]);
    run(cx, s.win, "tabs.close_others");
    assert_eq!(tab_names(cx, &s.ws), ["a.yml", "d.yml"]);
}

#[gpui_kit::test]
fn close_saved_skips_pinned_tabs(cx: &mut TestAppContext) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    pin(cx, &s, ids[2]);
    run(cx, s.win, "tabs.close_saved");
    assert_eq!(tab_names(cx, &s.ws), ["c.yml"]);
}

#[gpui_kit::test]
fn close_tab_still_closes_a_pinned_tab(cx: &mut TestAppContext) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    pin(cx, &s, ids[2]);
    activate(cx, &s, ids[2]);
    run(cx, s.win, "tabs.close");
    assert_eq!(tab_names(cx, &s.ws), ["a.yml", "b.yml", "d.yml"]);
}

#[gpui_kit::test]
fn the_dirty_prompt_lists_only_tabs_being_closed(cx: &mut TestAppContext) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    pin(cx, &s, ids[0]);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    run(cx, s.win, "tabs.close_all");
    assert_eq!(tab_names(cx, &s.ws), ["a.yml"]);
    assert!(!confirm_open(cx), "the pinned dirty tab is not asked about");
}

#[gpui_kit::test]
fn a_drop_across_the_pinned_boundary_changes_nothing_and_shows_no_indicator(
    cx: &mut TestAppContext,
) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    pin(cx, &s, ids[0]);
    let view = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let payload = DraggedTab::new(ids[2], s.root.clone(), "c".into());
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("tab-drop-indicator").is_some()
        })
        .unwrap()
    };
    view.update(cx, |v, cx| {
        v.hover_drop(&payload, ids[0], Some(Side::After), cx)
    });
    assert!(!shown(cx), "no landing place across the boundary");
    view.update(cx, |v, cx| v.drop_tab(&payload, ids[0], Side::After, cx));
    assert_eq!(tab_names(cx, &s.ws), ["a.yml", "b.yml", "c.yml", "d.yml"]);
    view.update(cx, |v, cx| {
        v.hover_drop(&payload, ids[3], Some(Side::After), cx)
    });
    assert!(shown(cx), "inside its own group");
}

#[gpui_kit::test]
fn pinned_tabs_are_saved_and_restored_first(cx: &mut TestAppContext) {
    let col = four();
    let (s, ids) = opened(cx, &col);
    pin(cx, &s, ids[2]);
    let session = s.ws.read_with(cx, |w, _| w.session().clone());
    let saved = session_value(cx, &session).collections[&s.root].clone();
    assert_eq!(saved.pinned, [SavedTab::Request("c.yml".into())]);

    let state = tempfile::tempdir().unwrap();
    let mut start = Session {
        open: vec![s.root.clone()],
        ..Session::default()
    };
    start.collections.insert(s.root.clone(), saved);
    let stored = session_at(cx, &state, start);
    let again = workspace(cx, &stored);
    again.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    assert_eq!(tab_names(cx, &again), ["c.yml", "a.yml", "b.yml", "d.yml"]);
    let flags: Vec<bool> = again.read_with(cx, |w, _| {
        w.tabs().entries().iter().map(|e| e.pinned).collect()
    });
    assert_eq!(flags, [true, false, false, false]);
}

#[gpui_kit::test]
fn a_session_saved_before_pinning_still_loads(cx: &mut TestAppContext) {
    let old = r#"{"tabs":["a.yml","b.yml"],"environment":null}"#;
    let parsed: CollectionSession = serde_json::from_str(old).unwrap();
    assert!(parsed.pinned.is_empty());
    let col = four();
    let (s, _) = opened(cx, &col);
    let session = s.ws.read_with(cx, |w, _| w.session().clone());
    let json = serde_json::to_string(&session_value(cx, &session).collections[&s.root]).unwrap();
    assert!(
        !json.contains("pinned"),
        "an empty list is not written: {json}"
    );
}
