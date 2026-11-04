use crate::support;

use apinata_app::send_model::SendState;
use apinata_app::tabs::{TabKind, Tabs};
use gpui_kit::TestAppContext;
use std::path::Path;
use support::*;

const USERS: &str = "info:\n  name: Users\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x/users\nexamples:\n  - name: ok\n    request:\n      url: http://x/users\n      method: GET\n    response:\n      status: 200\n      statusText: OK\n      body:\n        type: json\n        data: '{\"a\": 1}'\n  - name: missing\n    description: no such user\n    request:\n      url: http://x/users/{{id}}\n      method: GET\n      headers:\n        - name: Authorization\n          value: Bearer {{token}}\n    response:\n      status: 404\n      statusText: Not Found\n";

#[test]
fn deleting_an_example_closes_its_tab_and_shifts_later_ones() {
    let root = Path::new("/c");
    let mut tabs = Tabs::new();
    let ex = |index| TabKind::Example {
        item: ip("users.yml"),
        index,
    };
    let other = TabKind::Example {
        item: ip("other.yml"),
        index: 2,
    };
    for kind in [ex(0), ex(1), ex(2), other.clone()] {
        tabs.open(root, kind, false);
    }
    tabs.forget_example(root, &ip("users.yml"), 1);
    let kinds: Vec<TabKind> = tabs.entries().iter().map(|e| e.kind.clone()).collect();
    assert_eq!(kinds, [ex(0), ex(1), other]);
    assert_eq!(
        tabs.pop_closed(),
        None,
        "a deleted example's tab is not offered for reopen"
    );
}

#[gpui_kit::test]
fn an_example_opens_read_only_and_follows_a_delete(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("users.yml", USERS)]);
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| {
            w.open_example(&s.root, ip("users.yml"), 1, false, cx)
        })
        .expect("an example tab");
    frame(cx, s.win);
    let kind =
        s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().kind.clone());
    assert_eq!(
        kind,
        TabKind::Example {
            item: ip("users.yml"),
            index: 1
        }
    );
    for id in [
        "example-view",
        "example-method",
        "example-url",
        "example-headers",
        "response-status",
    ] {
        assert!(present(cx, s.win, id), "{id}");
    }
    assert!(
        !present(cx, s.win, "response-save-example"),
        "an example cannot be saved again"
    );
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let view = area
        .read_with(cx, |a, _| a.example_view(tab).cloned())
        .expect("the view");
    let send = view.read_with(cx, |v, _| v.send_model().clone());
    let status = send.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => r.response.as_ref().map(|r| r.status()),
        _ => None,
    });
    assert_eq!(status, Some(404));
    assert!(
        s.ws.update(cx, |w, cx| w.open_example(
            &s.root,
            ip("users.yml"),
            7,
            false,
            cx
        ))
        .is_none()
    );

    s.ws.update(cx, |w, cx| {
        w.delete_example(&s.root, ip("users.yml"), 0, cx)
    });
    wait_for(cx, "the delete", |_| {
        !std::fs::read_to_string(col.path().join("users.yml"))
            .unwrap()
            .contains("- name: ok\n")
    });
    frame(cx, s.win);
    let kind =
        s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().kind.clone());
    assert_eq!(
        kind,
        TabKind::Example {
            item: ip("users.yml"),
            index: 0
        },
        "same example, new index"
    );
    s.ws.update(cx, |w, cx| {
        w.delete_example(&s.root, ip("users.yml"), 0, cx)
    });
    wait_for(cx, "the second delete", |_| {
        !std::fs::read_to_string(col.path().join("users.yml"))
            .unwrap()
            .contains("examples")
    });
    frame(cx, s.win);
    assert!(s.ws.read_with(cx, |w, _| w.tabs().entries().is_empty()));
}

#[gpui_kit::test]
fn an_unsaved_request_edit_keeps_the_shown_example(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("users.yml", USERS)]);
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| {
            w.open_example(&s.root, ip("users.yml"), 0, false, cx)
        })
        .expect("an example tab");
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let view = area
        .read_with(cx, |a, _| a.example_view(tab).cloned())
        .expect("the view");
    let send = view.read_with(cx, |v, _| v.send_model().clone());
    let shown = |cx: &mut TestAppContext| {
        send.read_with(cx, |m, _| match m.state() {
            SendState::Done(r) => Some(r.clone()),
            _ => None,
        })
        .expect("a shown example")
    };
    let before = shown(cx);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("users.yml", "http://x/edited"), cx)
    });
    wait_for(cx, "the edit", |cx| {
        url(&ws_snap(cx, &s.ws, &s.root), "users.yml") == "http://x/edited"
    });
    frame(cx, s.win);
    assert!(
        std::sync::Arc::ptr_eq(&before, &shown(cx)),
        "an edit to the request leaves its example's response as shown"
    );
}

#[gpui_kit::test]
fn the_example_pane_shows_only_what_an_example_saves(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("users.yml", USERS)]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.open_example(&s.root, ip("users.yml"), 0, false, cx)
    })
    .expect("an example tab");
    frame(cx, s.win);
    for id in ["response-body", "response-size", "body-pretty"] {
        assert!(present(cx, s.win, id), "{id}");
    }
    assert!(present(cx, s.win, ("response-tab", 0usize)), "Body");
    assert!(present(cx, s.win, ("response-tab", 1usize)), "Headers");
    for ix in 2usize..6 {
        assert!(
            !present(cx, s.win, ("response-tab", ix)),
            "no Cookies, Timeline, Tests or Console tab ({ix})"
        );
    }
    assert!(
        !present(cx, s.win, "response-time"),
        "no elapsed time is saved"
    );
    assert!(!present(cx, s.win, "body-edit"), "the body is read-only");
    assert!(!present(cx, s.win, "body-view"));
}

const OTHER: &str =
    "info:\n  name: Other\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: http://x/other\n";

// An example opened mid-session sits right after its request tab and the examples of that
// request already beside it, not at the end of the strip.
#[gpui_kit::test]
fn an_example_opened_mid_session_follows_its_request_tab(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users.yml", USERS),
        ("other.yml", OTHER),
    ]);
    let s = started(cx, &col);
    for item in ["users.yml", "other.yml"] {
        open_tab(cx, &s.ws, &s.root, item);
    }
    for index in [0, 1] {
        s.ws.update(cx, |w, cx| {
            w.open_example(&s.root, ip("users.yml"), index, false, cx)
        })
        .expect("an example tab");
    }
    let kinds: Vec<TabKind> = s.ws.read_with(cx, |w, _| {
        w.tabs().entries().iter().map(|e| e.kind.clone()).collect()
    });
    let ex = |index| TabKind::Example {
        item: ip("users.yml"),
        index,
    };
    assert_eq!(
        kinds,
        [
            TabKind::Request(ip("users.yml")),
            ex(0),
            ex(1),
            TabKind::Request(ip("other.yml")),
        ]
    );
    let active =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.kind.clone()));
    assert_eq!(active, Some(ex(1)), "the opened example is active");
}

// Without its request tab open, an example has no anchor and lands at the end as before,
// whether it opens as a permanent tab or in the preview slot.
#[gpui_kit::test]
fn an_example_without_its_request_tab_mid_session_goes_to_the_end(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users.yml", USERS),
        ("other.yml", OTHER),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "other.yml");
    for (index, preview) in [(0, false), (1, true)] {
        s.ws.update(cx, |w, cx| {
            w.open_example(&s.root, ip("users.yml"), index, preview, cx)
        })
        .expect("an example tab");
    }
    let kinds: Vec<TabKind> = s.ws.read_with(cx, |w, _| {
        w.tabs().entries().iter().map(|e| e.kind.clone()).collect()
    });
    let ex = |index| TabKind::Example {
        item: ip("users.yml"),
        index,
    };
    assert_eq!(kinds, [TabKind::Request(ip("other.yml")), ex(0), ex(1)]);
}

fn kinds(cx: &mut TestAppContext, s: &Started) -> Vec<TabKind> {
    s.ws.read_with(cx, |w, _| {
        w.tabs().entries().iter().map(|e| e.kind.clone()).collect()
    })
}

// A preview example that takes over a preview slot left of its request tab still moves to
// sit right after that request.
#[gpui_kit::test]
fn a_preview_example_in_a_distant_preview_slot_moves_after_its_request(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users.yml", USERS),
        ("other.yml", OTHER),
    ]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("other.yml"), true, cx))
        .expect("a preview tab");
    open_tab(cx, &s.ws, &s.root, "users.yml");
    s.ws.update(cx, |w, cx| {
        w.open_example(&s.root, ip("users.yml"), 0, true, cx)
    })
    .expect("an example tab");
    let ex0 = TabKind::Example {
        item: ip("users.yml"),
        index: 0,
    };
    assert_eq!(kinds(cx, &s), [TabKind::Request(ip("users.yml")), ex0]);
}

// Opening an example that is already open only activates it, so a tab the user dragged away
// from its request stays where it was dropped.
#[gpui_kit::test]
fn reactivating_a_dragged_example_keeps_its_place(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users.yml", USERS),
        ("other.yml", OTHER),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "users.yml");
    let other = open_tab(cx, &s.ws, &s.root, "other.yml");
    let ex =
        s.ws.update(cx, |w, cx| {
            w.open_example(&s.root, ip("users.yml"), 0, false, cx)
        })
        .expect("an example tab");
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.move_after(ex, other)));
    let dragged = kinds(cx, &s);
    s.ws.update(cx, |w, cx| w.open_item(&s.root, ip("users.yml"), false, cx));
    s.ws.update(cx, |w, cx| {
        w.open_example(&s.root, ip("users.yml"), 0, false, cx)
    })
    .expect("the open example");
    assert_eq!(kinds(cx, &s), dragged);
    assert_eq!(
        dragged.last(),
        Some(&TabKind::Example {
            item: ip("users.yml"),
            index: 0
        })
    );
    let active = s.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.id));
    assert_eq!(active, Some(ex));
}

#[gpui_kit::test]
fn a_short_example_tab_scrolls_to_its_response(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, InputEvent as _, ScrollDelta, ScrollWheelEvent, VisualTestContext, point,
        px, size,
    };
    let col = write_tree(&[("opencollection.yml", ROOT), ("users.yml", USERS)]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.open_example(&s.root, ip("users.yml"), 1, false, cx)
    })
    .expect("an example tab");
    VisualTestContext::from_window(s.win, cx).dispatch_action(apinata_app::keymap::ToggleConsole);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(360.)));
    frame(cx, s.win);
    // The wheel goes over the title, outside the request block's own scroll area.
    let title = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let view = w.find("example-view").bounds();
            let dock = w.find("console-dock").bounds();
            assert!(
                view.bottom() <= dock.top(),
                "view {view:?} past the dock {dock:?}"
            );
            view.origin + point(px(40.), px(20.))
        })
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_event(
            ScrollWheelEvent {
                position: title,
                delta: ScrollDelta::Lines(point(0., -1000.)),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let view = w.find("example-view").bounds();
        let status = w.find("response-status").bounds();
        assert!(
            status.bottom() <= view.bottom() && status.top() >= view.top(),
            "response status {status:?} outside the view {view:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_roomy_example_tab_gives_its_response_the_spare_height(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, VisualTestContext, px, size};
    let col = write_tree(&[("opencollection.yml", ROOT), ("users.yml", USERS)]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.open_example(&s.root, ip("users.yml"), 0, false, cx)
    })
    .expect("an example tab");
    VisualTestContext::from_window(s.win, cx).dispatch_action(apinata_app::keymap::ToggleConsole);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1280.), px(800.)));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let view = w.find("example-view").bounds();
        let request = w.find("example-body").bounds();
        let status = w.find("response-status").bounds();
        let body = w.find("response-body").bounds();
        // With room to spare, the response sits right under the request and its body takes
        // the rest of the view instead of a two-line sliver.
        assert!(
            status.top() - request.bottom() < px(60.),
            "gap from request {request:?} to response {status:?}, body {body:?}"
        );
        assert!(
            body.size.height >= px(200.),
            "body {body:?} in view {view:?}"
        );
        assert!(
            body.bottom() <= view.bottom(),
            "body {body:?} past the view {view:?}"
        );
    })
    .unwrap();
}

/// An example whose request has far more headers and body lines than the capped block shows.
fn long_request_example() -> String {
    let mut text = String::from(
        "info:\n  name: Big\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x/big\nexamples:\n  - name: long\n    request:\n      url: http://x/big\n      method: POST\n      headers:\n",
    );
    for i in 0..40 {
        text.push_str(&format!(
            "        - name: X-H{i:02}\n          value: v{i}\n"
        ));
    }
    let body: Vec<String> = (0..60).map(|i| format!("line{i:02}")).collect();
    text.push_str(&format!(
        "      body:\n        type: text\n        data: \"{}\"\n",
        body.join("\\n")
    ));
    text.push_str("    response:\n      status: 200\n      statusText: OK\n      body:\n        type: json\n        data: '{\"a\": 1}'\n");
    text
}

#[gpui_kit::test]
fn a_long_example_request_scrolls_with_the_wheel(cx: &mut TestAppContext) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{InputEvent as _, ScrollDelta, ScrollWheelEvent, point, px};
    let long = long_request_example();
    let col = write_tree(&[("opencollection.yml", ROOT), ("big.yml", &long)]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.open_example(&s.root, ip("big.yml"), 0, false, cx)
    })
    .expect("an example tab");
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(1280.), px(800.)));
    frame(cx, s.win);
    let (row0, body_before) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find(("example-header", 0usize)).bounds(),
                w.find("example-body").bounds(),
            )
        })
        .unwrap();
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_event(
            ScrollWheelEvent {
                position: row0.center(),
                delta: ScrollDelta::Lines(point(0., -1000.)),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let body = w.find("example-body").bounds();
        assert!(
            body.top() < body_before.top() - px(100.),
            "the wheel did not move the request: body {body_before:?} -> {body:?}"
        );
        let block = w.find("example-request").bounds();
        assert!(
            body.bottom() <= block.bottom() + px(1.) && body.bottom() > block.top(),
            "the body end {body:?} is not brought into the block {block:?}"
        );
    })
    .unwrap();
}
