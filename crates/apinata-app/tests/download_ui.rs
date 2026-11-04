//! The Download full response button in a body's truncation notice.

use crate::support::body_server::{Reply, pattern};
use crate::support::*;
use apinata_app::body_view::BodyView;
use apinata_app::download_model::{DownloadModel, DownloadPolicy, DownloadState, download_policy};
use apinata_app::send_model::SendModel;
use apinata_core::RequestResult;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const CAP: u64 = 64 * 1024;

struct Rig {
    win: AnyWindowHandle,
    view: Entity<BodyView>,
    dl: Entity<DownloadModel>,
    send: Entity<SendModel>,
    result: Arc<RequestResult>,
    _server: crate::support::body_server::BodyServer,
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
}

// The app's executor with a small body cap, so a body just above it is truncated.
fn ready_capped(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(apinata_app::tokio_bridge::init);
    let mut settings = test_settings();
    settings.max_body_bytes = Some(CAP);
    let store = apinata_core::MemoryStore::new();
    cx.update(|cx| apinata_app::executor::install(&settings, &store, cx))
        .unwrap();
}

fn rig(cx: &mut TestAppContext, method: &str, reply: Reply) -> Rig {
    ready_capped(cx);
    let server = bodies(cx, vec![("/ok", reply)]);
    let url = server.url("/ok");
    let a = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: {method}\n  url: \"{url}\"\n"
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let (state, ws, root) = open_dir(cx, &col);
    let send = sender(cx, &ws);
    send.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let result = finished(cx, &send);
    let wsc = ws.clone();
    let dl = cx.new(|_| DownloadModel::new(wsc));
    crate::support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let (r, d) = (result.clone(), dl.clone());
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.set_download(Some((d, root.clone())), w, cx);
            v.show(Some(r), w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    Rig {
        win,
        view,
        dl,
        send,
        result,
        _server: server,
        _col: col,
        _state: state,
    }
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn running(cx: &mut TestAppContext, r: &Rig) -> bool {
    r.dl.read_with(cx, |m, _| m.is_running())
}

fn choose(cx: &mut TestAppContext, path: PathBuf) {
    assert!(cx.did_prompt_for_new_path(), "no save prompt");
    cx.simulate_new_path_selection(move |_| Some(path.clone()));
    cx.run_until_parked();
}

fn names(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn big() -> Vec<u8> {
    pattern(CAP as usize * 3)
}

#[gpui_kit::test]
fn a_truncated_binary_body_offers_the_download_in_its_notice(cx: &mut TestAppContext) {
    let r = rig(cx, "GET", Reply::plain(big()));
    assert!(r.result.response.as_ref().unwrap().truncated());
    assert!(present(cx, r.win, "body-binary-cut"));
    assert!(present(cx, r.win, "body-download-full"));
    assert!(!present(cx, r.win, "body-download-cancel"));
}

#[gpui_kit::test]
fn a_whole_body_has_no_download_button(cx: &mut TestAppContext) {
    let r = rig(cx, "GET", Reply::plain(pattern(1000)));
    assert!(!r.result.response.as_ref().unwrap().truncated());
    assert!(!present(cx, r.win, "body-download-full"));
}

#[gpui_kit::test]
fn a_truncated_text_body_offers_it_too(cx: &mut TestAppContext) {
    let text = "a".repeat(CAP as usize * 2).into_bytes();
    let r = rig(
        cx,
        "GET",
        Reply::with_headers(text, "content-type: text/plain\r\n"),
    );
    assert!(r.result.response.as_ref().unwrap().truncated());
    assert!(present(cx, r.win, "body-download-full"));
}

#[test]
fn only_get_options_and_head_skip_the_confirm_and_head_is_hidden() {
    assert_eq!(download_policy("HEAD"), DownloadPolicy::Hidden);
    assert_eq!(download_policy("get"), DownloadPolicy::Direct);
    assert_eq!(download_policy("OPTIONS"), DownloadPolicy::Direct);
    assert_eq!(download_policy("POST"), DownloadPolicy::Confirm);
    assert_eq!(download_policy("DELETE"), DownloadPolicy::Confirm);
}

#[gpui_kit::test]
fn a_post_asks_before_sending_again_and_cancel_starts_nothing(cx: &mut TestAppContext) {
    let r = rig(cx, "POST", Reply::plain(big()));
    click(cx, r.win, "body-download-full");
    assert!(present(cx, r.win, "confirm-dialog"));
    assert!(!cx.did_prompt_for_new_path());
    click(cx, r.win, "confirm-cancel");
    assert!(!cx.did_prompt_for_new_path());
    assert!(!running(cx, &r));
    click(cx, r.win, "body-download-full");
    click(cx, r.win, "confirm-send");
    assert!(cx.did_prompt_for_new_path());
}

#[gpui_kit::test]
fn a_get_goes_straight_to_the_save_prompt_and_saves_the_whole_body(cx: &mut TestAppContext) {
    let body = big();
    let r = rig(cx, "GET", Reply::plain(body.clone()));
    let out = tempfile::tempdir().unwrap();
    click(cx, r.win, "body-download-full");
    assert!(!present(cx, r.win, "confirm-dialog"));
    let file = out.path().join("full.bin");
    choose(cx, file.clone());
    wait_real(cx, |cx| !running(cx, &r));
    r.dl.read_with(cx, |m, _| {
        assert!(
            matches!(m.state(), DownloadState::Done { .. }),
            "{}",
            m.describe()
        )
    });
    assert!(std::fs::read(&file).unwrap() == body);
    // The shown response is the capped one still.
    let shown = r.result.response.as_ref().unwrap().body().len() as u64;
    assert_eq!(shown, CAP);
    r.send.read_with(cx, |m, _| {
        assert!(matches!(m.state(), apinata_app::send_model::SendState::Done(d) if Arc::ptr_eq(d, &r.result)))
    });
    frame(cx, r.win);
    assert!(present(cx, r.win, "body-download-full"));
}

#[gpui_kit::test]
fn progress_shows_while_running_and_cancel_removes_the_file(cx: &mut TestAppContext) {
    let reply = Reply::plain(pattern(CAP as usize * 6)).paced(8 * 1024, Duration::from_millis(40));
    let r = rig(cx, "GET", reply);
    let out = tempfile::tempdir().unwrap();
    click(cx, r.win, "body-download-full");
    choose(cx, out.path().join("full.bin"));
    wait_real(cx, |cx| {
        frame(cx, r.win);
        label(cx, r.win, "body-download-progress").is_some_and(|t| t.starts_with("Downloading "))
    });
    assert!(!present(cx, r.win, "body-download-full"));
    click(cx, r.win, "body-download-cancel");
    wait_real(cx, |cx| !running(cx, &r));
    r.dl.read_with(cx, |m, _| {
        assert!(
            matches!(m.state(), DownloadState::Cancelled),
            "{}",
            m.describe()
        )
    });
    wait_real(cx, |_| names(out.path()).is_empty());
    frame(cx, r.win);
    assert!(!present(cx, r.win, "body-download-progress"));
    assert!(present(cx, r.win, "body-download-full"));
    let _ = &r.view;
}

#[gpui_kit::test]
fn progress_and_cancel_stay_when_another_response_is_shown(cx: &mut TestAppContext) {
    let reply = Reply::plain(pattern(CAP as usize * 6)).paced(8 * 1024, Duration::from_millis(40));
    let r = rig(cx, "GET", reply);
    let out = tempfile::tempdir().unwrap();
    click(cx, r.win, "body-download-full");
    choose(cx, out.path().join("full.bin"));
    wait_real(cx, |cx| running(cx, &r));
    // A new, small and whole response replaces the shown one while the download runs.
    let small = Arc::new(apinata_core::RequestResult::from_parts(
        ip("a.yml"),
        "a",
        apinata_core::Outcome::Passed,
        Some(apinata_core::Response::new(200).with_body(vec![0xFF; 10], "")),
        Vec::new(),
    ));
    cx.update_window(r.win, |_, w, cx| {
        r.view.update(cx, |v, cx| v.show(Some(small), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, r.win);
    assert!(present(cx, r.win, "body-download-progress"));
    assert!(present(cx, r.win, "body-download-cancel"));
    // Losing the collection root must not drop the controls either.
    cx.update_window(r.win, |_, w, cx| {
        r.view.update(cx, |v, cx| v.set_download(None, w, cx))
    })
    .unwrap();
    frame(cx, r.win);
    assert!(present(cx, r.win, "body-download-cancel"));
    click(cx, r.win, "body-download-cancel");
    wait_real(cx, |cx| !running(cx, &r));
    wait_real(cx, |_| names(out.path()).is_empty());
}

// The helpers below never render a frame themselves: they read what the window last drew, so a
// model change that forgets to notify the view or invalidate the window shows up as stale.
fn drawn(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

fn drawn_label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn toast_count(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    cx.update_window(win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

// Records whether the download was still running at the view's latest notify. After the
// download ends it must read false, which only a notify made after the end can cause.
fn watch_notifies(cx: &mut TestAppContext, r: &Rig) -> Arc<std::sync::atomic::AtomicBool> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let last_running = Arc::new(AtomicBool::new(true));
    let (seen, dl) = (last_running.clone(), r.dl.clone());
    let sub = cx.update(|cx| {
        cx.observe(&r.view, move |_, cx| {
            seen.store(dl.read(cx).is_running(), Ordering::SeqCst);
        })
    });
    std::mem::forget(sub);
    last_running
}

fn assert_notified_after_end(seen: &std::sync::atomic::AtomicBool) {
    assert!(
        !seen.load(std::sync::atomic::Ordering::SeqCst),
        "the view was not notified after the download ended"
    );
}

fn slow() -> Reply {
    Reply::plain(pattern(CAP as usize * 6)).paced(8 * 1024, Duration::from_millis(40))
}

#[gpui_kit::test]
fn a_finished_download_repaints_and_toasts_without_a_forced_frame(cx: &mut TestAppContext) {
    let r = rig(cx, "GET", slow());
    let out = tempfile::tempdir().unwrap();
    click(cx, r.win, "body-download-full");
    choose(cx, out.path().join("full.bin"));
    wait_real(cx, |cx| {
        frame(cx, r.win);
        drawn_label(cx, r.win, "body-download-progress").is_some()
    });
    let toasts_before = toast_count(cx, r.win);
    let notified = watch_notifies(cx, &r);
    wait_real(cx, |cx| !running(cx, &r));
    cx.run_until_parked();
    r.dl.read_with(cx, |m, _| {
        assert!(
            matches!(m.state(), DownloadState::Done { .. }),
            "{}",
            m.describe()
        )
    });
    assert_notified_after_end(&notified);
    assert!(
        !drawn(cx, r.win, "body-download-progress"),
        "stale progress text after the end"
    );
    assert!(!drawn(cx, r.win, "body-download-cancel"), "stale Cancel");
    assert!(drawn(cx, r.win, "body-download-full"), "button not back");
    assert_eq!(toast_count(cx, r.win), toasts_before + 1, "no Saved toast");
}

#[gpui_kit::test]
fn a_cancelled_download_repaints_without_a_toast_or_a_forced_frame(cx: &mut TestAppContext) {
    let r = rig(cx, "GET", slow());
    let out = tempfile::tempdir().unwrap();
    click(cx, r.win, "body-download-full");
    choose(cx, out.path().join("full.bin"));
    wait_real(cx, |cx| {
        frame(cx, r.win);
        drawn_label(cx, r.win, "body-download-progress").is_some()
    });
    let toasts_before = toast_count(cx, r.win);
    let notified = watch_notifies(cx, &r);
    cx.update_window(r.win, |_, w, cx| w.click("body-download-cancel", cx))
        .unwrap();
    wait_real(cx, |cx| !running(cx, &r));
    cx.run_until_parked();
    assert_notified_after_end(&notified);
    assert!(
        !drawn(cx, r.win, "body-download-progress"),
        "stale progress"
    );
    assert!(drawn(cx, r.win, "body-download-full"), "button not back");
    assert_eq!(
        toast_count(cx, r.win),
        toasts_before,
        "a cancel toasts nothing"
    );
}

#[gpui_kit::test]
fn a_failed_download_repaints_and_toasts_the_error_without_a_forced_frame(cx: &mut TestAppContext) {
    let r = rig(cx, "GET", slow());
    let out = tempfile::tempdir().unwrap();
    click(cx, r.win, "body-download-full");
    let toasts_before = toast_count(cx, r.win);
    let notified = watch_notifies(cx, &r);
    // A target in a directory that does not exist makes the download fail.
    choose(cx, out.path().join("missing").join("full.bin"));
    wait_real(cx, |cx| !running(cx, &r));
    cx.run_until_parked();
    let reason = r.dl.read_with(cx, |m, _| match m.state() {
        DownloadState::Failed(msg) => msg.clone(),
        _ => panic!("{}", m.describe()),
    });
    assert!(!reason.is_empty(), "the failure carries its reason");
    assert_notified_after_end(&notified);
    assert!(
        !drawn(cx, r.win, "body-download-progress"),
        "stale progress"
    );
    assert!(drawn(cx, r.win, "body-download-full"), "button not back");
    // The harness exposes no toast text, so the count stands for the one error toast.
    assert_eq!(toast_count(cx, r.win), toasts_before + 1, "no error toast");
}

const OFFERED: &str = "The body was cut at the size limit; use Download full response";
const NOT_OFFERED: &str = "The body was cut at the size limit, so it cannot be saved";

fn save_lock(cx: &mut TestAppContext, r: &Rig) -> Option<&'static str> {
    r.view.read_with(cx, |v, _| v.save_lock())
}

#[gpui_kit::test]
fn save_body_on_a_cut_body_is_locked_and_points_at_the_download(cx: &mut TestAppContext) {
    let r = rig(cx, "GET", Reply::plain(big()));
    assert_eq!(save_lock(cx, &r), Some(OFFERED));
    // Still shown, so the reason can be read, but it writes nothing and asks for no path.
    assert!(present(cx, r.win, "body-save"));
    click(cx, r.win, "body-save");
    assert!(!cx.did_prompt_for_new_path());
}

#[gpui_kit::test]
fn save_body_on_a_cut_body_without_a_download_says_it_cannot_be_saved(cx: &mut TestAppContext) {
    let r = rig(cx, "GET", Reply::plain(big()));
    cx.update_window(r.win, |_, w, cx| {
        r.view.update(cx, |v, cx| v.set_download(None, w, cx))
    })
    .unwrap();
    frame(cx, r.win);
    assert_eq!(save_lock(cx, &r), Some(NOT_OFFERED));
    assert!(present(cx, r.win, "body-save"));
    click(cx, r.win, "body-save");
    assert!(!cx.did_prompt_for_new_path());
}

#[gpui_kit::test]
fn save_body_on_a_cut_body_refuses_even_when_called_directly(cx: &mut TestAppContext) {
    let r = rig(cx, "GET", Reply::plain(big()));
    cx.update_window(r.win, |_, w, cx| r.view.update(cx, |v, cx| v.save(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_new_path());
}

#[gpui_kit::test]
fn save_body_on_a_whole_body_is_unlocked_and_prompts(cx: &mut TestAppContext) {
    let r = rig(cx, "GET", Reply::plain(pattern(1000)));
    assert_eq!(save_lock(cx, &r), None);
    click(cx, r.win, "body-save");
    assert!(cx.did_prompt_for_new_path());
}
