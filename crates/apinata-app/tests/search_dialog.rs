//! The Search collection dialog: opening, listing hits grouped by request, keyboard and
//! click selection, focus return and secret hygiene.

use crate::support;

use apinata_app::request_editor::Section;
use apinata_app::search_dialog::{Latest, section_for, summary};
use apinata_core::SearchField;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn files() -> Vec<(&'static str, String)> {
    let with_header = "info:\n  name: b\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"http://x/b\"\n  headers:\n    - name: X-Needle\n      value: needle-value\n";
    vec![
        ("opencollection.yml", ROOT.to_owned()),
        ("a.yml", request("a", "http://x/needle")),
        ("b.yml", with_header.to_owned()),
    ]
}

fn collection() -> tempfile::TempDir {
    let owned = files();
    let refs: Vec<(&str, &str)> = owned.iter().map(|(p, t)| (*p, t.as_str())).collect();
    write_tree(&refs)
}

fn open_search(cx: &mut TestAppContext, s: &Started) {
    cx.simulate_keystrokes(s.win, &key("shift-f"));
    frame(cx, s.win);
}

fn type_query(cx: &mut TestAppContext, s: &Started, text: &str) {
    cx.update_window(s.win, |_, w, cx| w.input(text, cx))
        .unwrap();
    wait_real(cx, |cx| {
        frame(cx, s.win);
        present(cx, s.win, "search-summary")
    });
}

fn label(
    cx: &mut TestAppContext,
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
) -> Option<String> {
    let id = id.into();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

#[gpui_kit::test]
fn the_shortcut_opens_the_dialog(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    assert!(dialog_open(cx, &s));
    assert!(present(cx, s.win, "collection-search"));
}

#[gpui_kit::test]
fn hits_are_grouped_under_their_request(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "NEEDLE");
    assert_eq!(label(cx, &s, "search-summary").as_deref(), Some("3 hits"));
    assert_eq!(
        label(cx, &s, ("search-group", 0usize)).as_deref(),
        Some("a.yml")
    );
    assert_eq!(
        label(cx, &s, ("search-group", 1usize)).as_deref(),
        Some("b.yml")
    );
    assert_eq!(
        label(cx, &s, ("search-hit", 0usize)).as_deref(),
        Some("URL: http://x/needle")
    );
    assert_eq!(
        label(cx, &s, ("search-hit", 1usize)).as_deref(),
        Some("Header name: X-Needle")
    );
}

#[gpui_kit::test]
fn enter_opens_the_first_hit_and_closes_the_dialog(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "needle");
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    assert!(!dialog_open(cx, &s));
    assert_eq!(tab_names(cx, &s.ws), ["a.yml"]);
}

#[gpui_kit::test]
fn down_moves_the_selection_before_enter(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "needle");
    cx.simulate_keystrokes(s.win, "down enter");
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws), ["b.yml"]);
}

#[gpui_kit::test]
fn clicking_a_hit_opens_it(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "needle");
    cx.update_window(s.win, |_, w, cx| w.click(("search-hit", 1usize), cx))
        .unwrap();
    frame(cx, s.win);
    assert!(!dialog_open(cx, &s));
    assert_eq!(tab_names(cx, &s.ws), ["b.yml"]);
}

#[gpui_kit::test]
fn opening_a_header_hit_shows_the_headers_section(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "needle-value");
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    let section = s.shell.read_with(cx, |sh, cx| {
        let tab = sh.workspace().read(cx).tabs().active().unwrap().id;
        sh.tab_area()
            .read(cx)
            .editor(tab)
            .map(|e| e.read(cx).section())
    });
    assert_eq!(section, Some(Section::Headers));
}

#[gpui_kit::test]
fn escape_closes_the_dialog(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "needle");
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    assert!(!dialog_open(cx, &s));
}

#[gpui_kit::test]
fn a_query_without_hits_says_so(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "zzzz-none");
    assert_eq!(label(cx, &s, "search-summary").as_deref(), Some("No hits"));
    assert!(!present(cx, s.win, ("search-hit", 0usize)));
}

// Clicks a toggle, then waits for the summary to show `want`.
fn toggle_until(cx: &mut TestAppContext, s: &Started, toggle: &'static str, want: &str) {
    cx.update_window(s.win, |_, w, cx| w.click(toggle, cx))
        .unwrap();
    wait_real(cx, |cx| {
        frame(cx, s.win);
        label(cx, s, "search-summary").is_some_and(|l| l.starts_with(want))
    });
    let got = label(cx, s, "search-summary");
    assert!(
        got.as_deref().is_some_and(|l| l.starts_with(want)),
        "summary {got:?}, wanted {want:?}"
    );
}

#[gpui_kit::test]
fn the_dialog_offers_case_and_regex_toggles(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    assert_eq!(label(cx, &s, "search-case").as_deref(), Some("Match case"));
    assert_eq!(label(cx, &s, "search-regex").as_deref(), Some("Regex"));
}

#[gpui_kit::test]
fn the_case_toggle_reruns_the_search_case_sensitively(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "NEEDLE");
    assert_eq!(label(cx, &s, "search-summary").as_deref(), Some("3 hits"));
    toggle_until(cx, &s, "search-case", "No hits");
    toggle_until(cx, &s, "search-case", "3 hits");
}

#[gpui_kit::test]
fn the_regex_toggle_reruns_the_search_as_a_pattern(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "nee.le");
    assert_eq!(label(cx, &s, "search-summary").as_deref(), Some("No hits"));
    toggle_until(cx, &s, "search-regex", "3 hits");
}

#[gpui_kit::test]
fn an_invalid_regex_shows_its_error_instead_of_no_hits(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "(");
    assert_eq!(label(cx, &s, "search-summary").as_deref(), Some("No hits"));
    toggle_until(cx, &s, "search-regex", "invalid regular expression");
    assert!(!present(cx, s.win, ("search-hit", 0usize)));
}

#[gpui_kit::test]
fn a_secret_value_in_dot_env_is_never_found(cx: &mut TestAppContext) {
    let owned = files();
    let mut refs: Vec<(&str, &str)> = owned.iter().map(|(p, t)| (*p, t.as_str())).collect();
    refs.push((
        "environments/dev.yml",
        "name: dev\nvariables:\n  - secret: true\n    name: token\n",
    ));
    refs.push((".env", "token=s3cr3t-dotenv-value\n"));
    let s = started(cx, &write_tree(&refs));
    open_search(cx, &s);
    type_query(cx, &s, "s3cr3t-dotenv-value");
    assert_eq!(label(cx, &s, "search-summary").as_deref(), Some("No hits"));
}

#[gpui_kit::test]
fn the_note_says_only_secret_variable_values_are_excluded(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    let note = label(cx, &s, "search-note").unwrap_or_default();
    assert!(note.contains("secret variable values"), "{note}");
    assert!(!note.contains("credentials"), "{note}");
}

#[gpui_kit::test]
fn the_note_says_unsaved_edits_are_searched(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    let note = label(cx, &s, "search-note").unwrap_or_default();
    assert!(note.contains("unsaved"), "{note}");
}

#[gpui_kit::test]
fn a_form_field_name_with_a_colon_keeps_its_label(cx: &mut TestAppContext) {
    let form = "info:\n  name: f\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://x/f\"\n  body:\n    type: form-urlencoded\n    data:\n      - name: \"k: v\"\n        value: zebra\n";
    let s = started(
        cx,
        &write_tree(&[("opencollection.yml", ROOT), ("f.yml", form)]),
    );
    open_search(cx, &s);
    type_query(cx, &s, "zebra");
    assert_eq!(
        label(cx, &s, ("search-what", 0usize)).as_deref(),
        Some("Body k: v")
    );
}

#[gpui_kit::test]
fn the_first_group_header_is_visible_after_a_search(cx: &mut TestAppContext) {
    // Enough requests that the hit list overflows and scrolls.
    let many: Vec<(String, String)> = (0..40)
        .map(|i| {
            (
                format!("r{i:02}.yml"),
                request(&format!("r{i:02}"), "http://x/needle"),
            )
        })
        .collect();
    let mut refs: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    refs.extend(many.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let s = started(cx, &write_tree(&refs));
    open_search(cx, &s);
    type_query(cx, &s, "needle");
    frame(cx, s.win);
    let (header, list) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find(("search-group", 0usize)).bounds(),
                w.find("search-hits").bounds(),
            )
        })
        .unwrap();
    assert!(
        header.top() >= list.top() && header.bottom() <= list.bottom(),
        "header {header:?} outside list {list:?}"
    );
}

/// Twelve requests with three hits each (url, header name, header value), so hit `n` is in group `n / 3` and multiples of 3 are first.
fn two_hit_groups(cx: &mut TestAppContext) -> Started {
    let two = |name: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/needle\"\n  headers:\n    - name: X-Needle\n      value: needle-value\n"
        )
    };
    let many: Vec<(String, String)> = (0..12)
        .map(|i| (format!("r{i:02}.yml"), two(&format!("r{i:02}"))))
        .collect();
    let mut refs: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    refs.extend(many.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let s = started(cx, &write_tree(&refs));
    open_search(cx, &s);
    type_query(cx, &s, "needle");
    s
}

/// Asserts the selected hit's row is inside the list, and for a group's first hit that the
/// header is inside it too, directly above the row.
fn assert_visible(cx: &mut TestAppContext, s: &Started, hit: usize) {
    frame(cx, s.win);
    frame(cx, s.win);
    let (row, header, list) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find(("search-hit", hit)).bounds(),
                hit.is_multiple_of(3)
                    .then(|| w.find(("search-group", hit / 3)).bounds()),
                w.find("search-hits").bounds(),
            )
        })
        .unwrap();
    assert!(
        row.top() >= list.top() && row.bottom() <= list.bottom(),
        "hit {hit} {row:?} outside list {list:?}"
    );
    if let Some(header) = header {
        assert!(
            header.top() >= list.top() && header.bottom() <= row.top(),
            "header of hit {hit} {header:?} not above {row:?} inside {list:?}"
        );
    }
}

#[gpui_kit::test]
fn the_selected_hit_stays_fully_visible_while_stepping_down_across_groups(cx: &mut TestAppContext) {
    let s = two_hit_groups(cx);
    for step in 1..30usize {
        cx.simulate_keystrokes(s.win, "down");
        assert_visible(cx, &s, step);
    }
}

#[gpui_kit::test]
fn the_selected_hit_stays_fully_visible_while_stepping_up(cx: &mut TestAppContext) {
    let s = two_hit_groups(cx);
    for _ in 0..32 {
        cx.simulate_keystrokes(s.win, "down");
    }
    frame(cx, s.win);
    assert_visible(cx, &s, 32);
    for hit in (0..32usize).rev() {
        cx.simulate_keystrokes(s.win, "up");
        assert_visible(cx, &s, hit);
    }
}

#[gpui_kit::test]
fn a_new_query_while_scrolled_shows_the_selected_hit(cx: &mut TestAppContext) {
    let s = two_hit_groups(cx);
    for _ in 0..15 {
        cx.simulate_keystrokes(s.win, "down");
    }
    frame(cx, s.win);
    type_query(cx, &s, "-value");
    assert_visible(cx, &s, 0);
}

#[test]
fn the_summary_counts_and_notes_the_cap() {
    assert_eq!(summary(0, false), "No hits");
    assert_eq!(summary(1, false), "1 hit");
    assert_eq!(summary(7, false), "7 hits");
    assert_eq!(summary(500, true), "500 hits, showing the first 500");
}

#[test]
fn a_stale_answer_is_dropped() {
    let mut latest = Latest::default();
    let first = latest.start();
    let second = latest.start();
    assert!(!latest.accepts(first));
    assert!(latest.accepts(second));
}

#[test]
fn hits_pick_the_section_that_holds_them() {
    let section = |f, k: Option<&str>| section_for(f, k);
    assert_eq!(section(SearchField::Name, None), None);
    assert_eq!(section(SearchField::Url, None), None);
    assert_eq!(
        section(SearchField::HeaderValue, Some("X")),
        Some(Section::Headers)
    );
    assert_eq!(
        section(SearchField::PathParam, Some("id")),
        Some(Section::Params)
    );
    assert_eq!(section(SearchField::Body, None), Some(Section::Body));
    assert_eq!(
        section(SearchField::Script, Some("pre-request")),
        Some(Section::Script)
    );
    assert_eq!(
        section(SearchField::Script, Some("tests")),
        Some(Section::Tests)
    );
    assert_eq!(section(SearchField::Docs, None), Some(Section::Docs));
}

#[gpui_kit::test]
fn escape_returns_focus_to_where_it_was(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    let handle = s.shell.read_with(cx, gpui_kit::Focusable::focus_handle);
    cx.update_window(s.win, |_, w, cx| handle.focus(w, cx))
        .unwrap();
    open_search(cx, &s);
    assert!(
        !cx.update_window(s.win, |_, w, _| handle.is_focused(w))
            .unwrap()
    );
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    assert!(
        cx.update_window(s.win, |_, w, _| handle.is_focused(w))
            .unwrap()
    );
}

// ---- replace ----

use apinata_app::search_dialog::replace_notes;
use apinata_core::ReplaceReport;

fn report(skipped: &[&str], failed: &[(&str, &str)]) -> ReplaceReport {
    let mut r = ReplaceReport::default();
    r.skipped_unsaved = skipped.iter().map(|s| (*s).to_owned()).collect();
    r.failed = failed
        .iter()
        .map(|(n, w)| ((*n).to_owned(), (*w).to_owned()))
        .collect();
    r
}

#[test]
fn a_clean_replace_shows_only_the_summary() {
    let r = report(&[], &[]);
    assert_eq!(replace_notes(&r), vec![r.summary()]);
}

#[test]
fn skipped_requests_are_named_after_the_summary() {
    let r = report(&["A", "B"], &[]);
    assert_eq!(
        replace_notes(&r),
        vec![r.summary(), "Skipped, unsaved edits: A, B".to_owned()]
    );
}

#[test]
fn more_than_five_skipped_names_are_capped() {
    let r = report(&["a", "b", "c", "d", "e", "f", "g"], &[]);
    assert_eq!(
        replace_notes(&r)[1],
        "Skipped, unsaved edits: a, b, c, d, e and 2 more"
    );
}

#[test]
fn failed_requests_are_named_with_their_reason() {
    let r = report(&[], &[("A", "read only"), ("B", "gone")]);
    assert_eq!(
        replace_notes(&r)[1],
        "Could not replace in: A (read only); B (gone)"
    );
}

fn replace_toggle(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.click("search-replace-toggle", cx))
        .unwrap();
    frame(cx, s.win);
}

fn type_replacement(cx: &mut TestAppContext, s: &Started, text: &str) {
    cx.update_window(s.win, |_, w, cx| w.input(text, cx))
        .unwrap();
    frame(cx, s.win);
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

fn on_disk(s: &Started, rel: &str) -> String {
    std::fs::read_to_string(s.root.join(rel)).unwrap()
}

fn click_id(cx: &mut TestAppContext, s: &Started, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
}

// Searches `query`, opens the replace row and types `with` into it.
fn ready(cx: &mut TestAppContext, s: &Started, query: &str, with: &str) {
    open_search(cx, s);
    type_query(cx, s, query);
    replace_toggle(cx, s);
    type_replacement(cx, s, with);
}

#[gpui_kit::test]
fn replace_is_hidden_until_the_toggle_shows_it(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    open_search(cx, &s);
    type_query(cx, &s, "needle");
    assert_eq!(
        label(cx, &s, "search-replace-toggle").as_deref(),
        Some("Show replace")
    );
    for id in [
        "search-replace-input",
        "search-replace-hint",
        "search-replace-all",
    ] {
        assert!(!present(cx, s.win, id), "{id} hidden");
    }
    assert!(!present(cx, s.win, ("search-replace-hit", 0usize)));
    replace_toggle(cx, &s);
    assert_eq!(
        label(cx, &s, "search-replace-toggle").as_deref(),
        Some("Hide replace")
    );
    for id in [
        "search-replace-input",
        "search-replace-hint",
        "search-replace-all",
    ] {
        assert!(present(cx, s.win, id), "{id} shown");
    }
    assert_eq!(
        label(cx, &s, "search-replace-hint").as_deref(),
        Some("Changes are written to disk. Revert them with git.")
    );
    assert_eq!(
        label(cx, &s, ("search-replace-hit", 0usize)).as_deref(),
        Some("Replace this match")
    );
    assert_eq!(
        label(cx, &s, "search-replace-all").as_deref(),
        Some("Replace all")
    );
    replace_toggle(cx, &s);
    assert!(!present(cx, s.win, "search-replace-input"));
}

#[gpui_kit::test]
fn replacing_one_match_writes_it_and_refreshes_the_hits(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    ready(cx, &s, "needle", "pin");
    let shown = toasts(cx, &s);
    click_id(cx, &s, ("search-replace-hit", 0usize));
    wait_real(cx, |cx| {
        frame(cx, s.win);
        label(cx, &s, "search-summary").as_deref() == Some("2 hits")
    });
    assert!(on_disk(&s, "a.yml").contains("http://x/pin"));
    assert!(
        on_disk(&s, "b.yml").contains("X-Needle"),
        "other files stay"
    );
    assert!(dialog_open(cx, &s), "the dialog stays open");
    assert!(tab_names(cx, &s.ws).is_empty(), "the hit was not opened");
    assert_eq!(toasts(cx, &s), shown + 1);
}

#[gpui_kit::test]
fn replace_all_rewrites_every_file_and_reports_it(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    ready(cx, &s, "needle", "pin");
    let shown = toasts(cx, &s);
    click_id(cx, &s, "search-replace-all");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        label(cx, &s, "search-summary").as_deref() == Some("No hits")
    });
    assert!(on_disk(&s, "a.yml").contains("http://x/pin"));
    let b = on_disk(&s, "b.yml");
    assert!(b.contains("X-pin") && b.contains("pin-value"), "{b}");
    assert!(dialog_open(cx, &s));
    assert_eq!(toasts(cx, &s), shown + 1, "one summary toast");
}

#[gpui_kit::test]
fn a_request_with_unsaved_edits_is_skipped_and_named(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    s.ws.update(cx, |ws, cx| {
        ws.send(&s.root, edit_url("a.yml", "http://x/needle-edited"), cx)
    });
    cx.run_until_parked();
    ready(cx, &s, "needle", "pin");
    let shown = toasts(cx, &s);
    click_id(cx, &s, "search-replace-all");
    wait_real(cx, |_| on_disk(&s, "b.yml").contains("X-pin"));
    cx.run_until_parked();
    assert!(
        on_disk(&s, "a.yml").contains("http://x/needle"),
        "a untouched"
    );
    assert!(!on_disk(&s, "a.yml").contains("pin"));
    assert_eq!(toasts(cx, &s), shown + 2, "summary and the skipped names");
}

#[gpui_kit::test]
fn an_invalid_regex_replaces_nothing(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    open_search(cx, &s);
    click_id(cx, &s, "search-regex");
    type_query(cx, &s, "(");
    replace_toggle(cx, &s);
    type_replacement(cx, &s, "pin");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        label(cx, &s, "search-summary").is_some_and(|l| l.contains("invalid regular expression"))
    });
    let shown = toasts(cx, &s);
    click_id(cx, &s, "search-replace-all");
    cx.run_until_parked();
    assert_eq!(on_disk(&s, "a.yml"), request("a", "http://x/needle"));
    assert_eq!(on_disk(&s, "b.yml"), files()[2].1);
    assert_eq!(toasts(cx, &s), shown, "nothing to report");
}

#[gpui_kit::test]
fn the_published_snapshot_shows_the_replaced_text(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    ready(cx, &s, "needle", "pin");
    click_id(cx, &s, "search-replace-all");
    wait_real(cx, |cx| {
        url(&ws_snap(cx, &s.ws, &s.root), "a.yml") == "http://x/pin"
    });
}

// Replaces every hit from a focused Replace all, which then locks with nothing left to replace.
fn replace_all_by_key(cx: &mut TestAppContext, s: &Started) {
    ready(cx, s, "needle", "pin");
    enter_on(cx, s.win, "search-replace-all-button");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        label(cx, s, "search-summary").as_deref() == Some("No hits")
    });
    assert!(on_disk(s, "a.yml").contains("http://x/pin"));
}

#[gpui_kit::test]
fn replace_all_locking_its_button_moves_focus_to_the_query(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    replace_all_by_key(cx, &s);
    assert!(
        holds_focus(cx, s.win, "search-input"),
        "the query input takes focus"
    );
    press_key(cx, s.win, "escape");
    assert!(!dialog_open(cx, &s), "Escape closes the dialog");
}

// Once Replace all locks, Tab still moves within the dialog and Escape still closes it.
#[gpui_kit::test]
fn replace_all_with_no_hits_keeps_keys_alive(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    replace_all_by_key(cx, &s);
    let focused =
        |cx: &mut TestAppContext| cx.update_window(s.win, |_, w, cx| w.focused(cx)).unwrap();
    let before = focused(cx);
    press_key(cx, s.win, "tab");
    assert_ne!(focused(cx), before, "Tab moves focus");
    assert!(dialog_open(cx, &s));
    press_key(cx, s.win, "escape");
    assert!(!dialog_open(cx, &s), "Escape closes the dialog");
}

#[gpui_kit::test]
fn enter_on_the_focused_case_toggle_reruns_the_search(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "NEEDLE");
    enter_on(cx, s.win, "search-case-button");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        label(cx, &s, "search-summary").is_some_and(|l| l.starts_with("No hits"))
            || !dialog_open(cx, &s)
    });
    assert!(dialog_open(cx, &s), "the dialog stays open");
    assert_eq!(label(cx, &s, "search-summary").as_deref(), Some("No hits"));
}

#[gpui_kit::test]
fn enter_on_the_focused_replace_toggle_shows_the_replace_row(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    enter_on(cx, s.win, "search-replace-toggle-button");
    frame(cx, s.win);
    assert!(dialog_open(cx, &s), "the dialog stays open");
    assert!(present(cx, s.win, "search-replace-input"));
}

// A click on a toggle leaves Enter to the query, which has no hit to open: the dialog stays.
#[gpui_kit::test]
fn enter_after_clicking_a_toggle_keeps_the_dialog(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "zzzz-none");
    toggle_until(cx, &s, "search-case", "No hits");
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(dialog_open(cx, &s), "Enter after a click keeps the dialog");
    cx.update_window(s.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(!dialog_open(cx, &s), "Escape closes");
}

// A click on the summary text puts focus on no control; Enter still keeps the dialog.
#[gpui_kit::test]
fn enter_after_clicking_the_summary_keeps_the_dialog(cx: &mut TestAppContext) {
    let s = started(cx, &collection());
    open_search(cx, &s);
    type_query(cx, &s, "zzzz-none");
    cx.update_window(s.win, |_, w, cx| w.click("search-summary", cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(dialog_open(cx, &s), "Enter after a click keeps the dialog");
}

/// The text a replace toast carries, by its place among the toasts of one replace.
fn note(cx: &mut TestAppContext, s: &Started, ix: usize) -> Option<String> {
    label(cx, s, ("search-replace-note", ix))
}

#[gpui_kit::test]
fn the_skipped_names_reach_the_toast(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    s.ws.update(cx, |ws, cx| {
        ws.send(&s.root, edit_url("a.yml", "http://x/needle-edited"), cx)
    });
    cx.run_until_parked();
    ready(cx, &s, "needle", "pin");
    click_id(cx, &s, "search-replace-all");
    wait_real(cx, |cx| note(cx, &s, 1).is_some());
    assert_eq!(
        note(cx, &s, 0).as_deref(),
        Some("Replaced 2 in 1 file; skipped 1 with unsaved edits")
    );
    assert_eq!(
        note(cx, &s, 1).as_deref(),
        Some("Skipped, unsaved edits: a")
    );
}

#[gpui_kit::test]
fn an_invalid_regex_replace_toasts_the_actors_error(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    open_search(cx, &s);
    click_id(cx, &s, "search-regex");
    type_query(cx, &s, "needle");
    wait_real(cx, |cx| {
        label(cx, &s, "search-summary").as_deref() == Some("3 hits")
    });
    replace_toggle(cx, &s);
    type_replacement(cx, &s, "pin");
    // The query turns invalid and Replace all is pressed before the new search answers, so
    // the old hits keep the button live and only the actor can refuse the pattern.
    cx.update_window(s.win, |_, w, cx| {
        w.click("search-input", cx);
        w.input("(", cx);
        w.click("search-replace-all", cx);
    })
    .unwrap();
    wait_real(cx, |cx| note(cx, &s, 0).is_some());
    let text = note(cx, &s, 0).unwrap();
    assert!(text.starts_with("invalid regular expression"), "{text}");
    assert!(present(cx, s.win, "toast-error"), "an error toast");
    assert_eq!(on_disk(&s, "a.yml"), request("a", "http://x/needle"));
    assert_eq!(on_disk(&s, "b.yml"), files()[2].1);
}

#[gpui_kit::test]
fn a_clean_open_tab_follows_a_replace(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    cx.run_until_parked();
    frame(cx, s.win);
    let editor = editor_of(cx, &s, tab);
    assert_eq!(editor.read_with(cx, |e, cx| e.url(cx)), "http://x/needle");
    ready(cx, &s, "needle", "pin");
    click_id(cx, &s, "search-replace-all");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        editor.read_with(cx, |e, cx| e.url(cx)) == "http://x/pin"
    });
    cx.run_until_parked();
    assert!(
        on_disk(&s, "a.yml").contains("http://x/pin"),
        "the tab wrote nothing back"
    );
}
