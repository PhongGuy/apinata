use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_app::sidebar::SidebarEvent;
use apinata_app::sidebar::model::node_id;
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn keys(cx: &mut TestAppContext, h: &SidebarHarness) -> Vec<String> {
    h.view.read_with(cx, |s, _| {
        s.multi_keys()
            .into_iter()
            .map(|(_, p)| p.as_str().to_owned())
            .collect()
    })
}

fn plain(cx: &mut TestAppContext, h: &SidebarHarness, path: &str) {
    cx.update_window(h.win, |_, w, cx| w.click(row(path), cx))
        .unwrap();
    frame(cx, h.win);
}

fn focus(cx: &mut TestAppContext, h: &SidebarHarness) -> Option<String> {
    h.view.read_with(cx, |s, cx| {
        s.selected(cx).map(|(_, p)| p.as_str().to_owned())
    })
}

fn opened(events: &[SidebarEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            SidebarEvent::OpenItem { path, .. } => Some(path.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

#[gpui_kit::test]
fn secondary_click_seeds_with_the_focus_row_toggles_and_does_not_open(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    let seen = sidebar_events(cx, &h.view);
    plain(cx, &h, "a.yml");
    secondary_click(cx, h.win, row("c.yml"));
    assert_eq!(keys(cx, &h), ["a.yml", "c.yml"]);
    assert_eq!(
        focus(cx, &h).as_deref(),
        Some("c.yml"),
        "the clicked row takes focus"
    );
    assert_eq!(
        opened(&seen.borrow()),
        ["a.yml"],
        "only the plain click opened a tab"
    );
    secondary_click(cx, h.win, row("c.yml"));
    assert_eq!(keys(cx, &h), ["a.yml"]);
    assert!(!h.view.read_with(cx, |s, _| s.multi_active()));
    assert_eq!(
        focus(cx, &h).as_deref(),
        Some("a.yml"),
        "focus follows to the remaining member"
    );
}

#[gpui_kit::test]
fn shift_click_selects_the_visible_range_from_the_anchor(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    plain(cx, &h, "b.yml");
    shift_click(cx, h.win, row("d.yml"));
    assert_eq!(keys(cx, &h), ["b.yml", "c.yml", "d.yml"]);
    shift_click(cx, h.win, row("c.yml"));
    assert_eq!(keys(cx, &h), ["b.yml", "c.yml"], "the anchor stayed on b");
}

#[gpui_kit::test]
fn a_plain_click_clears_the_set_and_opens(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    plain(cx, &h, "a.yml");
    secondary_click(cx, h.win, row("c.yml"));
    let seen = sidebar_events(cx, &h.view);
    plain(cx, &h, "d.yml");
    assert!(keys(cx, &h).is_empty());
    assert_eq!(opened(&seen.borrow()), ["d.yml"]);
}

#[gpui_kit::test]
fn opaque_rows_join_the_set_and_a_folder_toggles_without_expanding(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    plain(cx, &h, "a.yml");
    secondary_click(cx, h.win, row("w.yml"));
    secondary_click(cx, h.win, row("f"));
    assert_eq!(keys(cx, &h), ["a.yml", "f", "w.yml"]);
    assert!(
        !shown(cx, h.win, row("f/x.yml")),
        "a modifier click does not expand the folder"
    );
}

#[gpui_kit::test]
fn modifier_clicks_on_collection_and_example_rows_change_nothing(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    plain(cx, &h, "a.yml");
    secondary_click(cx, h.win, row("b.yml"));
    let collection = node_id(0, &ItemPath::root());
    secondary_click(cx, h.win, collection.clone());
    shift_click(cx, h.win, collection);
    assert_eq!(keys(cx, &h), ["a.yml", "b.yml"]);
    assert_eq!(
        focus(cx, &h).as_deref(),
        Some("b.yml"),
        "the kit did not move the focus row"
    );
}

#[gpui_kit::test]
fn a_plain_click_on_the_collection_row_ends_the_set(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    plain(cx, &h, "a.yml");
    secondary_click(cx, h.win, row("b.yml"));
    cx.update_window(h.win, |_, w, cx| w.click(node_id(0, &ItemPath::root()), cx))
        .unwrap();
    frame(cx, h.win);
    assert!(keys(cx, &h).is_empty());
}

#[gpui_kit::test]
fn the_set_survives_an_unrelated_rebuild_and_follows_a_rename(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    plain(cx, &h, "a.yml");
    secondary_click(cx, h.win, row("b.yml"));
    // A change elsewhere rebuilds the tree and clears the kit's selection for a moment.
    h.ws.update(cx, |w, cx| {
        w.send(&h.root, edit_url("d.yml", "http://x/9"), cx)
    });
    frame(cx, h.win);
    assert_eq!(keys(cx, &h), ["a.yml", "b.yml"]);
    h.ws.update(cx, |w, cx| {
        w.send(
            &h.root,
            Command::Tree(TreeOp::Rename {
                item: ip("a.yml"),
                name: "Z".into(),
                rename_file: true,
            }),
            cx,
        )
    });
    frame(cx, h.win);
    let now = keys(cx, &h);
    assert_eq!(now.len(), 2, "{now:?}");
    assert!(
        now.contains(&"b.yml".to_owned()) && !now.contains(&"a.yml".to_owned()),
        "{now:?}"
    );
    assert!(
        now.iter().all(|p| h.root.join(p).exists()),
        "{now:?} follow the file"
    );
}

#[gpui_kit::test]
fn deleted_rows_leave_the_set_and_a_closed_collection_empties_it(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    plain(cx, &h, "a.yml");
    secondary_click(cx, h.win, row("b.yml"));
    secondary_click(cx, h.win, row("c.yml"));
    h.ws.update(cx, |w, cx| {
        w.send(
            &h.root,
            Command::Tree(TreeOp::Delete { item: ip("b.yml") }),
            cx,
        )
    });
    frame(cx, h.win);
    assert_eq!(keys(cx, &h), ["a.yml", "c.yml"]);
    h.ws.update(cx, |w, cx| w.close(&h.root, cx));
    frame(cx, h.win);
    assert!(keys(cx, &h).is_empty());
}

#[gpui_kit::test]
fn modifier_clicks_on_an_invalid_row_change_nothing(cx: &mut TestAppContext) {
    let mut files = multi_files();
    files.push(("bad.yml", "not: [a request".to_string()));
    let h = open_sidebar_with(cx, files);
    plain(cx, &h, "a.yml");
    secondary_click(cx, h.win, row("b.yml"));
    let bad = apinata_app::sidebar::model::invalid_id(0, 0);
    assert!(shown(cx, h.win, bad.clone()), "the invalid row is listed");
    secondary_click(cx, h.win, bad.clone());
    shift_click(cx, h.win, bad);
    assert_eq!(keys(cx, &h), ["a.yml", "b.yml"]);
    assert_eq!(focus(cx, &h).as_deref(), Some("b.yml"), "the focus stayed");
}

#[gpui_kit::test]
fn modifier_clicks_on_a_draft_row_change_nothing(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    let seen = sidebar_events(cx, &h.view);
    h.ws.update(cx, |w, cx| w.new_draft(&h.root, ItemPath::root(), cx))
        .expect("open");
    cx.run_until_parked();
    let draft = h.ws.read_with(cx, |w, _| {
        let e = w.tabs().active().expect("the draft tab");
        e.item().expect("a request").as_str().to_owned()
    });
    assert!(shown(cx, h.win, row(&draft)), "the draft row is listed");
    plain(cx, &h, "a.yml");
    secondary_click(cx, h.win, row("b.yml"));
    let before = opened(&seen.borrow()).len();
    secondary_click(cx, h.win, row(&draft));
    shift_click(cx, h.win, row(&draft));
    assert_eq!(keys(cx, &h), ["a.yml", "b.yml"]);
    assert_eq!(focus(cx, &h).as_deref(), Some("b.yml"), "the focus stayed");
    assert_eq!(opened(&seen.borrow()).len(), before, "no tab was opened");
}
