//! An invalid input carries an alert icon at its trailing edge, and a danger ring while it is
//! focused, so it differs from a valid one by more than hue. Pinned at every site that uses
//! `ui::invalid_input`: the host override rows, the proxy port and the Preferences fields.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::network_section::NetworkSection;
use apinata_app::settings::ProxyMode;
use apinata_core::{CollectionNetwork, CollectionProxy, HostOverride, ProxyConfig};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::path::PathBuf;
use support::*;

fn network_window(cx: &mut TestAppContext, network: CollectionNetwork) -> AnyWindowHandle {
    let (win, host) = host_section(cx, |w, cx| NetworkSection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, false, w, cx))
    })
    .unwrap();
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    win
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn host_rows_show_the_icon_on_an_invalid_input_and_the_ring_only_when_focused(
    cx: &mut TestAppContext,
) {
    let mut network = CollectionNetwork::default();
    network.host_overrides = vec![
        HostOverride::new("bad.test", "nope"),
        HostOverride::new("ok.test", "127.0.0.1"),
    ];
    let win = network_window(cx, network);
    assert!(shown(cx, win, ("host-invalid-icon", 0usize)), "unfocused");
    assert!(!shown(cx, win, ("host-danger-ring", 0usize)), "no ring");
    click(cx, win, ("host-name", 0usize));
    assert!(shown(cx, win, ("host-invalid-icon", 0usize)), "host");
    assert!(shown(cx, win, ("host-danger-ring", 0usize)), "ring stays");
    assert!(
        shown(cx, win, ("address-invalid-icon", 0usize)),
        "invalid too"
    );
    assert!(
        !shown(cx, win, ("address-danger-ring", 0usize)),
        "not focused"
    );
    click(cx, win, ("host-address", 0usize));
    assert!(shown(cx, win, ("address-invalid-icon", 0usize)), "address");
    assert!(shown(cx, win, ("host-invalid-icon", 0usize)), "icon stays");
    assert!(!shown(cx, win, ("host-danger-ring", 0usize)), "ring moved");
    click(cx, win, ("host-name", 1usize));
    assert!(!shown(cx, win, ("host-invalid-icon", 1usize)), "valid");
}

#[gpui_kit::test]
fn the_proxy_port_shows_the_icon_when_refused_and_the_ring_only_when_focused(
    cx: &mut TestAppContext,
) {
    let mut network = CollectionNetwork::default();
    let mut proxy = ProxyConfig::new("http", "proxy.local", "3128");
    proxy.auth = None;
    network.proxy = CollectionProxy::Custom(proxy);
    let win = network_window(cx, network);
    click(cx, win, "proxy-port");
    assert!(!shown(cx, win, "proxy-port-invalid-icon"), "valid");
    type_at_end(cx, win, None, "proxy-port", "x");
    assert!(shown(cx, win, "proxy-port-invalid-icon"), "focused invalid");
    assert!(shown(cx, win, "proxy-port-danger-ring"));
    click(cx, win, "proxy-host");
    assert!(shown(cx, win, "proxy-port-invalid-icon"), "unfocused");
    assert!(!shown(cx, win, "proxy-port-danger-ring"));
}

#[gpui_kit::test]
fn a_preferences_field_shows_the_icon_when_refused_and_the_ring_only_when_focused(
    cx: &mut TestAppContext,
) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    replace_input(cx, s.win, None, "zoom", "150");
    assert!(!shown(cx, s.win, "zoom-invalid-icon"), "focused valid");
    replace_input(cx, s.win, None, "zoom", "250");
    // A refusal is reached on commit, with the field still focused.
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(shown(cx, s.win, "zoom-invalid-icon"), "focused invalid");
    assert!(shown(cx, s.win, "zoom-danger-ring"));
    click(cx, s.win, "editor-font-size");
    assert!(present(cx, s.win, "zoom-error"), "still refused");
    assert!(shown(cx, s.win, "zoom-invalid-icon"), "unfocused");
    assert!(!shown(cx, s.win, "zoom-danger-ring"));
}

fn bounds(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    let id = id.into();
    frame(cx, win);
    cx.update_window(win, |_, w, _| w.find(id).bounds())
        .unwrap()
}

// The icon is the input's suffix inside a fixed-width frame, so marking never moves or
// resizes the input.
#[gpui_kit::test]
fn marking_an_input_does_not_change_its_bounds(cx: &mut TestAppContext) {
    let mut network = CollectionNetwork::default();
    network.host_overrides = vec![
        HostOverride::new("bad.test", "nope"),
        HostOverride::new("ok.test", "127.0.0.1"),
    ];
    let mut proxy = ProxyConfig::new("http", "proxy.local", "3128");
    proxy.auth = None;
    network.proxy = CollectionProxy::Custom(proxy);
    let win = network_window(cx, network);
    let ids: [gpui_kit::ElementId; 3] = [
        ("host-name", 0usize).into(),
        ("host-address", 0usize).into(),
        "proxy-port".into(),
    ];
    // Row 0 and the port start unfocused.
    click(cx, win, ("host-name", 1usize));
    let rest: Vec<_> = ids.iter().map(|i| bounds(cx, win, i.clone())).collect();
    for (focus, icon, ix) in [
        (("host-name", 0usize), ("host-invalid-icon", 0usize), 0),
        (
            ("host-address", 0usize),
            ("address-invalid-icon", 0usize),
            1,
        ),
    ] {
        click(cx, win, focus);
        assert!(shown(cx, win, icon), "marked input {ix}");
        assert_eq!(
            bounds(cx, win, ids[ix].clone()),
            rest[ix],
            "host input {ix}"
        );
    }
    click(cx, win, "proxy-port");
    type_at_end(cx, win, None, "proxy-port", "x");
    assert!(shown(cx, win, "proxy-port-invalid-icon"));
    assert_eq!(bounds(cx, win, "proxy-port"), rest[2], "proxy port");
}

// Every site below follows the same contract: the alert icon marks an invalid input focused or
// not, the danger ring only while it is focused.

fn assert_marked(cx: &mut TestAppContext, win: AnyWindowHandle, ids: (&'static str, &'static str)) {
    assert!(shown(cx, win, ids.0), "ring on {}", ids.0);
    assert!(shown(cx, win, ids.1), "icon on {}", ids.1);
}

fn assert_icon_only(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ids: (&'static str, &'static str),
) {
    assert!(!shown(cx, win, ids.0), "no ring on {}", ids.0);
    assert!(shown(cx, win, ids.1), "icon stays on {}", ids.1);
}

#[gpui_kit::test]
fn the_cookie_form_marks_the_field_its_error_is_about(cx: &mut TestAppContext) {
    use apinata_app::cookie_jar::open_cookie_jar;
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, s.win);
    click(cx, s.win, "jar-add");
    replace_input(cx, s.win, None, "jar-form-domain", ".app.test");
    // The bad value is typed before the name, so the value is not focused when Save refuses.
    replace_input(cx, s.win, None, "jar-form-value", "a;b");
    replace_input(cx, s.win, None, "jar-form-name", "sid");
    click(cx, s.win, "jar-form-save");
    let value = ("jar-form-value-danger-ring", "jar-form-value-invalid-icon");
    assert_icon_only(cx, s.win, value);
    click(cx, s.win, "jar-form-value");
    assert_marked(cx, s.win, value);
    assert!(!shown(cx, s.win, "jar-form-name-invalid-icon"));

    replace_input(cx, s.win, None, "jar-form-value", "v");
    replace_input(cx, s.win, None, "jar-form-expires", "soon");
    click(cx, s.win, "jar-form-save");
    click(cx, s.win, "jar-form-expires");
    assert_marked(
        cx,
        s.win,
        (
            "jar-form-expires-danger-ring",
            "jar-form-expires-invalid-icon",
        ),
    );
    assert!(!shown(cx, s.win, "jar-form-value-invalid-icon"), "moved");

    replace_input(cx, s.win, None, "jar-form-expires", "");
    replace_input(cx, s.win, None, "jar-form-name", "a b");
    click(cx, s.win, "jar-form-save");
    click(cx, s.win, "jar-form-name");
    assert_marked(
        cx,
        s.win,
        ("jar-form-name-danger-ring", "jar-form-name-invalid-icon"),
    );
}

#[gpui_kit::test]
fn the_custom_method_marks_a_refused_verb(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.open_custom_method(w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes(win, "g e space t enter");
    cx.run_until_parked();
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_some()));
    let ids = ("custom-method-danger-ring", "custom-method-invalid-icon");
    assert_marked(cx, win, ids);
    cx.update_window(win, |_, w, cx| w.blur(cx)).unwrap();
    cx.run_until_parked();
    assert_icon_only(cx, win, ids);
}

#[gpui_kit::test]
fn the_body_filter_marks_an_invalid_expression(cx: &mut TestAppContext) {
    use apinata_app::body_view::BodyView;
    use gpui_kit::WindowOptions;
    let s = send_once(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_raw(r#"{"a":1}"#, "application/json"),
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
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter("$.a[1", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.filter_error().is_some()));
    let ids = ("body-filter-danger-ring", "body-filter-invalid-icon");
    assert_icon_only(cx, win, ids);
    click(cx, win, "body-filter-input");
    assert_marked(cx, win, ids);
}

// The cookie form, custom method and body filter mark an input with the same icon frame, so
// the input keeps its bounds with the error present and absent.
#[gpui_kit::test]
fn the_cookie_form_value_keeps_its_bounds_when_marked(cx: &mut TestAppContext) {
    use apinata_app::cookie_jar::open_cookie_jar;
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, s.win);
    click(cx, s.win, "jar-add");
    replace_input(cx, s.win, None, "jar-form-domain", ".app.test");
    replace_input(cx, s.win, None, "jar-form-name", "sid");
    replace_input(cx, s.win, None, "jar-form-value", "a;b");
    let rest = bounds(cx, s.win, "jar-form-value").size;
    click(cx, s.win, "jar-form-save");
    assert!(shown(cx, s.win, "jar-form-value-invalid-icon"), "marked");
    // The error line re-centres the dialog, so only the size is compared.
    assert_eq!(bounds(cx, s.win, "jar-form-value").size, rest);
}

#[gpui_kit::test]
fn the_custom_method_keeps_its_bounds_when_marked(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.open_custom_method(w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes(win, "g e t");
    cx.run_until_parked();
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_none()));
    let rest = bounds(cx, win, "custom-method");
    cx.simulate_keystrokes(win, "space t enter");
    cx.run_until_parked();
    assert!(ed.read_with(cx, |e, _| e.custom_method_error().is_some()));
    assert!(shown(cx, win, "custom-method-invalid-icon"), "marked");
    assert_eq!(bounds(cx, win, "custom-method"), rest);
}

#[gpui_kit::test]
fn the_body_filter_keeps_its_bounds_when_marked(cx: &mut TestAppContext) {
    use apinata_app::body_view::BodyView;
    use gpui_kit::WindowOptions;
    let s = send_once(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_raw(r#"{"a":1}"#, "application/json"),
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
    for filter in ["$.a", "$.a[1"] {
        cx.update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| v.set_filter(filter, w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert!(shown(cx, win, "body-filter-invalid-icon"), "marked");
    let marked = bounds(cx, win, "body-filter-input");
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_filter("$.a", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!shown(cx, win, "body-filter-invalid-icon"), "cleared");
    assert_eq!(bounds(cx, win, "body-filter-input"), marked);
}

// Every Preferences field that can refuse a value is marked, one test per field with a value
// it refuses. Each opens its own app: an earlier refusal's message would push later rows off
// screen, and one window per test keeps every frame cheap.
fn assert_refusal_marked(cx: &mut TestAppContext, id: &'static str, bad: &str) {
    let col = two_requests();
    let s = started(cx, &col);
    // The proxy fields show only in the mode that uses them.
    let mode = match id {
        "global-proxy-port" => ProxyMode::Manual,
        "global-proxy-pac" => ProxyMode::Pac,
        _ => ProxyMode::System,
    };
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |s| s.proxy.mode = mode));
    });
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    reveal(cx, s.win, id);
    replace_input(cx, s.win, None, id, bad);
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, s.win, format!("{id}-error")), "{id} refused");
    let ring = format!("{id}-danger-ring");
    let icon = format!("{id}-invalid-icon");
    assert!(shown(cx, s.win, ring.clone()), "{id} ring");
    assert!(shown(cx, s.win, icon.clone()), "{id} icon focused");
    // Tab moves focus on, which commits the field again and keeps its refusal.
    cx.update_window(s.win, |_, w, cx| w.press("tab", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        present(cx, s.win, format!("{id}-error")),
        "{id} still refused"
    );
    assert!(!shown(cx, s.win, ring), "{id} no ring unfocused");
    assert!(shown(cx, s.win, icon), "{id} icon unfocused");
}

#[gpui_kit::test]
fn a_refused_zoom_is_marked(cx: &mut TestAppContext) {
    assert_refusal_marked(cx, "zoom", "250");
}

#[gpui_kit::test]
fn a_refused_editor_font_size_is_marked(cx: &mut TestAppContext) {
    assert_refusal_marked(cx, "editor-font-size", "x");
}

#[gpui_kit::test]
fn a_refused_response_font_size_is_marked(cx: &mut TestAppContext) {
    assert_refusal_marked(cx, "response-font-size", "x");
}

#[gpui_kit::test]
fn a_refused_body_cap_is_marked(cx: &mut TestAppContext) {
    assert_refusal_marked(cx, "body-cap", "x");
}

#[gpui_kit::test]
fn a_refused_history_limit_is_marked(cx: &mut TestAppContext) {
    assert_refusal_marked(cx, "history-limit", "x");
}

#[gpui_kit::test]
fn a_refused_script_timeout_is_marked(cx: &mut TestAppContext) {
    assert_refusal_marked(cx, "script-timeout", "x");
}

#[gpui_kit::test]
fn a_refused_request_timeout_is_marked(cx: &mut TestAppContext) {
    assert_refusal_marked(cx, "request-timeout", "x");
}

#[gpui_kit::test]
fn a_refused_global_proxy_port_is_marked(cx: &mut TestAppContext) {
    assert_refusal_marked(cx, "global-proxy-port", "x");
}

#[gpui_kit::test]
fn a_refused_global_proxy_pac_is_marked(cx: &mut TestAppContext) {
    assert_refusal_marked(cx, "global-proxy-pac", "ftp://example.com/proxy.pac");
}

// Scrolls the Preferences list until the input `id` is on screen.
fn reveal(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    use gpui_kit::{ScrollDelta, point, px};
    for _ in 0..40 {
        frame(cx, win);
        let visible = cx
            .update_window(win, |_, w, _| {
                // Clear of the bottom fade too: a click on a field under it would miss.
                let clear = w.find("preferences-rows").bounds().bottom()
                    - w.try_find("prefs-scroll-fade")
                        .map_or(px(0.), |f| f.bounds().size.height);
                w.try_find(id)
                    .is_some_and(|t| t.visible() && t.bounds().bottom() <= clear)
            })
            .unwrap();
        if visible {
            return;
        }
        cx.update_window(win, |_, w, cx| {
            w.scroll(
                "preferences-rows",
                ScrollDelta::Pixels(point(px(0.), px(-120.))),
                cx,
            )
        })
        .unwrap();
    }
    panic!("{id} never came on screen");
}

// Painted danger-coloured borders sitting on the input `id` of the host row 0.
fn danger_borders_at(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: (&'static str, usize),
) -> usize {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.find(id).bounds();
        let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
        use gpui_kit::component::ActiveTheme as _;
        let danger = cx.theme().danger;
        w.painted_quads()
            .into_iter()
            .filter(|q| {
                q.border_widths.left.0 > 0.
                    && q.border_color == danger
                    && (q.bounds.origin.x.0 - l).abs() < 6. * scale
                    && (q.bounds.origin.y.0 - t).abs() < 6. * scale
            })
            .count()
    })
    .unwrap()
}

#[gpui_kit::test]
fn an_unfocused_invalid_host_row_has_the_icon_and_no_danger_border(cx: &mut TestAppContext) {
    let mut network = CollectionNetwork::default();
    network.host_overrides = vec![HostOverride::new("bad.test", "nope")];
    let win = network_window(cx, network);
    assert!(shown(cx, win, ("host-invalid-icon", 0usize)), "icon");
    assert_eq!(danger_borders_at(cx, win, ("host-name", 0usize)), 0);
    assert_eq!(danger_borders_at(cx, win, ("host-address", 0usize)), 0);
    click(cx, win, ("host-name", 0usize));
    assert!(
        danger_borders_at(cx, win, ("host-name", 0usize)) > 0,
        "ring"
    );
}
