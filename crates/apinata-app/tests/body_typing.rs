//! Typing a long line into a request's Body editor stays cheap per key, and the cost of a
//! key does not climb with the line's length. Budgets are measured
//! on the section editor. The section test keeps an absolute p95;
//! the app test judges only a ratio, of the fastest burst on a short and a long line taken
//! in alternation, so load on the machine and stalls in many bursts do not decide it.

use crate::support;

use apinata_app::body_section::BodySection;
use apinata_app::request_editor::Section;
use apinata_core::BodySpec;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Keystroke, TestAppContext};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use support::*;

const LINE: usize = 3000;
const WINDOW: usize = 300;
// The slowest 5% of keys on the long line.
const SECTION_P95: Duration = Duration::from_millis(150);
// How many times a key on the 3000-character line may cost one on a short line. The kit
// editor re-wraps the whole line per key, so the section grows a little.
const SECTION_GROWTH: f64 = 8.0;
// The app test's long line. At this length a per-key cost that walks the line stands well
// clear of load noise, while the kit's per-frame cost barely grows with it.
const APP_LINE: usize = 12000;
// The app's own per-key steps must not add to the kit's growth. The bound leaves room for
// heavy load inside the full test binary.
const APP_GROWTH: f64 = 3.0;

fn request_with_body(text: &str) -> String {
    format!(
        "info:\n  name: j\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://127.0.0.1:9/j\"\n  body:\n    type: text\n    data: '{text}'\n"
    )
}

fn press(w: &mut gpui_kit::Window, cx: &mut gpui_kit::App, name: &str) {
    let mut key = Keystroke::parse(name).unwrap();
    if name.chars().count() == 1 {
        key.key_char = Some(name.into());
    }
    w.dispatch_keystroke(key, cx);
}

// One key in its own update, then whatever it started settles. The window draws itself
// before it dispatches a key, so a key costs one draw, as on a real platform.
fn timed_key(cx: &mut TestAppContext, win: AnyWindowHandle, name: &str) -> Duration {
    let t = Instant::now();
    cx.update_window(win, |_, w, cx| press(w, cx, name))
        .unwrap();
    cx.run_until_parked();
    t.elapsed()
}

fn mean(keys: &[Duration]) -> Duration {
    keys.iter().sum::<Duration>() / keys.len() as u32
}

fn min(bursts: &[Duration]) -> Duration {
    bursts.iter().copied().min().unwrap()
}

fn p95(keys: &[Duration]) -> Duration {
    let mut sorted = keys.to_vec();
    sorted.sort();
    sorted[sorted.len() * 95 / 100]
}

fn growth(short: Duration, long: Duration) -> f64 {
    long.as_secs_f64() / short.as_secs_f64().max(1e-6)
}

// A JSON body section, alone in its window, with its code editor emptied and focused.
fn focused_section(cx: &mut TestAppContext) -> AnyWindowHandle {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    cx.update(apinata_app::keymap::init);
    let section = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        section.update(cx, |s, cx| s.load(&BodySpec::Json(String::new()), w, cx))
    })
    .unwrap();
    frame(cx, win);
    replace_code(cx, win, None, "body-code", "");
    frame(cx, win);
    win
}

#[gpui_kit::test]
fn a_3000_character_line_typed_key_by_key_stays_cheap(cx: &mut TestAppContext) {
    // The reference editor types and deletes one character, so its line stays short.
    let line = focused_section(cx);
    let reference = focused_section(cx);
    let mut keys = Vec::with_capacity(LINE);
    let mut short = Vec::with_capacity(WINDOW);
    for i in 0..LINE {
        keys.push(timed_key(cx, line, "a"));
        if i >= LINE - WINDOW {
            let name = if i % 2 == 0 { "a" } else { "backspace" };
            short.push(timed_key(cx, reference, name));
        }
    }
    let long = mean(&keys[LINE - WINDOW..]);
    let measured = format!(
        "p95 {:?}, first {:?}, last {long:?}, short line {:?}",
        p95(&keys),
        mean(&keys[..WINDOW]),
        mean(&short)
    );
    assert!(p95(&keys) < SECTION_P95, "{measured}");
    assert!(growth(mean(&short), long) < SECTION_GROWTH, "{measured}");
}

// The whole app with request j.yml's Body section showing and the caret at its end.
fn app_body_focused(cx: &mut TestAppContext, body: &str) -> (Started, tempfile::TempDir) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("j.yml", &request_with_body(body)),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "j.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("body").click_at("body-code", code_at(), cx);
        w.press("end", cx);
    })
    .unwrap();
    frame(cx, s.win);
    (s, col)
}

// The pilot's shape: every key of one `type` call in a single update, with no frame between,
// then the edits settle.
fn burst(cx: &mut TestAppContext, s: &Started, n: usize) -> Duration {
    let t = Instant::now();
    cx.update_window(s.win, |_, w, cx| (0..n).for_each(|_| press(w, cx, "a")))
        .unwrap();
    frame(cx, s.win);
    t.elapsed()
}

fn body_text(cx: &mut TestAppContext, s: &Started) -> String {
    match &view_of(cx, &s.ws, &s.root, "j.yml").body {
        BodySpec::Text(t) => t.clone(),
        other => panic!("{other:?}"),
    }
}

#[gpui_kit::test]
fn a_key_in_the_app_costs_the_same_at_the_end_of_a_long_line(cx: &mut TestAppContext) {
    const ROUNDS: usize = 20;
    const KEYS: usize = 10;
    let (short, _col) = app_body_focused(cx, "");
    let (long, _col2) = app_body_focused(cx, &"a".repeat(APP_LINE - ROUNDS * KEYS));
    let (mut on_short, mut on_long) = (Vec::new(), Vec::new());
    for _ in 0..ROUNDS {
        on_short.push(burst(cx, &short, KEYS));
        on_long.push(burst(cx, &long, KEYS));
    }
    assert_eq!(
        body_text(cx, &long).len(),
        APP_LINE,
        "every key reached the body"
    );
    // Load only ever adds time to a burst, so the fastest burst on each side is the nearest
    // to the code's own cost; a key that costs more on the long line slows every long burst,
    // the fastest too. Twenty rounds give each side many chances at a quiet moment.
    let (short_min, long_min) = (min(&on_short), min(&on_long));
    let per_key = |burst: Duration| burst / KEYS as u32;
    let measured = format!(
        "fastest burst per key: short line {:?}, long line {:?}; growth {:.2}",
        per_key(short_min),
        per_key(long_min),
        growth(short_min, long_min)
    );
    assert!(growth(short_min, long_min) < APP_GROWTH, "{measured}");
}
