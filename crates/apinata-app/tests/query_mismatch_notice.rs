//! A request file whose URL query and params block disagree shows a notice above the Params
//! table, and either of its buttons settles the request in memory.

use crate::support;

use apinata_app::command::Command;
use apinata_core::{Param, ParamKind};
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

const BASE: &str = "http://127.0.0.1:9/a";

fn file(url: &str, params: &[(&str, &str)]) -> String {
    let block: String = params
        .iter()
        .map(|(n, v)| format!("    - name: {n}\n      value: \"{v}\"\n      type: query\n"))
        .collect();
    let block = if block.is_empty() {
        " []\n".to_owned()
    } else {
        format!("\n{block}")
    };
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{BASE}{url}\"\n  params:{block}"
    )
}

fn no_block(url: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{BASE}{url}\"\n"
    )
}

// The app over one file `a.yml`, open in a tab.
fn opened(cx: &mut TestAppContext, text: &str) -> (tempfile::TempDir, Started) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", text)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    (col, s)
}

fn notice(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    use gpui_kit::test::TestWindowExt as _;
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("params-mismatch")
            .map(|n| n.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}

fn z(value: &str) -> Vec<Param> {
    vec![Param::new("z", value, ParamKind::Query)]
}

#[gpui_kit::test]
fn a_disagreeing_file_names_the_param_above_the_table(cx: &mut TestAppContext) {
    let (_col, s) = opened(cx, &file("?z=6", &[("z", "3")]));
    assert_eq!(
        notice(cx, &s).as_deref(),
        Some("URL and table disagree on z. The URL's query is sent.")
    );
    assert!(present(cx, s.win, "params-keep-url"));
    assert!(present(cx, s.win, "params-keep-table"));
}

#[gpui_kit::test]
fn past_three_names_the_rest_are_counted(cx: &mut TestAppContext) {
    let (_col, s) = opened(cx, &file("?z=1&a=1&b=1&c=1&d=1", &[]));
    assert_eq!(
        notice(cx, &s).as_deref(),
        Some("URL and table disagree on z, a, b and 2 more. The URL's query is sent.")
    );
}

#[gpui_kit::test]
fn two_names_are_joined_with_and(cx: &mut TestAppContext) {
    let (_col, s) = opened(cx, &file("?z=1", &[("a", "1")]));
    assert_eq!(
        notice(cx, &s).as_deref(),
        Some("URL and table disagree on z and a. The URL's query is sent.")
    );
}

#[gpui_kit::test]
fn an_agreeing_file_shows_no_notice(cx: &mut TestAppContext) {
    let (_col, s) = opened(cx, &file("?a=1&z=6", &[("z", "6"), ("a", "1")]));
    assert_eq!(notice(cx, &s), None);
}

#[gpui_kit::test]
fn a_file_without_a_params_block_shows_no_notice(cx: &mut TestAppContext) {
    let (_col, s) = opened(cx, &no_block("?z=6"));
    assert_eq!(notice(cx, &s), None);
}

#[gpui_kit::test]
fn keep_url_fills_the_table_from_the_url(cx: &mut TestAppContext) {
    let text = file("?z=6", &[("z", "3")]);
    let (col, s) = opened(cx, &text);
    click(cx, &s, "params-keep-url");
    let view = view_of(cx, &s.ws, &s.root, "a.yml");
    assert_eq!(view.url, format!("{BASE}?z=6"));
    assert_eq!(view.params, z("6"));
    assert!(view.dirty, "settling is an unsaved edit");
    assert_eq!(notice(cx, &s), None);
    wait_save(cx);
    let on_disk = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert_eq!(on_disk, text, "nothing is written until Save");
}

#[gpui_kit::test]
fn keep_table_rebuilds_the_url_query_from_the_table(cx: &mut TestAppContext) {
    let text = file("?z=6", &[("z", "3")]);
    let (col, s) = opened(cx, &text);
    click(cx, &s, "params-keep-table");
    let view = view_of(cx, &s.ws, &s.root, "a.yml");
    assert_eq!(view.url, format!("{BASE}?z=3"));
    let tab =
        s.ws.read_with(cx, |w, _| w.tabs().active().expect("a tab").id);
    let bar = editor_of(cx, &s, tab).read_with(cx, |e, cx| e.url(cx));
    assert_eq!(
        bar,
        format!("{BASE}?z=3"),
        "the URL bar shows the rebuilt query"
    );
    assert_eq!(view.params, z("3"));
    assert!(view.dirty, "settling is an unsaved edit");
    assert_eq!(notice(cx, &s), None);
    wait_save(cx);
    let on_disk = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert_eq!(on_disk, text, "nothing is written until Save");
}

#[gpui_kit::test]
fn a_disagreement_written_outside_shows_after_reload(cx: &mut TestAppContext) {
    let (col, s) = opened(cx, &file("?z=1", &[("z", "1")]));
    assert_eq!(notice(cx, &s), None);
    let path = col.path().join("a.yml");
    std::fs::write(&path, file("?z=6", &[("z", "3")])).unwrap();
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::Reload(vec![path]), cx));
    cx.run_until_parked();
    assert_eq!(
        notice(cx, &s).as_deref(),
        Some("URL and table disagree on z. The URL's query is sent.")
    );
}
