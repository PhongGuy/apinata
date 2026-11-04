//! Parsing CSV and JSON data files into iteration rows.

use apinata_core::{DataFileError, IterationData, iteration_total};

fn rows(d: &IterationData) -> Vec<Vec<(String, String)>> {
    (0..d.len())
        .map(|i| d.row(i).expect("row").to_vec())
        .collect()
}

fn r(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn csv_header_names_each_column_and_values_stay_strings() {
    let d = IterationData::from_csv("id,name\n1,Ann\n2,Bob\n").expect("parses");
    assert_eq!(
        rows(&d),
        [
            r(&[("id", "1"), ("name", "Ann")]),
            r(&[("id", "2"), ("name", "Bob")])
        ]
    );
    assert!(!d.is_empty());
    assert!(d.row(2).is_none());
}

#[test]
fn csv_quotes_doubled_quotes_crlf_and_a_bom() {
    let text = "\u{feff}a,b\r\n\"x, y\",\"say \"\"hi\"\"\"\r\n\"line1\nline2\",\r\n";
    let d = IterationData::from_csv(text).expect("parses");
    assert_eq!(
        rows(&d),
        [
            r(&[("a", "x, y"), ("b", "say \"hi\"")]),
            r(&[("a", "line1\nline2"), ("b", "")])
        ]
    );
}

#[test]
fn csv_blank_lines_are_skipped_and_a_missing_final_newline_is_fine() {
    let d = IterationData::from_csv("a\n\n1\n\n2").expect("parses");
    assert_eq!(rows(&d), [r(&[("a", "1")]), r(&[("a", "2")])]);
}

#[test]
fn csv_errors_name_their_line() {
    let e = IterationData::from_csv("a,b\n1,2\n3\n").expect_err("short record");
    assert_eq!(e.to_string(), "line 3: expected 2 fields, found 1");
    let e = IterationData::from_csv("a\n\"open\n").expect_err("unterminated");
    assert_eq!(e.to_string(), "line 2: unterminated quoted field");
}

#[test]
fn files_without_rows_are_empty() {
    for text in ["", "\n", "a,b\n"] {
        assert_eq!(
            IterationData::from_csv(text),
            Err(DataFileError::Empty),
            "{text:?}"
        );
    }
    assert_eq!(IterationData::from_json("[]"), Err(DataFileError::Empty));
    assert_eq!(
        DataFileError::Empty.to_string(),
        "the data file has no rows"
    );
}

#[test]
fn json_values_bind_as_javascript_strings_in_key_order() {
    let text = r#"[{"s":"x","i":1,"f":1.5,"big":1e21,"t":true,"n":null,"a":[1,"b"],"o":{"k":2}},{"b":1,"a":2}]"#;
    let d = IterationData::from_json(text).expect("parses");
    assert_eq!(
        rows(&d),
        [
            r(&[
                ("s", "x"),
                ("i", "1"),
                ("f", "1.5"),
                ("big", "1e+21"),
                ("t", "true"),
                ("n", ""),
                ("a", "[1,\"b\"]"),
                ("o", "{\"k\":2}"),
            ]),
            r(&[("b", "1"), ("a", "2")]),
        ]
    );
}

#[test]
fn json_of_another_shape_or_broken_json_is_an_error() {
    for text in ["{}", "[1]", "[{\"a\":1},2]", "\"x\""] {
        assert_eq!(
            IterationData::from_json(text),
            Err(DataFileError::NotAnArrayOfObjects),
            "{text}"
        );
    }
    let e = IterationData::from_json("[{\"a\":1},\n{").expect_err("broken");
    assert!(
        matches!(e, DataFileError::Malformed { line: 2, .. }),
        "{e:?}"
    );
    assert_eq!(
        DataFileError::NotAnArrayOfObjects.to_string(),
        "the JSON data file must be an array of objects"
    );
}

#[test]
fn iteration_total_is_the_count_each_send_sees_as_iteration_count() {
    let three = IterationData::from_csv("id\n1\n2\n3\n").expect("parses");
    // A count caps the data file's rows, and either alone sets the total.
    assert_eq!(iteration_total(Some(&three), Some(2)), 2);
    assert_eq!(iteration_total(Some(&three), Some(5)), 3);
    assert_eq!(iteration_total(Some(&three), None), 3);
    assert_eq!(iteration_total(None, Some(4)), 4);
    // With neither, the single pass is iteration 0 of 1.
    assert_eq!(iteration_total(None, None), 1);
}
