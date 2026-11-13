use apinata_core::RequestFile;

#[test]
fn an_edited_assertion_keeps_its_description_and_unknown_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.yml");
    let yaml = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n      description: kept\n      x-note: mine\n";
    std::fs::write(&path, yaml).unwrap();
    let mut r = RequestFile::load(&path).unwrap();
    let mut list = r.assertions();
    list[0].set_operator("neq", Some("500".to_owned()));
    list[0].set_expression("res.code");
    assert_eq!(
        (
            list[0].expression(),
            list[0].operator(),
            list[0].value(),
            list[0].description()
        ),
        ("res.code", "neq", Some("500"), Some("kept"))
    );
    r.set_assertions(list);
    r.save().unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    let want = "    - expression: res.code\n      operator: neq\n      value: \"500\"\n      description: kept\n      x-note: mine\n";
    assert!(saved.ends_with(want), "{saved}");
    let mut unary = RequestFile::load(&path).unwrap().assertions();
    unary[0].set_operator("isNull", None);
    assert_eq!(
        (unary[0].value(), unary[0].description()),
        (None, Some("kept"))
    );
}

// A single-value check reads unquoted text that is no number, keyword, variable or template as
// a plain string; quoted text, lists, ranges and patterns are not flagged.
#[test]
fn an_unquoted_text_value_compares_as_text() {
    use apinata_core::Assertion;
    let check = |op: &str, value: &str| Assertion::new("res.body", op, Some(value.into()));
    for text in ["foo bar", "Leanne", "7a4"] {
        assert!(check("eq", text).compares_as_text(), "{text}");
    }
    assert!(check("contains", "foo bar").compares_as_text());
    for not_text in [
        "\"foo bar\"",
        "'foo bar'",
        "200",
        "-1.5",
        "true",
        "null",
        "undefined",
        "",
        "{{name}}",
        "`x`",
    ] {
        assert!(!check("eq", not_text).compares_as_text(), "{not_text:?}");
    }
    for (op, value) in [("in", "a, b"), ("between", "1, 2"), ("matches", "^a")] {
        assert!(!check(op, value).compares_as_text(), "{op}");
    }
    assert!(!Assertion::new("res.body", "isNull", None).compares_as_text());
}
