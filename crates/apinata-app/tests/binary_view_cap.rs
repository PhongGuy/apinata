//! Measures the binary views at candidate byte caps: background format time, the main-thread
//! apply step, and the memory the formatted rows hold.
//! Run in release: `cargo test -p apinata-app --release --test binary_view_cap -- --ignored --nocapture`.

use std::sync::Arc;
use std::time::Instant;

use apinata_app::body_text::{BASE64_ROW_WIDTH, base64_text, hex_rows, rows};
use gpui_kit::SharedString;

const CANDIDATES: [usize; 5] = [256 << 10, 1 << 20, 2 << 20, 4 << 20, 10 << 20];

fn median_ms(mut f: impl FnMut()) -> f64 {
    let mut times: Vec<f64> = (0..5)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[2]
}

/// Resident set size in bytes from /proc (Linux); 0 elsewhere.
fn rss() -> usize {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<usize>().ok())
        .map_or(0, |pages| pages * 4096)
}

/// Bytes the rows hold: each row's text plus its handle and shared-string header.
fn held(rows: &[SharedString]) -> usize {
    rows.iter()
        .map(|r| r.len() + std::mem::size_of::<SharedString>() + 16)
        .sum()
}

#[test]
#[ignore = "measurement; run with --release -- --ignored --nocapture"]
fn binary_view_cap() {
    for n in CANDIDATES {
        let body: Vec<u8> = (0..n).map(|i| (i * 31 % 256) as u8).collect();
        let hex_ms = median_ms(|| {
            std::hint::black_box(hex_rows(&body));
        });
        let b64_ms = median_ms(|| {
            std::hint::black_box(rows(&base64_text(&body), BASE64_ROW_WIDTH));
        });
        let before = rss();
        let hex = hex_rows(&body);
        let rss_mb = before.abs_diff(rss()) as f64 / 1_048_576.0;
        let held_mb = held(&hex) as f64 / 1_048_576.0;
        // The view's apply step: the rows move into `Arc<[SharedString]>` and the
        // previous body's rows of the same size are dropped, both on the main thread.
        let previous: Arc<[SharedString]> = hex_rows(&body).into();
        let start = Instant::now();
        let shown: Arc<[SharedString]> = hex.into();
        drop(previous);
        let apply_ms = start.elapsed().as_secs_f64() * 1000.0;
        std::hint::black_box(&shown);
        println!(
            "{n} bytes: hex {hex_ms:.1} ms, base64 {b64_ms:.1} ms, apply {apply_ms:.2} ms, held {held_mb:.1} MB, rss +{rss_mb:.1} MB"
        );
    }
}
