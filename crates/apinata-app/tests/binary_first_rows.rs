//! Measures a binary body from arriving at the view to its first rows: the stages
//! (kind scan, prepare, hex rows) and the whole `show` through the first rendered frame.
//! Run in release: `cargo test -p apinata-app --release --test integration binary_first_rows -- --ignored --nocapture`.

use crate::support;

use std::sync::Arc;
use std::time::Instant;

use apinata_app::body_text::{BinaryMode, binary_rows, body_kind, prepare_body};
use apinata_app::body_view::{BodyView, Shown};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};
use support::*;

/// Pseudo-random bytes from xorshift64; random data is almost surely not UTF-8.
fn random(n: usize) -> Vec<u8> {
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 24) as u8
        })
        .collect()
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[gpui_kit::test]
#[ignore = "measurement; run with --release -- --ignored --nocapture"]
fn binary_first_rows(cx: &mut TestAppContext) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    for n in [256usize << 10, 4 << 20] {
        let body = random(n);
        // Arrival: the response and result are built on the sending side.
        let t = Instant::now();
        let r = Response::new(200)
            .with_header("content-type", "application/octet-stream")
            .with_body(body.clone(), "");
        let build_ms = ms(t);
        let headers = r.headers().to_vec();

        let t = Instant::now();
        let kind = body_kind(&headers, r.body());
        let kind_ms = ms(t);
        let t = Instant::now();
        let prepared = prepare_body(kind, r.body(), r.text(), true, false);
        let prepare_ms = ms(t);
        let t = Instant::now();
        let rows = binary_rows(r.body(), BinaryMode::Hex);
        let rows_ms = ms(t);
        std::hint::black_box((&prepared, rows.len()));

        let result = Arc::new(RequestResult::from_parts(
            ip("a.yml"),
            "a",
            Outcome::Passed,
            Some(r),
            Vec::new(),
        ));
        // Whole path: `show` (background prepare + apply) then the first frame with rows.
        let t = Instant::now();
        cx.update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| v.show(Some(result), w, cx))
        })
        .unwrap();
        cx.run_until_parked();
        let shown_ms = ms(t);
        let t = Instant::now();
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        let frame_ms = ms(t);
        assert!(matches!(
            view.read_with(cx, |v, _| v.shown()),
            Shown::Binary(_)
        ));
        println!(
            "{n} bytes: build {build_ms:.1} ms, kind {kind_ms:.1} ms, prepare {prepare_ms:.1} ms, \
             hex rows {rows_ms:.1} ms | show+parked {shown_ms:.1} ms, first frame {frame_ms:.1} ms, \
             total {:.1} ms",
            shown_ms + frame_ms
        );
    }
}

/// Bytes this thread allocates while building the rows; the result is dropped after the count.
fn rows_allocated(body: &[u8], mode: BinaryMode) -> u64 {
    let (rows, allocated) = crate::counting(|| binary_rows(body, mode));
    drop(rows);
    allocated.bytes
}

/// Linear allocation is 4x for 4x the bytes and quadratic copying is 16x, so 4.5x catches it.
/// Non-allocating quadratic work is not caught, and no timing is involved.
#[test]
fn binary_rows_scale_linearly() {
    let small = random(1 << 20);
    let large = random(4 << 20);
    for mode in [BinaryMode::Hex, BinaryMode::Base64] {
        let a_small = rows_allocated(&small, mode);
        let a_large = rows_allocated(&large, mode);
        assert!(
            a_large as f64 <= 4.5 * a_small as f64,
            "{mode:?}: 4 MB allocated {a_large} bytes, 1 MB allocated {a_small} bytes"
        );
    }
}
