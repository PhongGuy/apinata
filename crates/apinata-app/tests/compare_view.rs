use apinata_app::compare_view::{CompareRow, badge, rows, summary};
use apinata_core::{BodyState, ResponseSnapshot, compare};

fn snap(code: u16, body: &str) -> ResponseSnapshot {
    ResponseSnapshot::new(code)
        .with_status_text(if code == 200 { "OK" } else { "Not Found" })
        .with_body(body)
}

#[test]
fn identical_responses_collapse_to_same_lines() {
    let c = compare(&snap(200, "x"), &snap(200, "x"));
    assert_eq!(summary(&c), "Identical");
    assert_eq!(
        rows(&c, false),
        vec![
            CompareRow::Group("Status"),
            CompareRow::Note("Same".into()),
            CompareRow::Group("Headers"),
            CompareRow::Note("Same".into()),
            CompareRow::Group("Body"),
            CompareRow::Note("Same".into()),
        ]
    );
}

#[test]
fn a_changed_status_shows_both_lines() {
    let c = compare(&snap(200, "x"), &snap(404, "x"));
    assert_eq!(
        rows(&c, false)[1],
        CompareRow::Status {
            old: "200 OK".into(),
            new: "404 Not Found".into()
        }
    );
    assert!(summary(&c).starts_with("Status changed, headers same, body same"));
}

#[test]
fn json_changes_are_one_row_each() {
    let c = compare(
        &snap(200, r#"{"a":1,"b":1,"c":1}"#),
        &snap(200, r#"{"a":2,"b":2,"c":2}"#),
    );
    assert_eq!(summary(&c), "Status same, headers same, 3 body changes");
    assert_eq!(badge(&c), Some(3));
    let json: Vec<_> = rows(&c, false)
        .into_iter()
        .filter(|r| matches!(r, CompareRow::Json { .. }))
        .collect();
    assert_eq!(json.len(), 3);
}

#[test]
fn key_order_only_reads_as_same_apart_from_order() {
    let c = compare(
        &snap(200, r#"{"a":1,"b":2}"#),
        &snap(200, r#"{"b":2,"a":1}"#),
    );
    assert!(rows(&c, false).contains(&CompareRow::Note("Same apart from key order".into())));
    assert_eq!(summary(&c), "Status same, headers same, key order differs");
}

#[test]
fn text_bodies_become_diff_rows_with_a_hunk_header() {
    let c = compare(&snap(200, "one\ntwo\n"), &snap(200, "one\n2\n"));
    let diff: Vec<_> = rows(&c, false)
        .into_iter()
        .filter_map(|r| match r {
            CompareRow::Diff(d) => Some(d.text),
            _ => None,
        })
        .collect();
    assert_eq!(diff[0], "@@ -1,2 +1,2 @@");
    assert_eq!(&diff[1..], ["one", "two", "2"]);
}

#[test]
fn volatile_headers_fold_until_opened() {
    let a = snap(200, "x")
        .with_header("date", "1")
        .with_header("etag", "a")
        .with_header("x-k", "1");
    let b = snap(200, "x")
        .with_header("date", "2")
        .with_header("etag", "b")
        .with_header("x-k", "2");
    let c = compare(&a, &b);
    let closed = rows(&c, false);
    assert!(closed.contains(&CompareRow::Volatile {
        count: 2,
        open: false
    }));
    assert_eq!(
        closed
            .iter()
            .filter(|r| matches!(r, CompareRow::Header { .. }))
            .count(),
        1
    );
    let open = rows(&c, true);
    assert_eq!(
        open.iter()
            .filter(|r| matches!(r, CompareRow::Header { .. }))
            .count(),
        3
    );
    assert_eq!(
        summary(&c),
        "Status same, 1 header (+2 that change on every send), body same"
    );
}

#[test]
fn the_summary_mentions_volatile_headers_even_when_nothing_else_differs() {
    let c = compare(
        &snap(200, "x").with_header("date", "1"),
        &snap(200, "x").with_header("date", "2"),
    );
    assert_eq!(
        summary(&c),
        "Status same, headers same (+1 that changes on every send), body same"
    );
}

#[test]
fn too_large_and_unavailable_are_one_sentence_each() {
    let big = "x".repeat(apinata_core::COMPARE_BODY_LIMIT + 1);
    let c = compare(&snap(200, &big), &snap(200, &big));
    let last = rows(&c, false).pop().unwrap();
    let CompareRow::Note(text) = last else {
        panic!()
    };
    assert!(
        text.starts_with("Too large to compare: ") && text.contains(", limit "),
        "{text}"
    );
    let c = compare(
        &snap(200, "x").with_body_state(BodyState::Dropped),
        &snap(200, "x"),
    );
    assert_eq!(
        rows(&c, false).pop(),
        Some(CompareRow::Note(
            "Cannot compare bodies: the baseline body was not kept".into()
        ))
    );
    assert_eq!(summary(&c), "Status same, headers same, body unavailable");
}

#[test]
fn an_identical_comparison_has_no_badge() {
    let c = compare(&snap(200, r#"{"a":1}"#), &snap(200, r#"{"a":1}"#));
    assert_eq!(badge(&c), None);
}

#[test]
fn one_item_reads_singular_and_many_plural() {
    use apinata_app::compare_rows::volatile_label;
    use apinata_app::compare_view::more_changes_note;
    assert_eq!(
        volatile_label(1, false),
        "Show 1 header that changes on every send"
    );
    assert_eq!(
        volatile_label(1, true),
        "Hide 1 header that changes on every send"
    );
    assert_eq!(
        volatile_label(3, false),
        "Show 3 headers that change on every send"
    );
    assert_eq!(more_changes_note(1), "1 more change not shown");
    assert_eq!(more_changes_note(2), "2 more changes not shown");
}
