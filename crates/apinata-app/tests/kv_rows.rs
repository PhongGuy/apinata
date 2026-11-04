//! Key-value rows are one row high, delete is an icon, and the trailing empty row lines up.

use crate::support;

use apinata_app::kv_table::{KvRow, KvTable};
use apinata_app::style::{ROW_HEIGHT, at_zoom};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled,
    TestAppContext, Window, WindowOptions, div,
};

struct Host {
    table: Entity<KvTable>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.table.clone())
    }
}

fn row(name: &str, value: &str, enabled: bool) -> KvRow {
    KvRow {
        name: name.into(),
        value: value.into(),
        enabled,
        ..KvRow::default()
    }
}

fn host(cx: &mut TestAppContext, rows: Vec<KvRow>) -> AnyWindowHandle {
    support::init(cx);
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| {
                    let table = cx.new(|cx| KvTable::new(w, cx));
                    table.update(cx, |t, cx| t.load(rows, w, cx));
                    Host { table }
                })
            })
        })
        .unwrap();
    support::frame(cx, win);
    win
}

#[gpui_kit::test]
fn rows_are_one_row_high_and_the_empty_row_matches(cx: &mut TestAppContext) {
    let win = host(cx, vec![row("a", "1", true), row("b", "2", false)]);
    cx.update_window(win, |_, w, _| {
        let first = w.find(("kv-row", 0usize)).bounds();
        let second = w.find(("kv-row", 1usize)).bounds();
        assert_eq!(first.size.height, at_zoom(ROW_HEIGHT, 1.0));
        assert_eq!(second.size.height, at_zoom(ROW_HEIGHT, 1.0));
        let blank = w.find(("kv-row", 2usize)).bounds();
        assert_eq!(blank.size.height, at_zoom(ROW_HEIGHT, 1.0));
        assert_eq!(blank.left(), first.left());
        let delete = w.find(("kv-delete", 0usize)).bounds();
        assert!(
            delete.size.width <= at_zoom(ROW_HEIGHT, 1.0),
            "an icon, not a labelled button: {delete:?}"
        );
        assert!(delete.right() <= first.right());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_inputs_fit_inside_the_row(cx: &mut TestAppContext) {
    let win = host(cx, vec![row("a", "1", true)]);
    cx.update_window(win, |_, w, _| {
        let r = w.find(("kv-row", 0usize)).bounds();
        for id in [("kv-name", 0usize), ("kv-value", 0usize)] {
            let b = w.find(id).bounds();
            assert!(
                b.top() >= r.top() && b.bottom() <= r.bottom(),
                "{id:?}: {b:?} in {r:?}"
            );
        }
    })
    .unwrap();
}
