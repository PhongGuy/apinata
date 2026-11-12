//! The sidebar at its real 240 px width: long labels stay inside their row and the panel.

use crate::support;

use apinata_app::sidebar::marker_id;
use apinata_app::sidebar::model::invalid_id;
use apinata_app::style::{ROW_HEIGHT, at_zoom};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

#[gpui_kit::test]
fn invalid_file_rows_stay_one_line_inside_their_row(cx: &mut TestAppContext) {
    let broken = "info: [\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        (
            "a broken file whose name alone is longer than the sidebar.yml",
            broken,
        ),
        ("another broken file with a long name.yml", broken),
    ]);
    let s = started(cx, &col);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let sidebar = w.find("sidebar").bounds();
        let mut above = None;
        for k in 0..2 {
            let row = w.find(invalid_id(0, k)).bounds();
            let label = w.find(marker_id("invalid", &invalid_id(0, k))).bounds();
            assert_eq!(row.size.height, at_zoom(ROW_HEIGHT, 1.0), "row {k}");
            assert!(
                label.top() >= row.top() && label.bottom() <= row.bottom(),
                "label {k} {label:?} spills out of its row {row:?}"
            );
            assert!(
                label.right() <= sidebar.right(),
                "label {k} {label:?} past {sidebar:?}"
            );
            if let Some(prev) = above {
                assert!(label.top() >= prev, "label {k} overlaps the one above");
            }
            above = Some(label.bottom());
        }
    })
    .unwrap();
}

const LONG_ROOT: &str =
    "opencollection: 1.0.0\ninfo:\n  name: A really quite long collection name for the sidebar\n";

#[gpui_kit::test]
fn a_long_collection_name_leaves_its_buttons_in_the_sidebar(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", LONG_ROOT),
        ("a.yml", &request("a", "http://x/0")),
    ]);
    let s = started(cx, &col);
    frame(cx, s.win);
    // The actions take their room only while shown, as on hover.
    cx.update_window(s.win, |_, w, cx| w.hover(row(""), cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let sidebar = w.find("sidebar").bounds();
        let name = w.find(("collection", 0usize)).bounds();
        let new_request = w.find(("new-request", 0usize)).bounds();
        let new_folder = w.find(("new-folder", 0usize)).bounds();
        for (what, b) in [("new request", new_request), ("new folder", new_folder)] {
            assert!(
                b.left() >= sidebar.left() && b.right() <= sidebar.right(),
                "{what} {b:?} outside {sidebar:?}"
            );
        }
        assert!(
            name.right() <= new_request.left(),
            "name {name:?} under the buttons"
        );
    })
    .unwrap();
}
