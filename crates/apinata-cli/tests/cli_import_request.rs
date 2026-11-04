//! `apinata import request`: one command from a file or stdin becomes one request file.

use crate::common;

use apinata_core::import::{add_request, parse_request};
use apinata_core::{Collection, ItemPath};
use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use std::fs;

const COMMAND: &str = r#"curl -X POST 'https://{{host}}/users?x=1' -H 'Authorization: Bearer {{token}}' -k --json '{"a":1}'"#;

#[test]
fn a_command_on_stdin_becomes_a_request_named_after_the_url() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .args(["import", "request", "--collection"])
        .arg(dir.path())
        .write_stdin(COMMAND)
        .assert()
        .code(0)
        .stdout("users.yml\n")
        .stderr(contains("note: dropped curl flag -k: "));
    assert!(dir.path().join("users.yml").is_file());
}

#[test]
fn the_file_is_what_core_writes_for_the_same_command() {
    let input = tempfile::tempdir().unwrap();
    let file = input.path().join("cmd.txt");
    fs::write(&file, COMMAND).unwrap();
    let cli = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .args(["import", "request", "--name", "Create user", "--collection"])
        .arg(cli.path())
        .arg(&file)
        .assert()
        .code(0)
        .stdout("Create user.yml\n");
    let core = write_tree(&[("opencollection.yml", ROOT)]);
    let mut collection = Collection::open(core.path()).unwrap();
    let parsed = parse_request(None, COMMAND).unwrap();
    add_request(
        &mut collection,
        &ItemPath::root(),
        "Create user",
        &parsed.request,
    )
    .unwrap();
    let read = |dir: &std::path::Path| fs::read_to_string(dir.join("Create user.yml")).unwrap();
    assert_eq!(read(cli.path()), read(core.path()));
    // The URL holds a variable, so encoding stays off: its value may already be encoded.
    assert!(
        read(cli.path()).contains("encodeUrl: false"),
        "{}",
        read(cli.path())
    );
}

#[test]
fn a_folder_places_it_and_json_reports_it() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "api/folder.yml",
            "info:\n  name: api\n  type: folder\n  seq: 1\n",
        ),
    ]);
    let out = apinata()
        .args([
            "import",
            "request",
            "-",
            "--from",
            "curl",
            "--format",
            "json",
            "--folder",
            "api",
            "--collection",
        ])
        .arg(dir.path())
        .write_stdin("curl https://x.test/ping")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    assert_eq!(report["collection"], root.display().to_string());
    assert_eq!(report["items"], serde_json::json!(["api/ping.yml"]));
    assert_eq!(report["notes"], serde_json::json!([]));
    assert_eq!(report["secrets_to_enter"], serde_json::json!([]));
}

#[test]
fn bad_input_exits_3_and_writes_nothing() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let run = |extra: &[&str], stdin: &str| {
        apinata()
            .args(["import", "request"])
            .args(extra)
            .arg("--collection")
            .arg(dir.path())
            .write_stdin(stdin.to_owned())
            .assert()
            .code(3)
    };
    run(&[], "wget https://x.test").stderr(contains("error: not a recognized request format"));
    run(&["--from", "curl"], "curl 'https://x.test").stderr(contains("unclosed quote"));
    run(&["--from", "wget"], "curl https://x.test").stderr(contains("expected auto, curl"));
    run(&["--folder", "nope"], "curl https://x.test/a").stderr(contains("error: "));
    run(&["--folder", "../out"], "curl https://x.test/a").stderr(contains("error: "));
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["opencollection.yml"]);
    let empty = tempfile::tempdir().unwrap();
    apinata()
        .args(["import", "request", "--collection"])
        .arg(empty.path())
        .write_stdin("curl https://x.test/a")
        .assert()
        .code(3)
        .stderr(contains("opencollection.yml"));
}

#[test]
fn help_lists_exit_codes_and_names_no_other_tool() {
    for args in [
        &["import", "--help"][..],
        &["import", "request", "--help"][..],
    ] {
        let out = apinata().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
        assert!(!text.contains("bruno"), "{args:?}: {text}");
    }
    let help = apinata().args(["import", "--help"]).output().unwrap();
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(
        text.contains("3  the import failed (usage, input, collection or write error)"),
        "{text}"
    );
    assert!(
        !text.contains("  2  ") && !text.contains("left behind"),
        "{text}"
    );
}

const AWS: &str =
    "curl --aws-sigv4 aws:amz:us-east-1:service --user AKID:TOPSECRET https://x.test/a";

/// An `import request --env` run with no keychain and no session bus, so no real store is touched.
fn import_aws_into(dir: &std::path::Path, env: &str) -> assert_cmd::assert::Assert {
    apinata()
        .env_remove("XDG_RUNTIME_DIR")
        .args([
            "import",
            "request",
            "--no-keychain",
            "--env",
            env,
            "--collection",
        ])
        .arg(dir)
        .write_stdin(AWS)
        .assert()
}

#[test]
fn an_unknown_env_with_one_environment_stores_nothing_and_says_so() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
    ]);
    import_aws_into(dir.path(), "nope")
        .code(0)
        .stderr(contains("environment \"nope\" not found; available: dev"));
    let env = fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(!env.contains("awsSecretAccessKey"), "{env}");
}

#[test]
fn an_existing_env_receives_the_secret_variable() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    import_aws_into(dir.path(), "prod").code(0);
    let prod = fs::read_to_string(dir.path().join("environments/prod.yml")).unwrap();
    assert!(prod.contains("awsSecretAccessKey"), "{prod}");
    let dev = fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(!dev.contains("awsSecretAccessKey"), "{dev}");
}
