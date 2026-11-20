//! The update form offers the source the collection was last imported or updated from.

use crate::import_collection::create_in;
use crate::import_update::{Rig, V1, V2, applied, preview, rig, serve};
use crate::import_update_reference::reference_only;
use crate::support::*;

use apinata_app::import_link;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use std::fs;

fn collections(cx: &mut TestAppContext, r: &Rig) -> usize {
    r.shell
        .read_with(cx, |s, cx| s.workspace().read(cx).collections().len())
}

/// Applies the previewed update and closes the dialog on its result.
fn apply_and_close(
    cx: &mut TestAppContext,
    r: &Rig,
    d: &gpui_kit::Entity<import_link::ImportLinkDialog>,
) {
    preview(cx, d);
    r.click(cx, "import-update-apply");
    applied(cx, d);
    cx.update_window(r.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, r.win);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_collection_imported_from_a_link_offers_that_link_on_update(cx: &mut TestAppContext) {
    let r = rig(cx);
    let server = serve(cx, V1);
    let url = format!("{}/spec.json", server.uri());
    switcher_click(cx, r.win, "switcher-import-collection");
    frame(cx, r.win);
    let d = cx.read(import_link::current).expect("dialog open");
    let link = url.clone();
    cx.update_window(r.win, |_, w, cx| {
        d.update(cx, |d, cx| d.set_link(&link, w, cx))
    })
    .unwrap();
    create_in(cx, r.parent.path());
    r.click(cx, "import-link-import");
    wait_for(cx, "the import to open", |cx| collections(cx, &r) == 1);
    r.click(cx, "import-report-close");
    let d = r.dialog(cx, "");
    r.click(cx, "import-update-target-0");
    assert_eq!(d.read_with(cx, |d, cx| d.link(cx)), url);
}

#[gpui_kit::test]
fn the_reference_source_is_offered_again_after_the_first_update(cx: &mut TestAppContext) {
    let r = rig(cx);
    let root = reference_only(r.parent.path(), "spec.json", "path");
    fs::write(root.join("spec.json"), V2).unwrap();
    r.open(cx, root.clone());
    let d = r.dialog(cx, "");
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    apply_and_close(cx, &r, &d);
    let d = r.dialog(cx, "");
    r.click(cx, "import-update-target-0");
    assert_eq!(
        d.read_with(cx, |d, cx| d.link(cx)),
        root.join("spec.json").to_string_lossy()
    );
}

#[gpui_kit::test]
fn a_typed_link_used_for_an_update_is_offered_on_the_next_one(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.open_v1(cx);
    let server = serve(cx, V2);
    let url = format!("{}/spec.json", server.uri());
    let d = r.dialog(cx, &url);
    r.click(cx, "import-update-target-0");
    r.click(cx, "import-link-import");
    apply_and_close(cx, &r, &d);
    let d = r.dialog(cx, "");
    r.click(cx, "import-update-target-0");
    assert_eq!(d.read_with(cx, |d, cx| d.link(cx)), url);
}
