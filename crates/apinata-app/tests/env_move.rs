//! Moving environment variable rows: the drag handle and Alt+Up and Alt+Down.

use crate::support;

use apinata_app::env_table::{EnvRow, EnvTableEvent, EnvVarTable};
use apinata_app::tabs::Side;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, Focusable as _, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, TestAppContext, Window, WindowOptions,
    div, point, px,
};
use support::*;

struct Host {
    table: Entity<EnvVarTable>,
    events: Vec<EnvTableEvent>,
    _sub: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.table.clone())
    }
}

fn var(name: &str, secret: bool, enabled: bool, source: usize) -> EnvRow {
    EnvRow {
        name: name.into(),
        value: if secret {
            String::new()
        } else {
            format!("v-{name}")
        },
        enabled,
        secret,
        source: Some(source),
        unsaved: false,
    }
}

fn host(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Host>, Entity<EnvVarTable>) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let rows = vec![
        var("a", false, true, 0),
        var("b", false, false, 1),
        var("tok", true, true, 2),
    ];
    let (win, host) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| {
                    let table = cx.new(|cx| EnvVarTable::new(w, cx));
                    table.update(cx, |t, cx| t.load(rows, w, cx));
                    let _sub = cx.subscribe(&table, |h: &mut Host, _, e: &EnvTableEvent, _| {
                        h.events.push(e.clone())
                    });
                    Host {
                        table,
                        events: Vec::new(),
                        _sub,
                    }
                })
            })
        })
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    let table = host.read_with(cx, |h, _| h.table.clone());
    (win, host, table)
}

fn names(cx: &mut TestAppContext, t: &Entity<EnvVarTable>) -> Vec<String> {
    t.read_with(cx, |t, cx| t.rows(cx).into_iter().map(|r| r.name).collect())
}

fn changes(cx: &mut TestAppContext, h: &Entity<Host>) -> usize {
    h.read_with(cx, |h, _| {
        h.events
            .iter()
            .filter(|e| matches!(e, EnvTableEvent::Changed(_)))
            .count()
    })
}

fn focused_row(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    t: &Entity<EnvVarTable>,
) -> Option<usize> {
    let t = t.clone();
    cx.update_window(win, |_, w, cx| {
        (0..3).find(|&i| {
            let input = t.read(cx).name_input(i).cloned().unwrap();
            input.focus_handle(cx).contains_focused(w, cx)
        })
    })
    .unwrap()
}

fn drag(cx: &mut TestAppContext, win: AnyWindowHandle, from: usize, onto: usize, lower: bool) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(("env-grip", from)).bounds().center();
        let b = w.find(("env-row", onto)).bounds();
        let y = if lower {
            b.bottom() - px(3.)
        } else {
            b.top() + px(3.)
        };
        w.drag(start, point(b.center().x, y), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn each_real_row_has_a_handle_and_the_columns_stay_aligned(cx: &mut TestAppContext) {
    let (win, _h, _t) = host(cx);
    cx.update_window(win, |_, w, _| {
        for ix in 0..3usize {
            assert!(w.try_find(("env-grip", ix)).is_some(), "row {ix}");
        }
        assert!(w.try_find(("env-grip", 3usize)).is_none());
        let head = w.find("env-head-name").bounds().left();
        for ix in 0..4usize {
            assert_eq!(w.find(("env-name", ix)).bounds().left(), head, "row {ix}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn dragging_a_handle_moves_the_row_as_one_edit_and_focuses_it(cx: &mut TestAppContext) {
    let (win, h, t) = host(cx);
    drag(cx, win, 0, 1, true);
    assert_eq!(names(cx, &t), ["b", "a", "tok"]);
    assert_eq!(changes(cx, &h), 1);
    assert_eq!(focused_row(cx, win, &t), Some(1));
    let rows = t.read_with(cx, |t, cx| t.rows(cx));
    assert!(!rows[0].enabled, "the disabled row moved like any row");
    assert_eq!(rows[1].value, "v-a");
}

#[gpui_kit::test]
fn dropping_on_the_upper_half_puts_the_row_before(cx: &mut TestAppContext) {
    let (win, _h, t) = host(cx);
    drag(cx, win, 2, 0, false);
    assert_eq!(names(cx, &t), ["tok", "a", "b"]);
}

#[gpui_kit::test]
fn a_drop_that_changes_nothing_emits_nothing(cx: &mut TestAppContext) {
    let (win, h, t) = host(cx);
    drag(cx, win, 1, 1, true);
    drag(cx, win, 1, 0, true);
    assert_eq!(names(cx, &t), ["a", "b", "tok"]);
    assert_eq!(changes(cx, &h), 0);
}

#[gpui_kit::test]
fn a_secret_row_keeps_its_typed_value_and_reveal_when_it_moves(cx: &mut TestAppContext) {
    let (win, _h, t) = host(cx);
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.show_secret("tok", Some("s3".into()), w, cx))
    })
    .unwrap();
    assert!(t.read_with(cx, |t, _| t.revealed(2)));
    drag(cx, win, 2, 0, false);
    assert_eq!(names(cx, &t), ["tok", "a", "b"]);
    assert!(
        t.read_with(cx, |t, _| t.revealed(0)),
        "the eye moved with the row"
    );
    let shown = t
        .read_with(cx, |t, cx| {
            t.value_input(0).map(|i| i.read(cx).value().to_string())
        })
        .unwrap();
    assert_eq!(shown, "s3");
    assert!(!t.read_with(cx, |t, _| t.revealed(2)));
}

#[gpui_kit::test]
fn the_drop_line_shows_only_where_a_drop_would_move_the_row(cx: &mut TestAppContext) {
    let (win, _h, t) = host(cx);
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("env-drop-line").is_some()
        })
        .unwrap()
    };
    assert!(!shown(cx));
    t.update(cx, |t, cx| t.hover_drop(0, 2, Some(Side::After), cx));
    assert!(shown(cx));
    t.update(cx, |t, cx| t.hover_drop(0, 2, None, cx));
    assert!(!shown(cx));
    t.update(cx, |t, cx| t.hover_drop(1, 1, Some(Side::Before), cx));
    assert!(!shown(cx));
}

#[gpui_kit::test]
fn alt_keys_move_the_focused_row_and_stop_at_the_ends(cx: &mut TestAppContext) {
    let (win, h, t) = host(cx);
    cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-up");
    cx.run_until_parked();
    assert_eq!(names(cx, &t), ["a", "b", "tok"]);
    assert_eq!(changes(cx, &h), 0);
    cx.simulate_keystrokes(win, "alt-down");
    cx.run_until_parked();
    assert_eq!(names(cx, &t), ["b", "a", "tok"]);
    assert_eq!(focused_row(cx, win, &t), Some(1));
    assert_eq!(changes(cx, &h), 1);
    cx.simulate_keystrokes(win, "alt-down alt-down");
    cx.run_until_parked();
    assert_eq!(names(cx, &t), ["b", "tok", "a"], "never into the empty row");
    assert_eq!(t.read_with(cx, |t, _| t.last_typed()), None);
}

#[gpui_kit::test]
fn alt_works_from_the_value_cell(cx: &mut TestAppContext) {
    let (win, _h, t) = host(cx);
    cx.update_window(win, |_, w, cx| w.click(("env-value", 1usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-up");
    cx.run_until_parked();
    assert_eq!(names(cx, &t), ["b", "a", "tok"]);
}

#[gpui_kit::test]
fn a_load_that_shows_the_move_undone_takes_focus_with_the_row(cx: &mut TestAppContext) {
    let (win, _h, t) = host(cx);
    cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-down");
    cx.run_until_parked();
    assert_eq!(focused_row(cx, win, &t), Some(1));
    let original = vec![
        var("a", false, true, 0),
        var("b", false, false, 1),
        var("tok", true, true, 2),
    ];
    cx.update_window(win, |_, w, cx| {
        t.update(cx, |t, cx| t.load(original, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(names(cx, &t), ["a", "b", "tok"]);
    assert_eq!(focused_row(cx, win, &t), Some(0));
}

#[gpui_kit::test]
fn a_moved_variable_undoes_in_one_step_and_saves_in_order(cx: &mut TestAppContext) {
    support::init(cx);
    let dev =
        "name: dev\nvariables:\n  - name: host\n    value: x\n  - name: port\n    value: \"80\"\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", dev)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    let order = |cx: &mut TestAppContext| -> Vec<String> {
        env_view(cx, &ws, &root, "dev")
            .variables
            .iter()
            .map(|v| v.name.clone())
            .collect()
    };
    cx.update_window(win, |_, w, cx| w.click(("env-name", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-down");
    cx.run_until_parked();
    assert_eq!(order(cx), ["port", "host"]);
    cx.simulate_keystrokes(win, &secondary("s"));
    cx.run_until_parked();
    let text = std::fs::read_to_string(root.join("environments/dev.yml")).unwrap();
    assert!(
        text.find("port").unwrap() < text.find("host").unwrap(),
        "{text}"
    );
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(order(cx), ["host", "port"]);
}

#[gpui_kit::test]
fn alt_works_with_the_row_checkbox_focused(cx: &mut TestAppContext) {
    let (win, _h, t) = host(cx);
    cx.update_window(win, |_, w, cx| w.click(("env-enabled", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-down");
    cx.run_until_parked();
    assert_eq!(names(cx, &t), ["b", "a", "tok"]);
}

#[gpui_kit::test]
fn a_drop_on_the_empty_row_leaves_focus_on_the_dragged_row(cx: &mut TestAppContext) {
    let (win, _h, t) = host(cx);
    drag(cx, win, 0, 3, false);
    assert_eq!(names(cx, &t), ["a", "b", "tok"]);
    assert_eq!(focused_row(cx, win, &t), Some(0));
}

#[gpui_kit::test]
fn ctrl_z_in_a_cell_undoes_a_move_in_the_environment_editor(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let dev =
        "name: dev\nvariables:\n  - name: host\n    value: x\n  - name: port\n    value: \"80\"\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", dev)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    let order = |cx: &mut TestAppContext| -> Vec<String> {
        env_view(cx, &ws, &root, "dev")
            .variables
            .iter()
            .map(|v| v.name.clone())
            .collect()
    };
    cx.update_window(win, |_, w, cx| w.click(("env-value", 0usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-down");
    cx.run_until_parked();
    assert_eq!(order(cx), ["port", "host"]);
    cx.simulate_keystrokes(win, &secondary("z"));
    cx.run_until_parked();
    assert_eq!(order(cx), ["host", "port"]);
}

#[gpui_kit::test]
fn focus_follows_a_row_moved_from_a_clicked_checkbox(cx: &mut TestAppContext) {
    let (win, _h, t) = host(cx);
    cx.update_window(win, |_, w, cx| w.click(("env-enabled", 2usize), cx))
        .unwrap();
    cx.simulate_keystrokes(win, "alt-up");
    cx.run_until_parked();
    frame(cx, win);
    cx.simulate_keystrokes(win, "alt-up");
    cx.run_until_parked();
    assert_eq!(names(cx, &t), ["tok", "a", "b"]);
}
