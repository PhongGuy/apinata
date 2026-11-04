//! Measures how long the response body views take at the large-body limits.
//! Run in release: `cargo test -p apinata-app --release --test large_body -- --ignored --nocapture`.

use crate::support;

use apinata_app::body_text::{BodyKind, EDITOR_MAX_BYTES, EDITOR_MAX_LINES, Prepared, prepare};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, TestAppContext,
    Window, WindowOptions, div,
};
use std::time::Instant;

struct Host(Entity<EditorState>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Editor::new(&self.0))
    }
}

/// A compact JSON array of objects at least `bytes` long.
fn array(bytes: usize) -> String {
    let mut s = String::from("[");
    let mut i = 0;
    while s.len() < bytes {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"id\":{i},\"name\":\"item {i}\",\"tags\":[\"a\",\"b\"],\"ok\":true}}"
        ));
        i += 1;
    }
    s.push(']');
    s
}

/// Main-thread time to load `text` into a fresh editor and draw its first frame.
fn set_value_ms(cx: &mut TestAppContext, text: String, language: &'static str) -> u128 {
    support::init(cx);
    let (win, host) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                let ed = cx.new(|cx| EditorState::new(w, cx).language(language));
                cx.new(|_| Host(ed))
            })
        })
        .unwrap();
    let ed = host.read_with(cx, |h, _| h.0.clone());
    let started = Instant::now();
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |s, cx| s.set_value(text, w, cx))
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    started.elapsed().as_millis()
}

#[gpui_kit::test]
#[ignore = "measurement; run with --release -- --ignored --nocapture"]
fn large_bodies(cx: &mut TestAppContext) {
    let cap = array(10_485_760);
    let started = Instant::now();
    let prepared = prepare(BodyKind::Json, &cap, cap.len() as u64, true);
    let prepare_ms = started.elapsed().as_millis();
    let Prepared::Lines(rows) = prepared else {
        panic!("the cap must fall back to rows")
    };
    println!(
        "cap: {} bytes, prepare {prepare_ms} ms, {} rows",
        cap.len(),
        rows.len()
    );

    let lines: Vec<String> = (0..EDITOR_MAX_LINES)
        .map(|i| format!("  \"k{i}\": {i},"))
        .collect();
    println!(
        "set_value at {EDITOR_MAX_LINES} lines: {} ms",
        set_value_ms(cx, lines.join("\n"), "json")
    );
    println!(
        "set_value at {EDITOR_MAX_BYTES} bytes, one line: {} ms",
        set_value_ms(cx, array(EDITOR_MAX_BYTES - 64), "json")
    );
}
