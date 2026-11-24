//! The runner turns a streaming send into ResponseStarted and ResponseChunk events.

use crate::common::sse_server::{Script, Step, event, slow_sse_server};
use crate::common::write_tree;
use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, Executor, Outcome, RunEvent, RunOptions,
    StreamLimits, VarContext, run,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn get(name: &str, seq: u32, url: &str, script: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  assertions:\n    - expression: res.body\n      operator: contains\n      value: \"e2\"\n{script}"
    )
}

async fn events_of(files: &[(&str, &str)], opts: RunOptions) -> (Vec<String>, Vec<Outcome>) {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).unwrap();
    let plan = c.plan(dir.path()).unwrap();
    let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    let report = run(
        plan,
        ctx,
        exec,
        opts,
        CancelHandle::new(),
        Box::new(move |e| {
            let line = match e {
                RunEvent::ResponseStarted {
                    index,
                    status,
                    event_stream,
                    ..
                } => {
                    format!("started {index} {status} {event_stream}")
                }
                RunEvent::ResponseChunk { index, bytes } => {
                    format!("chunk {index} {}", String::from_utf8_lossy(&bytes).trim())
                }
                RunEvent::ResponseEnded { index } => format!("ended {index}"),
                RunEvent::RequestFinished { index, .. } => format!("finish {index}"),
                _ => return,
            };
            sink.lock().unwrap().push(line);
        }),
    )
    .await;
    let lines = log.lock().unwrap().clone();
    (lines, report.results.iter().map(|r| r.outcome).collect())
}

fn three_events() -> Script {
    Script::sse(vec![
        event("e0"),
        Step::Pause(Duration::from_millis(40)),
        event("e1"),
        Step::Pause(Duration::from_millis(40)),
        event("e2"),
        Step::Finish,
    ])
}

#[tokio::test]
async fn response_events_precede_the_finish_in_order() {
    let s = slow_sse_server(three_events()).await;
    let a = get("a", 1, &s.url("/"), "");
    let (lines, outcomes) = events_of(
        &[("opencollection.yml", ROOT), ("a.yml", &a)],
        RunOptions::new().stream(StreamLimits::new()),
    )
    .await;
    assert_eq!(lines.first().unwrap(), "started 0 200 true");
    assert_eq!(lines.last().unwrap(), "finish 0");
    let body: String = lines
        .iter()
        .filter_map(|l| l.strip_prefix("chunk 0 "))
        .collect();
    assert!(body.contains("data: e2"), "{lines:?}");
    // The assertion saw the whole received body.
    assert_eq!(outcomes, [Outcome::Passed]);
}

#[tokio::test]
async fn without_the_option_no_response_event_is_emitted() {
    let s = slow_sse_server(three_events()).await;
    let a = get("a", 1, &s.url("/"), "");
    let (lines, _) = events_of(
        &[("opencollection.yml", ROOT), ("a.yml", &a)],
        RunOptions::new(),
    )
    .await;
    assert_eq!(lines, ["finish 0"]);
}

#[tokio::test]
async fn a_second_request_reports_its_own_index() {
    let s = slow_sse_server(three_events()).await;
    let a = get("a", 1, &s.url("/"), "");
    let b = get("b", 2, &s.url("/"), "");
    let (lines, _) = events_of(
        &[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)],
        RunOptions::new().stream(StreamLimits::new()),
    )
    .await;
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("started")).count(),
        2,
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l == "started 1 200 true"));
}

#[tokio::test]
async fn the_end_event_falls_between_the_last_chunk_and_the_finish_once_per_request() {
    let s = slow_sse_server(three_events()).await;
    let plain = slow_sse_server(Script::with_type(
        "text/plain",
        vec![Step::Chunk("hi".into()), Step::Finish],
    ))
    .await;
    let a = get("a", 1, &s.url("/"), "");
    let b = get("b", 2, &plain.url("/"), "");
    let (lines, _) = events_of(
        &[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)],
        RunOptions::new().stream(StreamLimits::new()),
    )
    .await;
    for i in 0..2 {
        let at = |needle: String| {
            lines
                .iter()
                .position(|l| *l == needle)
                .unwrap_or_else(|| panic!("{needle} in {lines:?}"))
        };
        let ended = at(format!("ended {i}"));
        let last_chunk = lines
            .iter()
            .rposition(|l| l.starts_with(&format!("chunk {i} ")))
            .unwrap();
        assert!(
            last_chunk < ended && ended < at(format!("finish {i}")),
            "{lines:?}"
        );
        assert_eq!(
            lines.iter().filter(|l| **l == format!("ended {i}")).count(),
            1
        );
    }
}

#[tokio::test]
async fn a_send_that_fails_before_headers_has_no_end_event() {
    let a = get("a", 1, "http://127.0.0.1:9/", "");
    let (lines, outcomes) = events_of(
        &[("opencollection.yml", ROOT), ("a.yml", &a)],
        RunOptions::new().stream(StreamLimits::new()),
    )
    .await;
    assert_eq!(lines, ["finish 0"]);
    assert_eq!(outcomes, [Outcome::Errored]);
}

#[tokio::test]
async fn the_start_carries_the_redirect_hop_masked_as_the_result_masks_it() {
    const SECRET: &str = "s3cr3t-stream-77";
    let redirect = "HTTP/1.1 302 Found\r\nlocation: /s\r\ncontent-length: 0\r\n\
                    connection: close\r\n\r\n";
    let s = crate::common::sse_server::routed_sse_server(
        vec![(
            "/hop",
            Script::raw_head(redirect.to_owned(), vec![Step::Abort]),
        )],
        Script::sse(vec![event("one"), Step::Finish]),
    )
    .await;
    let a = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}?key={{{{tok}}}}\"\n  headers:\n    - name: authorization\n      value: Bearer {{{{tok}}}}\n",
        s.url("/hop")
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let c = Collection::open(dir.path()).unwrap();
    let plan = c.plan(dir.path()).unwrap();
    let ctx = VarContext::new(&c, ContextOptions::new().sensitive_env_var("tok", SECRET)).unwrap();
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    let starts = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&starts);
    run(
        plan,
        ctx,
        exec,
        RunOptions::new().stream(StreamLimits::new()),
        CancelHandle::new(),
        Box::new(move |e| {
            if let RunEvent::ResponseStarted { sent, .. } = e {
                sink.lock().unwrap().push(sent);
            }
        }),
    )
    .await;
    let starts = starts.lock().unwrap();
    assert_eq!(starts.len(), 1);
    let sent = &starts[0];
    assert!(sent.response.is_none());
    let [hop] = sent.timeline.as_slice() else {
        panic!("hops: {:?}", sent.timeline.len());
    };
    assert_eq!(hop.status, 302);
    assert!(hop.url.contains(SECRET), "the hop keeps what was sent");
    let shown = sent.redact_url(&hop.url);
    assert!(!shown.contains(SECRET), "{shown}");
    let auth = hop
        .request_headers
        .iter()
        .find(|(n, _)| n == "authorization")
        .expect("authorization was sent");
    assert!(!sent.redact_header(&auth.0, &auth.1).contains(SECRET));
}
