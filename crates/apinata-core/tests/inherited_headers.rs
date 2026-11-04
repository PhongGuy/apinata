//! The headers an item inherits from its collection and folders, and which of them an inner
//! level or the request's own rows override.

use apinata_core::{Collection, HeaderSource, ItemPath, KeyValue};

use crate::common::write_tree;

const REQ: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n";

fn root(headers: &str) -> String {
    format!("opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  headers:\n{headers}")
}

fn folder(name: &str, headers: &str) -> String {
    format!("info:\n  name: {name}\n  type: folder\nrequest:\n  headers:\n{headers}")
}

fn open(files: &[(&str, &str)]) -> (tempfile::TempDir, Collection) {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn path(p: &str) -> ItemPath {
    ItemPath::new(p).unwrap()
}

#[test]
fn collection_and_folder_rows_list_with_their_sources() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-A\n      value: a\n"),
        ),
        (
            "f/folder.yml",
            &folder("Folder F", "    - name: X-B\n      value: \"{{token}}\"\n"),
        ),
        ("f/r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("f/r.yml"));
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].name(), got[0].value()), ("X-A", "a"));
    assert_eq!(got[0].source(), &HeaderSource::Collection);
    assert_eq!((got[1].name(), got[1].value()), ("X-B", "{{token}}"));
    assert_eq!(got[1].source(), &HeaderSource::Folder(path("f")));
    assert!(!got[0].overridden(&[]) && !got[1].overridden(&[]));
}

#[test]
fn nested_folders_list_outermost_first() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: A\n      value: 1\n"),
        ),
        (
            "f/folder.yml",
            &folder("f", "    - name: B\n      value: 2\n"),
        ),
        (
            "f/g/folder.yml",
            &folder("g", "    - name: C\n      value: 3\n"),
        ),
        ("f/g/r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("f/g/r.yml"));
    let names: Vec<_> = got.iter().map(|h| h.name()).collect();
    assert_eq!(names, ["A", "B", "C"]);
    assert_eq!(got[2].source(), &HeaderSource::Folder(path("f/g")));
}

#[test]
fn disabled_and_nameless_rows_are_skipped() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root(
                "    - name: Off\n      value: x\n      disabled: true\n    - name: \"\"\n      value: y\n    - name: On\n      value: z\n",
            ),
        ),
        ("r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("r.yml"));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name(), "On");
}

#[test]
fn an_inner_folder_row_overrides_a_collection_row() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-A\n      value: outer\n"),
        ),
        (
            "f/folder.yml",
            &folder("f", "    - name: X-A\n      value: inner\n"),
        ),
        ("f/r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("f/r.yml"));
    assert_eq!(got.len(), 2);
    assert!(got[0].overridden(&[]));
    assert!(!got[1].overridden(&[]));
}

#[test]
fn a_disabled_inner_row_does_not_override() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-A\n      value: outer\n"),
        ),
        (
            "f/folder.yml",
            &folder(
                "f",
                "    - name: X-A\n      value: inner\n      disabled: true\n",
            ),
        ),
        ("f/r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("f/r.yml"));
    assert_eq!(got.len(), 1);
    assert!(!got[0].overridden(&[]));
}

#[test]
fn own_rows_override_by_exact_name_only() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-A\n      value: a\n"),
        ),
        ("r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("r.yml"));
    assert!(got[0].overridden(&[KeyValue::new("X-A", "mine")]));
    let mut off = KeyValue::new("X-A", "mine");
    off.enabled = false;
    assert!(!got[0].overridden(&[off]));
    assert!(!got[0].overridden(&[KeyValue::new("", "mine")]));
}

#[test]
fn a_repeated_name_in_one_layer_lists_only_the_last() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-A\n      value: first\n    - name: X-A\n      value: last\n"),
        ),
        ("r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("r.yml"));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].value(), "last");
}

#[test]
fn nothing_inherited_is_empty() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        ),
        ("f/folder.yml", "info:\n  name: f\n  type: folder\n"),
        ("f/r.yml", REQ),
    ]);
    assert!(c.inherited_headers(&path("f/r.yml")).is_empty());
}

#[test]
fn whitespace_in_a_name_makes_it_a_different_name() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-A\n      value: a\n"),
        ),
        ("r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("r.yml"));
    assert!(!got[0].overridden(&[KeyValue::new(" X-A", "mine")]));
    assert!(!got[0].overridden(&[KeyValue::new("X-A ", "mine")]));
}

#[test]
fn a_folder_row_is_overridden_by_the_request_not_by_the_collection() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-A\n      value: outer\n"),
        ),
        (
            "f/folder.yml",
            &folder("f", "    - name: X-B\n      value: inner\n"),
        ),
        ("f/r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("f/r.yml"));
    assert!(!got[0].overridden(&[]) && !got[1].overridden(&[]));
    assert!(got[1].overridden(&[KeyValue::new("X-B", "mine")]));
    assert!(!got[0].overridden(&[KeyValue::new("X-B", "mine")]));
}

#[test]
fn any_enabled_own_row_of_the_name_overrides_even_after_a_disabled_one() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-A\n      value: a\n"),
        ),
        ("r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("r.yml"));
    let mut off = KeyValue::new("X-A", "1");
    off.enabled = false;
    let own = [off, KeyValue::new("X-A", "2"), KeyValue::new("X-A", "3")];
    assert!(got[0].overridden(&own));
    assert!(!got[0].overridden(&own[..1]));
}

/// The headers a send puts on the wire for `rel`, as resolved.
fn sent(files: &[(&str, &str)], rel: &str) -> Vec<(String, String)> {
    let (_d, c, ctx) = crate::common::setup(files);
    crate::common::at(&c, &ctx, rel)
        .expect("resolves")
        .headers()
        .to_vec()
}

#[test]
fn the_send_folds_names_that_differ_only_in_case() {
    // The send keeps the first spelling and the last value, so one header goes out.
    let got = sent(
        &[
            (
                "opencollection.yml",
                &root("    - name: X-Token\n      value: a\n"),
            ),
            (
                "r.yml",
                &format!("{REQ}  headers:\n    - name: x-token\n      value: b\n"),
            ),
        ],
        "r.yml",
    );
    assert_eq!(got, [("X-Token".to_owned(), "b".to_owned())]);
}

#[test]
fn a_case_different_own_row_overrides_as_the_send_folds_it() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-Token\n      value: a\n"),
        ),
        ("r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("r.yml"));
    assert!(got[0].overridden(&[KeyValue::new("x-token", "b")]));
}

#[test]
fn a_case_different_folder_row_overrides_the_collection_row() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-Token\n      value: a\n"),
        ),
        (
            "f/folder.yml",
            &folder("f", "    - name: x-token\n      value: b\n"),
        ),
        ("f/r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("f/r.yml"));
    assert!(got[0].overridden(&[]));
    assert!(!got[1].overridden(&[]));
}

#[test]
fn a_later_case_different_row_in_one_layer_shadows_the_earlier() {
    let (_d, c) = open(&[
        (
            "opencollection.yml",
            &root("    - name: X-A\n      value: 1\n    - name: x-a\n      value: 2\n"),
        ),
        ("r.yml", REQ),
    ]);
    let got = c.inherited_headers(&path("r.yml"));
    assert!(got[0].overridden(&[]));
    assert!(!got[1].overridden(&[]));
}
