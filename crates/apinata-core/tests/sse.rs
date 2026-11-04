use apinata_core::sse::{SseEvent, SseParser, is_event_stream, parse};

fn ev(data: &str) -> SseEvent {
    SseEvent::new(data)
}

#[test]
fn two_events_with_fields() {
    let got = parse(b"id: 7\nevent: tick\ndata: one\n\ndata: two\n\n");
    assert_eq!(
        got,
        vec![ev("one").with_id("7").with_event("tick"), ev("two")]
    );
}

#[test]
fn data_lines_join_with_a_newline_and_a_bare_data_line_is_empty() {
    assert_eq!(parse(b"data: a\ndata: b\n\n"), vec![ev("a\nb")]);
    assert_eq!(parse(b"data\n\n"), vec![ev("")]);
    assert_eq!(parse(b"data:\ndata: x\n\n"), vec![ev("\nx")]);
}

#[test]
fn lf_crlf_and_cr_all_end_lines() {
    assert_eq!(
        parse(b"data: a\r\n\r\ndata: b\r\rdata: c\n\n"),
        vec![ev("a"), ev("b"), ev("c")]
    );
}

#[test]
fn comments_and_unknown_fields_are_ignored() {
    assert_eq!(parse(b": ping\n\n"), vec![]);
    assert_eq!(parse(b": hi\nfoo: bar\ndata: x\n\n"), vec![ev("x")]);
}

#[test]
fn only_one_leading_space_is_dropped() {
    assert_eq!(parse(b"data:  two\n\n"), vec![ev(" two")]);
    assert_eq!(parse(b"data:none\n\n"), vec![ev("none")]);
}

#[test]
fn id_with_nul_and_non_numeric_retry_are_ignored() {
    assert_eq!(parse(b"id: a\0b\ndata: x\n\n"), vec![ev("x")]);
    assert_eq!(
        parse(b"retry: 1500\ndata: x\n\n"),
        vec![ev("x").with_retry(1500)]
    );
    assert_eq!(parse(b"retry: 15s\ndata: x\n\n"), vec![ev("x")]);
}

#[test]
fn an_event_without_data_is_not_dispatched() {
    assert_eq!(parse(b"event: x\n\ndata: y\n\n"), vec![ev("y")]);
}

#[test]
fn a_leading_bom_is_stripped_and_a_trailing_incomplete_event_dropped() {
    assert_eq!(parse(b"\xEF\xBB\xBFdata: a\n\ndata: half"), vec![ev("a")]);
}

#[test]
fn invalid_utf8_becomes_the_replacement_character() {
    assert_eq!(parse(b"data: a\xFFb\n\n"), vec![ev("a\u{FFFD}b")]);
}

const FIXTURE: &[u8] =
    "id: 1\r\nevent: e\r\ndata: caf\u{e9}\r\ndata: two\r\n\r\n: c\ndata: x\r\r".as_bytes();

#[test]
fn every_two_way_split_matches_the_one_shot_parse() {
    let want = parse(FIXTURE);
    assert_eq!(want.len(), 2);
    for cut in 0..=FIXTURE.len() {
        let mut p = SseParser::new();
        let mut got = p.push(&FIXTURE[..cut]);
        got.extend(p.push(&FIXTURE[cut..]));
        assert_eq!(got, want, "split at {cut}");
    }
}

#[test]
fn byte_at_a_time_matches_too() {
    let mut p = SseParser::new();
    let mut got = Vec::new();
    for b in FIXTURE {
        got.extend(p.push(std::slice::from_ref(b)));
    }
    assert_eq!(got, parse(FIXTURE));
}

#[test]
fn content_type_detection_follows_one_part_equality() {
    assert!(is_event_stream("text/event-stream"));
    assert!(is_event_stream("TEXT/Event-Stream; charset=utf-8"));
    assert!(is_event_stream("foo/bar; text/event-stream"));
    assert!(!is_event_stream("text/event-streams"));
    assert!(!is_event_stream("application/json"));
    assert!(!is_event_stream(""));
}

#[test]
fn an_overlong_line_is_dropped_and_buffering_stays_bounded() {
    let mut p = SseParser::with_max_line(16);
    let mut got = p.push(b"data: ok\n\n");
    // One endless line, fed in pieces: the buffer never passes the cap.
    for _ in 0..1000 {
        got.extend(p.push(&[b'x'; 64]));
        assert!(p.buffered() <= 16, "buffered {}", p.buffered());
    }
    // The dropped line ends at its terminator and parsing carries on.
    got.extend(p.push(b"\ndata: next\n\n"));
    assert_eq!(got, vec![ev("ok"), ev("next")]);
}

#[test]
fn a_line_exactly_at_the_cap_is_kept() {
    let mut p = SseParser::with_max_line(16);
    assert_eq!(p.push(b"data: 0123456789\n\n"), vec![ev("0123456789")]);
}

#[test]
fn the_default_cap_is_one_mebibyte() {
    let mut p = SseParser::new();
    p.push(&vec![b'x'; 3 * 1024 * 1024]);
    assert!(p.buffered() <= 1024 * 1024);
}

#[test]
fn endless_short_data_lines_keep_the_event_data_bounded() {
    let mut p = SseParser::with_max_line(16);
    let mut got = Vec::new();
    // Every line is under the line cap, but no blank line ever ends the event.
    for _ in 0..1000 {
        got.extend(p.push(b"data: 0123456789\n"));
        assert!(p.buffered_data() <= 16, "data {}", p.buffered_data());
    }
    // A line that would pass the cap is dropped whole; the event still ends.
    got.extend(p.push(b"\ndata: next\n\n"));
    assert_eq!(got, vec![ev("0123456789"), ev("next")]);
}
