//! A parallel run: requests overlap within an iteration, results come out in plan order, each
//! request reads the iteration-start cookie jar, and shared-state calls are refused at runtime.

use crate::script_support::{ROOT, script, write_tree};

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, Executor, Outcome, RunError, RunEvent,
    RunOptions, RunReport, RunStop, ScriptOptions, VarContext, Warning, run,
};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

/// How long `/hold` waits for `/release` before answering `no-overlap`, so a run that never
/// overlaps fails instead of hanging.
const FALLBACK: Duration = Duration::from_secs(10);

/// A 127.0.0.1 server: `/hold` answers `overlap` once `/release` has arrived; `/set?n=v`
/// sets cookie `n=v`; `/echo` answers with the Cookie header it got; `/redir` sets `r=1` and
/// redirects to `/echo`; anything else answers `ok`.
async fn server() -> String {
    counting_server().await.0
}

/// [`server`] plus the most requests it ever held open at once.
async fn counting_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let released = Arc::new(Notify::new());
    let open = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let peak = Arc::clone(&most);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let released = Arc::clone(&released);
            let (open, most) = (Arc::clone(&open), Arc::clone(&most));
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let target = head.split(' ').nth(1).unwrap_or("/").to_owned();
                let cookie = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .starts_with("cookie:")
                            .then(|| l[7..].trim().to_owned())
                    })
                    .unwrap_or_default();
                let (path, query) = target.split_once('?').unwrap_or((&target, ""));
                let mut extra = String::new();
                let (status, body) = match path {
                    "/hold" => {
                        let seen = tokio::time::timeout(FALLBACK, released.notified()).await;
                        (
                            "200 OK",
                            if seen.is_ok() {
                                "overlap"
                            } else {
                                "no-overlap"
                            }
                            .to_owned(),
                        )
                    }
                    "/release" => {
                        released.notify_one();
                        ("200 OK", "ok".to_owned())
                    }
                    "/set" => {
                        extra = format!("set-cookie: {query}; Path=/\r\n");
                        ("200 OK", "ok".to_owned())
                    }
                    "/echo" => ("200 OK", cookie),
                    "/count" => {
                        let now = open.fetch_add(1, Ordering::SeqCst) + 1;
                        most.fetch_max(now, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        open.fetch_sub(1, Ordering::SeqCst);
                        ("200 OK", "ok".to_owned())
                    }
                    "/redir" => {
                        extra = "set-cookie: r=1; Path=/\r\nlocation: /echo\r\n".to_owned();
                        ("302 Found", String::new())
                    }
                    _ => ("200 OK", "ok".to_owned()),
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n{extra}\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(reply.as_bytes()).await;
            });
        }
    });
    (format!("http://127.0.0.1:{port}"), peak)
}

/// A GET request to `url` at position `seq`, with `scripts` items.
fn req(name: &str, seq: u32, url: &str, scripts: &str) -> String {
    let runtime = if scripts.is_empty() {
        String::new()
    } else {
        format!("runtime:\n  scripts:\n{scripts}")
    };
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n{runtime}"
    )
}

fn on() -> RunOptions {
    RunOptions::new().scripts(ScriptOptions::new().enabled(true))
}

/// Runs the collection in `dir` `n` at a time on `exec`, returning the report and the events
/// as short strings.
async fn run_parallel(
    dir: &Path,
    n: usize,
    exec: Arc<Executor>,
    opts: RunOptions,
) -> (RunReport, Vec<String>, VarContext) {
    let c = Collection::open(dir).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let plan = c
        .plan(dir)
        .expect("plan")
        .parallel(n, &ctx, true)
        .expect("parallel");
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let on_event = Box::new(move |event: RunEvent| {
        let line = match event {
            RunEvent::Started { total } => format!("started {total}"),
            RunEvent::IterationStarted { index, .. } => format!("iteration {index}"),
            RunEvent::RequestStarted { index, .. } => format!("start {index}"),
            RunEvent::RequestFinished { index, result } => {
                format!("finish {index} {:?}", result.outcome)
            }
            RunEvent::Finished { summary } => format!("finished {}", summary.requests),
            _ => return,
        };
        sink.lock().expect("lock").push(line);
    });
    let report = run(plan, ctx.clone(), exec, opts, CancelHandle::new(), on_event).await;
    let events = events.lock().expect("lock").clone();
    (report, events, ctx)
}

fn executor() -> Arc<Executor> {
    Arc::new(Executor::new(ExecOptions::new()).expect("client"))
}

fn body(report: &RunReport, i: usize) -> String {
    let response = report.results[i].response.as_ref().expect("response");
    String::from_utf8_lossy(response.body()).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_overlap_and_finish_in_plan_order() {
    let base = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{base}/hold"), "")),
        ("b.yml", &req("b", 2, &format!("{base}/release"), "")),
    ]);
    let (report, events, _) = run_parallel(dir.path(), 2, executor(), on()).await;
    assert_eq!(body(&report, 0), "overlap");
    assert_eq!(
        events,
        [
            "started 2",
            "start 0",
            "start 1",
            "finish 0 Passed",
            "finish 1 Passed",
            "finished 2"
        ]
    );
    let paths: Vec<&str> = report.results.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["a.yml", "b.yml"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_more_than_n_requests_are_in_flight() {
    let (base, most) = counting_server().await;
    let files: Vec<(String, String)> = (1..=6)
        .map(|i| {
            (
                format!("r{i}.yml"),
                req(&format!("r{i}"), i, &format!("{base}/count"), ""),
            )
        })
        .collect();
    let mut all = vec![("opencollection.yml", ROOT)];
    all.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let dir = write_tree(&all);
    let (report, _, _) = run_parallel(dir.path(), 2, executor(), on()).await;
    assert_eq!(report.summary.passed, 6);
    assert!(
        most.load(Ordering::SeqCst) <= 2,
        "{}",
        most.load(Ordering::SeqCst)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_request_reads_the_jar_as_its_iteration_began() {
    let base = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{base}/set?x=1"), "")),
        ("b.yml", &req("b", 2, &format!("{base}/echo"), "")),
    ]);
    let exec = executor();
    let (report, _, _) =
        run_parallel(dir.path(), 2, Arc::clone(&exec), on().iteration_count(2)).await;
    // Iteration 0: the echo never saw a's cookie; iteration 1 began with it in the jar.
    assert_eq!(body(&report, 1), "");
    assert_eq!(body(&report, 3), "x=1");
    assert!(
        report.results[0]
            .warnings
            .contains(&Warning::ParallelCookiesUnseen)
    );
    assert!(
        !report.results[1]
            .warnings
            .contains(&Warning::ParallelCookiesUnseen)
    );
    let names: Vec<String> = exec.cookies().list().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["x"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookie_changes_apply_in_plan_order() {
    let base = server().await;
    let clear = script("tests", "bru.cookies.jar().clear();");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{base}/set?y=1"), "")),
        ("b.yml", &req("b", 2, &format!("{base}/set?y=2"), "")),
        ("c.yml", &req("c", 3, &format!("{base}/set?z=1"), "")),
        ("d.yml", &req("d", 4, &format!("{base}/ok"), &clear)),
        ("e.yml", &req("e", 5, &format!("{base}/set?w=1"), "")),
    ]);
    let exec = executor();
    let (report, _, _) = run_parallel(dir.path(), 4, Arc::clone(&exec), on()).await;
    assert_eq!(report.summary.errored, 0, "{:?}", report.results);
    // The clear in d removes y and z, as it would have run after them; e comes after it.
    let jar: Vec<String> = exec
        .cookies()
        .list()
        .into_iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect();
    assert_eq!(jar, ["w=1"]);
    // The last request's changes reach no later request, so it carries no warning.
    assert!(
        !report.results[4]
            .warnings
            .contains(&Warning::ParallelCookiesUnseen)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shared_name_ends_with_the_later_request_value() {
    let base = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{base}/set?y=1"), "")),
        ("b.yml", &req("b", 2, &format!("{base}/set?y=2"), "")),
    ]);
    let exec = executor();
    run_parallel(dir.path(), 2, Arc::clone(&exec), on()).await;
    let values: Vec<String> = exec.cookies().list().into_iter().map(|c| c.value).collect();
    assert_eq!(values, ["2"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redirect_hop_sends_the_cookie_its_previous_hop_set() {
    let base = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, &format!("{base}/redir"), "")),
        ("b.yml", &req("b", 2, &format!("{base}/ok"), "")),
    ]);
    let (report, _, _) = run_parallel(dir.path(), 2, executor(), on()).await;
    assert_eq!(body(&report, 0), "r=1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bail_stops_launching_and_skips_the_rest() {
    let base = server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", 1, "{{nope}}", "")),
        ("b.yml", &req("b", 2, &format!("{base}/ok"), "")),
        ("c.yml", &req("c", 3, &format!("{base}/ok"), "")),
    ]);
    let (report, events, _) = run_parallel(dir.path(), 2, executor(), on().bail(true)).await;
    let outcomes: Vec<Outcome> = report.results.iter().map(|r| r.outcome).collect();
    assert_eq!(
        outcomes,
        [Outcome::Errored, Outcome::Passed, Outcome::Skipped]
    );
    assert_eq!(report.summary.stop, Some(RunStop::FirstFailure));
    assert_eq!(
        events,
        [
            "started 3",
            "start 0",
            "start 1",
            "finish 0 Errored",
            "finish 1 Passed",
            "finish 2 Skipped",
            "finished 3"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_aliased_shared_write_errors_the_request_even_when_caught() {
    let base = server().await;
    let aliased = [
        "const f = \"set\" + \"Var\";\ntry { bru[f](\"x\", 1); } catch (e) {}",
        "const f = \"setEnv\" + \"Var\";\ntry { bru[f](\"x\", 1); } catch (e) {}",
        "const f = \"setNext\" + \"Request\";\ntry { bru[f](\"b\"); } catch (e) {}",
        "const f = \"run\" + \"Request\";\ntry { await bru[f](\"b\"); } catch (e) {}",
        "const f = \"resetOauth2\" + \"Credential\";\ntry { await bru[f](\"id\"); } catch (e) {}",
    ];
    for code in aliased {
        let dir = write_tree(&[
            ("opencollection.yml", ROOT),
            (
                "a.yml",
                &req(
                    "a",
                    1,
                    &format!("{base}/ok"),
                    &script("before-request", code),
                ),
            ),
            ("b.yml", &req("b", 2, &format!("{base}/ok"), "")),
        ]);
        let (report, _, ctx) = run_parallel(dir.path(), 2, executor(), on()).await;
        let a = &report.results[0];
        assert_eq!(a.outcome, Outcome::Errored, "{code}");
        match &a.error {
            Some(RunError::Script { message, .. }) => {
                assert!(
                    message.contains("not allowed in a parallel run"),
                    "{message}"
                )
            }
            other => panic!("{code}: expected a script error, got {other:?}"),
        }
        assert!(ctx.runtime_vars().is_empty(), "{code}");
        assert_eq!(report.results[1].outcome, Outcome::Passed, "{code}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sequential_run_still_allows_shared_writes() {
    let base = server().await;
    let code = "const f = \"set\" + \"Var\";\nbru[f](\"x\", 1);";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req(
                "a",
                1,
                &format!("{base}/ok"),
                &script("before-request", code),
            ),
        ),
    ]);
    let (report, _, ctx) = run_parallel(dir.path(), 1, executor(), on()).await;
    assert_eq!(report.results[0].outcome, Outcome::Passed);
    assert_eq!(ctx.runtime_vars().len(), 1);
}
