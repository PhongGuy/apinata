//! `run --show-response`, `--body` and `-o`: the response on stdout, verdicts on stderr.

use crate::common::{ROOT, apinata, write_tree};
use std::process::Output;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req(name: &str, seq: u32, url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}

const FAIL: &str = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"500\"\n";

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/json"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-extra", "hello")
                .set_body_raw(r#"{"id":7,"b":1}"#, "application/json"),
        )
        .mount(&s)
        .await;
    Mock::given(path("/bin"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            vec![0x89u8, 0x50, 0xff, 0x00, 0xfe],
            "application/octet-stream",
        ))
        .mount(&s)
        .await;
    Mock::given(path("/secret"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-echo", "SUPERSECRET")
                .set_body_raw("token=SUPERSECRET", "text/plain"),
        )
        .mount(&s)
        .await;
    Mock::given(path("/cookie"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "sid=abc123; Path=/")
                .set_body_raw("ok", "text/plain"),
        )
        .mount(&s)
        .await;
    s
}

fn run(dir: &std::path::Path, args: &[&str]) -> Output {
    apinata()
        .arg("run")
        .arg(dir)
        .args(args)
        .output()
        .expect("runs")
}

fn one(url: &str, extra: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("A", 1, url, extra)),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn show_response_prints_head_and_body_and_moves_verdicts_to_stderr() {
    let s = server().await;
    let dir = one(&format!("{}/json", s.uri()), "");
    let out = run(dir.path(), &["-i"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("HTTP/1.1 200 OK\n"), "{stdout}");
    assert!(stdout.contains("\nx-extra: hello\n"), "{stdout}");
    assert!(stdout.ends_with("\n\n{\"id\":7,\"b\":1}\n"), "{stdout}");
    assert!(!stdout.contains("passed"), "{stdout}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("A"), "verdict line on stderr: {stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_prints_only_the_body_bytes() {
    let s = server().await;
    let dir = one(&format!("{}/json", s.uri()), "");
    let out = run(dir.path(), &["--body"]);
    assert_eq!(out.stdout, br#"{"id":7,"b":1}"#);
    assert!(!out.stderr.is_empty(), "verdicts are on stderr");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_of_several_requests_is_concatenated_verbatim() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("A", 1, &format!("{}/json", s.uri()), "")),
        ("b.yml", &req("B", 2, &format!("{}/secret", s.uri()), "")),
    ]);
    let out = run(dir.path(), &["--body"]);
    assert_eq!(out.stdout, b"{\"id\":7,\"b\":1}token=SUPERSECRET");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_binary_body_is_raw_when_piped_and_stays_out_of_the_report() {
    let s = server().await;
    let dir = one(&format!("{}/bin", s.uri()), "");
    let out = run(dir.path(), &["--body"]);
    assert_eq!(out.stdout, [0x89, 0x50, 0xff, 0x00, 0xfe]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_format_with_show_response_carries_the_response_and_no_head() {
    let s = server().await;
    let dir = one(&format!("{}/json", s.uri()), "");
    let out = run(dir.path(), &["-i", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is the report");
    let r = &v["results"][0]["response"];
    assert_eq!(r["status"], 200);
    assert_eq!(r["body"], r#"{"id":7,"b":1}"#);
    assert_eq!(r["body_size"], 14);
    assert_eq!(r["truncated"], false);
    assert!(
        r["headers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h[0] == "x-extra" && h[1] == "hello")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_format_without_the_flag_has_no_response_key() {
    let s = server().await;
    let dir = one(&format!("{}/json", s.uri()), "");
    let out = run(dir.path(), &["--format", "json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["results"][0].get("response").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cut_body_prints_the_kept_bytes_and_notes_it_on_stderr() {
    let s = server().await;
    let dir = one(&format!("{}/json", s.uri()), "");
    let out = run(dir.path(), &["--body", "--max-body-bytes", "5"]);
    assert_eq!(out.stdout, b"{\"id\"");
    assert!(String::from_utf8_lossy(&out.stderr).contains("body truncated at 5 bytes"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_file_gets_the_true_bytes_and_verdicts_stay_on_stdout() {
    let s = server().await;
    let dir = one(&format!("{}/bin", s.uri()), "");
    let out_dir = tempfile::tempdir().unwrap();
    let file = out_dir.path().join("got.bin");
    let out = run(dir.path(), &["-o", file.to_str().unwrap()]);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        [0x89, 0x50, 0xff, 0x00, 0xfe]
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("A"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_directory_gets_one_numbered_file_per_request() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("A", 1, &format!("{}/json", s.uri()), "")),
        ("b.yml", &req("B", 2, &format!("{}/secret", s.uri()), "")),
    ]);
    let out_dir = tempfile::tempdir().unwrap();
    let target = out_dir.path().join("new");
    let arg = format!("{}/", target.display());
    run(dir.path(), &["-o", &arg]);
    assert_eq!(
        std::fs::read(target.join("001-a.body")).unwrap(),
        br#"{"id":7,"b":1}"#
    );
    assert_eq!(
        std::fs::read(target.join("002-b.body")).unwrap(),
        b"token=SUPERSECRET"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn secrets_are_scrubbed_from_the_printed_response_but_not_from_output_files() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - secret: true\n    name: key\n",
        ),
        (
            "a.yml",
            &req("A", 1, &format!("{}/secret?k={{{{key}}}}", s.uri()), ""),
        ),
    ]);
    let out_dir = tempfile::tempdir().unwrap();
    let file = out_dir.path().join("raw");
    let out = run(
        dir.path(),
        &[
            "--env",
            "dev",
            "--env-var",
            "key=SUPERSECRET",
            "-i",
            "-o",
            file.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!stdout.contains("SUPERSECRET"), "{stdout}");
    assert!(stdout.contains("x-echo: [redacted]"), "{stdout}");
    assert!(stdout.contains("token=[redacted]"), "{stdout}");
    assert_eq!(std::fs::read(&file).unwrap(), b"token=SUPERSECRET");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exit_codes_are_unchanged() {
    let s = server().await;
    let dir = one(&format!("{}/json", s.uri()), FAIL);
    assert_eq!(run(dir.path(), &["-i"]).status.code(), Some(1));
    assert_eq!(run(dir.path(), &["--body"]).status.code(), Some(1));
}

#[test]
fn conflicting_flags_and_a_missing_output_directory_are_usage_errors() {
    let dir = one("http://127.0.0.1:1/x", "");
    for args in [
        vec!["--body", "-i"],
        vec!["--body", "--format", "json"],
        vec!["-o", "/nonexistent-apinata-dir/x.body"],
    ] {
        assert_eq!(run(dir.path(), &args).status.code(), Some(3), "{args:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_response_carries_the_address_and_the_head_does_not() {
    let s = server().await;
    let dir = one(&format!("{}/json", s.uri()), "");
    let out = run(dir.path(), &["-i", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is the report");
    let r = &v["results"][0]["response"];
    assert_eq!(r["remote_addr"], s.address().to_string());
    assert!(r.get("via_proxy").is_none());
    // `-i` prints the head as before, with no address in it.
    let head = run(dir.path(), &["-i"]);
    let stdout = String::from_utf8(head.stdout).unwrap();
    assert!(!stdout.contains(&s.address().to_string()), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_set_cookie_value_never_prints_in_the_head_or_the_report() {
    let s = server().await;
    let dir = one(&format!("{}/cookie", s.uri()), "");
    let head = String::from_utf8(run(dir.path(), &["-i"]).stdout).unwrap();
    assert!(!head.contains("abc123"), "{head}");
    assert!(head.contains("set-cookie: sid=[redacted]\n"), "{head}");
    let json = String::from_utf8(run(dir.path(), &["-i", "--format", "json"]).stdout).unwrap();
    assert!(!json.contains("abc123"), "{json}");
    assert!(json.contains("sid=[redacted]"), "{json}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_set_cookie_without_an_equals_sign_never_prints() {
    let s = MockServer::start().await;
    Mock::given(path("/bare"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "sekrit-token; Path=/")
                .set_body_raw("ok", "text/plain"),
        )
        .mount(&s)
        .await;
    let dir = one(&format!("{}/bare", s.uri()), "");
    for args in [
        &["-i"][..],
        &["-i", "--format", "json"][..],
        &["--reporter-json", "-"][..],
    ] {
        let out = run(dir.path(), args);
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!all.contains("sekrit"), "{args:?}: {all}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unfollowed_redirect_location_never_prints_its_password() {
    let s = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "http://joe:hop-pa55-xyz@127.0.0.1:1/next"),
        )
        .mount(&s)
        .await;
    let dir = one(
        &format!("{}/start", s.uri()),
        "settings:\n  followRedirects: false\n",
    );
    for args in [&["-i"][..], &["-i", "--format", "json"][..]] {
        let out = run(dir.path(), args);
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(all.contains("127.0.0.1:1/next"), "{args:?}: {all}");
        assert!(!all.contains("hop-pa55-xyz"), "{args:?}: {all}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redirect_location_in_a_parser_only_shape_never_prints_its_password() {
    for location in [
        "\\\\joe:hop-pa55-xyz@127.0.0.1:1/next",
        "/\\joe:hop-pa55-xyz@127.0.0.1:1/next",
        "https:joe:hop-pa55-xyz@127.0.0.1:1/next",
        "https:/joe:hop-pa55-xyz@127.0.0.1:1/next",
        "https:\\\\joe:hop-pa55-xyz@127.0.0.1:1/next",
    ] {
        let s = MockServer::start().await;
        Mock::given(path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", location)
                    .insert_header("content-location", location)
                    .insert_header("refresh", format!("0; url={location}").as_str()),
            )
            .mount(&s)
            .await;
        let dir = one(
            &format!("{}/start", s.uri()),
            "settings:\n  followRedirects: false\n",
        );
        let report = dir.path().join("report.json");
        let report_arg = report.to_str().expect("utf-8 path");
        for args in [
            &["-i"][..],
            &["-i", "--format", "json"][..],
            &["-i", "--reporter-json", report_arg][..],
        ] {
            let out = run(dir.path(), args);
            let all = format!(
                "{}{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
                std::fs::read_to_string(&report).unwrap_or_default()
            );
            assert!(all.contains("302"), "{location} {args:?}: {all}");
            assert!(!all.contains("hop-pa55-xyz"), "{location} {args:?}: {all}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_followed_redirect_in_a_parser_only_shape_never_prints_its_password() {
    for shape in [
        "\\\\joe:hop-pa55-xyz@127.0.0.1:PORT/next",
        "/\\joe:hop-pa55-xyz@127.0.0.1:PORT/next",
        "http:\\\\joe:hop-pa55-xyz@127.0.0.1:PORT/next",
    ] {
        let s = MockServer::start().await;
        let port = s.uri().rsplit(':').next().expect("port").to_owned();
        Mock::given(path("/start"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", shape.replace("PORT", &port)),
            )
            .mount(&s)
            .await;
        Mock::given(path("/next"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("arrived", "text/plain"))
            .mount(&s)
            .await;
        let dir = one(&format!("{}/start", s.uri()), "");
        let report = dir.path().join("report.json");
        let report_arg = report.to_str().expect("utf-8 path");
        for args in [
            &["-i", "--verbose"][..],
            &["-i", "--format", "json"][..],
            &["-i", "--reporter-json", report_arg][..],
        ] {
            let out = run(dir.path(), args);
            let all = format!(
                "{}{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
                std::fs::read_to_string(&report).unwrap_or_default()
            );
            assert!(all.contains("arrived"), "{shape} {args:?}: {all}");
            assert!(!all.contains("hop-pa55-xyz"), "{shape} {args:?}: {all}");
        }
    }
}
