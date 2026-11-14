//! A header the request sets by hand that its auth replaces when sent is named in a notice
//! above the Headers table.

use crate::support;

use apinata_app::request_editor::Section;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

const NOTICE: &str = "headers-auth-replaced";

/// A request with one hand-set header `name`, switched on or off, under `auth`, the YAML
/// written after `auth:`.
fn file(name: &str, enabled: bool, auth: &str) -> String {
    let off = if enabled {
        ""
    } else {
        "      disabled: true\n"
    };
    format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0\"\n  headers:\n    - name: {name}\n      value: Bearer by-hand\n{off}  auth:{auth}"
    )
}

// The notice's text over the Headers section of `a.yml` in a collection whose root is `root`.
fn notice(cx: &mut TestAppContext, root: &str, a: &str) -> Option<String> {
    let col = write_tree(&[("opencollection.yml", root), ("a.yml", a)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, view) = editor_for(cx, &ws, &root, "a.yml", false);
    view.update(cx, |v, cx| v.show(Section::Headers, cx));
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(NOTICE)
            .map(|n| n.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

const BASIC: &str = "\n    type: basic\n    username: u\n    password: p\n";

#[gpui_kit::test]
fn basic_auth_names_the_hand_set_authorization_it_replaces(cx: &mut TestAppContext) {
    assert_eq!(
        notice(cx, ROOT, &file("Authorization", true, BASIC)).as_deref(),
        Some("Auth sets its own Authorization header, so the one here is not sent.")
    );
}

#[gpui_kit::test]
fn an_inherited_auth_names_the_header_too(cx: &mut TestAppContext) {
    let root = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  auth:\n    type: bearer\n    token: t\n";
    assert_eq!(
        notice(cx, root, &file("authorization", true, " inherit\n")).as_deref(),
        Some("Auth sets its own authorization header, so the one here is not sent.")
    );
}

#[gpui_kit::test]
fn an_api_key_names_its_own_header(cx: &mut TestAppContext) {
    let auth = "\n    type: apikey\n    key: X-Key\n    value: v\n    placement: header\n";
    assert_eq!(
        notice(cx, ROOT, &file("X-Key", true, auth)).as_deref(),
        Some("Auth sets its own X-Key header, so the one here is not sent.")
    );
}

#[gpui_kit::test]
fn no_notice_without_a_clash(cx: &mut TestAppContext) {
    // Inherit with nothing to inherit sends the hand-set header as written.
    assert_eq!(
        notice(cx, ROOT, &file("Authorization", true, " inherit\n")),
        None
    );
    assert_eq!(notice(cx, ROOT, &file("Authorization", false, BASIC)), None);
    assert_eq!(notice(cx, ROOT, &file("X-Other", true, BASIC)), None);
}
