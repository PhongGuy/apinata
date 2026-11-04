//! The extra console methods write plain lines through the same path as `console.log`, and
//! `clearTimeout` cancels a pending timer.

use crate::script_support;

use apinata_core::{ContextOptions, ExecOptions, Outcome};
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn tree(code: &str) -> (MockServer, tempfile::TempDir) {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let text = scripted(
        "R",
        &format!("{}/r", s.uri()),
        &script("before-request", code),
        "",
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    (s, dir)
}

/// Run `code` as a before-request script and return its console lines.
async fn lines(code: &str) -> Vec<String> {
    let (_s, dir) = tree(code).await;
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    console(&result)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn table_aligns_an_array_of_objects() {
    let got = lines("console.table([{a: 1, b: 'xx'}, {a: 30, b: 'y'}]);").await;
    assert_eq!(
        got,
        [
            "log:(index) | a  | b",
            "log:0       | 1  | xx",
            "log:1       | 30 | y"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn table_shows_primitives_under_a_values_column() {
    let got = lines("console.table(['p', 'q']);").await;
    assert_eq!(
        got,
        ["log:(index) | Values", "log:0       | p", "log:1       | q"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn table_uses_object_keys_as_the_index() {
    let got = lines("console.table({k: {a: 1}});").await;
    assert_eq!(got, ["log:(index) | a", "log:k       | 1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn table_caps_rows_at_one_hundred() {
    let got = lines("console.table(Array.from({length: 105}, (_, i) => i));").await;
    assert_eq!(got.len(), 1 + 100 + 1, "{got:?}");
    assert_eq!(got.last().unwrap(), "log:... 5 more rows");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn group_indents_every_line_and_group_end_floors_at_zero() {
    let code = "console.group('A'); console.log('x\\ny'); console.group(); console.warn('z');\n\
                console.groupEnd(); console.groupEnd(); console.groupEnd(); console.groupEnd();\n\
                console.log('w');";
    let got = lines(code).await;
    assert_eq!(got, ["log:A", "log:  x\n  y", "warn:    z", "log:w"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn time_end_reports_elapsed_milliseconds_per_label() {
    let got =
        lines("console.time(); console.time('t'); console.timeEnd('t'); console.timeEnd();").await;
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(
        got[0].starts_with("log:t: ") && got[0].ends_with("ms"),
        "{got:?}"
    );
    assert!(
        got[1].starts_with("log:default: ") && got[1].ends_with("ms"),
        "{got:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn time_end_on_an_unknown_label_warns() {
    let got = lines("console.timeEnd('nope');").await;
    assert_eq!(got, ["warn:Timer 'nope' does not exist"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dir_logs_like_log() {
    let got = lines("console.dir({a: 1}, {depth: 0});").await;
    assert_eq!(got, ["log:{\"a\":1}"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assert_writes_only_when_the_condition_is_falsy() {
    let got =
        lines("console.assert(true, 'no'); console.assert(0); console.assert(false, 'boom', 7);")
            .await;
    assert_eq!(
        got,
        ["error:Assertion failed", "error:Assertion failed: boom 7"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn count_numbers_each_label() {
    let got =
        lines("console.count(); console.count('a'); console.count('a'); console.count();").await;
    assert_eq!(
        got,
        ["log:default: 1", "log:a: 1", "log:a: 2", "log:default: 2"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_new_methods_are_redacted_like_log() {
    let (_s, dir) =
        tree("console.count(bru.getEnvVar('tok')); console.table([bru.getEnvVar('tok')]);").await;
    let ctx = ContextOptions::new().sensitive_env_var("tok", "S3cret-tok-77");
    let (r, _) = send_full(dir.path(), "r.yml", on(), ctx, ExecOptions::new()).await;
    let shown: Vec<String> = r.redacted_console().into_iter().map(|c| c.text).collect();
    assert!(r.console.iter().any(|c| c.text.contains("S3cret-tok-77")));
    assert!(
        shown.iter().all(|t| !t.contains("S3cret-tok-77")),
        "{shown:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_timeout_cancels_the_callback() {
    let code = "let ran = false; const p = setTimeout(() => { ran = true; }, 20);\n\
                clearTimeout(p); await p; await bru.sleep(30); console.log(ran);";
    assert_eq!(lines(code).await, ["log:false"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_timeout_ignores_ids_it_does_not_know() {
    let code = "clearTimeout(); clearTimeout(null); clearTimeout(42); clearTimeout('x');\n\
                clearTimeout({}); clearTimeout(Promise.resolve()); console.log('ok');";
    assert_eq!(lines(code).await, ["log:ok"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_interval_throws_a_message_naming_the_alternatives() {
    let code = "try { setInterval(() => {}, 5); } catch (e) { console.log(e.message); }";
    let got = lines(code).await;
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(
        got[0].contains("setTimeout") && got[0].contains("bru.sleep"),
        "{got:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn table_survives_a_sparse_array() {
    let got = lines("console.table([1, , 3]);").await;
    assert_eq!(
        got,
        [
            "log:(index) | Values",
            "log:0       | 1",
            "log:1       | undefined",
            "log:2       | 3"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn table_shows_a_map_or_set_as_one_value() {
    let got = lines("console.table([new Set([1]), new Map([['a', 1]])]);").await;
    assert_eq!(
        got,
        [
            "log:(index) | Values",
            "log:0       | Set [1]",
            "log:1       | Map [[\"a\",1]]"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn table_cells_stay_on_one_line() {
    let got = lines("console.table([{a: 'x\\ny'}, {a: 'z'}]);").await;
    assert_eq!(
        got,
        ["log:(index) | a", "log:0       | x\\ny", "log:1       | z"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn time_dir_and_assert_are_redacted_like_log() {
    let code = "const t = bru.getEnvVar('tok'); console.time(t); console.timeEnd(t); console.timeEnd(t);\n\
                console.dir(t); console.assert(false, t);";
    let (_s, dir) = tree(code).await;
    let ctx = ContextOptions::new().sensitive_env_var("tok", "S3cret-tok-77");
    let (r, _) = send_full(dir.path(), "r.yml", on(), ctx, ExecOptions::new()).await;
    assert_eq!(r.console.len(), 4, "{:?}", r.console);
    assert!(r.console.iter().all(|c| c.text.contains("S3cret-tok-77")));
    let shown: Vec<String> = r.redacted_console().into_iter().map(|c| c.text).collect();
    assert!(
        shown.iter().all(|t| !t.contains("S3cret-tok-77")),
        "{shown:?}"
    );
}
