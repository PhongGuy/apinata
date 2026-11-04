use crate::common;
use apinata_core::{CheckOptions, FindingKind, Severity};
use common::{ROOT, check_tree};

fn req(body: &str) -> String {
    format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"https://x/\"\n{body}"
    )
}
fn file_body(path: &str) -> String {
    format!(
        "  body:\n    type: file\n    data:\n      - filePath: {path}\n        contentType: application/json\n        selected: true\n"
    )
}
fn multipart(path: &str) -> String {
    format!(
        "  body:\n    type: multipart-form\n    data:\n      - name: up\n        type: file\n        value: [{path}]\n        contentType: text/plain\n"
    )
}
fn kinds(files: &[(&str, &str)]) -> Vec<(FindingKind, Severity, String)> {
    let (_d, r) = check_tree(files, &CheckOptions::default());
    r.findings()
        .iter()
        .map(|f| (f.kind(), f.severity(), f.message().to_owned()))
        .collect()
}

#[test]
fn an_existing_body_file_is_clean_and_a_missing_one_is_named() {
    assert!(
        kinds(&[
            ("opencollection.yml", ROOT),
            ("data.json", "{}"),
            ("a.yml", &req(&file_body("data.json")))
        ])
        .is_empty()
    );
    let found = kinds(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req(&file_body("nope.json"))),
    ]);
    assert_eq!(
        found,
        [(
            FindingKind::MissingFile,
            Severity::Error,
            "file `nope.json` does not exist".to_owned()
        )]
    );
}

#[test]
fn a_multipart_file_part_is_checked() {
    let found = kinds(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req(&multipart("up.bin"))),
    ]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].2, "file `up.bin` does not exist");
}

#[test]
fn absolute_and_escaping_body_paths_are_refused_even_when_they_exist() {
    let outside = tempfile::tempdir().unwrap();
    let abs = outside.path().join("x.json");
    std::fs::write(&abs, "{}").unwrap();
    let written = abs.display().to_string();
    let found = kinds(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req(&file_body(&written))),
    ]);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].2,
        format!("file `{written}` is outside the collection")
    );
    let found = kinds(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req(&file_body("../x.json"))),
    ]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].2, "file `../x.json` is outside the collection");
}

#[test]
fn a_climbing_path_that_stays_inside_is_checked_for_existence_only() {
    let found = kinds(&[
        ("opencollection.yml", ROOT),
        ("sub/keep", ""),
        ("a.yml", &req(&file_body("sub/../gone.json"))),
    ]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].2, "file `sub/../gone.json` does not exist");
}

#[test]
fn file_paths_are_interpolated_before_the_check() {
    let found = kinds(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: f\n      value: real.json\n",
        ),
        ("real.json", "{}"),
        ("a.yml", &req(&file_body("{{f}}"))),
    ]);
    assert!(found.is_empty(), "{found:?}");
}

fn root_with_cert(cert: &str) -> String {
    format!(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  clientCertificates:\n    - domain: \"x\"\n      type: pem\n      certificateFilePath: '{cert}'\n      privateKeyFilePath: '{cert}'\n"
    )
}

#[test]
fn a_client_certificate_path_is_checked_for_existence_only() {
    let abs = tempfile::tempdir().unwrap();
    let pem = abs.path().join("c.pem");
    std::fs::write(&pem, "not a certificate, never parsed").unwrap();
    assert!(
        kinds(&[
            ("opencollection.yml", &root_with_cert("c.pem")),
            ("c.pem", "x"),
            ("a.yml", &req(""))
        ])
        .is_empty()
    );
    assert!(
        kinds(&[
            (
                "opencollection.yml",
                &root_with_cert(&pem.display().to_string())
            ),
            ("a.yml", &req(""))
        ])
        .is_empty()
    );
    let found = kinds(&[
        ("opencollection.yml", &root_with_cert("gone.pem")),
        ("a.yml", &req("")),
    ]);
    assert!(!found.is_empty());
    assert!(
        found.iter().all(|f| f.0 == FindingKind::MissingFile
            && f.1 == Severity::Error
            && f.2 == "client certificate file `gone.pem` does not exist"),
        "{found:?}"
    );
}

#[test]
fn a_script_syntax_error_is_a_finding_with_the_engine_line() {
    let code = "        let ok = 1;\n        let = = 2;\n";
    let yaml = req(&format!(
        "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n{code}"
    ));
    let (_d, r) = check_tree(
        &[("opencollection.yml", ROOT), ("a.yml", &yaml)],
        &CheckOptions::default(),
    );
    let f = r
        .findings()
        .iter()
        .find(|f| f.kind() == FindingKind::ScriptSyntax)
        .expect("a ScriptSyntax finding");
    assert_eq!((f.item(), f.severity()), ("a.yml", Severity::Error));
    assert!(
        f.message().contains("request script, line 2"),
        "{}",
        f.message()
    );
}

#[test]
fn an_unresolved_request_still_gets_its_script_checked() {
    let yaml = req(
        "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        let = = 2;\n",
    )
    .replace("https://x/", "https://x/{{nope}}");
    let (_d, r) = check_tree(
        &[("opencollection.yml", ROOT), ("a.yml", &yaml)],
        &CheckOptions::default(),
    );
    assert!(
        r.findings()
            .iter()
            .any(|f| f.kind() == FindingKind::ScriptSyntax && f.severity() == Severity::Error)
    );
}

#[test]
fn an_unsupported_shape_expression_with_a_syntax_error_is_reported() {
    let yaml = req(
        "runtime:\n  assertions:\n    - expression: \"res.body.map(x => )\"\n      operator: eq\n      value: \"1\"\n",
    );
    let (_d, r) = check_tree(
        &[("opencollection.yml", ROOT), ("a.yml", &yaml)],
        &CheckOptions::default(),
    );
    assert!(
        r.findings()
            .iter()
            .any(|f| f.kind() == FindingKind::ScriptSyntax && f.severity() == Severity::Error),
        "{:?}",
        r.findings()
    );
}

#[test]
fn code_valid_in_the_engine_is_never_an_error_finding() {
    for code in ["var static = 1;", "x = 010;", "with (a) {}"] {
        let yaml = req(&format!(
            "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        {code}\n"
        ));
        let (_d, r) = check_tree(
            &[("opencollection.yml", ROOT), ("a.yml", &yaml)],
            &CheckOptions::default(),
        );
        assert!(
            r.findings().iter().all(|f| f.severity() != Severity::Error),
            "{code}: {:?}",
            r.findings()
        );
    }
}

#[test]
fn a_sloppy_only_script_is_a_warning_with_the_fixed_suffix() {
    let yaml = req(
        "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        with (a) {}\n",
    );
    let (_d, r) = check_tree(
        &[("opencollection.yml", ROOT), ("a.yml", &yaml)],
        &CheckOptions::default(),
    );
    let f = r
        .findings()
        .iter()
        .find(|f| f.kind() == FindingKind::ScriptSyntax)
        .expect("a warning");
    assert_eq!(f.severity(), Severity::Warning);
    assert!(
        f.message()
            .ends_with(" (valid only in sloppy mode, the engine may run it)"),
        "{}",
        f.message()
    );
}

#[test]
fn a_clean_script_and_an_assertion_expression_produce_no_finding() {
    let yaml = req(
        "runtime:\n  scripts:\n    - type: tests\n      code: |-\n        test('ok', () => expect(res.status).to.equal(200));\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n",
    );
    let (_d, r) = check_tree(
        &[("opencollection.yml", ROOT), ("a.yml", &yaml)],
        &CheckOptions::default(),
    );
    assert!(r.findings().is_empty(), "{:?}", r.findings());
}

fn script_of(code: &str) -> String {
    format!("runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        {code}\n")
}

fn syntax_items(files: &[(&str, &str)]) -> Vec<(String, String)> {
    let (_d, r) = check_tree(files, &CheckOptions::default());
    r.findings()
        .iter()
        .filter(|f| f.kind() == FindingKind::ScriptSyntax)
        .map(|f| (f.item().to_owned(), f.message().to_owned()))
        .collect()
}

#[test]
fn a_broken_folder_script_is_reported_once_against_its_folder_file() {
    let folder = format!(
        "info:\n  name: d\n  type: folder\nrequest:\n{}",
        script_of("let = = 1;").replace("runtime:\n", "")
    );
    let found = syntax_items(&[
        ("opencollection.yml", ROOT),
        ("d/folder.yml", &folder),
        ("d/a.yml", &req("")),
        ("d/b.yml", &req("").replace("seq: 1", "seq: 2")),
        ("d/c.yml", &req("").replace("seq: 1", "seq: 3")),
    ]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "d/folder.yml");
    assert!(
        found[0].1.starts_with("folder d script, line 1"),
        "{}",
        found[0].1
    );
}

#[test]
fn a_broken_collection_script_is_reported_once_against_the_collection_file() {
    let root = format!(
        "{ROOT}request:\n{}",
        script_of("let = = 1;").replace("runtime:\n", "")
    );
    let found = syntax_items(&[
        ("opencollection.yml", &root),
        ("a.yml", &req("")),
        ("b.yml", &req("").replace("seq: 1", "seq: 2")),
    ]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "opencollection.yml");
    assert!(
        found[0].1.starts_with("collection script"),
        "{}",
        found[0].1
    );
}

#[test]
fn an_expression_error_says_expression_not_request_script() {
    let yaml = req(
        "runtime:\n  assertions:\n    - expression: \"res.body.map(x => )\"\n      operator: eq\n      value: \"1\"\n",
    );
    let found = syntax_items(&[("opencollection.yml", ROOT), ("a.yml", &yaml)]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "a.yml");
    assert!(
        found[0].1.starts_with("assertion or variable expression"),
        "{}",
        found[0].1
    );
}

#[test]
fn a_multipart_path_is_interpolated_before_the_existence_check() {
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: f\n      value: real.bin\n";
    let found = kinds(&[
        ("opencollection.yml", root),
        ("real.bin", "x"),
        ("a.yml", &req(&multipart("\"{{f}}\""))),
    ]);
    assert!(found.is_empty(), "{found:?}");
    let found = kinds(&[
        ("opencollection.yml", root),
        ("a.yml", &req(&multipart("\"{{f}}\""))),
    ]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].2, "file `{{f}}` does not exist");
}
