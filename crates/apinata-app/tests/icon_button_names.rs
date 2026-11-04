//! Every icon-only button has an accessible name, and `icon_button` refuses an empty tooltip.

use crate::support;

use apinata_app::body_view::BodyView;
use apinata_app::settings_section::SettingsSection;
use apinata_app::ui;
use apinata_core::RequestSettings;
use gpui_kit::assets::IconName;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

const ENV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n";

// The ids of visible buttons in `win` whose accessible name is empty.
fn nameless(cx: &mut TestAppContext, win: AnyWindowHandle) -> Vec<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let mut out: Vec<String> = gpui_kit::base::test_support::snapshots(w)
            .into_iter()
            .filter(|e| e.visible() && e.role() == Some(Role::Button))
            // The kit editor draws its gutter fold chevrons and the app cannot reach them.
            .filter(|e| !format!("{:?}", e.path().last().unwrap()).contains("fold-icon"))
            .filter(|e| e.label().is_none_or(|l| l.trim().is_empty()))
            .map(|e| format!("{:?}", e.path().last().unwrap()))
            .collect();
        out.sort();
        out
    })
    .unwrap()
}

fn name_of(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> String {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

#[test]
#[should_panic(expected = "tooltip")]
fn an_icon_button_without_a_tooltip_is_refused() {
    let _ = ui::icon_button("nameless", IconName::Close, "", None);
}

#[gpui_kit::test]
fn the_main_window_has_no_nameless_buttons(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    open_tab(cx, &s.ws, &s.root, "b.yml");
    frame(cx, s.win);
    assert_eq!(nameless(cx, s.win), Vec::<String>::new());
    assert_eq!(name_of(cx, s.win, ("tab-close", 0usize)), "Close tab");
}

#[gpui_kit::test]
fn the_body_bar_copy_button_is_named(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(r#"{"a":1}"#, "application/json"),
        "",
    );
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(name_of(cx, win, "body-copy-button"), "Copy body");
    assert_eq!(nameless(cx, win), Vec::<String>::new());
}

#[gpui_kit::test]
fn tag_remove_is_named(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, SettingsSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let tags = vec!["a".to_owned()];
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.load(&RequestSettings::new(), w, cx);
            s.load_tags(&tags, cx);
        })
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(name_of(cx, win, ("tag-remove", 0usize)), "Remove tag");
    assert_eq!(nameless(cx, win), Vec::<String>::new());
}

#[gpui_kit::test]
fn env_reveal_is_named(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", ENV)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    assert_eq!(name_of(cx, win, ("env-eye", 1usize)), "Show value");
    assert_eq!(nameless(cx, win), Vec::<String>::new());
}
