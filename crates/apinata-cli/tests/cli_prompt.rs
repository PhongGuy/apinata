//! `apinata run` fails a request with prompt variables, naming them; no flag answers them.

use crate::common;

use common::{ROOT, column, get, run_json, write_tree};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_prompt_errors_by_name_and_env_vars_do_not_answer_it() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let url = format!("{}/{{{{?Order id}}}}?pin={{{{?pin}}}}", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("A", 1, &url, "    []\n")),
    ]);
    for extra in [
        &[][..],
        &["--env-var", "?pin=1", "--env-var", "?Order id=2"][..],
    ] {
        let (code, report) = run_json(dir.path(), extra);
        assert_eq!(code, 2, "{extra:?}");
        assert_eq!(column(&report, "outcome"), ["errored"]);
        assert_eq!(
            column(&report, "error"),
            ["no value for prompt variables: Order id, pin"]
        );
    }
    assert!(s.received_requests().await.expect("recording").is_empty());
}
