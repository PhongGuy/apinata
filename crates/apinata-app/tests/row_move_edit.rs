//! Moving a header row in a request tab: one undo step, focus on the moved row, and the saved
//! file in the new order.

use crate::support;

use apinata_app::request_editor::{RequestEditor, Section};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use support::*;

const FILE: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n  headers:\n    - name: H1\n      value: one\n    - name: H2\n      value: two\n      disabled: true\n    - name: H3\n      value: three\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n    - expression: res.body\n      operator: isJson\n";

struct Rig {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<apinata_app::workspace::Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ed: Entity<RequestEditor>,
}

fn rig(cx: &mut TestAppContext) -> Rig {
    support::init(cx);
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", FILE)]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Headers as usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
    }
}

impl Rig {
    fn names(&self, cx: &mut TestAppContext) -> Vec<String> {
        let v = view_of(cx, &self.ws, &self.root, "a.yml");
        v.headers.iter().map(|h| h.name.clone()).collect()
    }

    fn press(&self, cx: &mut TestAppContext, keys: &str) {
        cx.simulate_keystrokes(self.win, keys);
        cx.run_until_parked();
        frame(cx, self.win);
    }

    fn focus_name(&self, cx: &mut TestAppContext, ix: usize) {
        cx.update_window(self.win, |_, w, cx| {
            w.within("headers").click(("kv-name", ix), cx)
        })
        .unwrap();
    }

    // The table row whose name input holds focus.
    fn focused_row(&self, cx: &mut TestAppContext) -> Option<usize> {
        let win = self.win;
        let ed = self.ed.clone();
        cx.update_window(win, |_, w, cx| {
            let table = ed.read(cx).table(Section::Headers).cloned()?;
            let table = table.read(cx);
            (0..3).find(|&i| {
                let input = table.name_input(i).unwrap();
                gpui_kit::Focusable::focus_handle(input, cx).contains_focused(w, cx)
            })
        })
        .unwrap()
    }
}

#[gpui_kit::test]
fn alt_down_reorders_the_headers_and_one_undo_puts_them_back(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.focus_name(cx, 0);
    r.press(cx, "alt-down");
    assert_eq!(r.names(cx), ["H2", "H1", "H3"]);
    assert_eq!(r.focused_row(cx), Some(1), "focus stays on the moved row");
    r.press(cx, &secondary("z"));
    assert_eq!(r.names(cx), ["H1", "H2", "H3"]);
    assert_eq!(r.focused_row(cx), Some(0), "focus follows the row back");
    r.press(cx, &secondary("shift-z"));
    assert_eq!(r.names(cx), ["H2", "H1", "H3"]);
    assert_eq!(r.focused_row(cx), Some(1));
}

#[gpui_kit::test]
fn the_moved_order_is_written_to_the_file_with_the_disabled_flag(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.focus_name(cx, 1);
    r.press(cx, "alt-up");
    r.press(cx, &secondary("s"));
    let text = std::fs::read_to_string(r.root.join("a.yml")).unwrap();
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle} in {text}"))
    };
    assert!(at("H2") < at("H1") && at("H1") < at("H3"), "{text}");
    let h2 = &text[at("H2")..at("H1")];
    assert!(h2.contains("disabled: true"), "{text}");
}

#[gpui_kit::test]
fn dragging_a_handle_reorders_and_undoes_in_one_step(cx: &mut TestAppContext) {
    let r = rig(cx);
    cx.update_window(r.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w
            .within("headers")
            .find(("kv-grip", 2usize))
            .bounds()
            .center();
        let b = w.within("headers").find(("kv-row", 0usize)).bounds();
        w.drag(
            start,
            gpui_kit::point(b.center().x, b.top() + gpui_kit::px(3.)),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(r.names(cx), ["H3", "H1", "H2"]);
    r.press(cx, &secondary("z"));
    assert_eq!(r.names(cx), ["H1", "H2", "H3"]);
}

#[gpui_kit::test]
fn a_moved_assertion_undoes_in_one_step_and_saves_in_order(cx: &mut TestAppContext) {
    let r = rig(cx);
    cx.update_window(r.win, |_, w, cx| {
        w.click(("section", Section::Assertions as usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, r.win);
    let exprs = |cx: &mut TestAppContext| -> Vec<String> {
        let v = view_of(cx, &r.ws, &r.root, "a.yml");
        v.assertions
            .iter()
            .map(|a| a.expression().to_owned())
            .collect()
    };
    cx.update_window(r.win, |_, w, cx| {
        w.within("assertions").click(("as-expr", 0usize), cx)
    })
    .unwrap();
    r.press(cx, "alt-down");
    assert_eq!(exprs(cx), ["res.body", "res.status"]);
    r.press(cx, &secondary("s"));
    let text = std::fs::read_to_string(r.root.join("a.yml")).unwrap();
    assert!(
        text.find("res.body").unwrap() < text.find("res.status").unwrap(),
        "{text}"
    );
    r.press(cx, &secondary("z"));
    assert_eq!(exprs(cx), ["res.status", "res.body"]);
}

#[gpui_kit::test]
fn undo_of_a_move_across_a_path_row_keeps_focus_on_the_moved_row(cx: &mut TestAppContext) {
    support::init(cx);
    // The file stores the params in this order; the path row sits between the query rows.
    let params = "  params:\n    - name: q1\n      value: \"1\"\n      type: query\n    - name: id\n      value: \"7\"\n      type: path\n    - name: q2\n      value: \"2\"\n      type: query\n";
    let file = FILE
        .replace("http://x/0", "http://x/:id?q1=1&q2=2")
        .replace("  headers:", &format!("{params}  headers:"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Params as usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    let names = |cx: &mut TestAppContext| -> Vec<String> {
        ed.read_with(cx, |e, cx| {
            let t = e.table(Section::Params).unwrap().read(cx);
            t.rows(cx).into_iter().map(|r| r.name).collect()
        })
    };
    let focused = |cx: &mut TestAppContext| -> Option<usize> {
        let ed = ed.clone();
        cx.update_window(win, |_, w, cx| {
            let table = ed.read(cx).table(Section::Params).cloned()?;
            let table = table.read(cx);
            (0..3).find(|&i| {
                let input = table.name_input(i).unwrap();
                gpui_kit::Focusable::focus_handle(input, cx).contains_focused(w, cx)
            })
        })
        .unwrap()
    };
    let press = |cx: &mut TestAppContext, keys: &str| {
        cx.simulate_keystrokes(win, keys);
        cx.run_until_parked();
        frame(cx, win);
    };
    assert_eq!(names(cx), ["q1", "id", "q2"]);
    cx.update_window(win, |_, w, cx| {
        w.within("params").click(("kv-name", 2usize), cx)
    })
    .unwrap();
    press(cx, "alt-up");
    assert_eq!(names(cx), ["q2", "id", "q1"]);
    assert_eq!(focused(cx), Some(0));
    press(cx, &secondary("z"));
    assert_eq!(names(cx), ["q1", "id", "q2"]);
    assert_eq!(focused(cx), Some(2), "focus follows the row back");
}
