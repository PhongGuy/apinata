use crate::support;

use apinata_app::sidebar::menu::{BulkEntry, bulk_entries};
use apinata_app::sidebar::multi::Tally;
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

fn right_click(cx: &mut TestAppContext, h: &SidebarHarness, path: &str) {
    cx.update_window(h.win, |_, w, cx| {
        w.render_frame(cx);
        w.right_click(row(path), cx)
    })
    .unwrap();
    frame(cx, h.win);
}

#[test]
fn bulk_labels_count_and_pluralise() {
    let items = |n| Tally {
        requests: n,
        ..Tally::default()
    };
    assert_eq!(BulkEntry::Delete(items(3)).label(), "Delete 3 requests");
    assert_eq!(BulkEntry::Delete(items(1)).label(), "Delete 1 request");
    assert_eq!(BulkEntry::Run(2).label(), "Run 2 requests");
    assert_eq!(BulkEntry::Run(1).label(), "Run 1 request");
    assert_eq!(
        bulk_entries(items(3), None),
        [
            BulkEntry::Copy(3),
            BulkEntry::Cut(3),
            BulkEntry::Delete(items(3))
        ]
    );
    assert_eq!(
        bulk_entries(items(3), Some(5)),
        [
            BulkEntry::Run(5),
            BulkEntry::Copy(3),
            BulkEntry::Cut(3),
            BulkEntry::Delete(items(3))
        ]
    );
}

// A folder counts as one folder, never as what is inside it, and the label names it.
#[test]
fn the_delete_label_names_folders_apart_from_requests() {
    let t = |folders, requests, others| Tally {
        folders,
        requests,
        others,
    };
    let label = |t| BulkEntry::Delete(t).label();
    assert_eq!(label(t(2, 4, 0)), "Delete 2 folders and 4 requests");
    assert_eq!(label(t(1, 1, 0)), "Delete 1 folder and 1 request");
    assert_eq!(label(t(1, 0, 0)), "Delete 1 folder");
    assert_eq!(label(t(3, 0, 0)), "Delete 3 folders");
    assert_eq!(label(t(1, 1, 1)), "Delete 1 folder and 2 items");
    assert_eq!(label(t(0, 2, 1)), "Delete 3 items");
    assert_eq!(label(t(0, 4, 0)), "Delete 4 requests");
    assert_eq!(label(t(0, 0, 2)), "Delete 2 items");
    assert_eq!(t(2, 4, 1).total(), 7);
    assert_eq!(
        bulk_entries(t(1, 2, 0), None),
        [
            BulkEntry::Copy(3),
            BulkEntry::Cut(3),
            BulkEntry::Delete(t(1, 2, 0))
        ]
    );
}

// A shift range from e's request to f's first one crosses the f row: Run counts every request
// it will run, f's unselected one too, and Delete names the folder instead of counting it.
#[gpui_kit::test]
fn a_range_over_a_folder_row_labels_run_and_delete_honestly(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, range_files());
    for f in ["e", "f"] {
        cx.update_window(h.win, |_, w, cx| w.click(row(f), cx))
            .unwrap(); // expands the folder
    }
    cx.update_window(h.win, |_, w, cx| w.click(row("e/y.yml"), cx))
        .unwrap();
    shift_click(cx, h.win, row("f/x.yml"));
    assert_eq!(keys(cx, &h), ["e/y.yml", "f", "f/x.yml"]);
    right_click(cx, &h, "e/y.yml");
    let labels = cx
        .update_window(h.win, |_, w, _| {
            menu_labels(w).into_iter().flatten().collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(
        labels,
        [
            "Run 3 requests",
            "Copy 2 items",
            "Cut 2 items",
            "Delete 1 folder and 1 request"
        ]
    );
}

#[gpui_kit::test]
fn right_click_inside_the_set_opens_the_bulk_menu_and_keeps_the_set(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("c.yml"));
    right_click(cx, &h, "a.yml");
    assert_eq!(keys(cx, &h), ["a.yml", "c.yml"]);
    click_menu(cx, h.win, "Delete 2 requests");
    assert!(confirm_open(cx), "Delete 2 requests asks first");
    answer_confirm(cx, "confirm-delete");
    frame(cx, h.win);
    assert!(!h.root.join("a.yml").exists() && !h.root.join("c.yml").exists());
    assert!(h.root.join("b.yml").exists());
}

#[gpui_kit::test]
fn right_click_outside_the_set_clears_it_and_opens_the_row_menu(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, multi_files());
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("c.yml"));
    right_click(cx, &h, "b.yml");
    assert!(keys(cx, &h).is_empty());
    click_menu(cx, h.win, "Delete");
    answer_confirm(cx, "confirm-delete");
    frame(cx, h.win);
    assert!(
        !h.root.join("b.yml").exists(),
        "the single-row menu deleted b"
    );
    assert!(h.root.join("a.yml").exists() && h.root.join("c.yml").exists());
}

#[gpui_kit::test]
fn closing_a_root_drops_the_batches_sent_to_it(cx: &mut TestAppContext) {
    use apinata_app::sidebar::batch::{Batch, Verb};
    let h = open_sidebar_with(cx, multi_files());
    let other = write_tree(&[
        ("opencollection.yml", ROOT),
        ("o.yml", &request("O", "http://x/o")),
    ]);
    let other_root = other.path().to_path_buf();
    h.ws.update(cx, |w, cx| w.open(other_root.clone(), cx));
    frame(cx, h.win);
    h.view.update(cx, |s, _| {
        s.push_batch(Batch::new(
            Verb::Delete,
            h.root.clone(),
            0,
            vec![(1, ip("a.yml"))],
        ));
        s.push_batch(Batch::new(
            Verb::Delete,
            other_root.clone(),
            1,
            vec![(1, ip("o.yml"))],
        ));
    });
    assert_eq!(h.view.read_with(cx, |s, _| s.batch_roots()).len(), 2);
    h.ws.update(cx, |w, cx| w.close(&other_root, cx));
    frame(cx, h.win);
    assert_eq!(
        h.view.read_with(cx, |s, _| s.batch_roots()),
        std::slice::from_ref(&h.root),
        "the closed root's batch is gone"
    );
}

/// At 1000x480 the two-entry bulk menu (Run, Delete) opens whole inside the window.
#[gpui_kit::test]
fn the_four_entry_bulk_menu_fits_a_1000x480_window(cx: &mut TestAppContext) {
    use gpui_kit::{VisualTestContext, px, size};
    let h = open_sidebar_with(cx, multi_files());
    VisualTestContext::from_window(h.win, cx).simulate_resize(size(px(1000.), px(480.)));
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    secondary_click(cx, h.win, row("c.yml"));
    right_click(cx, &h, "a.yml");
    let (labels, menu, window) = cx
        .update_window(h.win, |_, w, _| {
            let labels: Vec<String> = menu_labels(w).into_iter().flatten().collect();
            (labels, w.find("popup-menu").bounds(), w.viewport_size())
        })
        .unwrap();
    assert_eq!(
        labels,
        [
            "Run 2 requests",
            "Copy 2 items",
            "Cut 2 items",
            "Delete 2 requests"
        ]
    );
    assert!(
        menu.left() >= px(0.)
            && menu.top() >= px(0.)
            && menu.right() <= window.width
            && menu.bottom() <= window.height,
        "menu {menu:?} outside the window {window:?}"
    );
}
