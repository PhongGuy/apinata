//! Bulk text for key-value tables: format, parsing, and row identity across an edit.

use apinata_app::kv_bulk::{bulk_rows, bulk_text};
use apinata_app::kv_table::{
    KvRow, header_rows, headers_from, param_rows, params_from, var_rows, vars_from,
};
use apinata_core::{KeyValue, Param, ParamKind, VarValue, Variable};

fn row(name: &str, value: &str, enabled: bool) -> KvRow {
    KvRow {
        name: name.into(),
        value: value.into(),
        enabled,
        ..KvRow::default()
    }
}

#[test]
fn rows_become_lines_with_disabled_ones_commented() {
    let rows = [row("a", "1", true), row("b", "x: y", false)];
    assert_eq!(bulk_text(&rows), "a: 1\n//b: x: y");
    assert_eq!(bulk_text(&[]), "");
}

#[test]
fn lines_parse_on_the_first_colon_and_trim() {
    let rows = bulk_rows(
        "  a :1 \n// b: x: y\n//c:3\nno colon\n\n: nameless\nd:",
        &[],
    );
    let got: Vec<_> = rows
        .iter()
        .map(|r| (r.name.as_str(), r.value.as_str(), r.enabled, r.source))
        .collect();
    assert_eq!(
        got,
        [
            ("a", "1", true, None),
            ("b", "x: y", false, None),
            ("c", "3", false, None),
            ("d", "", true, None)
        ]
    );
}

#[test]
fn crlf_and_duplicates_are_kept_as_rows() {
    let rows = bulk_rows("a: 1\r\na: 2\r\n", &[]);
    let got: Vec<_> = rows
        .iter()
        .map(|r| (r.name.as_str(), r.value.as_str()))
        .collect();
    assert_eq!(got, [("a", "1"), ("a", "2")]);
}

#[test]
fn text_round_trips_to_the_same_rows() {
    let mut current = vec![
        row("a", "1", true),
        row("b", "2", false),
        row("a", "3", true),
    ];
    for (ix, r) in current.iter_mut().enumerate() {
        r.source = Some(ix);
    }
    assert_eq!(bulk_rows(&bulk_text(&current), &current), current);
}

#[test]
fn a_moved_or_edited_row_keeps_its_source_and_a_renamed_one_is_new() {
    let mut current = vec![
        row("a", "1", true),
        row("b", "2", true),
        row("c", "3", true),
    ];
    for (ix, r) in current.iter_mut().enumerate() {
        r.source = Some(ix);
    }
    let rows = bulk_rows("c: 30\na: 1\nz: 2", &current);
    let got: Vec<_> = rows
        .iter()
        .map(|r| (r.name.as_str(), r.value.as_str(), r.source))
        .collect();
    assert_eq!(
        got,
        [("c", "30", Some(2)), ("a", "1", Some(0)), ("z", "2", None)]
    );
}

#[test]
fn duplicate_names_match_the_nearest_unused_row() {
    let mut current = vec![row("a", "1", true), row("x", "", true), row("a", "2", true)];
    for (ix, r) in current.iter_mut().enumerate() {
        r.source = Some(ix);
    }
    let rows = bulk_rows("x: \na: 2b\na: 1b", &current);
    let sources: Vec<_> = rows.iter().map(|r| r.source).collect();
    // Line 1 "a" is nearest to row 0 (distance 1 vs 1: tie goes to the lower index), line 2 takes the other.
    assert_eq!(sources, [Some(1), Some(0), Some(2)]);
}

#[test]
fn fixed_and_file_rows_stay_out_of_the_text_and_follow_unchanged() {
    let base = vec![
        Param::new("q", "1", ParamKind::Query),
        Param::new("id", "7", ParamKind::Path),
    ];
    let current = param_rows(&base);
    assert!(current[1].fixed, "a path param is a fixed row");
    assert_eq!(bulk_text(&current), "q: 1");
    let rows = bulk_rows("q: 2\nr: 3", &current);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2], current[1]);
    let out = params_from(&base, &rows);
    let got: Vec<_> = out
        .iter()
        .map(|p| (p.name.as_str(), p.value.as_str(), p.kind))
        .collect();
    assert_eq!(
        got,
        [
            ("q", "2", ParamKind::Query),
            ("r", "3", ParamKind::Query),
            ("id", "7", ParamKind::Path)
        ]
    );

    let file = KvRow {
        files: Some(vec!["/f".into()]),
        source: Some(1),
        ..row("f", "", true)
    };
    let current = vec![
        KvRow {
            source: Some(0),
            ..row("t", "1", true)
        },
        file.clone(),
    ];
    assert_eq!(bulk_text(&current), "t: 1");
    assert_eq!(bulk_rows("", &current), [file]);
}

#[test]
fn a_bulk_edit_keeps_header_descriptions() {
    let mut accept = KeyValue::new("Accept", "a/b");
    accept.description = Some("What we take".into());
    let base = vec![KeyValue::new("X-A", "1"), accept];
    let rows = bulk_rows("Accept: c/d\n//X-A: 1", &header_rows(&base));
    let out = headers_from(&base, &rows);
    assert_eq!(
        (out[0].name.as_str(), out[0].value.as_str()),
        ("Accept", "c/d")
    );
    assert_eq!(out[0].description.as_deref(), Some("What we take"));
    assert!(!out[1].enabled);
}

#[test]
fn a_bulk_edit_keeps_a_typed_var_kind() {
    let base = vec![Variable::new(
        "n",
        VarValue::Typed {
            kind: "number".into(),
            data: "1".into(),
        },
    )];
    let rows = bulk_rows("n: 2", &var_rows(&base));
    let out = vars_from(&base, &rows);
    assert!(
        out[0].value
            == VarValue::Typed {
                kind: "number".into(),
                data: "2".into()
            }
    );
}

#[test]
fn a_row_entry_outlives_its_source() {
    let mut accept = KeyValue::new("Accept", "a/b");
    accept.description = Some("What we take".into());
    // The entry left the document (a row deleted for a moment while typing), but the row keeps it.
    let row = KvRow {
        source: None,
        ..header_rows(&[accept])[0].clone()
    };
    let out = headers_from(&[], &[row]);
    assert_eq!(out[0].description.as_deref(), Some("What we take"));
}

#[test]
fn row_equality_ignores_the_entry() {
    let rows = header_rows(&[KeyValue::new("a", "1")]);
    assert_eq!(
        rows[0],
        KvRow {
            entry: None,
            ..rows[0].clone()
        }
    );
}

#[test]
fn a_multi_line_value_stays_out_of_the_text_and_is_kept() {
    let multi = KvRow {
        source: Some(1),
        ..row("m", "x\ny", true)
    };
    let current = vec![
        KvRow {
            source: Some(0),
            ..row("a", "1", true)
        },
        multi.clone(),
    ];
    assert_eq!(bulk_text(&current), "a: 1");
    let rows = bulk_rows("a: 2", &current);
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].value.as_str(), rows[0].source), ("2", Some(0)));
    assert_eq!(rows[1], multi);
}
