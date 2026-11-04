use crate::common;
use apinata_core::Collection;
use common::{ROOT, write_tree};

const REQ: &str = "info:\n  name: a\n  type: http\nhttp:\n  method: GET\n  url: http://x/a\n";

/// Open the tree and return the messages of its load errors, ordered by path.
fn messages(files: &[(&str, &str)]) -> Vec<(String, String)> {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("opens");
    let mut out: Vec<_> = c
        .load_errors()
        .iter()
        .map(|e| {
            let rel = e.path().strip_prefix(dir.path()).unwrap_or(e.path());
            (rel.display().to_string(), e.message().to_owned())
        })
        .collect();
    out.sort();
    out
}

const MSG3: &str = "merge conflict markers at line 3; resolve them in git";

#[test]
fn a_conflicted_request_file_names_the_first_marker_line() {
    let text = "info:\n  name: a\n<<<<<<< HEAD\n  type: http\n=======\n  type: ws\n>>>>>>> other\n";
    assert_eq!(
        messages(&[("opencollection.yml", ROOT), ("a.yml", text)]),
        [("a.yml".to_owned(), MSG3.to_owned())]
    );
}

#[test]
fn a_conflicted_folder_file_is_named() {
    let text = "info:\n  name: f\n<<<<<<< HEAD\n  x: 1\n=======\n  x: 2\n>>>>>>> o\n";
    assert_eq!(
        messages(&[("opencollection.yml", ROOT), ("f/folder.yml", text)]),
        [("f/folder.yml".to_owned(), MSG3.to_owned())]
    );
}

#[test]
fn a_conflicted_environment_file_is_named() {
    let text = "name: dev\n<<<<<<< HEAD\nvariables: []\n=======\nvariables: {}\n>>>>>>> o\n";
    assert_eq!(
        messages(&[("opencollection.yml", ROOT), ("environments/dev.yml", text)]),
        [(
            "environments/dev.yml".to_owned(),
            "merge conflict markers at line 2; resolve them in git".to_owned()
        )]
    );
}

#[test]
fn bare_markers_are_found_at_their_line() {
    let text = "info:\n  name: a\n<<<<<<<\n  x: 1\n>>>>>>>\n";
    assert_eq!(
        messages(&[("opencollection.yml", ROOT), ("a.yml", text)]),
        [("a.yml".to_owned(), MSG3.to_owned())]
    );
}

#[test]
fn a_crlf_file_reports_the_marker_line() {
    let text = "info:\r\n  name: a\r\n<<<<<<< HEAD\r\n  type: http\r\n=======\r\n  type: ws\r\n>>>>>>> o\r\n";
    assert_eq!(
        messages(&[("opencollection.yml", ROOT), ("a.yml", text)]),
        [("a.yml".to_owned(), MSG3.to_owned())]
    );
}

#[test]
fn a_valid_file_with_a_bom_loads_without_errors() {
    // Without the BOM strip the parser sees a second document and the load fails.
    let text = format!("\u{feff}{REQ}");
    assert_eq!(
        messages(&[("opencollection.yml", ROOT), ("a.yml", &text)]),
        []
    );
}

#[test]
fn marker_lines_inside_a_block_scalar_load_fine() {
    let text = format!(
        "{REQ}docs:\n  content: |\n    notes\n    <<<<<<< HEAD\n    =======\n    >>>>>>> x\n"
    );
    assert_eq!(
        messages(&[("opencollection.yml", ROOT), ("a.yml", &text)]),
        []
    );
}

#[test]
fn near_miss_markers_keep_the_yaml_message() {
    for line in ["<<<<<<<x", "  <<<<<<< a"] {
        let text = format!("info: [\n{line}\n");
        let got = messages(&[("opencollection.yml", ROOT), ("a.yml", &text)]);
        assert_eq!(got.len(), 1, "{line}");
        assert!(!got[0].1.contains("merge conflict"), "{line}: {}", got[0].1);
    }
}

#[test]
fn a_plain_syntax_error_keeps_the_yaml_message() {
    let got = messages(&[("opencollection.yml", ROOT), ("a.yml", "info: [\n")]);
    assert_eq!(got.len(), 1);
    assert!(!got[0].1.contains("merge conflict"), "{}", got[0].1);
}
