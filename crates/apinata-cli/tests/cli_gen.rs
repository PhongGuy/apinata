//! `apinata gen`: one request printed as code, never sent.

use crate::common;

use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn get(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

#[test]
fn curl_is_printed_with_variables_resolved() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &get("{{base}}/a"))]);
    apinata()
        .args(["gen", "--target", "curl", "--env-var", "base=http://x.test"])
        .arg(dir.path().join("r.yml"))
        .assert()
        .code(0)
        .stdout("curl --request 'GET' \\\n  --url 'http://x.test/a'\n");
}

#[test]
fn an_environment_feeds_the_python_snippet() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: base\n    value: http://x.test\n",
        ),
        ("r.yml", &get("{{base}}/py")),
    ]);
    apinata()
        .args(["gen", "--target", "python-requests", "--env", "dev"])
        .arg(dir.path().join("r.yml"))
        .assert()
        .code(0)
        .stdout(predicates::str::starts_with("import requests\n"))
        .stdout(contains("'http://x.test/py'"));
}

#[test]
fn an_undefined_variable_fails_and_prints_no_code() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &get("http://x.test/{{nope}}")),
    ]);
    apinata()
        .args(["gen", "--target", "go"])
        .arg(dir.path().join("r.yml"))
        .assert()
        .code(3)
        .stdout("")
        .stderr(contains("nope"));
}

#[test]
fn a_request_file_that_fails_to_load_prints_its_error_and_no_code() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", "info:\n  name: r\n  type: http\n  seq: 1\n"),
    ]);
    apinata()
        .args(["gen", "--target", "curl"])
        .arg(dir.path().join("r.yml"))
        .assert()
        .code(3)
        .stdout("")
        .stderr(contains("the file has no `http` mapping"));
}

#[test]
fn a_folder_is_refused() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &get("http://x.test/a")),
    ]);
    apinata()
        .args(["gen", "--target", "curl"])
        .arg(dir.path())
        .assert()
        .code(3)
        .stderr(contains("gen takes one request file"));
}

#[test]
fn an_unknown_target_lists_the_valid_ones() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &get("http://x.test/a")),
    ]);
    apinata()
        .args(["gen", "--target", "cobol"])
        .arg(dir.path().join("r.yml"))
        .assert()
        .code(3)
        .stderr(contains("python-requests"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oauth2_request_without_a_cached_token_fails_without_fetching() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let file = format!(
        "{}  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n",
        get("http://x.test/o"),
        server.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let cache = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("r.yml");
    let out = tokio::task::spawn_blocking(move || {
        apinata()
            .args(["gen", "--target", "curl"])
            .arg(path)
            .env("APINATA_TEST_TOKEN_CACHE", cache.path())
            .env(
                "APINATA_TEST_TOKEN_KEY",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            )
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("no token yet"), "{stderr}");
    assert!(out.stdout.is_empty());
}

#[test]
fn gen_help_lists_every_target_id() {
    let out = apinata().args(["gen", "--help"]).output().unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    for t in apinata_core::Target::ALL {
        assert!(help.contains(t.id()), "{} missing from {help}", t.id());
    }
}
