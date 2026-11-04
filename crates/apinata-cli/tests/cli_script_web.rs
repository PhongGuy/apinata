//! Web globals and lodash through `apinata run` and `apinata check`.

use crate::common;

use common::{ROOT, apinata, run_json, write_tree};
use serde_json::json;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn script(kind: &str, code: &str) -> String {
    let body: String = code.lines().map(|l| format!("        {l}\n")).collect();
    format!("    - type: {kind}\n      code: |-\n{body}")
}

fn scripted(url: &str, scripts: &str) -> String {
    format!(
        "info:\n  name: R\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  scripts:\n{scripts}"
    )
}

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

const CODE: &str = r#"const u = new URL("/p?x=1", "http://h.test");
u.searchParams.append("y", "a b");
console.log(u.href);
console.log(new TextDecoder().decode(new TextEncoder().encode("héllo")));
console.log(JSON.stringify(structuredClone({ a: [1, { b: 2 }] })));
console.log(_.chunk([1, 2, 3], 2).length, require("lodash") === _, require("querystring").stringify({ k: "v w" }));
test("web globals", () => expect(typeof URL).to.equal("function"));"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_and_check_accept_the_web_globals() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(&format!("{}/ok", s.uri()), &script("tests", CODE)),
        ),
    ]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 0, "{report}");
    let texts: Vec<_> = report["results"][0]["console"]
        .as_array()
        .expect("console")
        .iter()
        .map(|c| c["text"].as_str().expect("text").to_string())
        .collect();
    assert_eq!(
        texts,
        [
            "http://h.test/p?x=1&y=a+b",
            "héllo",
            "{\"a\":[1,{\"b\":2}]}",
            "2 true k=v%20w",
        ]
    );
    assert_eq!(
        report["results"][0]["tests"],
        json!([{"phase": "tests", "name": "web globals", "status": "pass"}])
    );
    apinata().arg("check").arg(dir.path()).assert().success();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_is_still_refused() {
    let s = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                &format!("{}/ok", s.uri()),
                &script("tests", "require('fs')"),
            ),
        ),
    ]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_ne!(code, 0);
    assert!(
        report.to_string().contains("Cannot find module fs"),
        "{report}"
    );
}
