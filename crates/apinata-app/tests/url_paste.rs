//! Pasting a curl command into the URL bar replaces the draft; other text pastes as text.

use crate::support;

use apinata_app::request_editor::{Pasted, RequestEditor};
use apinata_app::workspace::Workspace;
use apinata_core::import::NoteKind;
use apinata_core::{MemoryStore, SecretKey, SecretStore};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ClipboardItem, Entity, TestAppContext};
use std::path::PathBuf;
use std::sync::Arc;
use support::*;

struct Editor {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<Workspace>,
    root: PathBuf,
    win: AnyWindowHandle,
    editor: Entity<RequestEditor>,
}

fn editor(cx: &mut TestAppContext) -> Editor {
    let col = collection();
    let (state, ws, root) = open_dir(cx, &col);
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    Editor {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        editor,
    }
}

/// Puts `text` on the clipboard and pastes it at the end of the URL with the keyboard.
fn paste(cx: &mut TestAppContext, e: &Editor, text: &str) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
    cx.update_window(e.win, |_, w, cx| {
        w.click("url", cx);
        w.press("end", cx);
        w.press(&secondary("v"), cx);
    })
    .unwrap();
    frame(cx, e.win);
}

fn pasted(cx: &mut TestAppContext, e: &Editor) -> Option<Pasted> {
    e.editor.read_with(cx, |v, _| v.pasted().cloned())
}

#[gpui_kit::test]
fn a_pasted_curl_command_replaces_the_draft(cx: &mut TestAppContext) {
    let e = editor(cx);
    paste(cx, &e, "curl -X POST https://x.test/p -H 'X-A: 1' -k");
    let view = view_of(cx, &e.ws, &e.root, "a.yml");
    assert_eq!(
        (view.method.as_str(), view.url.as_str(), view.name.as_str()),
        ("POST", "https://x.test/p", "a")
    );
    assert!(view.dirty, "the draft stays unsaved");
    assert_eq!(
        e.editor.read_with(cx, |v, cx| v.url(cx)),
        "https://x.test/p"
    );
    match pasted(cx, &e) {
        Some(Pasted::Imported(notes)) => assert!(
            notes.iter().any(|n| n.source == "curl flag -k"),
            "{notes:?}"
        ),
        other => panic!("{other:?}"),
    }
    assert!(present(cx, e.win, "paste-banner"));
}

#[gpui_kit::test]
fn other_text_pastes_as_text(cx: &mut TestAppContext) {
    let e = editor(cx);
    paste(cx, &e, "/more");
    assert_eq!(e.editor.read_with(cx, |v, cx| v.url(cx)), "http://x/0/more");
    assert!(pasted(cx, &e).is_none());
    assert!(!present(cx, e.win, "paste-banner"));
}

#[gpui_kit::test]
fn a_typed_curl_command_is_not_imported(cx: &mut TestAppContext) {
    let e = editor(cx);
    replace_input(cx, e.win, None, "url", "curl https://y.test");
    frame(cx, e.win);
    assert_eq!(
        e.editor.read_with(cx, |v, cx| v.url(cx)),
        "curl https://y.test"
    );
    assert_eq!(view_of(cx, &e.ws, &e.root, "a.yml").method, "GET");
    assert!(pasted(cx, &e).is_none());
}

#[gpui_kit::test]
fn a_broken_curl_command_is_refused_and_reported(cx: &mut TestAppContext) {
    let e = editor(cx);
    paste(cx, &e, "curl 'https://x.test");
    match pasted(cx, &e) {
        Some(Pasted::Failed(message)) => assert!(message.contains("unclosed quote"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(e.editor.read_with(cx, |v, cx| v.url(cx)), "http://x/0");
    assert_eq!(view_of(cx, &e.ws, &e.root, "a.yml").url, "http://x/0");
    assert!(present(cx, e.win, "paste-banner"));
}

#[gpui_kit::test]
fn the_banner_dismisses(cx: &mut TestAppContext) {
    let e = editor(cx);
    paste(cx, &e, "curl https://x.test/p -k");
    cx.update_window(e.win, |_, w, cx| w.click("dismiss-paste-banner", cx))
        .unwrap();
    frame(cx, e.win);
    assert!(pasted(cx, &e).is_none());
    assert!(!present(cx, e.win, "paste-banner"));
}

#[gpui_kit::test]
fn a_curl_paste_keeps_a_graphql_body_and_says_so(cx: &mut TestAppContext) {
    let gql = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: http://x/0\n  body:\n    query: \"{ me }\"\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("g.yml", gql)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, editor) = editor_for(cx, &ws, &root, "g.yml", false);
    let before = view_of(cx, &ws, &root, "g.yml").body.clone();
    let e = Editor {
        _col: col,
        _state,
        ws,
        root,
        win,
        editor,
    };
    paste(cx, &e, r#"curl https://x.test/g --json '{"a":1}'"#);
    let view = view_of(cx, &e.ws, &e.root, "g.yml");
    assert_eq!(view.url, "https://x.test/g");
    assert_eq!(view.body, before);
    match pasted(cx, &e) {
        Some(Pasted::Imported(notes)) => assert!(
            notes
                .iter()
                .any(|n| n.detail.contains("GraphQL request keeps")),
            "{notes:?}"
        ),
        other => panic!("{other:?}"),
    }
}

const BODIED: &str = "info:\n  name: b\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: http://x/0\n  body:\n    type: json\n    data: '{\"keep\":1}'\n";

fn bodied(cx: &mut TestAppContext) -> Editor {
    let col = write_tree(&[("opencollection.yml", ROOT), ("b.yml", BODIED)]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, editor) = editor_for(cx, &ws, &root, "b.yml", false);
    Editor {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        editor,
    }
}

fn answer(cx: &mut TestAppContext, e: &Editor, id: &'static str) {
    assert!(present(cx, e.win, id), "the confirm dialog is open");
    cx.update_window(e.win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
    frame(cx, e.win);
}

#[gpui_kit::test]
fn a_paste_over_a_body_asks_and_cancel_keeps_the_request(cx: &mut TestAppContext) {
    let e = bodied(cx);
    let before = view_of(cx, &e.ws, &e.root, "b.yml");
    paste(cx, &e, "curl https://x.test/p");
    answer(cx, &e, "confirm-cancel");
    let after = view_of(cx, &e.ws, &e.root, "b.yml");
    assert_eq!(
        (after.url.as_str(), &after.body),
        (before.url.as_str(), &before.body)
    );
    assert!(!after.dirty);
    assert!(pasted(cx, &e).is_none());
    assert_eq!(
        e.editor.read_with(cx, |v, cx| v.url(cx)),
        "http://x/0",
        "nothing lands in the URL"
    );
}

#[gpui_kit::test]
fn a_paste_over_a_body_asks_and_replace_applies(cx: &mut TestAppContext) {
    let e = bodied(cx);
    paste(cx, &e, "curl https://x.test/p");
    answer(cx, &e, "confirm-replace");
    let view = view_of(cx, &e.ws, &e.root, "b.yml");
    assert_eq!(
        (view.method.as_str(), view.url.as_str()),
        ("GET", "https://x.test/p")
    );
    assert!(view.dirty);
}

#[gpui_kit::test]
fn a_paste_into_an_unsaved_request_asks(cx: &mut TestAppContext) {
    let e = editor(cx);
    type_at_end(cx, e.win, None, "url", "/x");
    cx.run_until_parked();
    assert!(view_of(cx, &e.ws, &e.root, "a.yml").dirty);
    paste(cx, &e, "curl https://x.test/q");
    answer(cx, &e, "confirm-replace");
    assert_eq!(view_of(cx, &e.ws, &e.root, "a.yml").url, "https://x.test/q");
}

#[gpui_kit::test]
fn a_clean_request_without_a_body_takes_the_paste_without_asking(cx: &mut TestAppContext) {
    let e = editor(cx);
    paste(cx, &e, "curl https://x.test/r");
    assert!(!present(cx, e.win, "confirm-replace"));
    assert_eq!(view_of(cx, &e.ws, &e.root, "a.yml").url, "https://x.test/r");
}

#[gpui_kit::test]
fn the_banner_wraps_inside_a_narrow_request_pane(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let e = editor(cx);
    gpui_kit::VisualTestContext::from_window(e.win, cx).simulate_resize(size(px(360.), px(700.)));
    frame(cx, e.win);
    paste(
        cx,
        &e,
        "curl -X POST https://x.test/a/long/path/that/keeps/going/on -H 'X-A: 1' -k --compressed --retry 3",
    );
    assert!(present(cx, e.win, "paste-banner"));
    cx.update_window(e.win, |_, w, _| {
        let column = w.find("request-column").bounds();
        for id in ["paste-banner", "dismiss-paste-banner"] {
            let b = w.find(id).bounds();
            assert!(
                b.left() >= column.left() - px(0.5) && b.right() <= column.right() + px(0.5),
                "{id} {b:?} leaves {column:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_banner_is_an_outlined_muted_box(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    let e = editor(cx);
    paste(cx, &e, "curl 'https://x.test");
    assert!(outlined(cx, e.win, "paste-banner"));
    let filled = cx
        .update_window(e.win, |_, w, cx| {
            w.render_frame(cx);
            let at = w.find("paste-banner").bounds();
            let scale = w.scale_factor();
            let fill = cx.theme().muted;
            w.painted_quads().into_iter().any(|q| {
                (q.bounds.origin.x.0 / scale - f32::from(at.left())).abs() < 1.
                    && (q.bounds.origin.y.0 / scale - f32::from(at.top())).abs() < 1.
                    && q.background.as_solid() == Some(fill)
            })
        })
        .unwrap();
    assert!(filled, "the banner has the muted fill");
}

// Side by side at 200% the request pane is the narrowest the layout offers; the banner of a
// GraphQL paste must show whole inside it, its dismiss button included.
#[gpui_kit::test]
fn the_banner_fits_a_graphql_request_pane_side_by_side(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    use apinata_app::settings::Orientation;
    use gpui_kit::{px, size};
    let gql = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: http://x/0\n  body:\n    query: \"{ me }\"\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("g.yml", gql)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "g.yml");
    frame(cx, s.win);
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1280.), px(800.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |s| {
                s.response_orientation = Orientation::Horizontal;
                s.zoom = 2.0;
            })
        });
    });
    cx.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
    cx.write_to_clipboard(ClipboardItem::new_string(
        "curl -X POST https://x.test/a/long/path -H 'X-A: 1' -k --compressed --retry 3 --json '{\"a\":1}'"
            .to_owned(),
    ));
    cx.update_window(s.win, |_, w, cx| {
        w.click("url", cx);
        w.press("end", cx);
        w.press(&secondary("v"), cx);
    })
    .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(present(cx, s.win, "paste-banner"));
    cx.update_window(s.win, |_, w, _| {
        let column = w.find("request-column").bounds();
        let pane = w.find("tab-area").bounds();
        for id in ["paste-banner", "dismiss-paste-banner"] {
            let b = w.find(id).bounds();
            assert!(
                b.left() >= column.left() - px(0.5) && b.right() <= column.right() + px(0.5),
                "{id} {b:?} leaves the request column {column:?}"
            );
            assert!(
                b.right() <= pane.right() + px(0.5) && b.bottom() <= pane.bottom() + px(0.5),
                "{id} {b:?} leaves the tab area {pane:?}"
            );
        }
    })
    .unwrap();
}

const AWS: &str = "curl --aws-sigv4 aws:amz:r:s --user AKID:TOPSECRET https://x.test/b";

fn aws_editor(cx: &mut TestAppContext, envs: &[(&str, &str)]) -> (Editor, Arc<MemoryStore>) {
    let mut files = vec![
        ("opencollection.yml", ROOT.to_owned()),
        ("a.yml", request("a", "http://x/0")),
    ];
    files.extend(envs.iter().map(|(p, t)| (*p, (*t).to_owned())));
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let col = write_tree(&refs);
    let store = Arc::new(MemoryStore::new());
    let (state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let e = Editor {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        editor,
    };
    (e, store)
}

#[gpui_kit::test]
fn a_stored_aws_secret_shows_in_the_banner_as_stored(cx: &mut TestAppContext) {
    let (e, store) = aws_editor(cx, &[("environments/dev.yml", "name: dev\n")]);
    paste(cx, &e, AWS);
    cx.run_until_parked();
    let key = SecretKey::new(
        std::fs::canonicalize(&e.root).unwrap(),
        "dev",
        "awsSecretAccessKey",
    );
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("TOPSECRET"));
    let Some(Pasted::Imported(notes)) = pasted(cx, &e) else {
        panic!("no banner")
    };
    assert!(
        notes.iter().all(|n| n.kind != NoteKind::NeedsInput),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|n| n.kind == NoteKind::Converted && n.detail.contains("dev")),
        "{notes:?}"
    );
}

#[gpui_kit::test]
fn an_aws_secret_without_an_environment_still_asks_for_input(cx: &mut TestAppContext) {
    let (e, store) = aws_editor(cx, &[]);
    paste(cx, &e, AWS);
    cx.run_until_parked();
    let Some(Pasted::Imported(notes)) = pasted(cx, &e) else {
        panic!("no banner")
    };
    let need = notes.iter().find(|n| n.kind == NoteKind::NeedsInput);
    let need = need.unwrap_or_else(|| panic!("{notes:?}"));
    assert!(need.detail.contains("no environment"), "{}", need.detail);
    assert!(!format!("{notes:?}").contains("TOPSECRET"));
    drop(store);
}

fn paste_twice_at_once(cx: &mut TestAppContext, e: &Editor, second: Option<&str>) {
    cx.update_window(e.win, |_, w, cx| {
        e.editor.update(cx, |v, cx| {
            assert!(v.paste_url(AWS, w, cx));
            match second {
                Some(text) => assert!(v.paste_url(text, w, cx)),
                None => v.dismiss_pasted(cx),
            }
        })
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_late_secret_reply_does_not_bring_back_a_dismissed_banner(cx: &mut TestAppContext) {
    let (e, _store) = aws_editor(cx, &[("environments/dev.yml", "name: dev\n")]);
    paste_twice_at_once(cx, &e, None);
    assert!(pasted(cx, &e).is_none());
}

#[gpui_kit::test]
fn a_late_secret_reply_does_not_replace_a_newer_paste(cx: &mut TestAppContext) {
    let (e, _store) = aws_editor(cx, &[("environments/dev.yml", "name: dev\n")]);
    paste_twice_at_once(cx, &e, Some("curl https://y.test/z"));
    assert!(pasted(cx, &e).is_none(), "{:?}", pasted(cx, &e));
}
