//! `--save-example` on `apinata run` and `apinata send`: the response lands in the request
//! file as an example, scrubbed of secrets unless `--keep-secrets` asks otherwise.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::fs;
use tempfile::TempDir;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
const TOKEN: &str = "cli-Secret-482";

/// A mock answering 201 with a session cookie and a body echoing the secret, `expected` times.
async fn echo_server(expected: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("set-cookie", "sid=cookie-Value-88; Path=/")
                .set_body_raw(format!("{{\"echo\":\"{TOKEN}\"}}"), "application/json"),
        )
        .expect(expected)
        .mount(&server)
        .await;
    server
}

/// A GET of `url` that sends the `token` secret as a header.
fn request(name: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\nhttp:\n  method: GET\n  url: \"{url}/ping\"\n  headers:\n    - name: X-Token\n      value: \"{{{{token}}}}\"\n"
    )
}

fn collection(url: &str) -> TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &request("r", url)),
    ])
}

fn run(dir: &TempDir, extra: &[&str]) -> assert_cmd::Command {
    let mut cmd = apinata();
    cmd.arg("run")
        .arg(dir.path().join("r.yml"))
        .args(["--no-keychain", "--env", "dev", "--env-var"])
        .arg(format!("token={TOKEN}"))
        .args(extra);
    cmd
}

fn disk(dir: &TempDir, rel: &str) -> String {
    fs::read_to_string(dir.path().join(rel)).expect("file")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_save_example_writes_a_scrubbed_example() {
    let s = echo_server(1).await;
    let dir = collection(&s.uri());
    run(&dir, &["--save-example", "created"]).assert().code(0);
    let text = disk(&dir, "r.yml");
    assert!(text.contains("examples:"), "{text}");
    assert!(text.contains("name: created"), "{text}");
    assert!(text.contains("status: 201"), "{text}");
    assert!(!text.contains(TOKEN), "{text}");
    assert!(!text.contains("cookie-Value-88"), "{text}");
    assert!(text.contains("{{token}}"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_secrets_saves_the_values_as_received() {
    let s = echo_server(1).await;
    let dir = collection(&s.uri());
    run(&dir, &["--save-example", "raw", "--keep-secrets"])
        .assert()
        .code(0);
    let text = disk(&dir, "r.yml");
    assert!(text.contains(TOKEN), "{text}");
    assert!(text.contains("cookie-Value-88"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_matching_two_requests_is_refused_before_sending() {
    let s = echo_server(0).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("a.yml", &request("a", &s.uri())),
        ("b.yml", &request("b", &s.uri())),
    ]);
    apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--no-keychain", "--save-example", "x"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains(
            "--save-example needs exactly one request",
        ));
    assert!(!disk(&dir, "a.yml").contains("examples:"));
    assert!(!disk(&dir, "b.yml").contains("examples:"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_save_example_needs_save() {
    let s = echo_server(0).await;
    let cwd = TempDir::new().expect("tempdir");
    apinata()
        .current_dir(cwd.path())
        .args(["send", "--no-keychain", "--save-example", "x"])
        .arg(format!("{}/ping", s.uri()))
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "required arguments were not provided",
        ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_save_example_lands_on_the_saved_file() {
    let s = echo_server(1).await;
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let cwd = TempDir::new().expect("tempdir");
    apinata()
        .current_dir(cwd.path())
        .args(["send", "--no-keychain", "--save"])
        .arg(dir.path().join("ping.yml"))
        .args(["--save-example", "first"])
        .arg(format!("{}/ping", s.uri()))
        .assert()
        .code(0);
    let text = disk(&dir, "ping.yml");
    assert!(text.contains("name: first"), "{text}");
    assert!(text.contains("status: 201"), "{text}");
    assert!(!text.contains("cookie-Value-88"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_last_iteration_saves_nothing() {
    let s = echo_server(1).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &request("r", "{{base}}")),
    ]);
    // The first row answers; the last points at a port nothing listens on.
    let rows = format!(
        "[{{\"base\":\"{}\"}},{{\"base\":\"http://127.0.0.1:{}\"}}]",
        s.uri(),
        common::dead_port()
    );
    fs::write(dir.path().join("rows.json"), rows).expect("rows");
    run(&dir, &["--save-example", "last"])
        .arg("--json-file-path")
        .arg(dir.path().join("rows.json"))
        .assert()
        .stderr(predicates::str::contains("no response to save"));
    assert!(!disk(&dir, "r.yml").contains("examples:"));
}

/// The body of the request's example `name`, as core reads it back from disk.
pub(crate) fn example_body(dir: &std::path::Path, rel: &str, name: &str) -> String {
    let c = apinata_core::Collection::open(dir).expect("collection");
    let item = apinata_core::ItemPath::new(rel).expect("item");
    let example = (c.request(&item).expect("request").examples())
        .into_iter()
        .find(|e| e.name == name)
        .expect("the example");
    example.response.and_then(|r| r.body).expect("a body").data
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_compact_json_body_is_saved_re_indented() {
    let s = echo_server(1).await;
    let dir = collection(&s.uri());
    run(&dir, &["--save-example", "pretty"]).assert().code(0);
    assert_eq!(
        example_body(dir.path(), "r.yml", "pretty"),
        "{\n  \"echo\": \"{{token}}\"\n}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_name_blank_to_javascript_is_refused_before_sending() {
    let s = echo_server(0).await;
    let dir = collection(&s.uri());
    run(&dir, &["--save-example", "\u{feff}"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not blank"));
    assert!(!disk(&dir, "r.yml").contains("examples:"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_padded_taken_name_is_refused_before_sending_and_named_trimmed() {
    // One send in all: the second run must stop before it reaches the server.
    let s = echo_server(1).await;
    let dir = collection(&s.uri());
    run(&dir, &["--save-example", "dup"]).assert().code(0);
    let before = disk(&dir, "r.yml");
    run(&dir, &["--save-example", "  dup  "])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "already has an example named `dup`",
        ));
    assert_eq!(disk(&dir, "r.yml"), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_data_file_run_saves_a_scrubbed_example() {
    let s = echo_server(2).await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &request("r", "{{base}}")),
    ]);
    fs::write(
        dir.path().join("rows.csv"),
        format!("base\n{}\n{}\n", s.uri(), s.uri()),
    )
    .expect("rows");
    run(&dir, &["--save-example", "rows"])
        .arg("--csv-file-path")
        .arg(dir.path().join("rows.csv"))
        .assert()
        .code(0);
    let text = disk(&dir, "r.yml");
    assert!(text.contains("name: rows"), "{text}");
    assert!(!text.contains(TOKEN), "{text}");
    assert!(text.contains("{{token}}"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_save_example_drops_the_url_userinfo() {
    let s = echo_server(1).await;
    let typed = s.uri().replacen("http://", "http://acct-7781:pw-55@", 1);
    let dir = collection(&typed);
    run(&dir, &["--save-example", "created"]).assert().code(0);
    let text = disk(&dir, "r.yml");
    assert_eq!(
        text.matches("acct-7781").count(),
        1,
        "only the request keeps it: {text}"
    );
    assert_eq!(text.matches("pw-55").count(), 1, "{text}");
    assert!(text.contains(&format!("{}/ping", s.uri())), "{text}");
}
