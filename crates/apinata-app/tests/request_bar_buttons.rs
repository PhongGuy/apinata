//! The request bar's Generate code and Save buttons.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::generate_code::current;
use apinata_app::tabs::TabKind;
use apinata_core::{ItemPath, Outcome};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::time::Duration;
use support::*;
use wiremock::ResponseTemplate;

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn code_text(cx: &mut TestAppContext) -> Option<String> {
    cx.update(|cx| current(cx).map(|v| v.read(cx).text().to_owned()))
}

fn new_draft(cx: &mut TestAppContext, s: &Started) -> ItemPath {
    s.ws.update(cx, |w, cx| w.new_draft(&s.root, ItemPath::root(), cx));
    frame(cx, s.win);
    s.ws.read_with(cx, |w, _| match w.tabs().active().map(|t| t.kind.clone()) {
        Some(TabKind::Request(item)) if item.is_draft() => item,
        other => panic!("a draft tab is active: {other:?}"),
    })
}

#[gpui_kit::test]
fn generate_code_opens_the_dialog_for_the_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    click(cx, s.win, "generate-code");
    let text = code_text(cx).expect("the Generate code dialog is open");
    assert!(text.contains("http://x/0"), "{text}");
}

#[gpui_kit::test]
fn a_draft_generates_code_too(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let item = new_draft(cx, &s);
    let edit = Command::Edit {
        target: Target::Request(item),
        edit: Edit::Url("http://127.0.0.1:9/d".into()),
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, edit, cx));
    click(cx, s.win, "generate-code");
    let text = code_text(cx).expect("the Generate code dialog is open");
    assert!(text.contains("http://127.0.0.1:9/d"), "{text}");
}

#[gpui_kit::test]
fn generate_code_stays_while_a_send_is_in_flight(cx: &mut TestAppContext) {
    http_ready(cx);
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_secs(30));
    let server = mock_server(cx, slow);
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, _| {
        let send = w.find("send-slot").bounds();
        let code = w.find("generate-code").bounds();
        assert!(send.right() <= code.left(), "send {send:?} code {code:?}");
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    assert!(present(cx, win, "cancel"));
    assert!(present(cx, win, "generate-code"));
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    cx.update_window(win, |_, w, cx| w.click("cancel", cx))
        .unwrap();
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    assert_eq!(finished(cx, &model).outcome, Outcome::Cancelled);
}

#[gpui_kit::test]
fn save_shows_only_while_dirty_and_saves(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert!(!shown(cx, win, "save-request"));
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://127.0.0.1:9/changed"), cx)
    });
    assert!(shown(cx, win, "save-request"));
    cx.update_window(win, |_, w, _| {
        let url = w.find("url").bounds();
        let save = w.find("save-request").bounds();
        let send = w.find("send-slot").bounds();
        assert!(url.right() <= save.left() && save.right() <= send.left());
        let gap = apinata_app::style::at_zoom(apinata_app::style::SPACE_2, 1.0);
        assert_eq!(
            send.left() - save.right(),
            gap,
            "save {save:?} send {send:?}"
        );
    })
    .unwrap();
    click(cx, win, "save-request");
    assert!(!view_of(cx, &ws, &root, "a.yml").dirty);
    let disk = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert!(disk.contains("http://127.0.0.1:9/changed"), "{disk}");
    assert!(!shown(cx, win, "save-request"));
}

#[gpui_kit::test]
fn autosave_hides_save(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started_with(cx, &col, |p| {
        let mut settings = test_settings();
        settings.autosave = true;
        apinata_app::persist::save_json(&p.settings, &settings).unwrap();
    });
    open_tab(cx, &s.ws, &s.root, "a.yml");
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://127.0.0.1:9/changed"), cx)
    });
    assert!(!shown(cx, s.win, "save-request"));
    assert!(
        view_of(cx, &s.ws, &s.root, "a.yml").dirty,
        "still dirty: the autosave delay has not passed"
    );
}

#[gpui_kit::test]
fn a_never_edited_draft_shows_no_save(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    new_draft(cx, &s);
    assert!(present(cx, s.win, "generate-code"));
    assert!(!shown(cx, s.win, "save-request"));
}

#[gpui_kit::test]
fn save_on_an_edited_draft_asks_for_a_name(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let item = new_draft(cx, &s);
    let edit = Command::Edit {
        target: Target::Request(item),
        edit: Edit::Url("http://127.0.0.1:9/d".into()),
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, edit, cx));
    assert!(shown(cx, s.win, "save-request"));
    click(cx, s.win, "save-request");
    assert!(present(cx, s.win, "save-draft-dialog"));
}

#[gpui_kit::test]
fn autosave_keeps_save_on_an_edited_draft(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started_with(cx, &col, |p| {
        let mut settings = test_settings();
        settings.autosave = true;
        apinata_app::persist::save_json(&p.settings, &settings).unwrap();
    });
    let item = new_draft(cx, &s);
    let edit = Command::Edit {
        target: Target::Request(item),
        edit: Edit::Url("http://127.0.0.1:9/d".into()),
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, edit, cx));
    assert!(shown(cx, s.win, "save-request"));
}

#[gpui_kit::test]
fn generate_codes_focus_ring_clears_the_column_edge(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    // The kit paints a keyboard ring 3px outside a button; the pane clips at the column edge.
    let ring = gpui_kit::px(3.);
    cx.update_window(s.win, |_, w, _| {
        let column = w.find("request-column").bounds();
        let code = w.find("generate-code").bounds();
        assert!(
            code.right() + ring <= column.right(),
            "code {code:?} leaves no ring room in {column:?}"
        );
    })
    .unwrap();
}

// In a column of about 370 px the dirty Save, Send and Generate code wrap below the URL,
// which takes the rest of its line and is never narrower than the method.
#[gpui_kit::test]
fn the_button_group_wraps_before_the_url_gets_unusable(cx: &mut TestAppContext) {
    use apinata_app::style::{SPACE_2, at_zoom};
    use gpui_kit::{VisualTestContext, px, size};
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://127.0.0.1:9/changed"), cx)
    });
    assert!(shown(cx, win, "save-request"));
    for width in [370., 400., 340.] {
        VisualTestContext::from_window(win, cx).simulate_resize(size(px(width), px(600.)));
        frame(cx, win);
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let url = w.find("url").bounds();
            let method = w.find("method").bounds();
            let column = w.find("request-column").bounds();
            let save = w.find("save-request").bounds();
            let gap = at_zoom(SPACE_2, 1.0);
            assert!(
                url.size.width >= method.size.width && url.right() >= column.right() - gap,
                "{width}: url {url:?} takes the rest of its line in {column:?}"
            );
            assert!(
                save.top() >= url.bottom(),
                "{width}: save {save:?} under url {url:?}"
            );
        })
        .unwrap();
    }
}

// The kit ring is a fixed 3 px while the rem inset shrinks with zoom, so below 75% the ring
// would reach past the column.
#[gpui_kit::test]
fn generate_codes_ring_stays_inside_the_column_at_low_zoom(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    for zoom in [0.67_f32, 0.5] {
        cx.update(|cx| {
            let st = cx.global::<AppState>().settings.clone();
            st.update(cx, |p, cx| p.update(cx, |p| p.zoom = zoom));
        });
        cx.run_until_parked();
        frame(cx, s.win);
        let ring = gpui_kit::px(3.);
        cx.update_window(s.win, |_, w, _| {
            let column = w.find("request-column").bounds();
            let code = w.find("generate-code").bounds();
            assert!(
                code.right() + ring <= column.right(),
                "{zoom}: code {code:?} leaves no ring room in {column:?}"
            );
        })
        .unwrap();
    }
}

// How many Save buttons assistive technology would be told about.
fn save_buttons(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        gpui_kit::base::test_support::snapshots(w)
            .into_iter()
            .filter(|e| e.role() == Some(gpui_kit::Role::Button) && e.label() == Some("Save"))
            .count()
    })
    .unwrap()
}

#[gpui_kit::test]
fn save_appearing_keeps_the_url_field_in_place(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let url_bounds = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("url").bounds()
        })
        .unwrap()
    };
    assert!(!shown(cx, win, "save-request"));
    assert!(shown(cx, win, "save-slot"));
    assert_eq!(
        save_buttons(cx, win),
        0,
        "the hidden slot is no Save button"
    );
    let clean = url_bounds(cx);
    ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://127.0.0.1:9/changed"), cx)
    });
    assert!(shown(cx, win, "save-request"));
    assert_eq!(save_buttons(cx, win), 1);
    assert_eq!(
        url_bounds(cx),
        clean,
        "the URL field moved when Save appeared"
    );
}

#[gpui_kit::test]
fn autosave_reserves_no_slot_for_a_save_that_never_shows(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started_with(cx, &col, |p| {
        let mut settings = test_settings();
        settings.autosave = true;
        apinata_app::persist::save_json(&p.settings, &settings).unwrap();
    });
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    assert!(present(cx, s.win, "send-slot"));
    assert!(
        !shown(cx, s.win, "save-slot"),
        "a blank gap sits before Send"
    );
    assert!(!shown(cx, s.win, "save-request"));
}
