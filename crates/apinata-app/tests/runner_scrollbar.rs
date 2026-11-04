//! The run table shows a vertical scrollbar while its rows overflow the pane, and none when
//! they fit.

use crate::support;

use apinata_app::runner_model::RunState;
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px, size};
use support::*;
use wiremock::ResponseTemplate;

// A collection of `n` requests against `uri`, run to the end in a runner tab.
fn finished_run(cx: &mut TestAppContext, n: usize) -> Started {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("ok"));
    let mut files = vec![("opencollection.yml".to_owned(), ROOT.to_owned())];
    for i in 0..n {
        files.push((
            format!("r{i:02}.yml"),
            request(&format!("r{i:02}"), &format!("{}/ok", server.uri())),
        ));
    }
    let tree: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let col = write_tree(&tree);
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("run-start", cx))
        .unwrap();
    cx.run_until_parked();
    let model = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view")
        .read_with(cx, |v, _| v.model().clone());
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    // Keep the collection and server alive until the end of the test.
    std::mem::forget((col, server));
    s
}

fn bar(cx: &mut TestAppContext, s: &Started) -> bool {
    frame(cx, s.win);
    frame(cx, s.win);
    present(cx, s.win, "run-table-scrollbar-y")
}

#[gpui_kit::test]
fn the_run_table_scrolls_with_a_bar_only_while_rows_overflow(cx: &mut TestAppContext) {
    let s = finished_run(cx, 40);
    cx.simulate_window_resize(s.win, size(px(900.), px(2400.)));
    assert!(!bar(cx, &s), "a tall window fits 40 rows");
    cx.simulate_window_resize(s.win, size(px(900.), px(400.)));
    assert!(bar(cx, &s), "a short window overflows the table");
}
