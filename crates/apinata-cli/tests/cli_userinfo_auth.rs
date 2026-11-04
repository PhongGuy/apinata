//! A redirect to a Location carrying `user:pass@` never prints that password or its Basic value.

use crate::common;

use base64::Engine;
use common::{ROOT, apinata, write_tree};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const USER: &str = "cli-user";
const PASS: &str = "CLIPWx7Q3vv";
const TOKEN: &str = "CLITOKEN8812";

fn encoded() -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{PASS}"))
}

/// A server whose `/start` redirects to `/next` with the credential in the Location, and whose
/// `/next` echoes the Basic value in a header and its body.
async fn userinfo_redirect() -> MockServer {
    let server = MockServer::start().await;
    let location = server
        .uri()
        .replacen("http://", &format!("http://{USER}:{PASS}@"), 1);
    Mock::given(path("/start"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{location}/next")),
        )
        .mount(&server)
        .await;
    Mock::given(path("/next"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-echo", encoded().as_str())
                .set_body_string(format!("{{\"echo\":\"{}\"}}", encoded())),
        )
        .mount(&server)
        .await;
    server
}

fn assert_hidden(stdout: &[u8], stderr: &[u8]) {
    for (name, bytes) in [("stdout", stdout), ("stderr", stderr)] {
        let text = String::from_utf8_lossy(bytes);
        for secret in [PASS, encoded().as_str(), TOKEN] {
            assert!(!text.contains(secret), "{secret} on {name}: {text}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_verbose_json_hides_the_location_password_and_its_basic() {
    let server = userinfo_redirect().await;
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/start\"\n  auth:\n    type: bearer\n    token: {TOKEN}\n",
        server.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &request)]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--verbose", "--format", "json"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        server.received_requests().await.expect("recorded")[1]
            .headers
            .get_all("authorization")
            .iter()
            .count(),
        1
    );
    assert_hidden(&out.stdout, &out.stderr);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_i_verbose_hides_the_location_password_and_its_basic() {
    let server = userinfo_redirect().await;
    let dir = tempfile::TempDir::new().expect("tempdir");
    let out = apinata()
        .current_dir(dir.path())
        .args(["send", "--no-keychain", "-i", "--verbose"])
        .args(["-H", &format!("Authorization: Bearer {TOKEN}")])
        .arg(format!("{}/start", server.uri()))
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_hidden(&out.stdout, &out.stderr);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_verbose_never_prints_a_url_username() {
    // A username alone can be a token, on the request url or a redirect's Location.
    let server = MockServer::start().await;
    let spliced = server.uri().replacen("http://", "http://acct-7781@", 1);
    Mock::given(path("/start"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{spliced}/next")),
        )
        .mount(&server)
        .await;
    Mock::given(path("/next"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{spliced}/start\"\n"
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &request)]);
    for format in ["json", "text"] {
        let out = apinata()
            .arg("run")
            .arg(dir.path())
            .args(["--verbose", "--format", format])
            .output()
            .expect("runs");
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        for bytes in [&out.stdout, &out.stderr] {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains("acct-7781"), "{format}: {text}");
        }
    }
}
