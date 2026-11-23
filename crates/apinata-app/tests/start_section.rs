use crate::support;

use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::tabs::TabId;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;

const G: &str = "info:\n  name: g\n  type: graphql\n  seq: 2\ngraphql:\n  method: POST\n  url: \"http://x/g\"\n  body:\n    query: \"{ a }\"\n";

fn area(cx: &mut TestAppContext) -> Area {
    open_area_of(
        cx,
        write_tree(&[
            ("opencollection.yml", ROOT),
            ("h.yml", &request("h", "http://x/h")),
            ("g.yml", G),
        ]),
    )
}

// Opens `rel` as the active tab and returns the editor the tab area made for it.
fn show(cx: &mut TestAppContext, a: &Area, rel: &str) -> (TabId, Entity<RequestEditor>) {
    let tab = open_tab(cx, &a.ws, &a.root, rel);
    cx.run_until_parked();
    cx.update_window(a.win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let ed = a.view.read_with(cx, |v, _| v.editor(tab).cloned());
    (tab, ed.expect("an editor for the tab"))
}

#[gpui_kit::test]
fn a_graphql_request_opens_on_its_query(cx: &mut TestAppContext) {
    let a = area(cx);
    let (_, ed) = show(cx, &a, "g.yml");
    assert_eq!(ed.read_with(cx, |e, _| e.section()), Section::Body);
}

#[gpui_kit::test]
fn an_http_request_opens_on_its_params(cx: &mut TestAppContext) {
    let a = area(cx);
    let (_, ed) = show(cx, &a, "h.yml");
    assert_eq!(ed.read_with(cx, |e, _| e.section()), Section::Params);
}

#[gpui_kit::test]
fn a_picked_section_survives_a_tab_switch(cx: &mut TestAppContext) {
    let a = area(cx);
    let (_, ed) = show(cx, &a, "g.yml");
    ed.update(cx, |e, cx| e.show(Section::Params, cx));
    show(cx, &a, "h.yml");
    let (_, back) = show(cx, &a, "g.yml");
    assert_eq!(back, ed);
    assert_eq!(back.read_with(cx, |e, _| e.section()), Section::Params);
}
