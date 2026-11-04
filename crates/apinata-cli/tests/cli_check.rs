//! `apinata check`: exit codes, text and JSON.

use crate::common;
use common::{ROOT, write_tree};
use serde_json::Value;

/// The binary with a throwaway HOME and XDG dirs, so nothing reads the real keyring or config.
fn apinata() -> (assert_cmd::Command, tempfile::TempDir) {
    let home = tempfile::TempDir::new().expect("home");
    let mut cmd = common::apinata();
    cmd.env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env_remove("XDG_RUNTIME_DIR");
    (cmd, home)
}

fn req(url: &str) -> String {
    format!("info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

fn stdout(out: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn a_clean_collection_exits_0_and_prints_only_the_summary() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &req("http://x/"))]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(0);
    let text = stdout(&out);
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains("0 errors"), "{text}");
}

#[test]
fn an_error_finding_exits_1_with_one_line_per_finding() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("http://{{nope}}/")),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(1);
    let text = stdout(&out);
    assert!(text.starts_with("error: a.yml: "), "{text}");
    assert!(text.contains("nope"), "{text}");
}

#[test]
fn warnings_alone_exit_0() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("http://x/{{process.env.NOPE_I344}}")),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(0);
    assert!(stdout(&out).starts_with("warning: a.yml: "));
}

#[test]
fn env_var_flags_define_names_and_path_is_positional() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("http://{{h}}/{{g}}")),
    ]);
    let (mut cmd, _home) = apinata();
    cmd.args([
        "check",
        dir.path().to_str().unwrap(),
        "--env-var",
        "h=x",
        "--global-env-var",
        "g=y",
    ])
    .assert()
    .code(0);
}

#[test]
fn usage_failures_exit_3() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let (mut cmd, _home) = apinata();
    cmd.current_dir(dir.path())
        .args(["check", "--env", "nope"])
        .assert()
        .code(3);
    let empty = write_tree(&[("x.txt", "x")]);
    let (mut cmd, _home) = apinata();
    cmd.current_dir(empty.path()).arg("check").assert().code(3);
    let (mut cmd, _home) = apinata();
    cmd.current_dir(dir.path())
        .args(["check", "--env-var", "novalue"])
        .assert()
        .code(3);
}

#[test]
fn json_has_a_fixed_shape_and_matches_the_text_counts() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("http://{{nope}}/")),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd
        .current_dir(dir.path())
        .args(["check", "--json"])
        .assert()
        .code(1);
    let raw = stdout(&out);
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["summary"]["errors"], 1);
    assert_eq!(v["summary"]["items_checked"], 1);
    assert_eq!(v["findings"][0]["item"], "a.yml");
    assert_eq!(v["findings"][0]["kind"], "undefined_variable");
    assert!(raw.find("\"version\"").unwrap() < raw.find("\"summary\"").unwrap());
    assert!(raw.find("\"summary\"").unwrap() < raw.find("\"findings\"").unwrap());

    let (mut cmd, _home) = apinata();
    let text = cmd.current_dir(dir.path()).arg("check").assert().code(1);
    assert!(stdout(&text).contains("1 error, 0 warnings, 1 request checked"));
}

#[test]
fn a_secret_value_never_reaches_the_output() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: token\n    secret: true\n",
        ),
        (".env", "token=canary-cli-5d1e\n"),
        ("a.yml", &req("http://x/{{token}}/{{gone}}")),
    ]);
    let runs: [&[&str]; 4] = [
        &["check"],
        &["check", "--json"],
        &["check", "--env-var", "other=canary-cli-5d1e"],
        &["check", "--json", "--env-var", "other=canary-cli-5d1e"],
    ];
    for args in runs {
        let (mut cmd, _home) = apinata();
        let out = cmd.current_dir(dir.path()).args(args).output().unwrap();
        let all = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(!all.contains("canary-cli-5d1e"), "{args:?}: {all}");
    }
}

#[test]
fn a_finding_two_folders_deep_prints_its_slash_path() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a/b/one.yml", &req("http://{{nope}}/")),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(1);
    let text = stdout(&out);
    assert_eq!(
        text.lines().next(),
        Some("error: a/b/one.yml: variable `nope` is not defined"),
        "{text}"
    );
}

#[test]
fn two_script_errors_in_one_request_print_two_lines_in_message_order() {
    let scripts = "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        let = = 1;\n    - type: after-response\n      code: |-\n        let x = ;\n";
    let yaml = format!("{}{scripts}", req("http://x/"));
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &yaml)]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(1);
    let lines: Vec<String> = stdout(&out).lines().map(str::to_owned).collect();
    assert_eq!(
        lines[..2],
        [
            "error: a.yml: request script, line 1: unexpected token in expression: ';'",
            "error: a.yml: request script, line 1: variable name expected",
        ]
    );
    assert!(lines[2].starts_with("2 errors, 0 warnings"), "{lines:?}");
}

#[test]
fn an_environment_tagged_finding_prints_the_name_in_brackets_and_in_json() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: example.test\n",
        ),
        (
            "environments/prod.yml",
            "name: prod\nvariables:\n  - name: other\n    value: x\n",
        ),
        ("a.yml", &req("http://{{host}}/")),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(1);
    let text = stdout(&out);
    assert_eq!(
        text.lines().next(),
        Some("error: a.yml [prod]: variable `host` is not defined; defined in environment dev"),
        "{text}"
    );

    let (mut cmd, _home) = apinata();
    let out = cmd
        .current_dir(dir.path())
        .args(["check", "--json"])
        .assert()
        .code(1);
    let v: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["findings"][0]["environment"], "prod");
}

/// The summary line's text for a collection built from `files` plus root, with `args` appended.
fn summary_of(files: &[(&str, &str)], args: &[&str], code: i32) -> String {
    let mut all = vec![("opencollection.yml", ROOT)];
    all.extend_from_slice(files);
    let dir = write_tree(&all);
    let (mut cmd, _home) = apinata();
    let out = cmd
        .current_dir(dir.path())
        .arg("check")
        .args(args)
        .assert()
        .code(code);
    stdout(&out)
}

#[test]
fn the_summary_line_uses_singular_nouns_for_a_count_of_one() {
    let text = summary_of(&[("a.yml", &req("http://{{nope}}/"))], &[], 1);
    assert!(
        text.contains("1 error, 0 warnings, 1 request checked\n"),
        "{text}"
    );
}

#[test]
fn the_summary_line_uses_plural_nouns_for_zero_and_many() {
    let text = summary_of(&[], &[], 0);
    assert!(
        text.contains("0 errors, 0 warnings, 0 requests checked\n"),
        "{text}"
    );
    let a = req("http://{{nope}}/");
    let b = a.replace("name: a", "name: b").replace("seq: 1", "seq: 2");
    let text = summary_of(&[("a.yml", &a), ("b.yml", &b)], &[], 1);
    assert!(
        text.contains("2 errors, 0 warnings, 2 requests checked\n"),
        "{text}"
    );
    let w = req("http://x/{{process.env.NOPE_I378}}");
    let wb = w.replace("name: a", "name: b").replace("seq: 1", "seq: 2");
    let text = summary_of(&[("a.yml", &w), ("b.yml", &wb)], &[], 0);
    assert!(text.contains("0 errors, 2 warnings, 2 requests"), "{text}");
}

#[test]
fn one_warning_reads_as_a_singular_warning() {
    let w = req("http://x/{{process.env.NOPE_I378}}");
    let text = summary_of(&[("a.yml", &w)], &[], 0);
    assert!(
        text.contains("0 errors, 1 warning, 1 request checked"),
        "{text}"
    );
}

#[test]
fn the_unresolved_note_agrees_with_its_count() {
    let a = req("http://{{v}}/");
    let b = a.replace("name: a", "name: b").replace("seq: 1", "seq: 2");
    let one = summary_of(&[("a.yml", &a)], &["--env-var", "v={{leak}}"], 0);
    assert!(
        one.contains("1 request not fully resolved: it needs run-time input\n"),
        "{one}"
    );
    let two = summary_of(
        &[("a.yml", &a), ("b.yml", &b)],
        &["--env-var", "v={{leak}}"],
        0,
    );
    assert!(
        two.contains("2 requests not fully resolved: they need run-time input\n"),
        "{two}"
    );
}

#[test]
fn a_valueless_secret_warns_with_no_keychain_note_since_run_reads_none() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: tok\n    secret: true\n",
        ),
        ("a.yml", &req("http://x/{{tok}}")),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd
        .current_dir(dir.path())
        .args(["check", "--no-keychain", "--env", "dev"])
        .assert()
        .code(0);
    let text = stdout(&out);
    assert!(
        text.contains(
            "tok is a secret in environment dev with no value; pass --env-var tok=VALUE or set it in the app's environment editor\n"
        ),
        "{text}"
    );
}

#[test]
fn a_header_that_cannot_be_sent_is_an_error_naming_it_even_after_substitution() {
    let env = "name: dev\nvariables:\n  - name: bad\n    value: \"x y\"\n";
    let literal = format!(
        "{}  headers:\n    - name: \"Bad Name\"\n      value: v\n",
        req("http://x/")
    );
    let templated = format!(
        "{}  headers:\n    - name: \"{{{{bad}}}}\"\n      value: v\n",
        req("http://x/")
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", env),
        ("a.yml", &literal),
        ("b.yml", &templated),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(1);
    let text = stdout(&out);
    assert!(
        text.contains("error: a.yml: header `Bad Name` contains a space in its name"),
        "{text}"
    );
    assert!(
        text.contains("error: b.yml: header `{{bad}}` contains a space in its name"),
        "{text}"
    );
}

#[test]
fn a_header_bad_in_one_environment_only_is_tagged_with_it() {
    let dev = "name: dev\nvariables:\n  - name: v\n    value: \"a\\nb\"\n";
    let prod = "name: prod\nvariables:\n  - name: v\n    value: fine\n";
    let request = format!(
        "{}  headers:\n    - name: X-A\n      value: \"{{{{v}}}}\"\n",
        req("http://x/")
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("environments/prod.yml", prod),
        ("a.yml", &request),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(1);
    let text = stdout(&out);
    assert!(
        text.contains("error: a.yml [dev]: value of header `X-A` contains a line break"),
        "{text}"
    );
    assert!(!text.contains("[prod]"), "{text}");
}

#[test]
fn a_literal_bad_url_is_an_error_even_with_a_secret_in_the_environment() {
    let env = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    for (url, text) in [
        ("", "the URL is empty"),
        ("   ", "the URL is empty"),
        ("http://exa mple.com/", "the URL is not valid"),
    ] {
        let dir = write_tree(&[
            ("opencollection.yml", ROOT),
            ("environments/dev.yml", env),
            ("a.yml", &req(url)),
        ]);
        let (mut cmd, _home) = apinata();
        let out = cmd
            .current_dir(dir.path())
            .args(["check", "--no-keychain"])
            .assert()
            .code(1);
        let line = format!("error: a.yml: {text}");
        let shown = stdout(&out);
        assert!(shown.contains(&line), "{url:?}: {shown}");
        assert!(!shown.contains("not fully resolved"), "{url:?}: {shown}");
    }
}

#[test]
fn an_unclosed_placeholder_warns_naming_the_request_and_field_and_exits_0() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("http://x/{{host}")),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(0);
    let text = stdout(&out);
    assert!(text.starts_with("warning: a.yml: "), "{text}");
    assert!(text.contains("{{host}") && text.contains("url"), "{text}");
}

#[test]
fn an_unclosed_placeholder_in_a_folder_header_warns_naming_the_folder_file() {
    let folder = "info:\n  name: sub\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: Authorization\n      value: \"{{token}\"\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("sub/folder.yml", folder),
        ("sub/a.yml", &req("http://x/")),
        ("sub/b.yml", &req("http://x/")),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(0);
    let text = stdout(&out);
    assert_eq!(text.matches("no closing").count(), 1, "{text}");
    assert!(text.contains("warning: sub/folder.yml: "), "{text}");
    assert!(
        text.contains("Authorization") && text.contains("{{token}"),
        "{text}"
    );
}

#[test]
fn json_reports_a_folder_finding_with_the_folder_file_as_its_item() {
    let folder = "info:\n  name: sub\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: Authorization\n      value: \"{{token}\"\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("sub/folder.yml", folder),
        ("sub/a.yml", &req("http://x/")),
    ]);
    let (mut cmd, _home) = apinata();
    let out = cmd
        .current_dir(dir.path())
        .args(["check", "--json"])
        .assert()
        .code(0);
    let v: Value = serde_json::from_str(&stdout(&out)).unwrap();
    let f = &v["findings"][0];
    assert_eq!(f["item"], "sub/folder.yml", "{v}");
    assert_eq!(f["kind"], "unclosed_placeholder", "{v}");
    assert_eq!(f["severity"], "warning", "{v}");
    assert_eq!(v["findings"].as_array().unwrap().len(), 1, "{v}");
}

#[test]
fn a_disabled_header_with_a_bad_name_is_not_reported() {
    let off = format!(
        "{}  headers:\n    - name: \"Bad Name\"\n      value: v\n      disabled: true\n",
        req("http://x/")
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &off)]);
    let (mut cmd, _home) = apinata();
    let out = cmd.current_dir(dir.path()).arg("check").assert().code(0);
    assert!(!stdout(&out).contains("Bad Name"), "{}", stdout(&out));
}
