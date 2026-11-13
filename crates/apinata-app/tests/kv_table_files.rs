use crate::support;

use apinata_app::kv_table::{
    KvColumns, KvEvent, KvRow, KvTable, PickFiles, file_rows, files_from, multipart_from,
    multipart_rows,
};
use apinata_core::{FileEntry, MultipartField, MultipartValue};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Subscription, TestAppContext, Window, WindowOptions, div,
};

fn s(v: &str) -> String {
    v.to_owned()
}
fn row(name: &str, value: &str) -> KvRow {
    KvRow {
        name: s(name),
        value: s(value),
        enabled: true,
        ..KvRow::default()
    }
}
fn entry(path: &str, selected: bool) -> FileEntry {
    let mut e = FileEntry::new(path);
    e.selected = selected;
    e
}

#[test]
fn multipart_rows_carry_files_and_content_type() {
    let mut text = MultipartField::text("a", "1");
    text.content_type = s("text/plain");
    let mut file = MultipartField::files("f", vec![s("x.png"), s("y.png")]);
    file.enabled = false;
    let base = vec![text, file];
    let mut rows = multipart_rows(&base);
    assert_eq!(
        (rows[0].files.clone(), rows[0].content_type.as_str()),
        (None, "text/plain")
    );
    assert_eq!(rows[1].files, Some(vec![s("x.png"), s("y.png")]));
    assert_eq!((rows[1].value.as_str(), rows[1].enabled), ("", false));
    rows[0].files = Some(vec![s("z.bin")]);
    rows[1].files = None;
    rows[1].value = s("plain");
    rows.push(row("n", "v"));
    let out = multipart_from(&base, &rows);
    assert_eq!(out[0].value, MultipartValue::Files(vec![s("z.bin")]));
    assert_eq!(out[0].content_type, "text/plain");
    assert_eq!(out[1].value, MultipartValue::Text(s("plain")));
    assert!(!out[1].enabled);
    assert_eq!(out[2], MultipartField::text("n", "v"));
}

#[test]
fn file_rows_select_one_file_at_a_time() {
    let base = vec![entry("a.bin", true), entry("b.json", false)];
    let mut rows = file_rows(&base);
    assert_eq!(rows[0].files, Some(vec![s("a.bin")]));
    assert!(rows[0].enabled && !rows[1].enabled);
    rows[1].enabled = true;
    assert_eq!(
        files_from(&base, &rows),
        vec![entry("a.bin", false), entry("b.json", true)]
    );
    rows[0].enabled = false;
    rows[1].enabled = false;
    assert_eq!(
        files_from(&base, &rows),
        vec![entry("a.bin", false), entry("b.json", false)]
    );
    let mut rows = file_rows(&base);
    rows[0].content_type = s("application/octet-stream");
    rows.push(KvRow {
        files: Some(vec![s("c.txt")]),
        ..KvRow::default()
    });
    let out = files_from(&base, &rows);
    assert_eq!(out[0].content_type, "application/octet-stream");
    assert_eq!(out[2], entry("c.txt", false));
    assert_eq!(
        files_from(&[], &[row("", ""), row("", "")]),
        vec![entry("", true), entry("", false)]
    );
}

struct Host {
    table: Entity<KvTable>,
    changes: Vec<Vec<KvRow>>,
    picks: Vec<usize>,
    _subs: Vec<Subscription>,
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

fn host(
    cx: &mut TestAppContext,
    columns: KvColumns,
    rows: Vec<KvRow>,
) -> (AnyWindowHandle, Entity<Host>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| {
                    let table = cx.new(|cx| KvTable::new(w, cx));
                    table.update(cx, |t, cx| {
                        t.set_columns(columns, cx);
                        t.load(rows, w, cx)
                    });
                    let a = cx.subscribe(&table, |h: &mut Host, _, e: &KvEvent, _| {
                        let KvEvent::Changed(rows) = e;
                        h.changes.push(rows.clone());
                    });
                    let b = cx.subscribe(&table, |h: &mut Host, _, e: &PickFiles, _| {
                        h.picks.push(e.0)
                    });
                    Host {
                        table,
                        changes: Vec::new(),
                        picks: Vec::new(),
                        _subs: vec![a, b],
                    }
                })
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, view)
}

fn last(cx: &mut TestAppContext, view: &Entity<Host>) -> Vec<KvRow> {
    view.read_with(cx, |h, _| h.changes.last().cloned().expect("an event"))
}

fn shown<const N: usize>(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ids: [(&'static str, usize); N],
) -> [bool; N] {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        ids.map(|id| w.try_find(id).is_some())
    })
    .unwrap()
}

#[gpui_kit::test]
fn pick_asks_and_set_files_turns_the_row_into_files(cx: &mut TestAppContext) {
    let (win, view) = host(cx, KvColumns::MULTIPART, vec![row("f", "")]);
    assert_eq!(shown(cx, win, [("kv-value", 0)]), [true]);
    cx.update_window(win, |_, w, cx| w.click(("kv-pick", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |h, _| h.picks.clone()), vec![0]);
    assert!(view.read_with(cx, |h, _| h.changes.is_empty()));
    let table = view.read_with(cx, |h, _| h.table.clone());
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.set_files(0, Some(vec![s("a.png")]), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(last(cx, &view)[0].files, Some(vec![s("a.png")]));
    assert_eq!(
        shown(
            cx,
            win,
            [("kv-value", 0), ("kv-files", 0), ("kv-clear-files", 0)]
        ),
        [false, true, true]
    );
    cx.update_window(win, |_, w, cx| w.click(("kv-clear-files", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    let rows = last(cx, &view);
    assert_eq!((rows[0].files.clone(), rows[0].value.as_str()), (None, ""));
}

#[gpui_kit::test]
fn content_type_typing_emits_the_rows(cx: &mut TestAppContext) {
    let (win, view) = host(cx, KvColumns::MULTIPART, vec![row("f", "")]);
    cx.update_window(win, |_, w, cx| {
        w.click(("kv-ctype", 0usize), cx);
        w.input("image/png", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(last(cx, &view)[0].content_type, "image/png");
}

#[gpui_kit::test]
fn the_file_body_layout_has_no_name_and_always_lists_files(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, KvColumns::FILE_BODY, file_rows(&[entry("a.bin", true)]));
    let ids = [
        ("kv-name", 0),
        ("kv-value", 0),
        ("kv-files", 0),
        ("kv-ctype", 0),
        ("kv-pick", 0),
        ("kv-clear-files", 0),
        ("kv-enabled", 0),
    ];
    assert_eq!(
        shown(cx, win, ids),
        [false, false, true, true, true, false, true]
    );
}

#[gpui_kit::test]
fn plain_tables_show_no_new_columns(cx: &mut TestAppContext) {
    let (win, view) = host(cx, KvColumns::PLAIN, vec![row("a", "1")]);
    assert_eq!(
        shown(
            cx,
            win,
            [
                ("kv-ctype", 0),
                ("kv-pick", 0),
                ("kv-files", 0),
                ("kv-value", 0)
            ]
        ),
        [false, false, false, true]
    );
    let table = view.read_with(cx, |h, _| h.table.clone());
    assert_eq!(table.read_with(cx, |t, _| t.columns()), KvColumns::PLAIN);
}

#[gpui_kit::test]
fn a_locked_table_ignores_pick(cx: &mut TestAppContext) {
    let (win, view) = host(cx, KvColumns::MULTIPART, vec![row("f", "")]);
    let table = view.read_with(cx, |h, _| h.table.clone());
    table.update(cx, |t, cx| t.set_locked(true, cx));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("kv-pick", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |h, _| h.picks.is_empty()));
}

#[gpui_kit::test]
fn picking_files_on_the_empty_row_makes_a_file_row(cx: &mut TestAppContext) {
    let (win, view) = host(cx, KvColumns::MULTIPART, vec![row("f", "1")]);
    cx.update_window(win, |_, w, cx| w.click(("kv-pick", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |h, _| h.picks.clone()), vec![1]);
    let table = view.read_with(cx, |h, _| h.table.clone());
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.set_files(1, None, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |h, _| h.changes.is_empty()));
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.set_files(1, Some(vec![s("a.png")]), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let rows = last(cx, &view);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[1].files.clone(), rows[1].enabled),
        (Some(vec![s("a.png")]), true)
    );
    assert_eq!(
        shown(
            cx,
            win,
            [
                ("kv-files", 1),
                ("kv-delete", 1),
                ("kv-pick", 2),
                ("kv-delete", 2)
            ]
        ),
        [true, true, true, false]
    );
}

#[gpui_kit::test]
fn file_rows_keep_the_columns_of_text_rows(cx: &mut TestAppContext) {
    let file = KvRow {
        files: Some(vec![s("a.png")]),
        ..row("f", "")
    };
    let (win, _view) = host(cx, KvColumns::MULTIPART, vec![row("t", "1"), file]);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for id in ["kv-name", "kv-ctype"] {
            let text = w.find((id, 0usize)).bounds();
            let file = w.find((id, 1usize)).bounds();
            assert_eq!(
                (file.left(), file.size.width),
                (text.left(), text.size.width),
                "{id}"
            );
        }
        let value = w.find(("kv-value", 0usize)).bounds();
        let files = w.find(("kv-files", 1usize)).bounds();
        assert_eq!(
            (files.left(), files.size.width),
            (value.left(), value.size.width),
            "files cell"
        );
        let head = w.find("kv-head-ctype").bounds();
        let ctype = w.find(("kv-ctype", 1usize)).bounds();
        assert_eq!(head.left(), ctype.left(), "content-type heading");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_file_body_row_shows_its_file_name_and_tips_the_full_path(cx: &mut TestAppContext) {
    let path = format!(
        "/srv/{}reports/final-report.pdf",
        "a-rather-long-folder-name/".repeat(12)
    );
    let (win, _view) = host(cx, KvColumns::FILE_BODY, file_rows(&[entry(&path, true)]));
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
    }
    let (shown, full) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let e = w.find(("kv-files", 0usize));
            (e.value().map(str::to_owned), e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(shown.as_deref(), Some("final-report.pdf"));
    assert_eq!(full.as_deref(), Some(path.as_str()));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(("kv-files", 0usize), cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    cx.run_until_parked();
    let tip = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("kv-files-tooltip")
                .map(|t| t.label().unwrap_or_default().to_owned())
        })
        .unwrap();
    assert_eq!(tip, Some(path));
}

// The mark on row `ix` is round and the picker's text, as drawn.
fn mark_and_picker(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ix: usize,
) -> (bool, Option<String>) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (
            w.within(("kv-enabled", ix))
                .try_find("checkbox-radio")
                .is_some(),
            w.find(("kv-pick", ix)).label().map(str::to_owned),
        )
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_file_body_row_draws_a_round_mark_and_picks_one_file(cx: &mut TestAppContext) {
    let (win, _view) = host(
        cx,
        KvColumns::FILE_BODY,
        file_rows(&[entry("a.bin", true), entry("b.bin", false)]),
    );
    for ix in [0, 1] {
        assert_eq!(
            mark_and_picker(cx, win, ix),
            (true, Some(s("Change\u{2026}"))),
            "row {ix}"
        );
    }
}

#[gpui_kit::test]
fn a_multipart_row_keeps_a_square_box_and_picks_files(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, KvColumns::MULTIPART, vec![row("f", "")]);
    assert_eq!(mark_and_picker(cx, win, 0), (false, Some(s("Files"))));
}

// Each file row's mark as drawn: (role, checked, accessible name).
fn marks(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    n: usize,
) -> Vec<(Option<gpui_kit::Role>, Option<bool>, Option<String>)> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (0..n)
            .map(|ix| {
                let e = w.find(("kv-enabled", ix));
                (e.role(), e.checked(), e.label().map(str::to_owned))
            })
            .collect()
    })
    .unwrap()
}

fn picked(marks: &[(Option<gpui_kit::Role>, Option<bool>, Option<String>)]) -> Vec<bool> {
    marks.iter().map(|m| m.1 == Some(true)).collect()
}

#[gpui_kit::test]
fn a_file_body_mark_is_a_radio_named_for_its_file(cx: &mut TestAppContext) {
    let (win, _view) = host(
        cx,
        KvColumns::FILE_BODY,
        file_rows(&[
            entry("data/a.bin", true),
            entry("", false),
            entry("b.json", false),
        ]),
    );
    let marks = marks(cx, win, 3);
    assert_eq!(marks[0].0, Some(gpui_kit::Role::RadioButton));
    let names: Vec<_> = marks.into_iter().map(|m| m.2).collect();
    assert_eq!(
        names,
        [
            Some(s("Send a.bin")),
            Some(s("Send file 2, none chosen")),
            Some(s("Send b.json")),
        ]
    );
}

#[gpui_kit::test]
fn picking_a_file_row_clears_the_others_at_once(cx: &mut TestAppContext) {
    let (win, view) = host(
        cx,
        KvColumns::FILE_BODY,
        file_rows(&[entry("a.bin", true), entry("b.bin", false)]),
    );
    cx.update_window(win, |_, w, cx| w.click(("kv-enabled", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(picked(&marks(cx, win, 2)), [false, true]);
    let rows = last(cx, &view);
    assert_eq!((rows[0].enabled, rows[1].enabled), (false, true));
}

#[gpui_kit::test]
fn clicking_the_picked_file_row_keeps_it_picked(cx: &mut TestAppContext) {
    let (win, view) = host(
        cx,
        KvColumns::FILE_BODY,
        file_rows(&[entry("a.bin", true), entry("b.bin", false)]),
    );
    cx.update_window(win, |_, w, cx| w.click(("kv-enabled", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(picked(&marks(cx, win, 2)), [true, false]);
    assert!(view.read_with(cx, |h, _| h.changes.is_empty()));
}

// A file that marks two entries selected shows only the first as picked.
#[gpui_kit::test]
fn two_selected_files_show_as_one(cx: &mut TestAppContext) {
    let rows = file_rows(&[entry("a.bin", true), entry("b.bin", true)]);
    assert_eq!((rows[0].enabled, rows[1].enabled), (true, false));
    let (win, _view) = host(cx, KvColumns::FILE_BODY, rows);
    assert_eq!(picked(&marks(cx, win, 2)), [true, false]);
}
