//! Success criterion 4: the acceptance collection against a mock server, with expected verdicts
//! from Bruno's assertion code.

use crate::common;

use common::column;
use std::path::Path;
use std::time::Duration;
use wiremock::matchers::{body_string, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mock_server() -> MockServer {
    let s = MockServer::start().await;
    let user = r#"{"id":7,"name":"Ann Lee","email":"ann@example.test","tags":["admin","ops"],"active":true,"archived":false,"score":4.5,"manager":null,"profile":{}}"#;
    let json = |status: u16, body: &str| {
        ResponseTemplate::new(status).set_body_raw(body.to_owned(), "application/json")
    };
    Mock::given(method("GET"))
        .and(path("/users/7"))
        .and(query_param("verbose", "true"))
        .and(header("authorization", "Bearer t0k3n"))
        .and(header("x-suite", "acceptance"))
        .respond_with(json(200, user).insert_header("x-request-id", "req-7"))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/users"))
        .and(body_string(r#"{"name": "Bob", "id": 7}"#))
        .and(header("content-type", "application/json"))
        .respond_with(json(201, r#"{"id":8,"name":"Bob"}"#))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/login"))
        .and(body_string("user=ann&scope=read+write"))
        .and(header("authorization", "Basic YW5uOnMzY3JldA=="))
        .respond_with(ResponseTemplate::new(200).set_body_raw("welcome ann", "text/plain"))
        .mount(&s)
        .await;
    Mock::given(method("PUT"))
        .and(path("/ping"))
        .and(body_string("<ping>t0k3n</ping>"))
        .and(header("content-type", "application/xml"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&s)
        .await;
    Mock::given(path("/keys"))
        .and(query_param("api_key", "k3y"))
        .respond_with(json(200, "[1,2,3]"))
        .mount(&s)
        .await;
    Mock::given(path("/keys/header"))
        .and(header("x-api-key", "k3y"))
        .and(header("x-from-env", "yes"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    Mock::given(path("/old"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "/new")
                .insert_header("set-cookie", "session=abc; Path=/"),
        )
        .mount(&s)
        .await;
    Mock::given(path("/new"))
        .and(header("cookie", "session=abc"))
        .respond_with(json(200, r#"{"landed":true}"#))
        .mount(&s)
        .await;
    Mock::given(path("/whoami"))
        .and(header("cookie", "session=abc"))
        .respond_with(json(200, r#"{"session":"abc"}"#))
        .mount(&s)
        .await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .mount(&s)
        .await;
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_acceptance_collection_matches_brunos_verdicts() {
    let s = mock_server().await;
    let collection = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/acceptance");
    let out = common::apinata()
        .arg("run")
        .arg(&collection)
        .args([
            "--env",
            "local",
            "--format",
            "json",
            "--env-var",
            "apiKey=k3y",
            "--no-scripts",
        ])
        .arg("--env-var")
        .arg(format!("baseUrl={}", s.uri()))
        .env("APINATA_ACCEPTANCE", "yes")
        .output()
        .expect("runs");
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(out.status.code(), Some(2), "{report:#}");
    assert_eq!(
        column(&report, "path"),
        [
            "users/get-user.yml",
            "users/create-user.yml",
            "forms/login.yml",
            "forms/ping.yml",
            "keys/query-key.yml",
            "keys/header-key.yml",
            "flow/redirect.yml",
            "flow/no-follow.yml",
            "flow/whoami.yml",
            "flow/slow.yml",
            "errors/multipart.yml",
            "errors/undefined.yml",
        ]
    );
    let outcomes = column(&report, "outcome");
    let expected = [
        "passed", "failed", "passed", "passed", "passed", "passed", "passed", "passed", "passed",
        "errored", "errored", "errored",
    ];
    assert_eq!(outcomes, expected, "{report:#}");
    // Bruno passes every assertion except create-user's third (8 vs 9) and fourth (`${8}` is the
    // string "8"). Apinata skips the fourth and fifth (`${...}` and `res.url` need JavaScript).
    let verdicts: Vec<Vec<String>> = report["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|r| {
            r["assertions"]
                .as_array()
                .expect("assertions")
                .iter()
                .map(|a| a["verdict"].as_str().expect("verdict").to_owned())
                .collect()
        })
        .collect();
    assert_eq!(verdicts[0], vec!["pass"; 32]);
    assert_eq!(verdicts[1], ["pass", "pass", "fail", "skipped", "skipped"]);
    assert_eq!(report["results"][1]["assertions"][2]["actual"], "8");
    for (i, n) in [(2, 3), (3, 2), (4, 2), (5, 1), (6, 2), (7, 2), (8, 1)] {
        assert_eq!(
            verdicts[i],
            vec!["pass"; n],
            "{}",
            report["results"][i]["path"]
        );
    }
    assert_eq!(
        report["results"][5]["warnings"],
        serde_json::json!(["scripts not executed"])
    );
    let errors = column(&report, "error");
    assert!(errors[9].contains("timed out"), "{}", errors[9]);
    // A multipart file that cannot be read stops the request before anything is sent.
    assert!(errors[10].contains("missing-upload.bin"), "{}", errors[10]);
    assert_eq!(errors[11], "variable `nope` is not defined");
    assert_eq!(report["summary"]["skipped_assertions"], 2);
}
