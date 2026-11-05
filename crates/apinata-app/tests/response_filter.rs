//! The JSON filter field replaces the body with the matches of a JSONPath expression.

use crate::support;

use std::time::Duration;

use apinata_app::body_view::{BodyMode, BodyView, SETTLE, Shown};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

const JSON: &str = r#"{"data":[{"id":7,"name":"a","price":12.5},{"id":8,"name":"b","price":3}]}"#;
const PRETTY: &str = "{\n  \"data\": [\n    {\n      \"id\": 7,\n      \"name\": \"a\",\n      \"price\": 12.5\n    },\n    {\n      \"id\": 8,\n      \"name\": \"b\",\n      \"price\": 3\n    }\n  ]\n}";

fn shown_with(
    cx: &mut TestAppContext,
    body: String,
    ct: &str,
) -> (AnyWindowHandle, Entity<BodyView>) {
    let s = send_once(cx, ResponseTemplate::new(200).set_body_raw(body, ct), "");
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    (win, view)
}

fn filter(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>, e: &str) {
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter(e, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn text(cx: &mut TestAppContext, view: &Entity<BodyView>) -> String {
    let ed = view.read_with(cx, |v, _| v.editor().clone());
    ed.read_with(cx, |e, _| e.value().to_string())
}

fn error(cx: &mut TestAppContext, view: &Entity<BodyView>) -> Option<String> {
    view.read_with(cx, |v, _| v.filter_error().map(str::to_owned))
}

#[gpui_kit::test]
fn a_filter_replaces_the_body_and_an_empty_one_restores_it(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    assert_eq!(text(cx, &view), PRETTY);
    filter(cx, win, &view, "$.data[0].id");
    assert_eq!(text(cx, &view), "[\n  7\n]");
    filter(cx, win, &view, "$.data[?(@.price > 10)].name");
    assert_eq!(text(cx, &view), "[\n  \"a\"\n]");
    assert_eq!(error(cx, &view), None);
    filter(cx, win, &view, "");
    assert_eq!(text(cx, &view), PRETTY);
}

#[gpui_kit::test]
fn an_invalid_filter_keeps_the_last_body_and_says_why(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, "$.data[1].name");
    filter(cx, win, &view, "$.data[1");
    assert_eq!(text(cx, &view), "[\n  \"b\"\n]");
    let e = error(cx, &view).expect("an error");
    assert!(e.contains("expected closing ']'"), "{e}");
    assert!(present(cx, win, "body-filter-error"));
    filter(cx, win, &view, "$.data[1].id");
    assert_eq!(
        (text(cx, &view), error(cx, &view)),
        ("[\n  8\n]".into(), None)
    );
}

#[gpui_kit::test]
fn the_toggle_opens_a_typed_field_and_closing_restores(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    frame(cx, win);
    assert!(present(cx, win, "body-edit"));
    cx.update_window(win, |_, w, cx| w.click("body-filter", cx))
        .unwrap();
    frame(cx, win);
    assert!(view.read_with(cx, |v, _| v.filter_open()));
    assert!(present(cx, win, "body-filter-input"));
    cx.update_window(win, |_, w, cx| w.input("data[1].id", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(text(cx, &view), "[\n  8\n]");
    assert!(!present(cx, win, "body-edit"), "no editing a filtered view");
    cx.update_window(win, |_, w, cx| w.click("body-raw", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.mode()), BodyMode::Raw);
    assert_eq!(text(cx, &view), "[\n  8\n]", "the same in raw");
    cx.update_window(win, |_, w, cx| w.click("body-filter", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!view.read_with(cx, |v, _| v.filter_open()));
    assert_eq!(text(cx, &view), JSON);
}

#[gpui_kit::test]
fn a_plain_text_body_offers_no_filter(cx: &mut TestAppContext) {
    let (win, _view) = shown_with(cx, "plain".into(), "text/plain");
    assert!(!present(cx, win, "body-filter"));
}

#[gpui_kit::test]
fn a_filter_on_a_long_json_body_shows_its_small_result(cx: &mut TestAppContext) {
    let items: Vec<String> = (0..30_000).map(|i| format!("{{\"id\":{i}}}")).collect();
    let (win, view) = shown_with(cx, format!("[{}]", items.join(",")), "application/json");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    filter(cx, win, &view, "$[1].id");
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    assert_eq!(text(cx, &view), "[\n  1\n]");
}

fn show_next(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    view: &Entity<BodyView>,
    body: &str,
    ct: &str,
) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(body.to_string(), ct),
        "",
    );
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

#[gpui_kit::test]
fn a_filter_leaves_a_non_json_result_whole_and_editable(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, "$.data[0].id");
    show_next(cx, win, &view, "plain words", "text/plain");
    assert_eq!(text(cx, &view), "plain words");
    assert!(present(cx, win, "body-edit"), "a text body stays editable");
    assert_eq!(label(cx, win, "body-filter-chip"), None);
    show_next(cx, win, &view, JSON, "application/json");
    assert_eq!(
        text(cx, &view),
        "[\n  7\n]",
        "the filter comes back for JSON"
    );
    assert!(!present(cx, win, "body-edit"));
}

#[gpui_kit::test]
fn a_filter_persists_into_a_new_json_result(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, "$.data[1].name");
    show_next(
        cx,
        win,
        &view,
        r#"{"data":[{"name":"x"},{"name":"y"}]}"#,
        "application/json",
    );
    assert_eq!(text(cx, &view), "[\n  \"y\"\n]");
    assert_eq!(
        label(cx, win, "body-filter-chip").as_deref(),
        Some("Filtered by $.data[1].name"),
        "a carried-over filter is never silent"
    );
}

#[gpui_kit::test]
fn an_invalid_filter_on_a_new_result_shows_the_whole_body(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, "$.data[1");
    show_next(cx, win, &view, JSON, "application/json");
    assert_eq!(text(cx, &view), PRETTY);
    assert!(error(cx, &view).is_some());
}

// A JSON array over the editor's byte cap, so it shows as rows.
fn long_json() -> String {
    let items: Vec<String> = (0..30_000).map(|i| format!("{{\"id\":{i}}}")).collect();
    format!("[{}]", items.join(","))
}

fn open_filter(cx: &mut TestAppContext, win: AnyWindowHandle) {
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("body-filter", cx))
        .unwrap();
    frame(cx, win);
}

// Types into the focused field and runs what is ready, without moving the clock.
fn type_text(cx: &mut TestAppContext, win: AnyWindowHandle, typed: &str) {
    cx.update_window(win, |_, w, cx| w.input(typed, cx))
        .unwrap();
    cx.run_until_parked();
}

fn wait(cx: &mut TestAppContext, by: Duration) {
    cx.executor().advance_clock(by);
    cx.run_until_parked();
}

fn in_rows(cx: &mut TestAppContext, view: &Entity<BodyView>) -> bool {
    matches!(view.read_with(cx, |v, _| v.shown()), Shown::Lines(_))
}

#[gpui_kit::test]
fn a_filter_typed_on_a_long_body_waits_for_the_settle_delay(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, long_json(), "application/json");
    assert!(in_rows(cx, &view));
    open_filter(cx, win);
    type_text(cx, win, "$[1].id");
    assert!(
        in_rows(cx, &view),
        "the body stays until the typing settles"
    );
    wait(cx, SETTLE - Duration::from_millis(1));
    assert!(in_rows(cx, &view), "one millisecond early");
    wait(cx, Duration::from_millis(1));
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    assert_eq!(text(cx, &view), "[\n  1\n]");
}

#[gpui_kit::test]
fn each_filter_keystroke_restarts_the_delay(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, long_json(), "application/json");
    open_filter(cx, win);
    type_text(cx, win, "$[1");
    wait(cx, SETTLE / 2);
    type_text(cx, win, "].id");
    // A full delay after the invalid prefix: had its timer survived, its error would show now.
    wait(cx, SETTLE / 2);
    assert_eq!(error(cx, &view), None);
    assert!(in_rows(cx, &view));
    wait(cx, SETTLE / 2);
    assert_eq!(error(cx, &view), None, "the invalid prefix never ran");
    assert_eq!(text(cx, &view), "[\n  1\n]");
}

#[gpui_kit::test]
fn closing_the_filter_restores_a_long_body_at_once(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, long_json(), "application/json");
    open_filter(cx, win);
    type_text(cx, win, "$[1].id");
    wait(cx, SETTLE);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("body-filter", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!view.read_with(cx, |v, _| v.filter_open()));
    assert!(in_rows(cx, &view), "closing does not wait");
}

// A filter that settles while a typed rows-mode find query still waits: the query then
// counts in the filtered text, never in the rows it was typed against.
#[gpui_kit::test]
fn a_filter_settling_over_a_waiting_row_query_counts_in_the_result(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, long_json(), "application/json");
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.open_find(w, cx)))
        .unwrap();
    frame(cx, win);
    type_text(cx, win, "1");
    open_filter(cx, win);
    type_text(cx, win, "$[1].id");
    assert!(in_rows(cx, &view));
    wait(cx, SETTLE);
    assert_eq!(text(cx, &view), "[\n  1\n]");
    assert_eq!(view.read_with(cx, |v, cx| v.find_label(cx)), "1 of 1");
}

const XML: &str = r#"<store><book id="1"><title>Alpha</title></book><book id="2"><title>Beta &amp; Gamma</title></book></store>"#;

#[gpui_kit::test]
fn an_xml_body_offers_the_filter_and_takes_xpath(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, XML.into(), "application/xml");
    assert!(present(cx, win, "body-filter"));
    filter(cx, win, &view, "//book/@id");
    assert_eq!(text(cx, &view), "1\n2");
    filter(cx, win, &view, "//book[2]/title/text()");
    assert_eq!(text(cx, &view), "Beta & Gamma", "a text match shows as is");
    assert_eq!(error(cx, &view), None);
}

#[gpui_kit::test]
fn xpath_matches_read_the_same_in_pretty_and_raw(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, XML.into(), "text/xml");
    filter(cx, win, &view, "//title");
    let matches = "<title>Alpha</title>\n<title>Beta &amp; Gamma</title>";
    assert_eq!(text(cx, &view), matches);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_mode(BodyMode::Raw, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(text(cx, &view), matches);
}

#[gpui_kit::test]
fn an_invalid_xpath_keeps_the_last_body_and_says_why(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, XML.into(), "application/xml");
    filter(cx, win, &view, "//book/@id");
    filter(cx, win, &view, "//book[");
    assert_eq!(text(cx, &view), "1\n2");
    let e = error(cx, &view).expect("an error");
    assert!(e.starts_with("Invalid filter: "), "{e}");
    assert!(present(cx, win, "body-filter-error"));
}

#[gpui_kit::test]
fn a_jsonpath_filter_on_an_xml_result_shows_the_whole_body(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, "$.data[0].id");
    show_next(cx, win, &view, XML, "application/xml");
    assert!(text(cx, &view).contains("<store>"), "{}", text(cx, &view));
    assert!(error(cx, &view).is_some());
}

#[gpui_kit::test]
fn the_placeholder_follows_the_body_kind(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    assert_eq!(view.read_with(cx, |v, _| v.filter_placeholder()), "$..id");
    show_next(cx, win, &view, XML, "application/atom+xml");
    assert_eq!(view.read_with(cx, |v, _| v.filter_placeholder()), "//id");
}

#[gpui_kit::test]
fn a_filter_on_a_long_xml_body_shows_its_small_result(cx: &mut TestAppContext) {
    let items: String = (0..30_000).map(|i| format!("<i n=\"{i}\"/>")).collect();
    let (win, view) = shown_with(cx, format!("<r>{items}</r>"), "application/xml");
    assert!(matches!(
        view.read_with(cx, |v, _| v.shown()),
        Shown::Lines(_)
    ));
    filter(cx, win, &view, "/r/i[2]/@n");
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Editor);
    assert_eq!(text(cx, &view), "1");
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn applied(cx: &mut TestAppContext, view: &Entity<BodyView>) -> Option<String> {
    view.read_with(cx, |v, _| v.applied_filter().map(str::to_owned))
}

#[gpui_kit::test]
fn a_filtered_body_says_so_and_clear_closes_the_filter(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    assert_eq!(label(cx, win, "body-filter-chip"), None);
    filter(cx, win, &view, "$.data[0].id");
    assert_eq!(
        label(cx, win, "body-filter-chip").as_deref(),
        Some("Filtered by $.data[0].id")
    );
    assert_eq!(applied(cx, &view).as_deref(), Some("$.data[0].id"));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("body-filter-clear", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!view.read_with(cx, |v, _| v.filter_open()));
    assert_eq!(text(cx, &view), PRETTY);
    assert_eq!(label(cx, win, "body-filter-chip"), None);
}

#[gpui_kit::test]
fn an_invalid_edit_keeps_naming_the_filter_on_screen(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, "$.data[1].name");
    filter(cx, win, &view, "$.data[1");
    assert_eq!(applied(cx, &view).as_deref(), Some("$.data[1].name"));
    assert_eq!(
        label(cx, win, "body-filter-chip").as_deref(),
        Some("Filtered by $.data[1].name")
    );
}

#[gpui_kit::test]
fn a_json_filter_that_matches_nothing_says_so(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, "$.nothing");
    assert!(view.read_with(cx, |v, _| v.filter_no_match()));
    assert_eq!(
        label(cx, win, "body-filter-no-match").as_deref(),
        Some("No match for $.nothing")
    );
    assert!(!present(cx, win, "body-editor"), "no bare [] on screen");
    assert!(present(cx, win, "body-filter-chip"));
    filter(cx, win, &view, "$.data[0].id");
    assert!(!view.read_with(cx, |v, _| v.filter_no_match()));
    assert!(present(cx, win, "body-editor"));
}

#[gpui_kit::test]
fn an_xpath_filter_that_matches_nothing_says_so(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, XML.into(), "application/xml");
    filter(cx, win, &view, "//nothing");
    assert_eq!(
        label(cx, win, "body-filter-no-match").as_deref(),
        Some("No match for //nothing")
    );
}

#[gpui_kit::test]
fn an_xpath_match_of_only_whitespace_is_a_match(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, "<a><b>   </b></a>".into(), "application/xml");
    filter(cx, win, &view, "//b/text()");
    assert!(!view.read_with(cx, |v, _| v.filter_no_match()));
    assert!(!present(cx, win, "body-filter-no-match"));
    assert_eq!(text(cx, &view), "   ");
}

#[gpui_kit::test]
fn a_padded_filter_is_named_without_its_spaces(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, " $.a ");
    assert_eq!(
        label(cx, win, "body-filter-chip").as_deref(),
        Some("Filtered by $.a")
    );
}

#[gpui_kit::test]
fn a_filter_matching_an_empty_array_shows_it(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, r#"{"items":[]}"#.into(), "application/json");
    filter(cx, win, &view, "$.items");
    assert!(!view.read_with(cx, |v, _| v.filter_no_match()));
    assert!(present(cx, win, "body-editor"));
}

#[gpui_kit::test]
fn copy_keeps_its_place_disabled_when_nothing_matches(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, "$.data[0].id");
    let bounds = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("body-copy-button").map(|e| e.bounds())
        })
        .unwrap()
    };
    let before = bounds(cx).expect("Copy shows over a match");
    filter(cx, win, &view, "$.nothing");
    assert_eq!(bounds(cx), Some(before), "Copy stays where it was");
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("kept".into()));
    cx.update_window(win, |_, w, cx| w.click("body-copy", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("kept"),
        "a disabled Copy copies nothing"
    );
}

#[gpui_kit::test]
fn the_filter_chip_reads_at_the_small_text_size(cx: &mut TestAppContext) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, "$.data[0].id");
    use apinata_app::style::{TEXT_SMALL, at_zoom};
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        // A line box is at least its font size tall, so a shorter one means smaller text.
        let chip = w.find("body-filter-chip-text").bounds();
        let small = at_zoom(TEXT_SMALL, 1.0);
        assert!(chip.size.height >= small, "chip {chip:?} under {small:?}");
    })
    .unwrap();
}

fn press_on(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str, key: &str) {
    tab_to(cx, win, id);
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    cx.run_until_parked();
}

fn filter_opens_by(cx: &mut TestAppContext, key: &str) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    frame(cx, win);
    press_on(cx, win, "body-filter-button", key);
    assert!(
        view.read_with(cx, |v, _| v.filter_open()),
        "{key} opens the filter"
    );
}

#[gpui_kit::test]
fn enter_on_the_filter_button_opens_the_filter(cx: &mut TestAppContext) {
    filter_opens_by(cx, "enter");
}

#[gpui_kit::test]
fn space_on_the_filter_button_opens_the_filter(cx: &mut TestAppContext) {
    filter_opens_by(cx, "space");
}

fn clear_by(cx: &mut TestAppContext, key: &str) {
    let (win, view) = shown_with(cx, JSON.into(), "application/json");
    filter(cx, win, &view, "$.data[0].id");
    frame(cx, win);
    press_on(cx, win, "body-filter-clear-button", key);
    assert!(!view.read_with(cx, |v, _| v.filter_open()), "{key} clears");
    assert_eq!(text(cx, &view), PRETTY);
    assert_eq!(label(cx, win, "body-filter-chip"), None);
}

#[gpui_kit::test]
fn enter_on_clear_closes_the_filter(cx: &mut TestAppContext) {
    clear_by(cx, "enter");
}

#[gpui_kit::test]
fn space_on_clear_closes_the_filter(cx: &mut TestAppContext) {
    clear_by(cx, "space");
}

mod parsed_once {
    use super::*;
    use gpui_kit::Focusable as _;

    fn parses(cx: &mut TestAppContext, view: &Entity<BodyView>) -> usize {
        view.read_with(cx, |v, _| v.filter_parses())
    }

    fn show(cx: &mut TestAppContext, win: AnyWindowHandle, view: &Entity<BodyView>, s: &Sent) {
        let r = s.result.clone();
        cx.update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| v.show(Some(r), w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn json(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json")
    }

    #[gpui_kit::test]
    fn keystrokes_reuse_one_parse_and_a_new_response_parses_again(cx: &mut TestAppContext) {
        let first = send_once(cx, json(JSON), "");
        support::init(cx);
        let (win, view) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                    cx.new(|cx| BodyView::new(w, cx))
                })
            })
            .unwrap();
        show(cx, win, &view, &first);
        assert_eq!(parses(cx, &view), 0, "no filter, no parse");
        for e in ["$.data", "$.data[0]", "$.data[0].id", "$.data[0", "$..name"] {
            filter(cx, win, &view, e);
        }
        assert_eq!(text(cx, &view), "[\n  \"a\",\n  \"b\"\n]");
        assert_eq!(parses(cx, &view), 1);
        show(cx, win, &view, &first);
        assert_eq!(parses(cx, &view), 1, "the same response keeps its parse");

        let other = send_once(cx, json(r#"{"data":[{"name":"z"}]}"#), "");
        show(cx, win, &view, &other);
        assert_eq!(text(cx, &view), "[\n  \"z\"\n]");
        assert_eq!(parses(cx, &view), 2);
        filter(cx, win, &view, "$.data[0].name");
        assert_eq!(parses(cx, &view), 2);
    }

    fn holds(cx: &mut TestAppContext, view: &Entity<BodyView>) -> bool {
        view.read_with(cx, |v, _| v.holds_parsed_body())
    }

    #[gpui_kit::test]
    fn an_edit_to_the_body_keeps_the_parse(cx: &mut TestAppContext) {
        // The filter reads the response, never the editor, so an edit changes nothing it parsed.
        let (win, view) = shown_with(cx, JSON.into(), "application/json");
        filter(cx, win, &view, "$.data[0].id");
        assert_eq!(parses(cx, &view), 1);
        cx.update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| v.set_editing(true, w, cx))
        })
        .unwrap();
        let ed = view.read_with(cx, |v, _| v.editor().clone());
        cx.update_window(win, |_, w, cx| {
            ed.read(cx).focus_handle(cx).focus(w, cx);
            w.input("x", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(
            ed.read_with(cx, |e, _| e.value().contains('x')),
            "the edit took"
        );
        filter(cx, win, &view, "$.data[1].id");
        assert_eq!(text(cx, &view), "[\n  8\n]");
        assert_eq!(parses(cx, &view), 1);
    }

    #[gpui_kit::test]
    fn closing_or_clearing_the_filter_frees_the_parse(cx: &mut TestAppContext) {
        let (win, view) = shown_with(cx, JSON.into(), "application/json");
        filter(cx, win, &view, "$.data[0].id");
        assert!(holds(cx, &view));
        filter(cx, win, &view, "");
        assert!(!holds(cx, &view), "an empty expression frees it");

        filter(cx, win, &view, "$.data[0].id");
        assert_eq!(parses(cx, &view), 2);
        assert!(holds(cx, &view));
        cx.update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| v.toggle_filter(w, cx))
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!view.read_with(cx, |v, _| v.filter_open()));
        assert!(!holds(cx, &view), "closing the bar frees it");
    }

    #[gpui_kit::test]
    fn a_newer_filter_cancels_the_one_still_running(cx: &mut TestAppContext) {
        let (win, view) = shown_with(cx, JSON.into(), "application/json");
        // Both start before either job runs. Whichever order the scheduler picks, the first is
        // dropped unrun or stops at its raised cancel, and lands nothing either way.
        cx.update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| {
                v.set_filter("$..id", w, cx);
                v.set_filter("$..name", w, cx);
            })
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(text(cx, &view), "[\n  \"a\",\n  \"b\"\n]");
        assert_eq!(view.read_with(cx, |v, _| v.cancelled_jobs()), 1);
        assert_eq!(parses(cx, &view), 1);
    }
}

mod xml_parsed_once {
    use super::*;
    use apinata_app::body_view::XML_PARSE_CACHE_MAX_BYTES;

    fn parses(cx: &mut TestAppContext, view: &Entity<BodyView>) -> usize {
        view.read_with(cx, |v, _| v.filter_parses())
    }

    fn holds(cx: &mut TestAppContext, view: &Entity<BodyView>) -> bool {
        view.read_with(cx, |v, _| v.holds_parsed_body())
    }

    #[gpui_kit::test]
    fn keystrokes_on_an_xml_body_reuse_one_parse(cx: &mut TestAppContext) {
        let (win, view) = shown_with(cx, XML.into(), "application/xml");
        assert_eq!(parses(cx, &view), 0, "no filter, no parse");
        for e in [
            "//book",
            "//book/@id",
            "//book[",
            "count(//book)",
            "//title/text()",
        ] {
            filter(cx, win, &view, e);
        }
        assert_eq!(text(cx, &view), "Alpha\nBeta & Gamma");
        assert_eq!(parses(cx, &view), 1);
        assert!(holds(cx, &view));
        filter(cx, win, &view, "");
        assert!(!holds(cx, &view), "an empty expression frees it");
    }

    #[gpui_kit::test]
    fn a_new_xml_response_parses_again(cx: &mut TestAppContext) {
        let (win, view) = shown_with(cx, XML.into(), "application/xml");
        filter(cx, win, &view, "//book/@id");
        assert_eq!(parses(cx, &view), 1);
        show_next(
            cx,
            win,
            &view,
            "<store><book id=\"9\"/></store>",
            "application/xml",
        );
        assert_eq!(text(cx, &view), "9");
        assert_eq!(parses(cx, &view), 2);
    }

    #[gpui_kit::test]
    fn an_xml_body_over_the_cache_limit_is_not_kept(cx: &mut TestAppContext) {
        let n = XML_PARSE_CACHE_MAX_BYTES / "<i/>".len() + 1;
        let (win, view) = shown_with(cx, format!("<r>{}</r>", "<i/>".repeat(n)), "text/xml");
        filter(cx, win, &view, "count(/r/i)");
        assert_eq!(text(cx, &view), n.to_string());
        assert!(!holds(cx, &view), "a body this large keeps no parsed copy");
        filter(cx, win, &view, "count(/r)");
        assert_eq!(parses(cx, &view), 2, "each filter parses it afresh");
    }

    #[gpui_kit::test]
    fn an_empty_attribute_match_is_not_no_match(cx: &mut TestAppContext) {
        let (win, view) = shown_with(cx, r#"<a b=""/>"#.into(), "application/xml");
        filter(cx, win, &view, "//@b");
        assert!(!view.read_with(cx, |v, _| v.filter_no_match()));
        assert!(!present(cx, win, "body-filter-no-match"));
        filter(cx, win, &view, "//@c");
        assert!(view.read_with(cx, |v, _| v.filter_no_match()));
    }

    #[gpui_kit::test]
    fn a_newer_xml_filter_cancels_the_one_still_running(cx: &mut TestAppContext) {
        let (win, view) = shown_with(cx, XML.into(), "application/xml");
        cx.update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| {
                v.set_filter("//book/@id", w, cx);
                v.set_filter("//title/text()", w, cx);
            })
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(text(cx, &view), "Alpha\nBeta & Gamma");
        assert_eq!(view.read_with(cx, |v, _| v.cancelled_jobs()), 1);
        assert_eq!(parses(cx, &view), 1);
    }
}
