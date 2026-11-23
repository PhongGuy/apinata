use crate::support;

use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn active_name(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    s.ws.read_with(cx, |w, _| {
        (w.tabs().active())
            .and_then(|e| e.item())
            .map(|p| p.as_str().to_owned())
    })
}

// A press and release of the mouse's Back or Forward side button over the window.
fn side_button(cx: &mut TestAppContext, s: &Started, dir: gpui_kit::NavigationDirection) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent, point, px};
    let at = point(px(400.), px(300.));
    let button = MouseButton::Navigate(dir);
    let down = MouseDownEvent {
        button,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    };
    let up = MouseUpEvent {
        button,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
    };
    for event in [down.to_platform_input(), up.to_platform_input()] {
        cx.update_window(s.win, |_, w, cx| {
            w.dispatch_event(event, cx);
        })
        .unwrap();
        frame(cx, s.win);
    }
}

fn go_key(key: &str) -> String {
    match cfg!(target_os = "macos") {
        true => format!("ctrl-{key}"),
        false => format!("ctrl-alt-{key}"),
    }
}

#[gpui_kit::test]
fn the_back_button_reopens_a_replaced_preview_and_the_keys_step_both_ways(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
    ]);
    let s = started(cx, &col);
    for rel in ["a.yml", "b.yml"] {
        s.ws.update(cx, |w, cx| w.open_item(&s.root, ip(rel), true, cx));
    }
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws), ["b.yml"]);
    side_button(cx, &s, gpui_kit::NavigationDirection::Back);
    assert_eq!(active_name(cx, &s).as_deref(), Some("a.yml"));
    assert_eq!(tab_names(cx, &s.ws).len(), 2);
    cx.simulate_keystrokes(s.win, &go_key("_"));
    frame(cx, s.win);
    assert_eq!(active_name(cx, &s).as_deref(), Some("b.yml"));
    cx.simulate_keystrokes(s.win, &go_key("-"));
    frame(cx, s.win);
    assert_eq!(active_name(cx, &s).as_deref(), Some("a.yml"));
    side_button(cx, &s, gpui_kit::NavigationDirection::Forward);
    assert_eq!(active_name(cx, &s).as_deref(), Some("b.yml"));
}
