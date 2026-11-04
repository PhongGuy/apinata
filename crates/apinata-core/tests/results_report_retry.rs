//! A run cancelled during the pause before a retry: the reports carry the failure of the
//! attempt before it, redacted, next to the cancel.

use crate::script_support::{ROOT, on, write_tree};

use apinata_core::{
    CancelHandle, Collection, ConnectCause, ContextOptions, ExecError, ExecOptions, Executor,
    RequestResult, ResultsReport, RunError, RunEvent, VarContext, run,
};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "s3cr3t-token-77";
const TS: &str = "2026-10-12T10:00:00.000";

/// Runs `r.yml` with a 30 s retry pause and cancels once the pause is announced; `tok` is a
/// sensitive variable so a URL that mentions it must come out redacted.
async fn cancelled_in_the_pause(url: &str) -> RequestResult {
    let yml = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &yml)]);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new().sensitive_env_var("tok", SECRET))
        .expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let cancel = CancelHandle::new();
    let opts = on().retries(2).retry_delay(Duration::from_secs(30));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let on_event = Box::new(move |e: RunEvent| {
        if let RunEvent::Retrying { .. } = e {
            let _ = tx.send(());
        }
    });
    let task = tokio::spawn(run(plan, ctx, exec, opts, cancel.clone(), on_event));
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("a retry is announced");
    cancel.cancel();
    let report = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("run ends at once")
        .expect("joins");
    (*report.results[0]).clone()
}

/// A listener that drops each connection it accepts, so every send fails in transit.
fn dropping_url(userinfo: &str) -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!(
        "http://{userinfo}127.0.0.1:{}/",
        l.local_addr().expect("addr").port()
    );
    std::thread::spawn(move || {
        while let Ok((conn, _)) = l.accept() {
            drop(conn);
        }
    });
    url
}

fn report(r: &RequestResult) -> ResultsReport {
    ResultsReport::new(vec![], std::slice::from_ref(r))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_json_carries_the_failure_the_cancel_hid() {
    let r = cancelled_in_the_pause(&dropping_url("")).await;
    let want = r.last_attempt_error.as_ref().expect("carried").to_string();
    let value: serde_json::Value = serde_json::from_str(&report(&r).to_json()).expect("json");
    assert_eq!(value["results"][0]["last_attempt_error"], want.as_str());
    assert_ne!(value["results"][0]["error"], want.as_str());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_after_a_502_has_no_last_attempt_error_key() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(502))
        .mount(&s)
        .await;
    let r = cancelled_in_the_pause(&s.uri()).await;
    let value: serde_json::Value = serde_json::from_str(&report(&r).to_json()).expect("json");
    assert_eq!(value["results"][0]["status"], 502);
    assert!(value["results"][0].get("last_attempt_error").is_none());
    assert!(
        !report(&r)
            .to_junit(TS, "h", false)
            .contains("last attempt error")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn junit_and_html_show_the_carried_failure() {
    let r = cancelled_in_the_pause(&dropping_url("")).await;
    let want = r.last_attempt_error.as_ref().expect("carried").to_string();
    let xml = report(&r).to_junit(TS, "h", false);
    assert!(xml.contains("last attempt error: "), "{xml}");
    let html = report(&r).to_html(TS);
    assert!(html.contains("last attempt error: "), "{html}");
    // The carried text names the refused connection and the address it tried.
    assert!(want.contains("127.0.0.1"), "{want}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_in_the_carried_failure_is_masked_everywhere() {
    let r = cancelled_in_the_pause(&dropping_url("{{tok}}@")).await;
    let carried = r.last_attempt_error.as_ref().expect("carried").to_string();
    let rep = report(&r);
    let json = rep.to_json();
    let xml = rep.to_junit(TS, "h", false);
    let html = rep.to_html(TS);
    for out in [&json, &xml, &html] {
        assert!(!out.contains(SECRET), "{out}");
    }
    // The userinfo is masked, and the carried text is still there to read.
    assert!(json.contains("last_attempt_error"), "{json}");
    assert!(carried.contains("127.0.0.1"), "{carried}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_carried_failure_takes_the_same_connect_advice_as_the_error() {
    let mut r = cancelled_in_the_pause(&dropping_url("")).await;
    r.last_attempt_error = Some(RunError::Exec(ExecError::connect(
        "https://h.test/",
        "invalid peer certificate: UnknownIssuer",
        ConnectCause::TlsUnknownIssuer,
    )));
    let advice = |cause: ConnectCause| match cause {
        ConnectCause::TlsUnknownIssuer => Some("Try --cacert <file> or --insecure.".to_owned()),
        _ => None,
    };
    let json: serde_json::Value =
        serde_json::from_str(&report(&r).with_connect_advice(advice).to_json()).expect("json");
    let text = json["results"][0]["last_attempt_error"]
        .as_str()
        .expect("carried");
    let sentence_end = text.find("Try --cacert").expect(text);
    assert!(
        text[..sentence_end].ends_with("verification off. "),
        "{text}"
    );
    assert!(
        text.ends_with("(invalid peer certificate: UnknownIssuer)"),
        "{text}"
    );
    // Without advice the line is the plain hinted one.
    let plain = report(&r).to_json();
    assert!(!plain.contains("Try --cacert"), "{plain}");
}
