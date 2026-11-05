//! A find scan asks whether it was superseded once per line and stops as soon as it was.

use std::cell::Cell;

use apinata_app::body_find::{
    FIND_HIT_CAP, FindOptions, Finder, find_in_chunked_until, find_in_chunked_with,
};
use apinata_app::body_text::chunk_lines;

fn finder(query: &str, regex: bool) -> Finder {
    let options = FindOptions {
        regex,
        ..FindOptions::default()
    };
    Finder::new(query, options).unwrap().unwrap()
}

fn text() -> String {
    vec!["{\"id\": 1, \"name\": \"x\"}"; 1_000].join("\n")
}

// A cancel question that answers yes from its `stop`th asking on, counting every asking.
fn stop_at(asked: &Cell<usize>, stop: usize) -> impl Fn() -> bool + '_ {
    move || {
        asked.set(asked.get() + 1);
        asked.get() >= stop
    }
}

#[test]
fn an_editor_scan_stops_on_the_line_it_is_cancelled_at() {
    let asked = Cell::new(0);
    let found = finder("id", false).find_in_text_until(&text(), FIND_HIT_CAP, stop_at(&asked, 4));
    assert_eq!(found, None);
    assert_eq!(asked.get(), 4);
}

#[test]
fn an_editor_scan_never_cancelled_finds_what_the_capped_scan_finds() {
    let (text, finder) = (text(), finder("i.", true));
    let found = finder.find_in_text_until(&text, 50, || false);
    assert_eq!(found, Some(finder.find_in_text_capped(&text, 50)));
}

#[test]
fn a_rows_scan_stops_on_the_line_it_is_cancelled_at() {
    let chunked = chunk_lines(&text(), 8);
    let asked = Cell::new(0);
    let found = find_in_chunked_until(&chunked, &finder("name", false), stop_at(&asked, 3));
    assert_eq!(found, None);
    assert_eq!(asked.get(), 3);
}

#[test]
fn a_rows_scan_never_cancelled_finds_what_the_plain_scan_finds() {
    let (chunked, finder) = (chunk_lines(&text(), 8), finder("name", false));
    let found = find_in_chunked_until(&chunked, &finder, || false).unwrap();
    assert_eq!(found, find_in_chunked_with(&chunked, &finder));
    assert_eq!(found.matches.len(), 1_000);
}
