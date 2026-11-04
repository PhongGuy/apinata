//! A stream limit on the top-level send does not apply to sends made by its scripts.

use crate::common::sse_server::{Script, Step, event, slow_sse_server};
use crate::script_support::{ROOT, console, on, script, scripted, send, write_tree};
use apinata_core::{Outcome, StreamLimits};

fn three() -> Script {
    Script::sse(vec![event("e0"), event("e1"), event("e2"), Step::Finish])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_nested_send_is_not_bounded_by_the_top_level_limit() {
    let top = slow_sse_server(three()).await;
    let nested = slow_sse_server(three()).await;
    let code = format!(
        "const r = await bru.sendRequest({{ method: \"get\", url: \"{}\" }});\nconsole.log(String(r.data).split(\"data:\").length - 1);",
        nested.url("/")
    );
    let text = scripted("R", &top.url("/"), &script("before-request", &code), "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let opts = on().stream(StreamLimits::new().max_events(Some(1)));
    let result = send(dir.path(), "r.yml", opts).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(console(&result), ["log:3"]);
    // The top-level response itself was bounded.
    let response = result.response.as_ref().expect("response");
    assert_eq!(response.events().len(), 1);
}
