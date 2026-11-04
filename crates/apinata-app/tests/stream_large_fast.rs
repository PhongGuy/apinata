//! A multi-MiB stream is kept as a 1 MiB head, and a flood of events neither loses any nor
//! blocks Stop.

use crate::support::sse_server::{FAST_COUNT, LARGE_BYTES, fast_bytes};
use crate::support::*;
use apinata_app::last_response::{ResponseStore, STREAM_BODY_LIMIT};
use apinata_core::StreamEnd;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};

const _: () = assert!(LARGE_BYTES > STREAM_BODY_LIMIT);

// Pinned, not fixed: the cap already held when this test was written.
#[gpui_kit::test]
fn a_large_stream_is_kept_as_a_one_mib_head(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = qa_sse(cx);
    let col = collection_for(&format!("http://127.0.0.1:{}/large", server.port()), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| apinata_app::last_response::install(store.clone(), cx));
    let model = sender(cx, &ws);
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    let live = result.response.as_ref().unwrap();
    assert_eq!(live.stream_end(), Some(&StreamEnd::Closed));
    assert!(live.body().len() >= LARGE_BYTES, "the whole stream arrived");
    wait_real(cx, |_| store.load(&root, &ip("a.yml")).is_some());
    let kept = store.load(&root, &ip("a.yml")).unwrap();
    let kept = kept.result.response.unwrap();
    assert!(kept.truncated());
    assert!(kept.body().len() <= STREAM_BODY_LIMIT);
    assert!(kept.body().len() > STREAM_BODY_LIMIT - 2000);
}

#[gpui_kit::test]
fn a_fast_stream_is_read_whole_and_stop_still_works(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = qa_sse(cx);
    let col = collection_for(&format!("http://127.0.0.1:{}/fast", server.port()), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    // Every byte of the flood reaches the live buffer; the condition, not a sleep, ends the wait.
    let want = fast_bytes(FAST_COUNT);
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.bytes() >= want))
    });
    assert!(present(cx, win, "stop-button"));
    cx.update_window(win, |_, w, cx| w.click("stop-button", cx))
        .unwrap();
    let result = finished(cx, &model);
    assert_eq!(
        result.response.as_ref().unwrap().stream_end(),
        Some(&StreamEnd::Stopped)
    );
}
