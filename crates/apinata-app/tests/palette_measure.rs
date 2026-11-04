//! Both palettes keep their measured rows across frames that change nothing, and measure
//! again when a query or the availability changes which rows they list.

use crate::support;

use std::time::{Duration, UNIX_EPOCH};

use apinata_app::app::AppState;
use apinata_app::keymap::ToggleSidebar;
use apinata_app::last_response::{Limits, ResponseStore, STORE_LIMIT};
use apinata_app::palette::{index_path, palette_items, row_builds};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Action, AppContext as _, TestAppContext};
use support::*;

const SENDS: usize = 100;

fn kept(url: &str) -> RequestResult {
    let response = Response::new(200).with_text_body("k").with_url(url);
    RequestResult::from_parts(
        ip("a.yml"),
        "x",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    )
}

// One request with a hundred kept sends to https://h/n/0 through https://h/n/99.
fn seeded(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let root = col.path().to_path_buf();
    started_with(cx, col, move |paths| {
        let store = ResponseStore::new(paths.responses.clone());
        let limits = Limits {
            per_request: 200,
            total_bytes: STORE_LIMIT,
        };
        for i in 0..SENDS {
            let at = UNIX_EPOCH + Duration::from_millis(1_000 + i as u64);
            let url = format!("https://h/n/{i}");
            store
                .save_with(&root, &ip("a.yml"), &kept(&url), at, limits)
                .unwrap();
        }
    })
}

fn type_query(cx: &mut TestAppContext, s: &Started, text: &str) {
    cx.update_window(s.win, |_, w, cx| w.input(text, cx))
        .unwrap();
    frame(cx, s.win);
}

// The rows built over three forced redraws that change nothing.
fn builds_over_idle_frames(cx: &mut TestAppContext, s: &Started) -> usize {
    let before = row_builds();
    for _ in 0..3 {
        cx.update_window(s.win, |_, w, _| w.refresh()).unwrap();
        frame(cx, s.win);
    }
    row_builds() - before
}

// The rows built by the frame that answers `query`.
fn builds_for_query(cx: &mut TestAppContext, s: &Started, query: &str) -> usize {
    let before = row_builds();
    type_query(cx, s, query);
    row_builds() - before
}

#[gpui_kit::test]
fn the_history_palette_keeps_its_rows_measured(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    type_query(cx, &s, "History");
    cx.simulate_keystrokes(s.win, "enter");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        present(cx, s.win, "history-palette")
    });
    frame(cx, s.win);
    let idle = builds_over_idle_frames(cx, &s);
    assert!(idle < SENDS, "three idle frames built {idle} rows");
    // n/1 and n/10 through n/19.
    let typed = builds_for_query(cx, &s, "n/1");
    assert!(typed >= 11, "the new query built only {typed} rows");
    // The rows on screen are the query's, not the unfiltered ones kept from before.
    let targets = listed_targets(cx, &s);
    assert!(!targets.is_empty(), "the matching sends are listed");
    assert!(
        targets.iter().all(|t| t.contains("n/1")),
        "only matching sends are listed: {targets:?}"
    );
}

// The target of every listed History row, top down, as the last frame drew them.
fn listed_targets(cx: &mut TestAppContext, s: &Started) -> Vec<String> {
    cx.update_window(s.win, |_, w, _| {
        let palette = w.within("history-palette");
        (0usize..)
            .map_while(|r| {
                palette
                    .try_find(("history-target", r))?
                    .label()
                    .map(str::to_owned)
            })
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_command_palette_keeps_its_rows_measured(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    frame(cx, s.win);
    let total = palette_items().len();
    let idle = builds_over_idle_frames(cx, &s);
    assert!(
        idle < total,
        "three idle frames built {idle} rows of {total}"
    );
    let toggles = palette_items()
        .iter()
        .filter(|e| e.label.to_lowercase().contains("toggle"))
        .count();
    let typed = builds_for_query(cx, &s, "toggle");
    assert!(
        typed >= toggles,
        "the new query built {typed} rows for {toggles} matches"
    );
    let items = palette_items();
    let row = |action: &dyn Action| {
        let ix = (items.iter())
            .position(|e| e.action.partial_eq(action))
            .expect("listed");
        index_path(&items, ix)
    };
    // The first row shows when the palette opens (see `palette::the_key_opens_the_palette`).
    assert!(!items[0].label.to_lowercase().contains("toggle"));
    let (toggle, other) = (row(&ToggleSidebar), index_path(&items, 0));
    cx.update_window(s.win, |_, w, _| {
        let palette = w.within("command-palette");
        assert!(
            palette.try_find(toggle).is_some(),
            "Toggle sidebar is listed"
        );
        assert!(
            palette.try_find(other).is_none(),
            "a non-match is filtered out"
        );
    })
    .unwrap();
}

// The palette's ListBox rows the last frame drew.
fn listed_rows(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, _| {
        gpui_kit::base::test_support::snapshots(w)
            .iter()
            .filter(|n| n.role() == Some(gpui_kit::Role::ListBoxOption))
            .count()
    })
    .unwrap()
}

// A tab opening or closing under the open palette changes no query, so only the
// availability in the rows key tells the palette its cached rows are stale.
#[gpui_kit::test]
fn the_command_palette_rebuilds_its_rows_when_availability_changes(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    let close = palette_items()
        .into_iter()
        .find(|e| e.action.partial_eq(&apinata_app::tab_area::CloseTab))
        .expect("Close tab is a command")
        .label;
    type_query(cx, &s, close);
    assert_eq!(listed_rows(cx, &s), 0, "no tab, so no Close tab");
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    assert!(
        present(cx, s.win, "command-palette"),
        "the palette stays open"
    );
    assert!(listed_rows(cx, &s) > 0, "the new tab lists Close tab");
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    cx.update_window(s.win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![tab], w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    assert!(
        present(cx, s.win, "command-palette"),
        "the palette stays open"
    );
    assert_eq!(listed_rows(cx, &s), 0, "the closed tab drops Close tab");
}

fn app_zoom(cx: &mut TestAppContext) -> f32 {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().zoom)
}

// The value of the field holding focus as the last frame drew it: the palette's query.
fn focused_text(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    cx.update_window(s.win, |_, w, _| {
        gpui_kit::base::test_support::snapshots(w)
            .iter()
            .find(|n| n.focused() == Some(true) && n.value().is_some())
            .and_then(|n| n.value().map(str::to_owned))
    })
    .unwrap()
}

// Each zoom chord, pressed in the open palette `id`, steps the app zoom, leaves the palette
// open on `query`, and builds its rows again at the new size.
fn zoom_keys_reach(cx: &mut TestAppContext, s: &Started, id: &'static str, query: &str) {
    assert_eq!(focused_text(cx, s).as_deref(), Some(query));
    for (k, want) in [("=", 1.1), ("+", 1.2), ("-", 1.1), ("0", 1.0)] {
        let before = row_builds();
        cx.simulate_keystrokes(s.win, &secondary(k));
        frame(cx, s.win);
        assert_eq!(app_zoom(cx), want, "{id}: {} zooms the app", secondary(k));
        assert!(present(cx, s.win, id), "{id} stays open");
        assert_eq!(
            focused_text(cx, s).as_deref(),
            Some(query),
            "{id} keeps its query"
        );
        assert!(row_builds() > before, "{id} re-measures its rows");
    }
}

#[gpui_kit::test]
fn the_zoom_keys_zoom_the_app_from_the_command_palette(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    type_query(cx, &s, "toggle");
    zoom_keys_reach(cx, &s, "command-palette", "toggle");
}

#[gpui_kit::test]
fn the_zoom_keys_zoom_the_app_from_the_history_palette(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    type_query(cx, &s, "History");
    cx.simulate_keystrokes(s.win, "enter");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        present(cx, s.win, "history-palette")
    });
    type_query(cx, &s, "n/1");
    zoom_keys_reach(cx, &s, "history-palette", "n/1");
}

// A user rebinding of zoom in reaches the palette too, since overrides bind every target.
#[gpui_kit::test]
fn a_rebound_zoom_in_zooms_the_app_from_the_command_palette(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let rebound = overrides(&[("view.zoom_in", &["secondary-u"])]);
    cx.update(|cx| apinata_app::keymap::apply(&rebound, cx));
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    type_query(cx, &s, "toggle");
    cx.simulate_keystrokes(s.win, &secondary("u"));
    frame(cx, s.win);
    assert_eq!(app_zoom(cx), 1.1, "the rebound chord zooms the app");
    assert!(
        present(cx, s.win, "command-palette"),
        "the palette stays open"
    );
    assert_eq!(focused_text(cx, &s).as_deref(), Some("toggle"));
}
