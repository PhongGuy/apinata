//! `apinata env ls` and `env show`: environments and their variables, secrets masked.

use crate::common;

use common::{ROOT, apinata, write_tree};
use serde_json::Value;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: example.test\n  - name: off\n    value: gone\n    disabled: true\n  - name: token\n    secret: true\n";
const PROD: &str = "name: prod\nvariables:\n  - name: host\n    value: prod.test\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/prod.yml", PROD),
        (".env", "token=dotenv-secret-value\n"),
    ])
}

fn run(args: &[&str], dir: &std::path::Path) -> String {
    let out = apinata().current_dir(dir).args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn env_ls_prints_names_sorted() {
    let dir = tree();
    assert_eq!(run(&["env", "ls"], dir.path()), "dev\nprod\n");
    let json: Value = serde_json::from_str(&run(&["env", "ls", "--json"], dir.path())).unwrap();
    assert_eq!(json["environments"], serde_json::json!(["dev", "prod"]));
}

#[test]
fn env_show_lists_variables_in_file_order_with_markers() {
    let dir = tree();
    let text = run(&["env", "show", "dev"], dir.path());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(
        lines[0].contains("host") && lines[0].contains("example.test"),
        "{text}"
    );
    assert!(
        lines[1].contains("off") && lines[1].contains("disabled"),
        "{text}"
    );
    assert!(
        lines[2].contains("token") && lines[2].contains("<secret>"),
        "{text}"
    );
}

#[test]
fn env_show_never_prints_a_secret_value_in_text_or_json() {
    let dir = tree();
    let out = apinata()
        .current_dir(dir.path())
        .args(["env", "show", "dev", "--env-var", "token=cli-secret-value"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let out = apinata()
        .current_dir(dir.path())
        .args([
            "env",
            "show",
            "dev",
            "--json",
            "--env-var",
            "token=cli-secret-value",
        ])
        .assert()
        .success();
    let json_text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    for shown in [&text, &json_text] {
        assert!(!shown.contains("dotenv-secret-value"), "{shown}");
        assert!(!shown.contains("cli-secret-value"), "{shown}");
    }
    let json: Value = serde_json::from_str(&json_text).unwrap();
    assert_eq!(json["name"], "dev");
    let vars = json["variables"].as_array().unwrap();
    assert_eq!(vars[0]["value"], "example.test");
    assert_eq!(vars[1]["enabled"], false);
    assert_eq!(vars[2]["secret"], true);
    assert!(vars[2].get("value").is_none());
}

#[test]
fn env_show_unknown_name_exits_3() {
    let dir = tree();
    let out = apinata()
        .current_dir(dir.path())
        .args(["env", "show", "nope"])
        .assert()
        .code(3);
    assert!(!out.get_output().stderr.is_empty());
}

#[test]
fn env_outside_a_collection_exits_3() {
    let dir = write_tree(&[("note.txt", "hi")]);
    apinata()
        .current_dir(dir.path())
        .args(["env", "ls"])
        .assert()
        .code(3);
}

#[test]
fn env_text_escapes_control_characters_and_shows_a_type() {
    let env = "name: x\nvariables:\n  - name: \"a\\u001b[31mb\\nforged\"\n    value: \"v\\u001b[2J\\nmore\"\n  - name: count\n    value:\n      type: number\n      data: \"5\"\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/ev\u{1b}il.yml", env),
    ]);
    let names = run(&["env", "ls"], dir.path());
    assert!(!names.contains('\u{1b}'), "{names:?}");
    assert_eq!(names.lines().count(), 1, "{names:?}");
    let stem = names.trim_end().replace("\\u{1b}", "\u{1b}");
    let text = run(&["env", "show", &stem], dir.path());
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert_eq!(text.lines().count(), 2, "{text:?}");
    let count = text.lines().nth(1).unwrap();
    assert!(
        count.contains("count") && count.contains("number"),
        "{text:?}"
    );
    let json = run(&["env", "show", &stem, "--json"], dir.path());
    assert!(json.contains("forged"), "JSON keeps the real name");
}

fn dev_file(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("environments/dev.yml")).unwrap()
}

#[test]
fn env_set_creates_a_variable_and_a_repeat_is_byte_identical() {
    let dir = tree();
    let out = run(&["env", "set", "dev", "port=8080"], dir.path());
    assert!(out.contains("port"), "{out}");
    let first = dev_file(dir.path());
    assert!(
        first.contains("name: port") && first.contains("8080"),
        "{first}"
    );
    run(&["env", "set", "dev", "port=8080"], dir.path());
    assert_eq!(dev_file(dir.path()), first);
    run(&["env", "set", "dev", "host=other.test"], dir.path());
    assert!(dev_file(dir.path()).contains("other.test"));
    // A value may hold an equals sign; only the first one splits.
    run(&["env", "set", "dev", "q=a=b"], dir.path());
    assert!(run(&["env", "show", "dev"], dir.path()).contains("a=b"));
}

#[test]
fn env_set_refuses_a_secret_unknown_environment_and_a_missing_value() {
    let dir = tree();
    let before = dev_file(dir.path());
    for args in [
        &["env", "set", "dev", "token=x"][..],
        &["env", "set", "nope", "k=v"][..],
        &["env", "set", "dev", "novalue"][..],
    ] {
        let out = apinata()
            .current_dir(dir.path())
            .args(args)
            .assert()
            .code(3);
        assert!(!out.get_output().stderr.is_empty(), "{args:?}");
    }
    assert_eq!(dev_file(dir.path()), before);
}

#[test]
fn env_set_secret_takes_no_value_on_the_command_line() {
    let dir = tree();
    let before = dev_file(dir.path());
    let out = apinata()
        .current_dir(dir.path())
        .args(["env", "set", "--secret", "dev", "token=hunter2"])
        .write_stdin("")
        .assert()
        .code(3);
    let err = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(err.contains("stdin") && !err.contains("hunter2"), "{err}");
    assert_eq!(dev_file(dir.path()), before);
}

#[test]
fn env_set_secret_without_a_keychain_fails_and_writes_nothing() {
    let dir = tree();
    let before = dev_file(dir.path());
    let out = apinata()
        .current_dir(dir.path())
        .args(["env", "set", "--secret", "--no-keychain", "dev", "apikey"])
        .write_stdin("hunter2\n")
        .assert()
        .code(3);
    let err = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        err.contains("keychain") && !err.contains("hunter2"),
        "{err}"
    );
    assert_eq!(dev_file(dir.path()), before);
}

#[test]
fn env_unset_removes_a_plain_variable_and_refuses_a_secret_without_the_flag() {
    let dir = tree();
    run(&["env", "unset", "dev", "host"], dir.path());
    assert!(!dev_file(dir.path()).contains("host"));
    apinata()
        .current_dir(dir.path())
        .args(["env", "unset", "dev", "host"])
        .assert()
        .code(3);
    apinata()
        .current_dir(dir.path())
        .args(["env", "unset", "dev", "token"])
        .assert()
        .code(3);
    assert!(dev_file(dir.path()).contains("token"));
    apinata()
        .current_dir(dir.path())
        .args(["env", "unset", "--secret", "--no-keychain", "dev", "token"])
        .assert()
        .success();
    assert!(!dev_file(dir.path()).contains("token"));
}
