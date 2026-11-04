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
