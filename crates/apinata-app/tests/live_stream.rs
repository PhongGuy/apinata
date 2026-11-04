use apinata_app::live_stream::{EVENT_CAP, LiveStream, MAX_LINE, TAIL_LINES};

fn live(event_stream: bool) -> LiveStream {
    LiveStream::new(
        200,
        vec![("content-type".into(), "text/plain".into())],
        event_stream,
    )
}

fn rows(l: &LiveStream) -> Vec<String> {
    (0..l.row_count()).map(|i| l.row(i).unwrap()).collect()
}

#[test]
fn complete_lines_and_the_open_line_are_rows() {
    let mut l = live(false);
    l.push(b"one\ntwo\nthr");
    assert_eq!(rows(&l), ["one", "two", "thr"]);
    l.push(b"ee\n");
    assert_eq!(rows(&l), ["one", "two", "three"]);
    assert_eq!(l.row(3), None);
}

#[test]
fn crlf_ends_a_line_without_leaving_a_cr() {
    let mut l = live(false);
    l.push(b"a\r\nb\r\n");
    assert_eq!(rows(&l), ["a", "b"]);
}

#[test]
fn a_multibyte_char_split_across_pushes_is_whole() {
    let mut l = live(false);
    l.push(b"caf\xC3");
    l.push(b"\xA9\n");
    assert_eq!(rows(&l), ["caf\u{e9}"]);
}

#[test]
fn the_tail_keeps_the_last_lines_and_counts_the_rest() {
    let mut l = live(false);
    for i in 0..2500 {
        l.push(format!("line {i}\n").as_bytes());
    }
    assert_eq!(l.row_count(), TAIL_LINES);
    assert_eq!(l.row(0).unwrap(), "line 500");
    assert_eq!(l.row(TAIL_LINES - 1).unwrap(), "line 2499");
    assert_eq!(l.skipped_lines(), 500);
}

#[test]
fn one_endless_line_is_cut_into_bounded_rows() {
    let mut l = live(false);
    l.push(&vec![b'x'; MAX_LINE * 2 + 5]);
    let lens: Vec<usize> = rows(&l).iter().map(|r| r.len()).collect();
    assert_eq!(lens, [MAX_LINE, MAX_LINE, 5]);
}

#[test]
fn bytes_counts_every_byte_pushed() {
    let mut l = live(false);
    l.push(b"abc");
    l.push(b"\ndef");
    assert_eq!(l.bytes(), 7);
}

#[test]
fn an_event_stream_yields_events_across_split_pushes() {
    let mut l = live(true);
    l.push(b"data: a\n");
    assert!(l.events().is_empty());
    l.push(b"\ndata: b\n\n");
    let data: Vec<&str> = l.events().iter().map(|e| e.data.as_str()).collect();
    assert_eq!(data, ["a", "b"]);
    assert_eq!(rows(&l), ["data: a", "", "data: b", ""]);
}

#[test]
fn a_plain_body_yields_no_events() {
    let mut l = live(false);
    l.push(b"data: a\n\n");
    assert!(l.events().is_empty());
}

#[test]
fn invalid_utf8_is_replaced_lossily() {
    let mut l = live(false);
    l.push(b"a\xFFb\n");
    assert_eq!(rows(&l), ["a\u{FFFD}b"]);
}

#[test]
fn cr_and_lf_split_across_pushes_make_one_line() {
    let mut l = live(false);
    l.push(b"a\r");
    assert_eq!(rows(&l), ["a"]);
    l.push(b"\nb\n");
    assert_eq!(rows(&l), ["a", "b"]);
}

#[test]
fn a_lone_cr_open_line_is_not_a_row() {
    let mut l = live(false);
    l.push(b"\r");
    assert_eq!(l.row_count(), 0);
    assert_eq!(l.row(0), None);
}

#[test]
fn a_max_line_followed_by_crlf_is_one_row() {
    let mut l = live(false);
    l.push(&vec![b'x'; MAX_LINE]);
    l.push(b"\r\nnext\n");
    assert_eq!(rows(&l), ["x".repeat(MAX_LINE), "next".to_string()]);
}

#[test]
fn the_cut_never_splits_a_multibyte_char() {
    let mut l = live(false);
    // The two-byte char straddles the MAX_LINE boundary.
    let mut body = vec![b'x'; MAX_LINE - 1];
    body.extend_from_slice("\u{e9}tail\n".as_bytes());
    l.push(&body);
    let r = rows(&l);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0], "x".repeat(MAX_LINE - 1));
    assert_eq!(r[1], "\u{e9}tail");
}

#[test]
fn events_are_capped_and_the_dropped_ones_counted() {
    let mut l = live(true);
    let total = EVENT_CAP + 5;
    for i in 0..total {
        l.push(format!("data: {i}\n\n").as_bytes());
    }
    assert_eq!(l.events().len(), EVENT_CAP);
    assert_eq!(l.dropped_events(), 5);
    assert_eq!(l.events()[0].data, "5");
    assert_eq!(l.events()[EVENT_CAP - 1].data, (total - 1).to_string());
}
