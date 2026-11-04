use crate::support;

use apinata_app::kv_table::KvTable;
use apinata_app::request_editor::{RequestEditor, Section};
use gpui_kit::AppContext as _;
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, TestAppContext};
use support::*;

const VARS_ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  variables:\n    - name: host\n      value: c.test\n";
const REQ: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"https://x.test/\"\n  headers:\n    - name: X-A\n      value: \"\"\n  params:\n    - name: q\n      value: \"\"\n      type: query\n";

struct Opened {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    win: AnyWindowHandle,
    ed: Entity<RequestEditor>,
}

fn opened(cx: &mut TestAppContext) -> Opened {
    let col = write_tree(&[("opencollection.yml", VARS_ROOT), ("a.yml", REQ)]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    wait_for(cx, "variables", |cx| {
        ed.read_with(cx, |e, _| e.variables().is_some())
    });
    Opened {
        _col: col,
        _state: state,
        win,
        ed,
    }
}

fn table(cx: &mut TestAppContext, o: &Opened, section: Section) -> Entity<KvTable> {
    cx.update_window(o.win, |_, w, cx| w.click(("section", section as usize), cx))
        .unwrap();
    cx.run_until_parked();
    o.ed.read_with(cx, |e, _| e.table(section).cloned())
        .unwrap()
}

fn focus_end(cx: &mut TestAppContext, win: AnyWindowHandle, input: &Entity<InputState>) {
    cx.update_window(win, |_, w, cx| {
        input.update(cx, |s, cx| s.focus(w, cx));
        w.render_frame(cx);
        w.press("end", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn keys(cx: &mut TestAppContext, win: AnyWindowHandle, text: &str, then: &[&str]) {
    cx.update_window(win, |_, w, cx| {
        w.input(text, cx);
        for k in then {
            w.render_frame(cx);
            w.press(k, cx);
        }
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The hint list's row labels, or `None` while it is closed.
fn offered(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<Vec<String>> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("hint-complete")?;
        let mut found = Vec::new();
        while let Some(row) = w.try_find(("hint-complete-row", found.len())) {
            found.push(row.label().unwrap_or_default().to_owned());
        }
        Some(found)
    })
    .unwrap()
}

fn listed_vars(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("var-complete").is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_header_name_completes_to_canonical_casing_without_adding_rows(cx: &mut TestAppContext) {
    let o = opened(cx);
    let t = table(cx, &o, Section::Headers);
    let name = t.read_with(cx, |t, _| t.name_input(1).cloned().expect("blank row"));
    focus_end(cx, o.win, &name);
    keys(cx, o.win, "con", &[]);
    let got = offered(cx, o.win).expect("a list");
    assert!(got.contains(&"Content-Type".to_owned()), "{got:?}");
    assert!(got.contains(&"Content-Length".to_owned()), "{got:?}");
    let rows = t.read_with(cx, |t, cx| t.rows(cx).len());
    keys(cx, o.win, "", &["enter"]);
    assert!(offered(cx, o.win).is_none());
    assert_eq!(
        name.read_with(cx, |s, _| s.value().to_string()),
        "Content-Type"
    );
    // The typed-in blank row became exactly one real row; accepting added none.
    assert_eq!(rows, 2);
    assert_eq!(t.read_with(cx, |t, cx| t.rows(cx).len()), rows);
}

#[gpui_kit::test]
fn tab_accepts_a_header_name_too(cx: &mut TestAppContext) {
    let o = opened(cx);
    let t = table(cx, &o, Section::Headers);
    let name = t.read_with(cx, |t, _| t.name_input(1).cloned().unwrap());
    focus_end(cx, o.win, &name);
    keys(cx, o.win, "auth", &["tab"]);
    assert_eq!(
        name.read_with(cx, |s, _| s.value().to_string()),
        "Authorization"
    );
}

#[gpui_kit::test]
fn a_value_cell_offers_the_values_of_the_row_name(cx: &mut TestAppContext) {
    let o = opened(cx);
    let t = table(cx, &o, Section::Headers);
    let name = t.read_with(cx, |t, _| t.name_input(1).cloned().unwrap());
    focus_end(cx, o.win, &name);
    keys(cx, o.win, "content-type", &["enter"]);
    let value = t.read_with(cx, |t, _| t.value_input(1).cloned().unwrap());
    focus_end(cx, o.win, &value);
    keys(cx, o.win, "app", &[]);
    let got = offered(cx, o.win).expect("a list");
    assert_eq!(got[0], "application/json");
    keys(cx, o.win, "", &["enter"]);
    assert_eq!(
        value.read_with(cx, |s, _| s.value().to_string()),
        "application/json"
    );
}

#[gpui_kit::test]
fn a_placeholder_still_opens_the_variable_list_not_the_header_list(cx: &mut TestAppContext) {
    let o = opened(cx);
    let t = table(cx, &o, Section::Headers);
    let value = t.read_with(cx, |t, _| t.value_input(0).cloned().unwrap());
    focus_end(cx, o.win, &value);
    keys(cx, o.win, "{{ho", &[]);
    assert!(listed_vars(cx, o.win));
    assert!(offered(cx, o.win).is_none());
}

#[gpui_kit::test]
fn the_params_table_offers_no_header_hints(cx: &mut TestAppContext) {
    let o = opened(cx);
    let t = table(cx, &o, Section::Params);
    let name = t.read_with(cx, |t, _| t.name_input(1).cloned().unwrap());
    focus_end(cx, o.win, &name);
    keys(cx, o.win, "con", &[]);
    assert!(offered(cx, o.win).is_none());
}
