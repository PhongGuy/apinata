//! The read-only key-value view: headed columns, text and flag cells, and a hover delete.

use crate::support;

use std::sync::atomic::{AtomicUsize, Ordering};

use apinata_app::kv_grid::KvColumn;
use apinata_app::style::{COOKIE_COL_WIDTH, HEADER_NAME_WIDTH, at_zoom};
use apinata_app::ui::{KvCell, kv_view};
use gpui_kit::component::v_flex;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyElement, AnyWindowHandle, AppContext as _, Context, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, Window, WindowOptions, div,
};
use support::*;

static DELETED: AtomicUsize = AtomicUsize::new(usize::MAX);

struct Host;

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(table())
    }
}

fn table() -> AnyElement {
    let columns = vec![
        KvColumn::fixed("t-head-name", "t-name", "Name", HEADER_NAME_WIDTH),
        KvColumn::flex("t-head-value", "t-value", "Value"),
        KvColumn::fixed("t-head-secure", "t-secure", "Secure", COOKIE_COL_WIDTH),
    ];
    let rows = vec![
        vec![
            KvCell::Text("a".into()),
            KvCell::Text("1".into()),
            KvCell::Flag(true),
        ],
        vec![
            KvCell::Text("b".into()),
            KvCell::Text("2".into()),
            KvCell::Flag(false),
        ],
    ];
    v_flex()
        .w_full()
        .child(
            kv_view("t", columns, rows)
                .row_ids("t-row")
                .headed("t-head")
                .on_delete("t-delete", "Remove", |ix, _, _| {
                    DELETED.store(ix, Ordering::SeqCst)
                }),
        )
        .into_any_element()
}

fn host(cx: &mut TestAppContext) -> AnyWindowHandle {
    support::init(cx);
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Host)))
        .unwrap();
    frame(cx, win);
    win
}

#[gpui_kit::test]
fn headings_sit_over_their_cells(cx: &mut TestAppContext) {
    let win = host(cx);
    cx.update_window(win, |_, w, _| {
        for (head, cell) in [
            ("t-head-name", "t-name"),
            ("t-head-value", "t-value"),
            ("t-head-secure", "t-secure"),
        ] {
            let h = w.find(head).bounds();
            let c = w.find((cell, 0usize)).bounds();
            assert_eq!(h.left(), c.left(), "{head}");
            assert!(h.bottom() <= c.top(), "{head}");
        }
        assert_eq!(
            w.find(("t-name", 1usize)).bounds().size.width,
            at_zoom(HEADER_NAME_WIDTH, 1.0)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn cells_carry_their_text_and_flags_read_yes_or_no(cx: &mut TestAppContext) {
    let win = host(cx);
    cx.update_window(win, |_, w, _| {
        assert_eq!(w.find(("t-value", 1usize)).label(), Some("2"));
        assert_eq!(w.find(("t-secure", 0usize)).label(), Some("Yes"));
        assert_eq!(w.find(("t-secure", 1usize)).label(), Some("No"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_delete_waits_for_hover_and_names_its_row(cx: &mut TestAppContext) {
    let win = host(cx);
    cx.update_window(win, |_, w, cx| {
        w.hover("t-head", cx);
        w.render_frame(cx);
        assert!(!w.within(("t-row", 1usize)).find("kv-action").visible());
        w.hover(("t-row", 1usize), cx);
        w.render_frame(cx);
        assert!(w.within(("t-row", 1usize)).find("kv-action").visible());
        assert!(!w.within(("t-row", 0usize)).find("kv-action").visible());
        w.click(("t-delete", 1usize), cx);
    })
    .unwrap();
    assert_eq!(DELETED.load(Ordering::SeqCst), 1);
}

struct Long;

impl Render for Long {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let columns = vec![
            KvColumn::fixed("l-head-name", "l-name", "Name", HEADER_NAME_WIDTH),
            KvColumn::flex("l-head-value", "l-value", "Value"),
        ];
        let long = "a long wrapping value ".repeat(200);
        let rows = vec![
            vec![KvCell::Text("csp".into()), KvCell::Text(long.into())],
            vec![KvCell::Text("short".into()), KvCell::Text("1".into())],
        ];
        div()
            .size_full()
            .child(kv_view("l", columns, rows).row_ids("l-row"))
    }
}

#[gpui_kit::test]
fn a_wrapped_value_keeps_its_name_on_the_first_line(cx: &mut TestAppContext) {
    support::init(cx);
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Long)))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        let name = w.find(("l-name", 0usize)).bounds();
        let value = w.find(("l-value", 0usize)).bounds();
        assert!(
            value.size.height > name.size.height * 2.,
            "{value:?} does not wrap"
        );
        // The first text lines share a band, not just the cell tops.
        let line = |w: &mut Window, id: &'static str, ix: usize| {
            w.within((id, ix)).find("kv-text").bounds()
        };
        assert_eq!(line(w, "l-name", 0).top(), line(w, "l-value", 0).top());
        // A one-line row centres its text on the row.
        let row = w.find(("l-row", 1usize)).bounds();
        assert_eq!(line(w, "l-name", 1).center().y, row.center().y);
    })
    .unwrap();
}

// A long value stops short of the table's right edge by the table's padding instead of
// running flush to the pane.
#[gpui_kit::test]
fn a_long_value_keeps_a_right_gutter(cx: &mut TestAppContext) {
    use apinata_app::style::SPACE_2;
    support::init(cx);
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Long)))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        let table = w.find("l").bounds();
        let value = w.find(("l-value", 0usize)).bounds();
        assert!(
            value.right() + at_zoom(SPACE_2, 1.0) <= table.right(),
            "{value:?} in {table:?}"
        );
    })
    .unwrap();
}
