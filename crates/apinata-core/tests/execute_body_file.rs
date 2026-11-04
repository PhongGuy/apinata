//! Streaming the final body to a file: the whole body is saved while memory keeps the cap.

use crate::common::body_server::{Reply, Tail, body_server, pattern};
use crate::common::resolved_one;
use crate::common::sse_server::{Script, event, slow_sse_server};
use apinata_core::{
    CancelHandle, ExecError, ExecOptions, Executor, Response, StreamEnd, StreamEvent, StreamOptions,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MIB: usize = 1024 * 1024;
type Seen = Arc<Mutex<Vec<StreamEvent>>>;

fn exec(cap: Option<u64>) -> Executor {
    Executor::new(ExecOptions::new().max_body_bytes(cap)).unwrap()
}

fn request(method: &str, url: &str, timeout_ms: Option<u64>) -> apinata_core::ResolvedRequest {
    let top = timeout_ms.map_or(String::new(), |t| format!("settings:\n  timeout: {t}\n"));
    resolved_one(method, url, "", &top)
}

fn recording(file: &Path) -> (StreamOptions, Seen) {
    let seen: Seen = Arc::default();
    let sink = Arc::clone(&seen);
    let opts = StreamOptions::new()
        .body_file(file)
        .sink(move |e| sink.lock().unwrap().push(e));
    (opts, seen)
}

async fn save(
    exec: &Executor,
    req: &apinata_core::ResolvedRequest,
    opts: StreamOptions,
) -> Result<Response, ExecError> {
    exec.send_streaming(req, &CancelHandle::new(), opts).await
}

/// Everything in `dir` by name, so a stray temp file shows up.
fn names(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

fn progress(seen: &Seen) -> Vec<(u64, Option<u64>)> {
    seen.lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Progress { received, total } => Some((*received, *total)),
            _ => None,
        })
        .collect()
}

fn target(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("out.bin")
}

#[tokio::test]
async fn a_body_past_the_cap_is_saved_whole_while_memory_keeps_the_cap() {
    let body = pattern(30 * MIB);
    let s = body_server(vec![("/", Reply::plain(body.clone()))]).await;
    let dir = tempfile::tempdir().unwrap();
    let out = target(&dir);
    let (opts, seen) = recording(&out);
    let res = save(
        &exec(Some(10 * MIB as u64)),
        &request("GET", &s.url("/"), None),
        opts,
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), body);
    assert_eq!(res.body().len(), 10 * MIB);
    assert!(res.truncated());
    assert_eq!(res.size(), 30 * MIB as u64);
    assert_eq!(res.body_file(), Some(out.as_path()));
    assert_eq!(names(dir.path()), ["out.bin"]);
    // Live chunks carry only the kept bytes; progress counts everything received.
    let kept: usize = seen
        .lock()
        .unwrap()
        .iter()
        .map(|e| match e {
            StreamEvent::Chunk(b) => b.len(),
            _ => 0,
        })
        .sum();
    assert_eq!(kept, 10 * MIB);
    assert_eq!(progress(&seen).last().unwrap().0, 30 * MIB as u64);
}

#[tokio::test]
async fn a_send_without_a_file_reports_none() {
    let s = body_server(vec![("/", Reply::plain(b"hi".to_vec()))]).await;
    let res = save(
        &exec(None),
        &request("GET", &s.url("/"), None),
        StreamOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(res.body_file(), None);
}

#[tokio::test]
async fn a_connection_that_drops_mid_body_leaves_no_file_and_keeps_an_old_one() {
    let s = body_server(vec![(
        "/",
        Reply::short(pattern(2 * MIB), 5 * MIB, Tail::Close),
    )])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let out = target(&dir);
    std::fs::write(&out, b"old").unwrap();
    let (opts, _) = recording(&out);
    let err = save(
        &exec(Some(MIB as u64)),
        &request("GET", &s.url("/"), None),
        opts,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ExecError::Transport { .. }), "{err:?}");
    assert_eq!(std::fs::read(&out).unwrap(), b"old");
    assert_eq!(names(dir.path()), ["out.bin"]);
}

#[tokio::test]
async fn an_abort_with_no_old_file_leaves_the_directory_empty() {
    let s = body_server(vec![(
        "/",
        Reply::short(pattern(MIB), 3 * MIB, Tail::Close),
    )])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let (opts, _) = recording(&target(&dir));
    let res = save(&exec(Some(1024)), &request("GET", &s.url("/"), None), opts).await;
    assert!(res.is_err());
    assert!(names(dir.path()).is_empty());
}

#[tokio::test]
async fn a_cancel_mid_body_leaves_neither_target_nor_temp() {
    let slow = Reply::short(pattern(4 * MIB), 8 * MIB, Tail::Hold)
        .paced(64 * 1024, Duration::from_millis(20));
    let s = body_server(vec![("/", slow)]).await;
    let dir = tempfile::tempdir().unwrap();
    let (opts, seen) = recording(&target(&dir));
    let cancel = CancelHandle::new();
    let later = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        later.cancel();
    });
    let res = exec(Some(1024))
        .send_streaming(&request("GET", &s.url("/"), None), &cancel, opts)
        .await;
    assert!(matches!(res, Err(ExecError::Cancelled)), "{res:?}");
    assert!(!progress(&seen).is_empty(), "the body had started");
    assert!(names(dir.path()).is_empty());
}

#[tokio::test]
async fn a_download_slower_than_the_request_timeout_completes_while_chunks_keep_coming() {
    let body = pattern(10 * 16 * 1024);
    let paced = Reply::plain(body.clone()).paced(16 * 1024, Duration::from_millis(100));
    let s = body_server(vec![("/", paced)]).await;
    let dir = tempfile::tempdir().unwrap();
    let out = target(&dir);
    let (opts, _) = recording(&out);
    // The whole body takes about a second, past the 400 ms timeout, but no gap exceeds it.
    let res = save(
        &exec(Some(1024)),
        &request("GET", &s.url("/"), Some(400)),
        opts,
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), body);
    assert_eq!(res.size(), body.len() as u64);
}

#[tokio::test]
async fn a_stalled_body_past_the_idle_window_errors_and_leaves_no_file() {
    let s = body_server(vec![(
        "/",
        Reply::short(pattern(16 * 1024), MIB, Tail::Hold),
    )])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let (opts, _) = recording(&target(&dir));
    let res = save(
        &exec(Some(1024)),
        &request("GET", &s.url("/"), Some(300)),
        opts,
    )
    .await;
    assert!(matches!(res, Err(ExecError::Timeout { .. })), "{res:?}");
    assert!(names(dir.path()).is_empty());
}

#[tokio::test]
async fn a_gzip_body_is_saved_decoded_and_reports_no_total() {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    let plain = pattern(3 * MIB);
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&plain).unwrap();
    let gz = enc.finish().unwrap();
    let reply = Reply::with_headers(gz, "content-encoding: gzip\r\n");
    let s = body_server(vec![("/", reply)]).await;
    let dir = tempfile::tempdir().unwrap();
    let out = target(&dir);
    let (opts, seen) = recording(&out);
    save(&exec(Some(1024)), &request("GET", &s.url("/"), None), opts)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), plain);
    let seen = progress(&seen);
    assert!(seen.iter().all(|(_, total)| total.is_none()));
    assert_eq!(seen.last().unwrap().0, plain.len() as u64);
}

#[tokio::test]
async fn a_content_length_body_reports_its_total_and_a_chunked_one_does_not() {
    let body = pattern(MIB);
    let s = body_server(vec![
        ("/chunked", Reply::chunked(body.clone())),
        ("/", Reply::plain(body.clone())),
    ])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let (opts, seen) = recording(&target(&dir));
    save(&exec(Some(1024)), &request("GET", &s.url("/"), None), opts)
        .await
        .unwrap();
    let seen = progress(&seen);
    assert!(
        seen.iter()
            .all(|(_, total)| *total == Some(body.len() as u64))
    );
    assert_eq!(seen.last().unwrap().0, body.len() as u64);

    let (opts, seen) = recording(&target(&dir));
    save(
        &exec(Some(1024)),
        &request("GET", &s.url("/chunked"), None),
        opts,
    )
    .await
    .unwrap();
    assert!(progress(&seen).iter().all(|(_, total)| total.is_none()));
    assert_eq!(std::fs::read(target(&dir)).unwrap(), body);
}

#[tokio::test]
async fn a_redirect_writes_only_the_final_hop() {
    let body = pattern(200_000);
    let s = body_server(vec![
        ("/final", Reply::plain(body.clone())),
        ("/", Reply::redirect("/final")),
    ])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let out = target(&dir);
    let (opts, _) = recording(&out);
    let res = save(
        &exec(Some(1024)),
        &request("GET", &s.url("/start"), None),
        opts,
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), body);
    assert_eq!(res.timeline().len(), 2);
    assert_eq!(names(dir.path()), ["out.bin"]);
}

#[tokio::test]
async fn an_empty_body_writes_an_empty_file() {
    let s = body_server(vec![("/", Reply::plain(Vec::new()))]).await;
    let dir = tempfile::tempdir().unwrap();
    let out = target(&dir);
    let (opts, _) = recording(&out);
    let res = save(&exec(None), &request("GET", &s.url("/"), None), opts)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"");
    assert_eq!(res.body_file(), Some(out.as_path()));
}

#[tokio::test]
async fn head_and_204_commit_an_empty_file() {
    let no_content = Reply {
        head: "HTTP/1.1 204 No Content\r\n\r\n".to_owned(),
        ..Reply::plain(Vec::new())
    };
    let head_reply = Reply {
        head: "HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\n".to_owned(),
        ..Reply::plain(Vec::new())
    };
    let s = body_server(vec![("/204", no_content), ("/", head_reply)]).await;
    for (method, path) in [("GET", "/204"), ("HEAD", "/head")] {
        let dir = tempfile::tempdir().unwrap();
        let out = target(&dir);
        let (opts, _) = recording(&out);
        let res = save(&exec(None), &request(method, &s.url(path), None), opts)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"", "{method} {path}");
        assert_eq!(res.body_file(), Some(out.as_path()));
        assert_eq!(names(dir.path()), ["out.bin"]);
    }
}

#[tokio::test]
async fn an_event_stream_stopped_midway_commits_what_arrived() {
    let s = slow_sse_server(Script::sse(vec![event("a"), event("b")])).await;
    let dir = tempfile::tempdir().unwrap();
    let out = target(&dir);
    let (opts, _) = recording(&out);
    let cancel = CancelHandle::new();
    let later = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        later.stop();
    });
    let res = exec(None)
        .send_streaming(&request("GET", &s.url("/"), None), &cancel, opts)
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Stopped));
    assert_eq!(std::fs::read(&out).unwrap(), b"data: a\n\ndata: b\n\n");
    assert_eq!(names(dir.path()), ["out.bin"]);
}

#[tokio::test]
async fn an_unwritable_target_is_a_transport_error_naming_the_path() {
    let s = body_server(vec![("/", Reply::plain(b"hi".to_vec()))]).await;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("missing").join("out.bin");
    let (opts, _) = recording(&out);
    let err = save(&exec(None), &request("GET", &s.url("/"), None), opts)
        .await
        .unwrap_err();
    let ExecError::Transport { message, .. } = err else {
        panic!("{err:?}");
    };
    assert!(message.contains("out.bin"), "{message}");
}

#[tokio::test]
async fn stop_on_a_plain_body_with_a_file_is_an_error_and_discards_the_partial() {
    let slow = Reply::short(pattern(4 * MIB), 8 * MIB, Tail::Hold)
        .paced(64 * 1024, Duration::from_millis(20));
    let s = body_server(vec![("/", slow)]).await;
    let dir = tempfile::tempdir().unwrap();
    let (opts, seen) = recording(&target(&dir));
    let cancel = CancelHandle::new();
    let later = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        later.stop();
    });
    let res = exec(Some(1024))
        .send_streaming(&request("GET", &s.url("/"), None), &cancel, opts)
        .await;
    assert!(matches!(res, Err(ExecError::Transport { .. })), "{res:?}");
    assert!(!progress(&seen).is_empty(), "the body had started");
    assert!(names(dir.path()).is_empty());
}

const TEMP_NAME: &str = ".out.bin.apinata-part";

async fn save_hi(out: &Path) {
    let s = body_server(vec![("/", Reply::plain(b"hi".to_vec()))]).await;
    let (opts, _) = recording(out);
    save(&exec(None), &request("GET", &s.url("/"), None), opts)
        .await
        .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_at_the_temp_name_is_never_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let victim = dir.path().join("victim.txt");
    std::fs::write(&victim, b"precious").unwrap();
    std::os::unix::fs::symlink(&victim, dir.path().join(TEMP_NAME)).unwrap();
    let out = target(&dir);
    save_hi(&out).await;
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
    assert_eq!(std::fs::read(&out).unwrap(), b"hi");
}

#[cfg(unix)]
#[tokio::test]
async fn a_dangling_symlink_at_the_temp_name_is_not_created_through() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("planted.txt");
    std::os::unix::fs::symlink(&missing, dir.path().join(TEMP_NAME)).unwrap();
    let out = target(&dir);
    save_hi(&out).await;
    assert!(!missing.exists());
    assert_eq!(std::fs::read(&out).unwrap(), b"hi");
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_at_the_target_is_replaced_not_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let victim = dir.path().join("victim.txt");
    std::fs::write(&victim, b"precious").unwrap();
    let out = target(&dir);
    std::os::unix::fs::symlink(&victim, &out).unwrap();
    save_hi(&out).await;
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
    assert!(!std::fs::symlink_metadata(&out).unwrap().is_symlink());
    assert_eq!(std::fs::read(&out).unwrap(), b"hi");
}

#[tokio::test]
async fn a_stale_regular_file_at_the_temp_name_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let stale = dir.path().join(TEMP_NAME);
    std::fs::write(&stale, b"stale").unwrap();
    let out = target(&dir);
    save_hi(&out).await;
    assert_eq!(std::fs::read(&stale).unwrap(), b"stale");
    assert_eq!(std::fs::read(&out).unwrap(), b"hi");
    assert_eq!(names(dir.path()), [TEMP_NAME, "out.bin"]);
}

#[tokio::test]
async fn a_taken_first_temp_name_falls_back_to_the_next_and_the_body_arrives_whole() {
    let body = pattern(2 * MIB);
    let s = body_server(vec![("/", Reply::plain(body.clone()))]).await;
    let dir = tempfile::tempdir().unwrap();
    let stale = dir.path().join(TEMP_NAME);
    std::fs::write(&stale, b"stale").unwrap();
    let out = target(&dir);
    let (opts, _) = recording(&out);
    save(&exec(None), &request("GET", &s.url("/"), None), opts)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), body);
    assert_eq!(std::fs::read(&stale).unwrap(), b"stale");
    assert_eq!(names(dir.path()), [TEMP_NAME, "out.bin"]);
}

#[cfg(unix)]
#[tokio::test]
async fn with_every_temp_name_taken_the_save_fails_and_touches_nothing() {
    let s = body_server(vec![("/", Reply::plain(b"hi".to_vec()))]).await;
    let dir = tempfile::tempdir().unwrap();
    let victim = dir.path().join("victim.txt");
    std::fs::write(&victim, b"precious").unwrap();
    // The base name and .1 to .7: plain files, with symlinks to the victim mixed in.
    let mut taken = vec![TEMP_NAME.to_string()];
    taken.extend((1..8).map(|n| format!("{TEMP_NAME}.{n}")));
    for (i, name) in taken.iter().enumerate() {
        let path = dir.path().join(name);
        if i % 3 == 1 {
            std::os::unix::fs::symlink(&victim, &path).unwrap();
        } else {
            std::fs::write(&path, format!("stale{i}")).unwrap();
        }
    }
    let before = names(dir.path());
    let out = target(&dir);
    let (opts, _) = recording(&out);
    let err = save(&exec(None), &request("GET", &s.url("/"), None), opts)
        .await
        .unwrap_err();
    let ExecError::Transport { message, .. } = err else {
        panic!("{err:?}");
    };
    assert!(message.contains("out.bin"), "{message}");
    assert!(message.contains("no free temporary name"), "{message}");
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
    for (i, name) in taken.iter().enumerate() {
        let path = dir.path().join(name);
        if i % 3 == 1 {
            assert!(std::fs::symlink_metadata(&path).unwrap().is_symlink());
        } else {
            assert_eq!(
                std::fs::read(&path).unwrap(),
                format!("stale{i}").as_bytes()
            );
        }
    }
    assert!(!out.exists());
    assert_eq!(names(dir.path()), before);
}
