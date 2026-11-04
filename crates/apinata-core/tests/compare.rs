use apinata_core::{
    BodyComparison, BodyState, COMPARE_BODY_LIMIT, HeaderChangeKind, ResponseSnapshot,
    StatusChange, compare,
};

fn ok() -> ResponseSnapshot {
    ResponseSnapshot::new(200)
        .with_status_text("OK")
        .with_body("same")
}

#[test]
fn identical_snapshots_are_identical() {
    let c = compare(&ok(), &ok());
    assert!(c.is_identical());
    assert_eq!(c.change_count(), 0);
}

#[test]
fn a_status_change_carries_both_codes_and_texts() {
    let new = ResponseSnapshot::new(404)
        .with_status_text("Not Found")
        .with_body("same");
    let c = compare(&ok(), &new);
    let StatusChange::Changed { old, new } = &c.status else {
        panic!("{:?}", c.status)
    };
    assert_eq!(
        (old.code, old.text.as_str(), new.code, new.text.as_str()),
        (200, "OK", 404, "Not Found")
    );
    assert_eq!(c.change_count(), 1);
    assert!(!c.is_identical());
}

#[test]
fn the_same_code_with_different_text_is_the_same_status() {
    let new = ResponseSnapshot::new(200)
        .with_status_text("")
        .with_body("same");
    assert_eq!(compare(&ok(), &new).status, StatusChange::Same);
}

#[test]
fn header_names_match_case_insensitively_and_order_is_noise() {
    let old = ok()
        .with_header("Content-Type", "a")
        .with_header("X-A", "1");
    let new = ok()
        .with_header("x-a", "1")
        .with_header("content-type", "a");
    assert!(compare(&old, &new).is_identical());
}

#[test]
fn added_removed_and_changed_headers_are_reported_in_order() {
    let old = ok().with_header("a", "1").with_header("b", "2");
    let new = ok().with_header("b", "3").with_header("c", "4");
    let h = compare(&old, &new).headers;
    let got: Vec<_> = h.iter().map(|h| (h.name.as_str(), h.kind)).collect();
    assert_eq!(
        got,
        [
            ("a", HeaderChangeKind::Removed),
            ("b", HeaderChangeKind::Changed),
            ("c", HeaderChangeKind::Added)
        ]
    );
    assert_eq!(
        (h[1].old.clone(), h[1].new.clone()),
        (vec!["2".to_owned()], vec!["3".to_owned()])
    );
}

#[test]
fn repeated_headers_compare_as_an_ordered_list() {
    let old = ok().with_header("x-m", "a").with_header("x-m", "b");
    let new = ok().with_header("x-m", "b").with_header("x-m", "a");
    let h = compare(&old, &new).headers;
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].old, ["a", "b"]);
    assert_eq!(h[0].new, ["b", "a"]);
}

#[test]
fn volatile_headers_are_flagged_and_do_not_count() {
    let old = ok().with_header("Date", "t1").with_header("x-keep", "1");
    let new = ok().with_header("Date", "t2").with_header("x-keep", "2");
    let c = compare(&old, &new);
    let volatile: Vec<_> = c
        .headers
        .iter()
        .map(|h| (h.name.as_str(), h.volatile))
        .collect();
    assert_eq!(volatile, [("Date", true), ("x-keep", false)]);
    assert_eq!(c.change_count(), 1);
    let only_date = compare(
        &ok().with_header("date", "1"),
        &ok().with_header("date", "2"),
    );
    assert_eq!(only_date.change_count(), 0);
    assert!(!only_date.is_identical());
}

#[test]
fn every_listed_volatile_name_is_flagged() {
    for name in [
        "date",
        "age",
        "etag",
        "last-modified",
        "server-timing",
        "x-request-id",
        "x-trace-id",
        "set-cookie",
    ] {
        let c = compare(&ok().with_header(name, "1"), &ok().with_header(name, "2"));
        assert!(c.headers[0].volatile, "{name}");
    }
}

#[test]
fn json_changes_and_text_hunks_count() {
    let j = compare(
        &ResponseSnapshot::new(200).with_body(r#"{"a":1,"b":1}"#),
        &ResponseSnapshot::new(200).with_body(r#"{"a":2,"b":2}"#),
    );
    assert_eq!(j.change_count(), 2);
    let t = compare(
        &ResponseSnapshot::new(200).with_body("x\n"),
        &ResponseSnapshot::new(200).with_body("y\n"),
    );
    assert_eq!(t.change_count(), 1);
}

#[test]
fn key_order_only_is_not_identical_and_counts_zero() {
    let c = compare(
        &ResponseSnapshot::new(200).with_body(r#"{"a":1,"b":2}"#),
        &ResponseSnapshot::new(200).with_body(r#"{"b":2,"a":1}"#),
    );
    assert_eq!((c.change_count(), c.is_identical()), (0, false));
}

#[test]
fn a_body_over_the_cap_is_too_large_but_status_still_compares() {
    let big = "x".repeat(COMPARE_BODY_LIMIT + 1);
    let c = compare(
        &ResponseSnapshot::new(200).with_body(big.clone()),
        &ResponseSnapshot::new(500).with_body(big),
    );
    assert_eq!(
        c.body,
        BodyComparison::TooLarge {
            limit: COMPARE_BODY_LIMIT,
            old_len: COMPARE_BODY_LIMIT + 1,
            new_len: COMPARE_BODY_LIMIT + 1
        }
    );
    assert!(matches!(c.status, StatusChange::Changed { .. }));
    assert!(!c.is_identical());
}

#[test]
fn an_unavailable_body_does_not_hide_header_changes() {
    let old = ok()
        .with_body_state(BodyState::Dropped)
        .with_header("a", "1");
    let c = compare(&old, &ok().with_header("a", "2"));
    assert!(matches!(c.body, BodyComparison::Unavailable { .. }));
    assert_eq!(c.headers.len(), 1);
}
