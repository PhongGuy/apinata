//! The app-wide session log: one ring shared by every executor the app builds.

use crate::support;

use apinata_app::log_feed;
use apinata_app::log_feed::{
    Feed, SOURCE_FILTERS, Sequenced, clock_label, console_keeps, network_keeps, omitted_label,
    source_label, status_label,
};
use apinata_core::{ConsoleLevel, SendSource};
use gpui_kit::TestAppContext;
use std::sync::Arc;
use support::*;
use wiremock::ResponseTemplate;

#[gpui_kit::test]
fn there_is_no_log_before_the_first_install(cx: &mut TestAppContext) {
    init(cx);
    assert!(cx.update(|cx| log_feed::shared(cx)).is_none());
}

#[gpui_kit::test]
fn the_shared_and_schema_executors_record_into_one_log(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("ok"));
    let log = cx
        .update(|cx| log_feed::shared(cx))
        .expect("install sets the log");
    let shared = cx.update(|cx| apinata_app::executor::shared(cx)).unwrap();
    let schema = cx.update(|cx| apinata_app::executor::schema(cx)).unwrap();
    send_direct(cx, &shared, &server.uri());
    send_direct(cx, &schema, &server.uri());
    let got = log.network_since(0);
    assert_eq!(got.entries.len(), 2);
    for e in &got.entries {
        assert_eq!(e.source, SendSource::Request);
        assert_eq!(e.status, Some(200));
    }
}

#[gpui_kit::test]
fn a_reinstall_keeps_the_same_log(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let before = cx.update(|cx| log_feed::shared(cx)).unwrap();
    let first = cx.update(|cx| apinata_app::executor::shared(cx)).unwrap();
    send_direct(cx, &first, &server.uri());
    let store = apinata_core::MemoryStore::new();
    cx.update(|cx| apinata_app::executor::install(&test_settings(), &store, cx))
        .unwrap();
    let second = cx.update(|cx| apinata_app::executor::shared(cx)).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&first, &second), "a new executor");
    send_direct(cx, &second, &server.uri());
    let after = cx.update(|cx| log_feed::shared(cx)).unwrap();
    assert_eq!(before.network_since(0).entries.len(), 2);
    assert_eq!(after.network_since(0).entries.len(), 2, "one ring");
}

#[gpui_kit::test]
fn an_app_send_logs_its_exchange_and_its_console_lines(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"console.log('hello from a')\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    finished(cx, &model);
    let log = cx.update(|cx| log_feed::shared(cx)).unwrap();
    let net = log.network_since(0);
    assert_eq!(net.entries.len(), 1);
    assert_eq!(net.entries[0].source, SendSource::Request);
    let console = log.console_since(0);
    assert_eq!(console.entries.len(), 1);
    assert_eq!(console.entries[0].request, "a");
    assert_eq!(console.entries[0].level, ConsoleLevel::Log);
    assert_eq!(console.entries[0].text, "hello from a");
}

struct E(u64);
impl Sequenced for E {
    fn seq(&self) -> u64 {
        self.0
    }
}

fn arcs(seqs: &[u64]) -> Vec<Arc<E>> {
    seqs.iter().map(|&n| Arc::new(E(n))).collect()
}

fn seqs(f: &Feed<E>) -> Vec<u64> {
    f.entries().iter().map(|e| e.0).collect()
}

#[test]
fn a_feed_appends_newer_entries_and_reports_movement() {
    let mut f = Feed::<E>::default();
    assert_eq!(f.next(), 0);
    assert!(f.absorb(arcs(&[0, 2]), 3, 0));
    assert_eq!(seqs(&f), [0, 2]);
    assert_eq!(f.next(), 3);
    assert!(!f.absorb(Vec::new(), 3, 0), "nothing moved");
}

#[test]
fn a_feed_follows_next_even_when_the_other_kind_took_seqs() {
    let mut f = Feed::<E>::default();
    f.absorb(arcs(&[0]), 1, 0);
    // Seqs 1 and 2 went to the other kind; only `next` moved.
    assert!(f.absorb(Vec::new(), 3, 0));
    assert_eq!(f.next(), 3);
    f.absorb(arcs(&[3]), 4, 0);
    assert_eq!(seqs(&f), [0, 3]);
}

#[test]
fn a_feed_ignores_entries_it_already_holds() {
    let mut f = Feed::<E>::default();
    f.absorb(arcs(&[0, 1]), 2, 0);
    f.absorb(arcs(&[1, 2]), 3, 0);
    assert_eq!(seqs(&f), [0, 1, 2]);
}

#[test]
fn a_feed_drops_entries_below_first() {
    let mut f = Feed::<E>::default();
    f.absorb(arcs(&[0, 1, 2]), 3, 0);
    assert!(f.absorb(Vec::new(), 3, 2), "eviction moved first");
    assert_eq!(seqs(&f), [2]);
}

#[test]
fn a_clear_empties_the_feed_and_keeps_the_cursor() {
    let mut f = Feed::<E>::default();
    f.absorb(arcs(&[0, 1]), 2, 0);
    // After a clear the log reports first == next.
    assert!(f.absorb(Vec::new(), 2, 2));
    assert!(f.entries().is_empty());
    assert_eq!(f.next(), 2);
    f.absorb(arcs(&[2]), 3, 2);
    assert_eq!(seqs(&f), [2]);
}

#[test]
fn the_console_filter_matches_level_and_search() {
    use ConsoleLevel::*;
    assert!(console_keeps(Log, "a", "Hello", None, ""));
    assert!(console_keeps(Log, "a", "Hello", None, "   "));
    assert!(console_keeps(Warn, "a", "x", Some(Warn), ""));
    assert!(!console_keeps(Log, "a", "x", Some(Warn), ""));
    assert!(console_keeps(Log, "a", "Hello", None, "hELLo"));
    assert!(
        console_keeps(Log, "Get users", "x", None, "USERS"),
        "matches the request"
    );
    assert!(!console_keeps(Log, "a", "Hello", None, "nope"));
    assert!(!console_keeps(Log, "a", "Hello", Some(Error), "hello"));
}

#[test]
fn the_source_filter_keeps_its_source_or_all() {
    assert!(network_keeps(SendSource::OAuth2, None));
    assert!(network_keeps(SendSource::OAuth2, Some(SendSource::OAuth2)));
    assert!(!network_keeps(
        SendSource::Request,
        Some(SendSource::Script)
    ));
    let labels: Vec<_> = SOURCE_FILTERS.iter().map(|f| source_label(*f)).collect();
    assert_eq!(
        labels,
        [
            "All",
            "Request",
            "Runner",
            "Script",
            "OAuth2",
            "GraphQL schema"
        ]
    );
}

#[test]
fn status_and_clock_labels() {
    assert_eq!(status_label(Some(200)), "200");
    assert_eq!(status_label(None), "error");
    let t = clock_label(std::time::SystemTime::now());
    let b = t.as_bytes();
    assert_eq!(t.len(), 12, "{t}");
    assert_eq!((b[2], b[5], b[8]), (b':', b':', b'.'), "{t}");
    assert!(t.bytes().filter(|c| c.is_ascii_digit()).count() == 9, "{t}");
}

#[test]
fn the_omitted_note_agrees_with_its_count() {
    assert_eq!(omitted_label(1, "line"), "1 earlier line not shown");
    assert_eq!(omitted_label(3, "request"), "3 earlier requests not shown");
}
