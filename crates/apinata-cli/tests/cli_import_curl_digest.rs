//! `apinata import request` maps `--digest -u` to digest auth.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::fs;

#[test]
fn a_digest_command_becomes_a_digest_request_with_no_note_about_dropped_credentials() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let out = apinata()
        .args(["import", "request", "--collection"])
        .arg(dir.path())
        .env_remove("XDG_RUNTIME_DIR")
        .write_stdin("curl --digest -u a:b https://x.test/d")
        .assert()
        .code(0)
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("not supported"), "{stderr}");
    let file = fs::read_to_string(dir.path().join("d.yml")).unwrap();
    assert!(file.contains("type: digest"), "{file}");
    assert!(file.contains("username: a"), "{file}");
    assert!(file.contains("password: b"), "{file}");
}

const AWS: &str = "curl --aws-sigv4 aws:amz:us-east-1:s --user AKID:TOPSECRET https://x.test/d";

fn run_aws(dir: &std::path::Path, extra: &[&str]) -> String {
    let out = apinata()
        .args(["import", "request", "--no-keychain", "--collection"])
        .arg(dir)
        .args(extra)
        .env_remove("XDG_RUNTIME_DIR")
        .write_stdin(AWS)
        .assert()
        .code(0)
        .get_output()
        .clone();
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn an_aws_secret_becomes_a_secret_variable_of_the_only_environment() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
    ]);
    let stderr = run_aws(dir.path(), &[]);
    let request = fs::read_to_string(dir.path().join("d.yml")).unwrap();
    let env = fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(request.contains("{{awsSecretAccessKey}}"), "{request}");
    assert!(
        env.contains("awsSecretAccessKey") && env.contains("secret: true"),
        "{env}"
    );
    for text in [&stderr, &request, &env] {
        assert!(!text.contains("TOPSECRET"), "{text}");
    }
    // --no-keychain keeps no value, so the user is told to enter it.
    assert!(stderr.contains("awsSecretAccessKey"), "{stderr}");
}

#[test]
fn env_flag_picks_among_several_environments() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    run_aws(dir.path(), &["--env", "prod"]);
    let dev = fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    let prod = fs::read_to_string(dir.path().join("environments/prod.yml")).unwrap();
    assert!(!dev.contains("awsSecretAccessKey"), "{dev}");
    assert!(prod.contains("awsSecretAccessKey"), "{prod}");
}
