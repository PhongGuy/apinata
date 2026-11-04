//! History rows: unsaved drafts by method and URL, runner sends tagged.

use crate::support;

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use apinata_app::history_palette::{RUN_TAG, UNSAVED_TAG, history_entries};
use apinata_app::last_response::{Limits, ResponseStore, SendExtra};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

const KEY: &str = "untitled-17600000000000000001.draft";
const BARE: &str = "untitled-17600000000000000002.draft";
const SNAP: &str = r#"{"info":{"name":"Kept draft","type":"http","seq":1},"http":{"method":"POST","url":"http://127.0.0.1:9/kept"}}"#;

fn seeded(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let root = col.path().to_path_buf();
    started_with(cx, col, move |paths| {
        let store = ResponseStore::new(paths.responses.clone());
        let save = |root: &Path, item: &str, url: &str, ms: u64, extra: SendExtra| {
            let response = Response::new(200).with_text_body("k").with_url(url);
            let result = RequestResult::from_parts(
                ip(item),
                "x",
                Outcome::Passed,
                Some(response),
                Vec::new(),
            );
            let at = UNIX_EPOCH + Duration::from_millis(ms);
            store
                .save_send(root, &ip(item), &result, &[], at, Limits::default(), &extra)
                .unwrap()
        };
        let draft = || SendExtra {
            request: Some(SNAP.into()),
            origin: None,
        };
        let run = SendExtra {
            request: None,
            origin: Some("run".into()),
        };
        save(
            &root,
            "b.yml",
            "http://127.0.0.1:9/b",
            1_000,
            SendExtra::default(),
        );
        save(&root, "a.yml", "http://127.0.0.1:9/a", 2_000, run);
        save(&root, BARE, "", 2_500, draft());
        save(&root, KEY, "http://127.0.0.1:9/kept", 3_000, draft());
        save(
            Path::new("/elsewhere"),
            KEY,
            "http://127.0.0.1:9/z",
            4_000,
            draft(),
        );
    })
}

#[gpui_kit::test]
fn draft_entries_of_open_collections_list_by_method_and_url(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let mut recent = store.recent(&[s.root.clone(), PathBuf::from("/elsewhere")], 100);
    for r in recent.iter_mut().filter(|r| r.item.is_draft()) {
        r.send.method = "POST".into();
    }
    let got: Vec<(String, String, String, Option<&str>)> =
        s.ws.read_with(cx, |w, cx| history_entries(&recent, w, cx))
            .into_iter()
            .map(|e| (e.path.as_str().to_owned(), e.label, e.target, e.tag))
            .collect();
    let row = |p: &str, l: &str, t: &str, tag| (p.to_owned(), l.to_owned(), t.to_owned(), tag);
    let want = vec![
        row(KEY, "POST http://127.0.0.1:9/kept", "", Some(UNSAVED_TAG)),
        row(BARE, "Untitled request", "", Some(UNSAVED_TAG)),
        row("a.yml", "a", "http://127.0.0.1:9/a", Some(RUN_TAG)),
        row("b.yml", "b", "http://127.0.0.1:9/b", None),
    ];
    assert_eq!(got, want);
}

#[gpui_kit::test]
fn the_palette_rows_show_their_tags(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("History", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        present(cx, s.win, "history-palette")
    });
    let tag = |cx: &mut TestAppContext, r: usize| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(("history-tag", r))
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap()
    };
    let tags: Vec<Option<String>> = (0..4).map(|r| tag(cx, r)).collect();
    let s = |t: &str| Some(t.to_owned());
    assert_eq!(tags, [s(UNSAVED_TAG), s(UNSAVED_TAG), s(RUN_TAG), None]);
}

#[gpui_kit::test]
fn the_tags_share_one_column(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = seeded(cx, &col);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("History", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    wait_real(cx, |cx| {
        frame(cx, s.win);
        present(cx, s.win, "history-palette")
    });
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        // Rows 0 and 1 are drafts with no target, row 2 a runner send with one.
        let tags: Vec<_> = (0..3usize)
            .map(|r| w.find(("history-tag", r)).bounds())
            .collect();
        assert!(
            tags.iter().all(|t| t.left() == tags[0].left()),
            "the tags start in one column: {tags:?}"
        );
        // Every row keeps the fixed slot, tagged or not, and each tag fits inside it.
        let slots: Vec<_> = (0..4usize)
            .map(|r| w.find(("history-tag-slot", r)).bounds())
            .collect();
        assert!(
            slots.iter().all(|s| s.left() == slots[0].left()),
            "{slots:?}"
        );
        for (tag, slot) in tags.iter().zip(&slots) {
            assert!(
                tag.right() <= slot.right(),
                "tag {tag:?} overflows {slot:?}"
            );
        }
    })
    .unwrap();
}
