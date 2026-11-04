//! The description column: last, on for headers, params and form bodies, and written back.

use crate::support;

use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::kv_table::{
    KvColumns, KvEvent, KvRow, KvTable, header_rows, headers_from, multipart_from, multipart_rows,
    param_rows, params_from, var_rows,
};
use apinata_app::request_editor::Section;
use apinata_core::{KeyValue, MultipartField, Param, ParamKind, VarValue, Variable};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Subscription, TestAppContext, VisualTestContext, Window, WindowOptions, div,
};
use support::*;

fn described(name: &str, value: &str, description: &str) -> KvRow {
    KvRow {
        name: name.into(),
        value: value.into(),
        enabled: true,
        description: description.into(),
        ..KvRow::default()
    }
}

#[test]
fn header_descriptions_go_both_ways() {
    let mut accept = KeyValue::new("Accept", "a/b");
    accept.description = Some("What we take".into());
    let base = vec![accept, KeyValue::new("X-A", "1")];
    let mut rows = header_rows(&base);
    assert_eq!(
        (rows[0].description.as_str(), rows[1].description.as_str()),
        ("What we take", "")
    );
    rows[0].description = "Changed".into();
    rows[1].description = "New note".into();
    rows.push(described("X-B", "2", "Fresh"));
    let out = headers_from(&base, &rows);
    assert_eq!(out[0].description.as_deref(), Some("Changed"));
    assert_eq!(out[1].description.as_deref(), Some("New note"));
    assert_eq!(out[2].description.as_deref(), Some("Fresh"));
}

#[test]
fn a_cleared_description_is_none() {
    let mut h = KeyValue::new("A", "1");
    h.description = Some("gone soon".into());
    let base = vec![h];
    let mut rows = header_rows(&base);
    rows[0].description.clear();
    assert_eq!(headers_from(&base, &rows)[0].description, None);
}

#[test]
fn params_and_multipart_fields_carry_descriptions() {
    let mut q = Param::new("q", "1", ParamKind::Query);
    q.description = Some("Search".into());
    let params = vec![q];
    let mut rows = param_rows(&params);
    assert_eq!(rows[0].description, "Search");
    rows[0].description = "Term".into();
    assert_eq!(
        params_from(&params, &rows)[0].description.as_deref(),
        Some("Term")
    );
    let mut part = MultipartField::text("t", "v");
    part.description = Some("Text part".into());
    let fields = vec![part];
    let mut rows = multipart_rows(&fields);
    assert_eq!(rows[0].description, "Text part");
    rows[0].description = "Edited".into();
    assert_eq!(
        multipart_from(&fields, &rows)[0].description.as_deref(),
        Some("Edited")
    );
}

#[test]
fn variable_rows_have_no_description() {
    let rows = var_rows(&[Variable::new("v", VarValue::Text("1".into()))]);
    assert!(rows[0].description.is_empty());
}

#[test]
fn rows_differing_only_in_description_differ() {
    assert_ne!(described("a", "1", "x"), described("a", "1", "y"));
}

#[test]
fn only_describable_tables_have_the_column() {
    let described = [
        KvColumns::DESCRIBED,
        KvColumns::MULTIPART,
        KvColumns::PLAIN,
        KvColumns::FILE_BODY,
    ]
    .map(|c| c.description);
    assert_eq!(described, [true, true, false, false]);
    let d = KvColumns::DESCRIBED;
    assert_eq!((d.name, d.content_type, d.files), (true, false, false));
}

struct Host {
    table: Entity<KvTable>,
    events: Vec<Vec<KvRow>>,
    _sub: Subscription,
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
                        t.load(rows, w, cx);
                    });
                    let sub = cx.subscribe(&table, |h: &mut Host, _, e: &KvEvent, _| {
                        let KvEvent::Changed(rows) = e;
                        h.events.push(rows.clone());
                    });
                    Host {
                        table,
                        events: Vec::new(),
                        _sub: sub,
                    }
                })
            })
        })
        .unwrap();
    frame(cx, win);
    (win, view)
}

#[gpui_kit::test]
fn the_description_column_is_last_and_editable(cx: &mut TestAppContext) {
    let (win, view) = host(cx, KvColumns::DESCRIBED, vec![described("a", "1", "d")]);
    let table = view.read_with(cx, |h, _| h.table.clone());
    assert_eq!(
        table.read_with(cx, |t, _| t.headings()),
        ["Key", "Value", "Description"]
    );
    assert!(present(cx, win, "kv-head-description"));
    type_at_end(cx, win, None, ("kv-description", 0usize), "x");
    cx.run_until_parked();
    let last = view.read_with(cx, |h, _| h.events.last().cloned().expect("an event"));
    assert_eq!(last[0].description, "dx");
    table.update(cx, |t, cx| t.set_columns(KvColumns::MULTIPART, cx));
    assert_eq!(
        table.read_with(cx, |t, _| t.headings()),
        ["Key", "Value", "Content type", "Description"]
    );
}

#[gpui_kit::test]
fn a_plain_table_shows_no_description(cx: &mut TestAppContext) {
    let (win, _) = host(cx, KvColumns::PLAIN, vec![described("a", "1", "d")]);
    assert!(!present(cx, win, ("kv-description", 0usize)));
    assert!(!present(cx, win, "kv-head-description"));
}

const A: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0?a=1\"\n  params:\n    - name: a\n      value: \"1\"\n      type: query\n  headers:\n    - name: X-One\n      value: \"1\"\n      description: kept\nruntime:\n  variables:\n    - name: token\n      value: abc\n";

const FORM: &str = "info:\n  name: b\n  type: http\n  seq: 2\nhttp:\n  method: POST\n  url: \"https://x.test\"\n  body:\n    type: form-urlencoded\n    data:\n      - name: f\n        value: \"1\"\n";

#[gpui_kit::test]
fn a_header_description_edit_is_saved(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", A), ("b.yml", FORM)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let on = |s: Section, cx: &mut TestAppContext| {
        ed.read_with(cx, |e, cx| {
            e.table(s).expect("a table").read(cx).columns().description
        })
    };
    assert!(on(Section::Params, cx) && on(Section::Headers, cx));
    assert!(!on(Section::Vars, cx));
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Headers as usize), cx)
    })
    .unwrap();
    type_at_end(cx, win, Some("headers"), ("kv-description", 0usize), "!");
    cx.run_until_parked();
    assert_eq!(
        view_of(cx, &ws, &root, "a.yml").headers[0]
            .description
            .as_deref(),
        Some("kept!")
    );
    let mut vcx = VisualTestContext::from_window(win, cx);
    vcx.simulate_keystrokes(&key("s"));
    vcx.run_until_parked();
    let disk = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert!(disk.contains("description: kept!"), "{disk}");
    let (_win, form) = editor_for(cx, &ws, &root, "b.yml", false);
    assert!(form.read_with(cx, |e, cx| {
        e.body().read(cx).table().read(cx).columns().description
    }));
}

#[gpui_kit::test]
fn collection_headers_have_the_column(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let (_state, ws, root) = open_dir(cx, &col);
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (ws2, root2) = (ws.clone(), root.clone());
    let (_win, ed) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws2, root2, ConfigTarget::Collection, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    let columns = ed.read_with(cx, |e, cx| {
        let headers = e
            .table(ConfigSection::Headers)
            .expect("headers")
            .read(cx)
            .columns();
        let vars = e
            .table(ConfigSection::Vars)
            .expect("vars")
            .read(cx)
            .columns();
        (headers.description, vars.description)
    });
    assert_eq!(columns, (true, false));
}
