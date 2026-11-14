//! Check findings in shapes the first tests missed: deep folders, two script errors in one
//! request, and a finding tied to one environment.

use crate::common;
use apinata_core::{CheckOptions, FindingKind, Severity};
use common::{ROOT, check_tree};

fn req(url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}

/// A `runtime.scripts` block with one entry per `(type, code)` pair, code on one line.
fn scripts(entries: &[(&str, &str)]) -> String {
    let items: String = entries
        .iter()
        .map(|(kind, code)| format!("    - type: {kind}\n      code: |-\n        {code}\n"))
        .collect();
    format!("runtime:\n  scripts:\n{items}")
}

type Row = (String, Option<String>, Severity, FindingKind, String);

fn rows(files: &[(&str, &str)]) -> Vec<Row> {
    let (_d, r) = check_tree(files, &CheckOptions::default());
    r.findings()
        .iter()
        .map(|f| {
            (
                f.item().to_owned(),
                f.environment().map(str::to_owned),
                f.severity(),
                f.kind(),
                f.message().to_owned(),
            )
        })
        .collect()
}

#[test]
fn findings_two_folders_deep_use_slash_paths_and_a_folder_script_is_reported_once() {
    let bad = scripts(&[("before-request", "let = = 1;")]).replace("runtime:\n", "request:\n");
    let folder = format!("info:\n  name: b\n  type: folder\n{bad}");
    let found = rows(&[
        ("opencollection.yml", ROOT),
        ("a/b/folder.yml", &folder),
        ("a/b/c/one.yml", &req("http://{{nope}}/", "")),
        (
            "a/b/c/two.yml",
            &req("http://x/", "").replace("seq: 1", "seq: 2"),
        ),
        (
            "a/b/three.yml",
            &req("http://x/", "").replace("seq: 1", "seq: 3"),
        ),
    ]);
    let error = |item: &str, kind, message: &str| {
        (
            item.to_owned(),
            None,
            Severity::Error,
            kind,
            message.to_owned(),
        )
    };
    assert_eq!(
        found,
        [
            error(
                "a/b/c/one.yml",
                FindingKind::UndefinedVariable,
                "variable `nope` is not defined"
            ),
            error(
                "a/b/folder.yml",
                FindingKind::ScriptSyntax,
                "folder a/b script, line 1: variable name expected"
            ),
        ]
    );
}

#[test]
fn two_script_errors_in_one_request_are_two_findings_in_message_order() {
    let both = scripts(&[
        ("before-request", "let = = 1;"),
        ("after-response", "let x = ;"),
    ]);
    let found = rows(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("http://x/", &both)),
    ]);
    let message = |text: &str| {
        (
            "a.yml".to_owned(),
            None,
            Severity::Error,
            FindingKind::ScriptSyntax,
            text.to_owned(),
        )
    };
    assert_eq!(
        found,
        [
            message("this request's script, line 1: unexpected token in expression: ';'"),
            message("this request's script, line 1: variable name expected"),
        ]
    );
}

#[test]
fn two_scripts_failing_with_the_same_message_are_reported_once() {
    let both = scripts(&[
        ("before-request", "let = = 1;"),
        ("after-response", "let = = 2;"),
    ]);
    let found = rows(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("http://x/", &both)),
    ]);
    assert_eq!(
        found,
        [(
            "a.yml".to_owned(),
            None,
            Severity::Error,
            FindingKind::ScriptSyntax,
            "this request's script, line 1: variable name expected".to_owned()
        )]
    );
}

#[test]
fn a_finding_in_one_environment_carries_its_name() {
    let dev = "name: dev\nvariables:\n  - name: host\n    value: example.test\n";
    let prod = "name: prod\nvariables:\n  - name: other\n    value: x\n";
    let found = rows(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("environments/prod.yml", prod),
        ("a.yml", &req("http://{{host}}/", "")),
    ]);
    assert_eq!(
        found,
        [(
            "a.yml".to_owned(),
            Some("prod".to_owned()),
            Severity::Error,
            FindingKind::UndefinedVariable,
            "variable `host` is not defined; defined in environment dev".to_owned()
        )]
    );
}
