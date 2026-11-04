use crate::support;

use apinata_app::executor;
use apinata_app::response_pane::{CookiesView, ResponsePane, ResponseTab, cookie_matches};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

#[test]
fn hosts_match_exact_and_dot_suffix_domains() {
    assert!(cookie_matches("api.example.com", "api.example.com", true));
    assert!(cookie_matches("API.example.com", "api.example.com", true));
    assert!(cookie_matches("api.example.com", "example.com", false));
    assert!(!cookie_matches("api.example.com", "example.com", true));
    assert!(!cookie_matches("badexample.com", "example.com", false));
    assert!(!cookie_matches("localhost", "127.0.0.1", true));
}

#[gpui_kit::test]
fn the_tab_lists_the_hosts_cookies_and_deletes_one(cx: &mut TestAppContext) {
    // The same server answers as 127.0.0.1 and as localhost, so the jar holds one cookie per host.
    let s = send_once(
        cx,
        ResponseTemplate::new(200).insert_header("set-cookie", "c=1; Path=/"),
        "",
    );
    let port = s.server.uri().rsplit(':').next().unwrap().to_owned();
    let other = format!("http://localhost:{port}/ok");
    s.ws.update(cx, |w, cx| w.send(&s.root, edit_url("a.yml", &other), cx));
    let root = s.root.clone();
    s.model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &s.model);
    let jar = cx.update(|cx| executor::shared(cx).unwrap());
    assert_eq!(jar.cookies().list().len(), 2);

    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    let rows = view.read_with(cx, |p, _| p.cookie_rows());
    assert_eq!(
        rows,
        [("c".into(), "1".into(), "localhost".into(), "/".into())]
    );

    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("cookie-delete", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |p, _| p.cookie_rows()).is_empty());
    assert!(present(cx, win, "cookies-none"));
    let left: Vec<_> = jar.cookies().list().into_iter().map(|c| c.domain).collect();
    assert_eq!(left, ["127.0.0.1"]);
}

#[test]
fn expiry_reads_as_a_coarse_countdown() {
    use apinata_app::response_pane::expires_label;
    use std::time::{Duration, SystemTime};
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    assert_eq!(expires_label(None, now), "Session");
    assert_eq!(
        expires_label(Some(now - Duration::from_secs(1)), now),
        "Expired"
    );
    assert_eq!(
        expires_label(Some(now + Duration::from_secs(45)), now),
        "in 45 s"
    );
    assert_eq!(
        expires_label(Some(now + Duration::from_secs(5400)), now),
        "in 1 h"
    );
    assert_eq!(
        expires_label(Some(now + Duration::from_secs(3 * 86_400 + 5)), now),
        "in 3 d"
    );
    assert_eq!(
        expires_label(Some(now + Duration::from_secs(600)), now),
        "in 10 min"
    );
}

#[gpui_kit::test]
fn the_cookie_table_has_headed_columns(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200)
            .insert_header("set-cookie", "c=1; Path=/; Max-Age=5400; HttpOnly"),
        "",
    );
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    let cells = view.read_with(cx, |p, _| p.cookie_table(std::time::SystemTime::now()));
    assert_eq!(cells.len(), 1);
    assert_eq!(
        (
            cells[0].expires.as_str(),
            cells[0].http_only,
            cells[0].secure
        ),
        ("in 1 h", true, false)
    );
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let head = w.find(("cookies-head-cell", 4usize)).bounds();
        let cell = w.find(("cookie-expires", 0usize)).bounds();
        assert_eq!(head.left(), cell.left());
    })
    .unwrap();
}

// Counts the pane's redraw requests while the test clock moves `secs` seconds.
fn redraws_over(
    cx: &mut TestAppContext,
    view: &gpui_kit::Entity<ResponsePane>,
    secs: u64,
) -> usize {
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = count.clone();
    let _watch = cx.update(|cx| cx.observe(view, move |_, _| seen.set(seen.get() + 1)));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(secs));
    cx.run_until_parked();
    count.get()
}

fn cookie_pane(cx: &mut TestAppContext, set_cookie: &str) -> gpui_kit::Entity<ResponsePane> {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).insert_header("set-cookie", set_cookie),
        "",
    );
    support::init(cx);
    let model = s.model.clone();
    let (_win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view
}

#[gpui_kit::test]
fn an_expiring_cookie_redraws_its_countdown(cx: &mut TestAppContext) {
    let view = cookie_pane(cx, "c=1; Path=/; Max-Age=90");
    view.update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    cx.run_until_parked();
    assert!(
        redraws_over(cx, &view, 1) >= 1,
        "the countdown never redraws"
    );
}

#[gpui_kit::test]
fn session_cookies_and_hidden_tabs_do_not_tick(cx: &mut TestAppContext) {
    let session = cookie_pane(cx, "c=1; Path=/");
    session.update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    cx.run_until_parked();
    assert_eq!(redraws_over(cx, &session, 5), 0);
    let hidden = cookie_pane(cx, "d=1; Path=/; Max-Age=90");
    hidden.update(cx, |p, cx| p.set_tab(ResponseTab::Body, cx));
    cx.run_until_parked();
    assert_eq!(redraws_over(cx, &hidden, 5), 0);
}

#[gpui_kit::test]
fn cookie_flags_read_yes_or_no_and_delete_waits_for_hover(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).insert_header("set-cookie", "c=1; Path=/; HttpOnly"),
        "",
    );
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find(("cookie-http-only", 0usize)).label(), Some("Yes"));
        assert_eq!(w.find(("cookie-secure", 0usize)).label(), Some("No"));
        w.hover("cookies-head", cx);
        w.render_frame(cx);
        assert!(
            !w.within(("cookie", 0usize))
                .within(("kv-delete-slot", 0usize))
                .find("kv-action")
                .visible()
        );
        w.hover(("cookie", 0usize), cx);
        w.render_frame(cx);
        assert!(
            w.within(("cookie", 0usize))
                .within(("kv-delete-slot", 0usize))
                .find("kv-action")
                .visible()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_focused_cookie_delete_shows_without_hover(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).insert_header("set-cookie", "c=1; Path=/"),
        "",
    );
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    frame(cx, win);
    let focused = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("cookie-delete", 0usize)).focused()
        })
        .unwrap()
    };
    let mut presses = 0;
    while focused(cx) != Some(true) {
        presses += 1;
        assert!(presses < 40, "Tab never reached the cookie delete");
        // The pane is not under the kit's Root here, so step focus as its Tab binding does.
        cx.update_window(win, |_, w, cx| w.focus_next(cx)).unwrap();
    }
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        assert!(
            w.within(("cookie", 0usize))
                .within(("kv-delete-slot", 0usize))
                .find("kv-action")
                .visible()
        );
    })
    .unwrap();
}

// Guard: a press in the pane focuses the pane, so deleting a cookie never leaves the window
// without focus; the keys of the editor hosting the pane keep working.
#[gpui_kit::test]
fn guard_deleting_a_cookie_leaves_something_focused(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200)
            .append_header("set-cookie", "a=1; Path=/")
            .append_header("set-cookie", "b=2; Path=/"),
        "",
    );
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    frame(cx, win);
    assert_eq!(view.read_with(cx, |p, _| p.cookie_rows().len()), 2);
    cx.update_window(win, |_, w, cx| w.click(("cookie-delete", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |p, _| p.cookie_rows().len()), 1);
    let focused = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.focused(cx).is_some()
        })
        .unwrap();
    assert!(focused, "something has focus after the delete");
}

// Sends `a.yml` to a server whose `/ok` on 127.0.0.1 sets `a=1` and redirects to localhost,
// whose `/next` sets `b=2` plus a cookie for a foreign domain; opens a pane on the result.
fn redirect_pane(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<ResponsePane>,
    u16,
    Sent,
) {
    redirect_pane_to(cx, "/next")
}

// A sent request whose first answer redirects to `next_path`, with the server's port.
fn redirect_sent(cx: &mut TestAppContext, next_path: &str) -> (u16, Sent) {
    use wiremock::matchers::path;
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    let port = server.address().port();
    let next = format!("http://localhost:{port}{next_path}");
    handle.block_on(
        wiremock::Mock::given(path("/ok"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("set-cookie", "a=1; Path=/")
                    .insert_header("location", next.as_str()),
            )
            .mount(&server),
    );
    handle.block_on(
        wiremock::Mock::given(path(next_path))
            .respond_with(
                ResponseTemplate::new(200)
                    .append_header("set-cookie", "b=2; Path=/")
                    .append_header("set-cookie", "d=4; Domain=other.example"),
            )
            .mount(&server),
    );
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    let sent = Sent {
        server,
        col,
        state,
        ws,
        root,
        model,
        result,
    };
    (port, sent)
}

// `redirect_pane` with the second hop at `next` instead of `/next`.
fn redirect_pane_to(
    cx: &mut TestAppContext,
    next_path: &str,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<ResponsePane>,
    u16,
    Sent,
) {
    let (port, sent) = redirect_sent(cx, next_path);
    support::init(cx);
    let m = sent.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Cookies, cx));
    frame(cx, win);
    (win, view, port, sent)
}

// The text of each "This response" cell in row order: (name, value, hop, result).
fn response_cells(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    rows: usize,
) -> Vec<[String; 4]> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (0..rows)
            .map(|ix| {
                [
                    "set-cookie-name",
                    "set-cookie-value",
                    "set-cookie-hop",
                    "set-cookie-result",
                ]
                .map(|id| w.find((id, ix)).label().unwrap_or_default().to_owned())
            })
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn this_response_lists_every_hops_cookies_with_hop_and_result(cx: &mut TestAppContext) {
    let (win, view, port, _s) = redirect_pane(cx);
    assert_eq!(
        view.read_with(cx, |p, _| p.cookies_view()),
        CookiesView::Response
    );
    assert!(present(cx, win, "cookies-view-response"));
    assert!(present(cx, win, "cookies-view-jar"));
    let row = |n: &str, hop: &str, result: &str| [n, "[redacted]", hop, result].map(str::to_owned);
    assert_eq!(
        response_cells(cx, win, 3),
        [
            row("a", &format!("127.0.0.1:{port}/ok"), "Stored"),
            row("b", &format!("localhost:{port}/next"), "Stored"),
            row(
                "d",
                &format!("localhost:{port}/next"),
                "Domain does not match host"
            ),
        ]
    );
    let count = cx.update(|cx| view.read(cx).tab_count(ResponseTab::Cookies, cx));
    assert_eq!(count, Some(3));

    // The reveal is per row, so revealing `b` leaves `a` masked.
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("set-cookie-reveal", 1usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let cells = response_cells(cx, win, 2);
    assert_eq!(
        (cells[0][1].as_str(), cells[1][1].as_str()),
        ("[redacted]", "2")
    );
}

#[gpui_kit::test]
fn the_view_toggles_to_the_jar_and_survives_a_new_result(cx: &mut TestAppContext) {
    let (win, view, _port, s) = redirect_pane(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("cookies-view-jar", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |p, _| p.cookies_view()),
        CookiesView::Jar
    );
    // The jar view lists the final host's cookies, as before.
    let jar = cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(("cookie-name", 0usize)).label().map(str::to_owned)
    });
    assert_eq!(jar.unwrap().as_deref(), Some("b"));
    assert!(!present(cx, win, ("set-cookie-name", 0usize)));
    let count = cx.update(|cx| view.read(cx).tab_count(ResponseTab::Cookies, cx));
    assert_eq!(count, Some(1));

    let root = s.root.clone();
    s.model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &s.model);
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |p, _| p.cookies_view()),
        CookiesView::Jar
    );

    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("cookies-view-response", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |p, _| p.cookies_view()),
        CookiesView::Response
    );
    assert!(present(cx, win, ("set-cookie-name", 0usize)));
}

#[gpui_kit::test]
fn a_response_without_cookies_says_so(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Cookies, cx));
    frame(cx, win);
    let text = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("set-cookies-none").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(text.as_deref(), Some("This response set no cookies."));
    assert!(present(cx, win, "cookies-view-jar"));
}

const SECRET: &str = "s3cr3t-value-123";

// A send whose path holds the secret variable `token`; the server sets one cookie.
fn secret_path_pane(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    (wiremock::MockServer, tempfile::TempDir, Started),
) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).insert_header("set-cookie", "c=1; Path=/"),
    );
    let a = request("a", &format!("{}/ok/{{{{token}}}}", server.uri()));
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "token"), SECRET).unwrap();
    let s = started_on(cx, &col, |_| {}, std::sync::Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model.clone(), w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Cookies, cx));
    frame(cx, win);
    (win, (server, col, s))
}

#[gpui_kit::test]
fn a_secret_in_the_hop_url_never_shows_in_the_hop_column(cx: &mut TestAppContext) {
    let (win, _keep) = secret_path_pane(cx);
    let hop = response_cells(cx, win, 1)[0][2].clone();
    assert!(!hop.contains(SECRET) && hop.contains("/ok/"), "{hop}");
    assert!(
        !hop.contains("%5B"),
        "the mask is not percent-encoded: {hop}"
    );
    cx.update_window(win, |_, w, cx| w.click("cookies-copy", cx))
        .unwrap();
    let copied = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
    assert!(
        copied.contains("c=") && !copied.contains(SECRET),
        "{copied}"
    );
    // The filter reads the masked label, so the secret matches no row.
    replace_input(cx, win, None, "cookies-filter", SECRET);
    frame(cx, win);
    assert!(present(cx, win, "set-cookies-no-match"));
    assert!(!present(cx, win, ("set-cookie-name", 0usize)));
}

// A path long enough that its hop never fits the Hop column.
const LONG_NEXT: &str = "/next/a-rather-long-path-segment/that-keeps-going/well-past-the-column";

// A path whose hop is cut even when Hop takes the free width at 1280 px.
const VERY_LONG_NEXT: &str = "/next/a-rather-long-path-segment/that-keeps-going/well-past-the-column/and-on-past-the-free-width/of-a-1280-px-window/too";

// A path that makes a 60-character hop: wider than a 240 px Hop, narrower than the free width.
const MID_NEXT: &str = "/next/a-forty-five-character-path-for-the-hop";

// The width `text` takes in the kv view's mono font at the window's text size.
fn mono_width(w: &mut gpui_kit::Window, cx: &mut gpui_kit::App, text: &str) -> gpui_kit::Pixels {
    use gpui_kit::component::ActiveTheme as _;
    let mut style = w.text_style();
    style.font_family = cx.theme().mono_font_family.clone();
    let run = style.to_run(text.len());
    let size = style.font_size.to_pixels(w.rem_size());
    w.text_system()
        .shape_line(text.to_owned().into(), size, &[run], None)
        .width
}

fn cookies_at_1280(cx: &mut TestAppContext, next: &str) -> (gpui_kit::AnyWindowHandle, Sent) {
    use gpui_kit::{VisualTestContext, px, size};
    let (win, _view, _port, s) = redirect_pane_to(cx, next);
    VisualTestContext::from_window(win, cx).simulate_resize(size(px(1280.), px(800.)));
    frame(cx, win);
    (win, s)
}

// The longest result phrase fits its column whole at a common window width.
#[gpui_kit::test]
fn a_rejection_reason_is_not_cut_at_1280_px(cx: &mut TestAppContext) {
    let (win, _s) = cookies_at_1280(cx, "/next");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let cell = w.find(("set-cookie-result", 2usize));
        let text = cell.label().unwrap_or_default().to_owned();
        assert_eq!(text, "Domain does not match host");
        let room = cell.bounds().size.width;
        let longest = ["Domain does not match host", "Rejected by the cookie jar"]
            .map(|t| mono_width(w, cx, t));
        for need in longest {
            assert!(
                need <= room,
                "the result needs {need:?}, its column is {room:?}"
            );
        }
    })
    .unwrap();
}

// The tooltip shown over the hop cell of row `ix` after a hover, if any.
fn hop_tip(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, ix: usize) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(("set-cookie-hop", ix), cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("kv-cut-tooltip")
            .map(|t| t.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

#[gpui_kit::test]
fn hovering_a_cut_hop_shows_it_in_full(cx: &mut TestAppContext) {
    let (win, s) = cookies_at_1280(cx, VERY_LONG_NEXT);
    let port = s.server.address().port();
    let full = format!("localhost:{port}{VERY_LONG_NEXT}");
    assert_eq!(response_cells(cx, win, 2)[1][2], full);
    assert_eq!(hop_tip(cx, win, 1).as_deref(), Some(full.as_str()));
}

#[gpui_kit::test]
fn a_hop_that_fits_shows_no_tooltip(cx: &mut TestAppContext) {
    let (win, _s) = cookies_at_1280(cx, LONG_NEXT);
    assert_eq!(hop_tip(cx, win, 0), None);
}

// Bounds of the "This response" head cell `col` and of the same column's cell in row `ix`.
fn column_bounds(
    w: &mut gpui_kit::Window,
    col: usize,
    ix: usize,
) -> (
    gpui_kit::Bounds<gpui_kit::Pixels>,
    gpui_kit::Bounds<gpui_kit::Pixels>,
) {
    let cell = [
        "set-cookie-name",
        "set-cookie-value",
        "set-cookie-hop",
        "set-cookie-result",
    ][col];
    let head = w.find(("set-cookies-head-cell", col)).bounds();
    (head, w.find((cell, ix)).bounds())
}

// Every head cell starts where its column's row cell starts and is as wide.
fn heads_line_up(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, layout: &str) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for col in 0..4 {
            let (head, cell) = column_bounds(w, col, 1);
            assert_eq!(
                (head.origin.x, head.size.width),
                (cell.origin.x, cell.size.width),
                "head cell {col} lines up with its column in the {layout} layout"
            );
        }
    })
    .unwrap();
}

// (Value cell width, Hop cell width) of row `ix` in a fresh frame.
fn value_and_hop_widths(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    ix: usize,
) -> (gpui_kit::Pixels, gpui_kit::Pixels) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (
            w.find(("set-cookie-value", ix)).bounds().size.width,
            w.find(("set-cookie-hop", ix)).bounds().size.width,
        )
    })
    .unwrap()
}

// With every value masked, Value is just wide enough for the mask and Hop takes the free width.
#[gpui_kit::test]
fn a_long_hop_is_not_cut_while_values_are_masked(cx: &mut TestAppContext) {
    let (win, s) = cookies_at_1280(cx, MID_NEXT);
    let port = s.server.address().port();
    let full = format!("localhost:{port}{MID_NEXT}");
    assert_eq!(full.chars().count(), 60);
    assert_eq!(response_cells(cx, win, 2)[1][2], full);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let need = mono_width(w, cx, &full);
        let room = w.find(("set-cookie-hop", 1usize)).bounds().size.width;
        assert!(
            need <= room,
            "the hop needs {need:?}, its column is {room:?}"
        );
        let mask = mono_width(w, cx, "[redacted]");
        let text = w
            .within(("set-cookie-value", 1usize))
            .find("kv-text")
            .bounds()
            .size
            .width;
        assert!(mask <= text, "the mask needs {mask:?}, it has {text:?}");
    })
    .unwrap();
    assert_eq!(hop_tip(cx, win, 1), None);
    heads_line_up(cx, win, "masked");
}

// Resizes the window to `width` and moves the pointer onto the hop of row 1 in the frame the
// resize drew, with no frame in between, as a pointer already over the table would land. Returns
// the tooltip, if any, and whether the hop's text is wider than its cell in that frame.
fn hop_tip_right_after_resize(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    full: &str,
    width: f32,
) -> (Option<String>, bool) {
    use gpui_kit::{VisualTestContext, px, size};
    VisualTestContext::from_window(win, cx).simulate_resize(size(px(width), px(800.)));
    let cut = cx
        .update_window(win, |_, w, cx| {
            let cell = w.find(("set-cookie-hop", 1usize)).bounds();
            w.simulate_mouse_move(cell.center(), cx);
            mono_width(w, cx, full) > cell.size.width
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    let tip = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("kv-cut-tooltip")
                .map(|t| t.label().unwrap_or_default().to_owned())
        })
        .unwrap();
    (tip, cut)
}

// A hop that fits at 1280 px is cut once the window shrinks; hovering it right away shows it.
#[gpui_kit::test]
fn a_hop_cut_by_a_resize_tips_on_the_first_hover(cx: &mut TestAppContext) {
    let (win, s) = cookies_at_1280(cx, MID_NEXT);
    let full = format!("localhost:{}{MID_NEXT}", s.server.address().port());
    let (tip, cut) = hop_tip_right_after_resize(cx, win, &full, 800.);
    assert!(cut, "the hop is cut at 800 px");
    assert_eq!(tip.as_deref(), Some(full.as_str()));
}

// The reverse: a hop cut at 800 px fits once the window widens, so hovering it shows nothing.
#[gpui_kit::test]
fn a_hop_freed_by_a_resize_does_not_tip_on_the_first_hover(cx: &mut TestAppContext) {
    use gpui_kit::{VisualTestContext, px, size};
    let (win, _view, port, _s) = redirect_pane_to(cx, MID_NEXT);
    VisualTestContext::from_window(win, cx).simulate_resize(size(px(800.), px(800.)));
    frame(cx, win);
    frame(cx, win);
    let full = format!("localhost:{port}{MID_NEXT}");
    let (tip, cut) = hop_tip_right_after_resize(cx, win, &full, 1280.);
    assert!(!cut, "the hop fits at 1280 px");
    assert_eq!(tip, None);
}

// Revealing a value gives Value its width back and Hop its 240 px; hiding it restores both.
#[gpui_kit::test]
fn revealing_a_value_switches_the_cookie_columns_and_hiding_switches_back(cx: &mut TestAppContext) {
    let (win, s) = cookies_at_1280(cx, MID_NEXT);
    let port = s.server.address().port();
    let full = format!("localhost:{port}{MID_NEXT}");
    let (masked_value, masked_hop) = value_and_hop_widths(cx, win, 1);
    let click = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.click((id, 1usize), cx)
        })
        .unwrap();
        cx.run_until_parked();
        frame(cx, win);
    };
    click(cx, "set-cookie-reveal");
    assert_eq!(response_cells(cx, win, 2)[1][1], "2");
    let (value, hop) = value_and_hop_widths(cx, win, 1);
    let fixed = cx
        .update_window(win, |_, w, _| {
            apinata_app::style::SET_COOKIE_FIXED_WIDTHS[1].to_pixels(w.rem_size())
        })
        .unwrap();
    assert!(
        value > masked_value,
        "revealed Value {value:?} is wider than masked {masked_value:?}"
    );
    assert_eq!(hop, fixed, "Hop is back to its fixed width");
    assert_eq!(hop_tip(cx, win, 1).as_deref(), Some(full.as_str()));
    heads_line_up(cx, win, "revealed");
    click(cx, "set-cookie-hide");
    assert_eq!(
        value_and_hop_widths(cx, win, 1),
        (masked_value, masked_hop),
        "hiding the value restores the masked layout"
    );
    heads_line_up(cx, win, "masked");
}

#[gpui_kit::test]
fn a_script_cookie_row_reads_its_phase_script_with_its_verdict(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).insert_header("set-cookie", "a=1; Path=/"),
    );
    let code = r#"await bru.cookies.add({ key: \"s\", value: \"2\" }); await bru.cookies.add({ key: \"e\", value: \"3\", domain: \"evil.example\" });"#;
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: after-response\n      code: \"{code}\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    finished(cx, &model);
    let port = server.address().port();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Cookies, cx));
    frame(cx, win);
    let row = |n: &str, hop: &str, result: &str| [n, "[redacted]", hop, result].map(str::to_owned);
    assert_eq!(
        response_cells(cx, win, 3),
        [
            row("a", &format!("127.0.0.1:{port}/ok"), "Stored"),
            row("s", "post-response script", "Stored"),
            row("e", "post-response script", "Domain does not match host"),
        ]
    );
    let count = cx.update(|cx| view.read(cx).tab_count(ResponseTab::Cookies, cx));
    assert_eq!(count, Some(3));
}

// The text of each row of the error view's script cookie list: (name, from, result).
fn error_cookie_cells(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    rows: usize,
) -> Vec<[String; 3]> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (0..rows)
            .map(|ix| {
                [
                    "error-cookie-name",
                    "error-cookie-from",
                    "error-cookie-result",
                ]
                .map(|id| w.find((id, ix)).label().unwrap_or_default().to_owned())
            })
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_failed_send_lists_its_script_cookies_under_the_error_without_values(cx: &mut TestAppContext) {
    http_ready(cx);
    let code = r#"await bru.cookies.add({ key: \"pre\", value: \"ErrVal-Q1z\" }); await bru.cookies.add({ key: \"evil\", value: \"ErrVal-Q2z\", domain: \"evil.example\" });"#;
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{code}\"\n",
        request("a", "http://127.0.0.1:1/r")
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.response.is_none(), "{result:?}");
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "response-error"));
    assert!(present(cx, win, "response-error-cookies"));
    let cells = error_cookie_cells(cx, win, 2);
    assert_eq!(
        cells,
        [
            ["pre", "pre-request script", "Stored"].map(str::to_owned),
            ["evil", "pre-request script", "Domain does not match host"].map(str::to_owned),
        ]
    );
    assert!(!present(cx, win, ("error-cookie-value", 0usize)));
    for cell in cells.iter().flatten() {
        assert!(!cell.contains("ErrVal"), "{cell}");
    }
}

#[gpui_kit::test]
fn a_failed_send_without_script_cookies_shows_no_cookie_list(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = request("a", "http://127.0.0.1:1/r");
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started_with(cx, &col, |_| {});
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    finished(cx, &model);
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "response-error"));
    assert!(!present(cx, win, "response-error-cookies"));
}

// The Cookies pane inside a box of a set width, so a test can narrow the pane while the window,
// and so the pointer, stays put.
struct Boxed {
    pane: gpui_kit::Entity<ResponsePane>,
    width: gpui_kit::Pixels,
}

impl gpui_kit::Render for Boxed {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{ParentElement as _, Styled as _};
        gpui_kit::div().size_full().child(
            gpui_kit::div()
                .h_full()
                .w(self.width)
                .child(self.pane.clone()),
        )
    }
}

// A 1280 px window whose Cookies pane is `width` wide, after a redirect to MID_NEXT; returns the
// hop's full text too.
fn boxed_cookies(
    cx: &mut TestAppContext,
    width: f32,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<Boxed>,
    String,
    Sent,
) {
    use gpui_kit::{VisualTestContext, px, size};
    let (port, sent) = redirect_sent(cx, MID_NEXT);
    support::init(cx);
    let m = sent.model.clone();
    let (win, boxed) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                let pane = cx.new(|cx| ResponsePane::new(m, w, cx));
                cx.new(|_| Boxed {
                    pane,
                    width: px(width),
                })
            })
        })
        .unwrap();
    cx.run_until_parked();
    VisualTestContext::from_window(win, cx).simulate_resize(size(px(1280.), px(800.)));
    let pane = boxed.read_with(cx, |b, _| b.pane.clone());
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Cookies, cx));
    frame(cx, win);
    frame(cx, win);
    (win, boxed, format!("localhost:{port}{MID_NEXT}"), sent)
}

// Rests the pointer near the left end of row 1's hop, where it stays at either pane width, and
// lets the hover delay pass. Returns whether a cut tooltip shows.
fn rest_on_hop(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let cell = w.find(("set-cookie-hop", 1usize)).bounds();
        let at = gpui_kit::point(cell.left() + gpui_kit::px(8.), cell.center().y);
        w.simulate_mouse_move(at, cx);
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("kv-cut-tooltip").is_some()
    })
    .unwrap()
}

// Sets the pane to `width` with the pointer held still and returns the tooltip the one frame
// drawn after it shows, if any, and whether row 1's hop is cut in it.
fn tip_after_still_resize(
    cx: &mut TestAppContext,
    (win, boxed): (gpui_kit::AnyWindowHandle, &gpui_kit::Entity<Boxed>),
    full: &str,
    width: f32,
) -> (Option<String>, bool) {
    let before = cx.update_window(win, |_, w, _| w.mouse_position()).unwrap();
    boxed.update(cx, |b, cx| {
        b.width = gpui_kit::px(width);
        cx.notify();
    });
    // One frame only: the tooltip must decide in the frame that lays out the new width.
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.mouse_position(), before, "the pointer never moved");
        let cell = w.find(("set-cookie-hop", 1usize)).bounds();
        assert!(cell.contains(&before), "the pointer is still over the hop");
        let tip = w
            .try_find("kv-cut-tooltip")
            .map(|t| t.label().unwrap_or_default().to_owned());
        (tip, mono_width(w, cx, full) > cell.size.width)
    })
    .unwrap()
}

// A tooltip already showing over a hop that fits re-decides once a resize cuts it, with no
// new hover.
#[gpui_kit::test]
fn a_showing_hop_tooltip_appears_when_a_resize_cuts_the_hop(cx: &mut TestAppContext) {
    let (win, boxed, full, _s) = boxed_cookies(cx, 1280.);
    assert!(!rest_on_hop(cx, win), "the hop fits, so nothing shows");
    let (tip, cut) = tip_after_still_resize(cx, (win, &boxed), &full, 960.);
    assert!(cut, "the hop is cut in a 960 px pane");
    assert_eq!(tip.as_deref(), Some(full.as_str()));
}

// The reverse: the cut tooltip showing goes once a resize lets the hop fit.
#[gpui_kit::test]
fn a_showing_hop_tooltip_goes_when_a_resize_frees_the_hop(cx: &mut TestAppContext) {
    let (win, boxed, full, _s) = boxed_cookies(cx, 960.);
    assert!(rest_on_hop(cx, win), "the hop is cut, so its tooltip shows");
    let (tip, cut) = tip_after_still_resize(cx, (win, &boxed), &full, 1280.);
    assert!(!cut, "the hop fits in a 1280 px pane");
    assert_eq!(tip, None);
}
