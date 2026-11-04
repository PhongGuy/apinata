//! A secret a script prints or names never reaches `apinata run`'s output or report files.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::fs;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "qa-tok-777";

const ENV: &str = "name: local\nvariables:\n  - name: token\n    secret: true\n";

// Logs the secret beside plain text, then names and fails a test with it.
const PRE: &str = "console.log(bru.getEnvVar('token'))\nconsole.log('plain-note-41')";
const TESTS: &str =
    "test('saw ' + bru.getEnvVar('token'), () => expect(bru.getEnvVar('token')).to.equal('x'))";

fn request(url: &str) -> String {
    let block = |kind: &str, code: &str| {
        let body: String = code.lines().map(|l| format!("        {l}\n")).collect();
        format!("    - type: {kind}\n      code: |-\n{body}")
    };
    format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  scripts:\n{}{}",
        block("before-request", PRE),
        block("tests", TESTS)
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_a_script_logs_is_redacted_in_every_output() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/local.yml", ENV),
        ("r.yml", &request(&format!("{}/ok", s.uri()))),
    ]);
    let out_dir = tempfile::TempDir::new().unwrap();
    let files = ["r.json", "r.xml", "r.html"].map(|f| out_dir.path().join(f));
    for format in ["text", "json"] {
        let out = apinata()
            .arg("run")
            .arg(dir.path())
            .args(["--env", "local", "--verbose", "--format", format])
            .arg("--env-var")
            .arg(format!("token={SECRET}"))
            .arg("--reporter-json")
            .arg(&files[0])
            .arg("--reporter-junit")
            .arg(&files[1])
            .arg("--reporter-html")
            .arg(&files[2])
            .output()
            .expect("runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{stdout}\n{stderr}");
        assert!(stdout.contains("plain-note-41"), "{format}: {stdout}");
        assert!(stdout.contains("[redacted]"), "{format}: {stdout}");
        for (name, text) in [("stdout", &stdout), ("stderr", &stderr)] {
            assert!(!text.contains(SECRET), "{format} {name}: {text}");
        }
        // JUnit carries no console lines, only the redacted test name and message.
        for file in &files {
            let text = fs::read_to_string(file).unwrap();
            let console = !file.ends_with("r.xml");
            assert_eq!(
                text.contains("plain-note-41"),
                console,
                "{}",
                file.display()
            );
            assert!(text.contains("[redacted]"), "{}: {text}", file.display());
            assert!(!text.contains(SECRET), "{}: {text}", file.display());
        }
    }
}

// A failing assertion prints its left side, which a server can make the secret by echoing it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_an_assertion_prints_as_its_actual_value_is_redacted() {
    let s = MockServer::start().await;
    Mock::given(path("/echo"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "echo": SECRET })),
        )
        .mount(&s)
        .await;
    let request = format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/echo\"\nruntime:\n  assertions:\n    - expression: res.body.echo\n      operator: neq\n      value: \"{{{{token}}}}\"\n",
        s.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/local.yml", ENV),
        ("r.yml", &request),
    ]);
    let out_dir = tempfile::TempDir::new().unwrap();
    let files = ["r.json", "r.xml", "r.html"].map(|f| out_dir.path().join(f));
    for format in ["text", "json"] {
        let out = apinata()
            .arg("run")
            .arg(dir.path())
            .args(["--env", "local", "--format", format])
            .arg("--env-var")
            .arg(format!("token={SECRET}"))
            .arg("--reporter-json")
            .arg(&files[0])
            .arg("--reporter-junit")
            .arg(&files[1])
            .arg("--reporter-html")
            .arg(&files[2])
            .output()
            .expect("runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{stdout}\n{stderr}");
        assert!(stdout.contains("res.body.echo"), "{format}: {stdout}");
        assert!(stdout.contains("[redacted]"), "{format}: {stdout}");
        for (name, text) in [("stdout", &stdout), ("stderr", &stderr)] {
            assert!(!text.contains(SECRET), "{format} {name}: {text}");
        }
        for file in &files {
            let text = fs::read_to_string(file).unwrap();
            assert!(text.contains("[redacted]"), "{}: {text}", file.display());
            assert!(!text.contains(SECRET), "{}: {text}", file.display());
        }
    }
}
