//! The Console tab: the last send's script output, filtered by level and copied.

use crate::support;

use apinata_app::response_pane::{
    CONSOLE_PANE_CHARS, CONSOLE_PANE_LINES, CONSOLE_ROW_CHARS, CONSOLE_ROW_LINES, ResponsePane,
    ResponseTab, console_line, console_rows, console_shown, console_text,
};
use apinata_core::{ConsoleEntry, ConsoleLevel, ScriptPhase};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

fn entries() -> Vec<ConsoleEntry> {
    vec![
        ConsoleEntry::new(ScriptPhase::PreRequest, ConsoleLevel::Log, "pre 1"),
        ConsoleEntry::new(ScriptPhase::Tests, ConsoleLevel::Warn, "careful"),
        ConsoleEntry::new(
            ScriptPhase::PostResponse,
            ConsoleLevel::Error,
            "{\n  \"a\": 1\n}",
        ),
    ]
}

// Returns the `Sent` too, so its mock server and temp dirs live as long as the window.
fn shown(
    cx: &mut TestAppContext,
    console: Vec<ConsoleEntry>,
) -> (Sent, AnyWindowHandle, Entity<ResponsePane>) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    support::init(cx);
    let mut r = (*s.result).clone();
    r.console = console;
    s.model
        .update(cx, |m, cx| m.show(std::sync::Arc::new(r), cx));
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Console, cx));
    cx.run_until_parked();
    (s, win, view)
}

#[test]
fn lines_carry_level_phase_and_text() {
    assert_eq!(console_line(&entries()[0]), "log [pre-request] pre 1");
    assert_eq!(
        console_text(&entries(), Some(ConsoleLevel::Warn)),
        "warn [tests] careful"
    );
    assert_eq!(
        console_text(&entries(), None),
        "log [pre-request] pre 1\nwarn [tests] careful\nerror [post-response] {\n  \"a\": 1\n}"
    );
    assert_eq!(console_text(&entries(), Some(ConsoleLevel::Debug)), "");
}

#[gpui_kit::test]
fn the_tab_lists_filters_and_copies(cx: &mut TestAppContext) {
    let (_s, win, view) = shown(cx, entries());
    assert_eq!(
        view.read_with(cx, |p, cx| p.tab_count(ResponseTab::Console, cx)),
        Some(3)
    );
    assert!((0..3usize).all(|ix| present(cx, win, ("console", ix))));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.within("console-filter").click(4usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |p, _| p.console_filter()),
        Some(ConsoleLevel::Warn)
    );
    assert!(present(cx, win, ("console", 0usize)) && !present(cx, win, ("console", 1usize)));
    cx.update_window(win, |_, w, cx| w.click("console-copy", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("warn [tests] careful")
    );
    view.update(cx, |p, cx| {
        p.set_console_filter(Some(ConsoleLevel::Debug), cx)
    });
    assert!(present(cx, win, "console-none"));
}

#[gpui_kit::test]
fn no_output_says_so(cx: &mut TestAppContext) {
    let (_s, win, view) = shown(cx, Vec::new());
    assert!(present(cx, win, "console-none"));
    assert_eq!(
        view.read_with(cx, |p, cx| p.tab_count(ResponseTab::Console, cx)),
        Some(0)
    );
}

#[test]
fn a_long_entry_shows_a_bounded_part() {
    let (lines, more) = console_rows(&"x\n".repeat(1_000_000));
    assert_eq!(lines.len(), CONSOLE_ROW_LINES);
    assert_eq!(more.as_deref(), Some("... 999001 more lines"));
    let (lines, more) = console_rows(&"y".repeat(1_000_000));
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].chars().count() <= CONSOLE_ROW_CHARS + 1,
        "{}",
        lines[0].len()
    );
    assert!(lines[0].ends_with('…'));
    assert_eq!(more, None);
    let (lines, more) = console_rows(&format!("{}\nnext\nlast", "z".repeat(CONSOLE_ROW_CHARS)));
    assert_eq!(lines.len(), 1, "a spent budget shows no further line");
    assert_eq!(more.as_deref(), Some("... 2 more lines"));
    let (_, more) = console_rows(&format!("{}\nlast", "z".repeat(CONSOLE_ROW_CHARS)));
    assert_eq!(more.as_deref(), Some("... 1 more line"));
    let (lines, _) = console_rows(&format!("{}\n\n\n", "z".repeat(CONSOLE_ROW_CHARS)));
    assert_eq!(lines.len(), 1, "no blank rows after the budget is spent");
}

#[test]
fn the_pane_shows_a_bounded_part_of_many_entries() {
    // Each entry is CONSOLE_ROW_LINES lines, so the pane's line budget runs out first.
    let tall = "x\n".repeat(CONSOLE_ROW_LINES - 1);
    let (shown, left) = console_shown(vec![tall.as_str(); 1000]);
    let rows: usize = shown.iter().map(|(lines, _)| lines.len()).sum();
    assert!(rows <= CONSOLE_PANE_LINES, "{rows}");
    assert_eq!(shown.len() + left, 1000);
    assert!(left > 0);
    // Each entry spends a full row's characters, so the pane's character budget runs out.
    let wide = "y".repeat(CONSOLE_ROW_CHARS - 1);
    let (shown, left) = console_shown(vec![wide.as_str(); 1000]);
    let chars: usize = shown
        .iter()
        .flat_map(|(l, _)| l)
        .map(|l| l.chars().count())
        .sum();
    assert!(chars <= CONSOLE_PANE_CHARS, "{chars}");
    assert_eq!(shown.len() + left, 1000);
    assert!(left > 0);
}

#[gpui_kit::test]
fn many_entries_end_in_a_note_and_copy_stays_whole(cx: &mut TestAppContext) {
    let tall = "x\n".repeat(CONSOLE_ROW_LINES - 1);
    let console = vec![ConsoleEntry::new(ScriptPhase::Tests, ConsoleLevel::Log, tall.clone()); 10];
    let (_s, win, _view) = shown(cx, console.clone());
    assert!(present(cx, win, "console-more-entries"));
    assert!(!present(cx, win, ("console", 9usize)));
    cx.update_window(win, |_, w, cx| w.click("console-copy", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some(console_text(&console, None))
    );
}

#[test]
fn blank_lines_stay_visible() {
    let (lines, more) = console_rows("a\n\nb\r");
    assert_eq!(lines, ["a", " ", "b"]);
    assert_eq!(more, None);
}

#[gpui_kit::test]
fn copy_keeps_a_long_entry_whole(cx: &mut TestAppContext) {
    let long = "x\n".repeat(5_000);
    let (_s, win, _view) = shown(
        cx,
        vec![ConsoleEntry::new(
            ScriptPhase::Tests,
            ConsoleLevel::Log,
            long.clone(),
        )],
    );
    assert!(present(cx, win, ("console-more", 0usize)));
    cx.update_window(win, |_, w, cx| w.click("console-copy", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some(format!("log [tests] {long}"))
    );
}

#[gpui_kit::test]
fn no_output_is_the_shared_empty_state(cx: &mut TestAppContext) {
    let (_s, win, _view) = shown(cx, Vec::new());
    let title = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.within("console-none").try_find("empty-title").is_some()
        })
        .unwrap();
    assert!(title, "the Console reads like the dock's empty state");
}
