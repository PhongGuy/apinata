use crate::hostile::hostile_yaml;
use apinata_format::{ReadErrorKind, read_mapping};
use yaml_serde::Value;

fn refused(bytes: &[u8]) -> (ReadErrorKind, Option<(usize, usize)>, String) {
    let e = read_mapping(bytes).unwrap_err();
    (e.kind(), e.position(), e.to_string())
}

#[test]
fn a_mapping_reads_in_document_order() {
    let map = read_mapping(b"b: 1\na: [x]\n").unwrap();
    let keys: Vec<_> = map.keys().map(|k| k.as_str().unwrap()).collect();
    assert_eq!(keys, ["b", "a"]);
    assert_eq!(map.get("b"), Some(&Value::from(1)));
}

#[test]
fn invalid_utf8_is_not_utf8() {
    assert_eq!(
        refused(b"a: \xff\n"),
        (
            ReadErrorKind::NotUtf8,
            None,
            "file is not valid UTF-8".to_owned()
        )
    );
}

#[test]
fn a_hostile_file_is_refused_quickly_as_too_many_values() {
    let text = format!("info:\n  name: r\n{}", hostile_yaml());
    let started = std::time::Instant::now();
    let got = refused(text.as_bytes());
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(
        got,
        (
            ReadErrorKind::TooManyValues,
            None,
            "file is too large: more than 2000000 values once aliases expand".to_owned()
        )
    );
}

#[test]
fn aliases_past_a_prepass_error_are_unfollowable() {
    let text = format!("k: \"a\n\tb\"\n{}", hostile_yaml());
    assert_eq!(
        refused(text.as_bytes()),
        (
            ReadErrorKind::UnfollowableAliases,
            None,
            "file has aliases and a syntax the safety check cannot follow".to_owned()
        )
    );
}

#[test]
fn a_syntax_error_carries_the_parser_text_and_position() {
    let text = "a: [1\nb: 2\n";
    let parser = yaml_serde::from_str::<Value>(text).unwrap_err();
    let at = parser.location().map(|l| (l.line(), l.column()));
    assert!(at.is_some());
    assert_eq!(
        refused(text.as_bytes()),
        (ReadErrorKind::Syntax, at, parser.to_string())
    );
}

#[test]
fn a_bom_is_stripped_before_positions_are_counted() {
    let (kind, at, _) = refused("\u{feff}@x: 1\n".as_bytes());
    assert_eq!((kind, at), (ReadErrorKind::Syntax, Some((1, 1))));
    assert!(read_mapping("\u{feff}a: 1\n".as_bytes()).is_ok());
}

#[test]
fn conflict_markers_name_the_first_marker_line() {
    let text = "name: dev\n<<<<<<< HEAD\nvariables: []\n=======\nvariables: {}\n>>>>>>> o\n";
    let (kind, at, message) = refused(text.as_bytes());
    assert_eq!(kind, ReadErrorKind::Syntax);
    assert!(at.is_some());
    assert_eq!(
        message,
        "merge conflict markers at line 2; resolve them in git"
    );
}

#[test]
fn marker_lines_in_a_block_scalar_still_read() {
    assert!(read_mapping(b"a: |\n  <<<<<<< x\n  =======\n").is_ok());
}

#[test]
fn a_tag_anywhere_is_refused() {
    assert_eq!(
        refused(b"a:\n  b: !x 1\n"),
        (
            ReadErrorKind::Tag,
            None,
            "YAML tags are not supported".to_owned()
        )
    );
}

#[test]
fn a_tagged_root_is_a_tag_not_a_non_mapping() {
    assert_eq!(refused(b"!x [1]\n").0, ReadErrorKind::Tag);
}

#[test]
fn a_root_that_is_not_a_mapping_is_refused() {
    for text in ["", "  \n", "- 1\n", "just text\n", "~\n"] {
        assert_eq!(
            refused(text.as_bytes()),
            (
                ReadErrorKind::NotMapping,
                None,
                "the file root must be a mapping".to_owned()
            ),
            "{text:?}"
        );
    }
}
