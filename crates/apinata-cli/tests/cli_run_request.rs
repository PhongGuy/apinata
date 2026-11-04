//! `apinata run` with a script that requires a collection module and runs another request.

use crate::common;

use common::{ROOT, column, run_json, write_tree};
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn indent(code: &str) -> String {
    code.lines().map(|l| format!("        {l}\n")).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_runs_another_request_and_a_local_module() {
    let s = MockServer::start().await;
    // Once for the nested run and once as the run's own second request.
    Mock::given(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "t1"})))
        .expect(2)
        .mount(&s)
        .await;
    Mock::given(path("/api"))
        .and(header("authorization", "Bearer t1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    let pre = "const { bearer } = require('./lib/auth.js');\nconst r = await bru.runRequest('token');\nbru.setVar('auth', bearer(r.data.token));";
    let api = format!(
        "info:\n  name: Api\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/api\"\n  headers:\n    - name: Authorization\n      value: \"{{{{auth}}}}\"\nruntime:\n  scripts:\n    - type: before-request\n      code: |-\n{}",
        s.uri(),
        indent(pre)
    );
    let token = format!(
        "info:\n  name: Token\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{}/token\"\n",
        s.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("api.yml", &api),
        ("token.yml", &token),
        (
            "lib/auth.js",
            "module.exports = { bearer: (t) => 'Bearer ' + t };\n",
        ),
    ]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 0, "{report:#}");
    assert_eq!(column(&report, "path"), ["api.yml", "token.yml"]);
    assert_eq!(column(&report, "outcome"), ["passed", "passed"]);
}
