//! HTML Preview: an HTML body laid out by the kit's renderer from the format menu, with links
//! that open nothing and images that load only from data: URLs.

use crate::support;

use std::sync::Arc;

use apinata_app::body_view::BodyView;
use apinata_app::html_preview;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, ImageFormat, ImageSource, SharedUri, TestAppContext,
    WindowOptions, point, px,
};
use support::*;

fn typed(content_type: &str, body: &str) -> Arc<RequestResult> {
    let r = Response::new(200)
        .with_header("content-type", content_type)
        .with_body(body.as_bytes().to_vec(), body);
    Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(r),
        Vec::new(),
    ))
}

fn host(cx: &mut TestAppContext, r: Arc<RequestResult>) -> (AnyWindowHandle, Entity<BodyView>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    (win, view)
}

const PAGE: &str = "<html><head><style>p{color:red}</style><script>alert(1)</script></head>\
    <body><h1>Title</h1><p>Hello</p><ul><li>one</li></ul></body></html>";

#[gpui_kit::test]
fn preview_is_offered_for_html_only(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, typed("text/html", PAGE));
    let entries = format_entries(cx, win);
    assert!(entries.iter().any(|e| e == "Preview"), "{entries:?}");

    let (win, _view) = host(cx, typed("application/json", r#"{"a":1}"#));
    let entries = format_entries(cx, win);
    assert!(!entries.iter().any(|e| e == "Preview"), "{entries:?}");
}

#[gpui_kit::test]
fn preview_replaces_the_editor_until_raw_is_chosen(cx: &mut TestAppContext) {
    let (win, view) = host(cx, typed("text/html", PAGE));
    assert!(present(cx, win, "body-editor"));
    choose_format(cx, win, "Preview");
    assert!(cx.update(|cx| view.read(cx).previewing()));
    assert!(present(cx, win, "body-html-preview"));
    assert!(!present(cx, win, "body-editor"));
    let html = cx.update(|cx| view.read(cx).preview_markdown()).unwrap();
    assert!(html.starts_with("# Title\n\nHello\n\n- one"), "{html}");
    assert!(!html.contains("alert") && !html.contains("color"), "{html}");

    choose_format(cx, win, "Raw");
    assert!(!cx.update(|cx| view.read(cx).previewing()));
    assert!(present(cx, win, "body-editor"));
    assert!(!present(cx, win, "body-html-preview"));
}

#[gpui_kit::test]
fn preview_stays_on_for_the_next_html_body_only(cx: &mut TestAppContext) {
    let (win, view) = host(cx, typed("text/html", PAGE));
    choose_format(cx, win, "Preview");
    let json = typed("application/json", r#"{"a":1}"#);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(json), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!cx.update(|cx| view.read(cx).previewing()));
    assert!(present(cx, win, "body-editor"));

    let html = typed("text/html", PAGE);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(html), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| view.read(cx).previewing()));
}

#[test]
fn a_remote_or_file_image_is_never_handed_to_the_loader() {
    for uri in [
        "http://127.0.0.1:9/p.png",
        "https://example.invalid/p.png",
        "file:///etc/passwd",
        "p.png",
    ] {
        let source = html_preview::image_source(&SharedUri::from(uri.to_string()));
        assert!(
            !matches!(source, ImageSource::Resource(_)),
            "{uri} would be fetched"
        );
    }
}

#[test]
fn a_data_image_is_decoded_inline() {
    // The eight-byte PNG signature, base64.
    let uri = SharedUri::from("data:image/png;base64,iVBORw0KGgo=".to_string());
    match html_preview::image_source(&uri) {
        ImageSource::Image(image) => {
            assert_eq!(image.format(), ImageFormat::Png);
            assert_eq!(image.bytes(), b"\x89PNG\r\n\x1a\n");
        }
        _ => panic!("a data: image should be decoded inline"),
    }
}

// A bare window laying out Markdown `page` with `view`, under the text selection layer the
// app's root mounts, so link clicks are handled as in the app. The preview writes no links
// itself; this guards the view should one ever appear.
struct Page {
    markdown: gpui_kit::SharedString,
    kit_default: bool,
}

impl gpui_kit::Render for Page {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{ParentElement as _, Styled as _};
        let page = if self.kit_default {
            gpui_kit::component::text::TextView::markdown("page", self.markdown.clone())
        } else {
            html_preview::view("page", self.markdown.clone())
        };
        gpui_kit::div()
            .size_full()
            .child(gpui_kit::base::TextSelectionLayer)
            .child(page)
    }
}

fn click_first_line(cx: &mut TestAppContext, kit_default: bool) -> Option<String> {
    support::init(cx);
    let markdown = format!("[{}](file:///etc/passwd)", "x".repeat(80));
    let (_view, vcx) = cx.add_window_view(|_, _| Page {
        markdown: markdown.into(),
        kit_default,
    });
    vcx.run_until_parked();
    vcx.simulate_click(point(px(16.), px(8.)), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    vcx.opened_url()
}

#[gpui_kit::test]
fn the_kit_alone_would_open_a_page_link(cx: &mut TestAppContext) {
    // The control for the test below: the same click with the kit's default handler opens it.
    assert_eq!(
        click_first_line(cx, true).as_deref(),
        Some("file:///etc/passwd")
    );
}

#[gpui_kit::test]
fn a_page_link_in_the_preview_opens_nothing(cx: &mut TestAppContext) {
    assert_eq!(click_first_line(cx, false), None);
}

// The kit's Markdown parse options, with raw HTML left on so escaping alone is what keeps
// HTML out of the tree.
fn kit_options() -> markdown::ParseOptions {
    let mut options = markdown::ParseOptions::gfm();
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    options
}

// The depth of the Markdown tree `md` parses to, walked without recursion, and whether any
// node in it is raw HTML.
fn mdast_depth(md: &str) -> (usize, bool) {
    let root = markdown::to_mdast(md, &kit_options()).unwrap();
    let (mut deepest, mut html) = (0, false);
    let mut stack = vec![(&root, 1usize)];
    while let Some((node, depth)) = stack.pop() {
        deepest = deepest.max(depth);
        html |= matches!(node, markdown::mdast::Node::Html(_));
        for child in node.children().into_iter().flatten() {
            stack.push((child, depth + 1));
        }
    }
    (deepest, html)
}

// Pages built to nest past any bound: each tag the tree builder keeps open, foreign content,
// a comment closed by `--!>`, tags joined across a removed script, and formatting elements
// the adoption agency rebuilds one level deeper each round.
fn hostile_pages() -> Vec<(&'static str, String)> {
    let reconstruct: String = (0..10_000).map(|n| format!("<b><i id={n}></b>x")).collect();
    vec![
        ("divs", "<div>".repeat(50_000)),
        ("optgroup", "<optgroup>".repeat(25_000)),
        ("rt", "<rt>".repeat(60_000)),
        ("dd li", "<dd><li>".repeat(30_000)),
        ("svg td", format!("<svg>{}", "<td>".repeat(60_000))),
        ("math tr", format!("<math>{}", "<tr>".repeat(60_000))),
        ("comment", format!("<!-- --!>{}-->", "<div>".repeat(50_000))),
        ("joined", "<<script></script>div>".repeat(10_000)),
        ("reconstruct", reconstruct),
        (
            "rebuilt script",
            "<<script></script>script>alert(1)</script>".repeat(1_000),
        ),
    ]
}

const CHILD: &str = "APINATA_HTML_PREVIEW_CHILD";

#[test]
fn hostile_pages_preview_within_a_shallow_tree() {
    // A stack overflow aborts the whole process, so the pages run in a re-run of this test,
    // on a small stack; a regression fails here instead of taking the binary down.
    if std::env::var_os(CHILD).is_some() {
        let worker = std::thread::Builder::new().stack_size(1 << 20);
        let checked = worker.spawn(|| {
            for (name, page) in hostile_pages() {
                let page = html_preview::prepare(&page);
                let (depth, html) = mdast_depth(&page.markdown);
                assert!(depth <= 24, "{name}: the preview tree is {depth} deep");
                assert!(!html, "{name}: raw HTML reached the preview");
                assert!(!page.markdown.contains("alert(1)"), "{name}");
            }
        });
        checked.unwrap().join().unwrap();
        return;
    }
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "body_html_preview::hostile_pages_preview_within_a_shallow_tree",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .output()
        .expect("the test binary re-runs");
    assert!(
        child.status.success(),
        "{}{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
}

#[test]
fn a_page_past_a_bound_is_cut() {
    let page = html_preview::prepare(&"<br>".repeat(html_preview::MAX_NODES + 1_000));
    assert!(page.cut);
    let deep = "<div>".repeat(html_preview::MAX_DEPTH + 10);
    assert!(html_preview::prepare(&deep).cut);
    assert!(!html_preview::prepare(&"<div>".repeat(100)).cut);
    assert!(!html_preview::prepare("<p>short</p>").cut);
}

#[test]
fn script_style_and_title_text_is_left_out() {
    let page = "<title>T</title><h1>a<script>alert(1)</script>b<STYLE>p{}</style>c</h1>\
        <span><script src=x></script></span><p>d</p><script>never closed";
    assert_eq!(html_preview::prepare(page).markdown, "# abc\n\nd\n");
}

#[test]
fn page_text_is_escaped_so_it_makes_no_markup() {
    let page = "<p>*not* <b>bold</b> # [x](y) &lt;i&gt; $5 | `c` www.example.com</p>";
    let md = html_preview::prepare(page).markdown;
    let root = markdown::to_mdast(&md, &kit_options()).unwrap();
    let mut kinds = Vec::new();
    let mut stack = vec![&root];
    while let Some(node) = stack.pop() {
        kinds.push(
            format!("{node:?}")
                .split([' ', '('])
                .next()
                .unwrap()
                .to_owned(),
        );
        stack.extend(node.children().into_iter().flatten());
    }
    kinds.sort();
    kinds.dedup();
    assert_eq!(kinds, ["Paragraph", "Root", "Strong", "Text"], "{md}");
}

#[test]
fn headings_lists_tables_and_code_keep_their_shape() {
    let page = "<h2>Users</h2><ul><li>one</li><li>two<ol><li>inner</li></ol></li></ul>\
        <blockquote><p>quoted</p></blockquote>\
        <table><tr><th>Name</th><th>Role</th></tr><tr><td>Ada</td><td><i>admin</i></td></tr></table>\
        <pre>let x = 1;\n  y</pre><p>a <code>c</code> b</p>";
    assert_eq!(
        html_preview::prepare(page).markdown,
        "## Users\n\n- one\n\n- two\n\n  1. inner\n\n> quoted\n\n\
         | Name | Role |\n| --- | --- |\n| Ada | *admin* |\n\n\
         ```\nlet x = 1;\n  y\n```\n\na `c` b\n"
    );
}

#[test]
fn only_a_raster_data_image_is_kept_as_an_image() {
    let page = concat!(
        r#"<img src="data:image/png;base64,iVBORw0KGgo=" alt="dot">"#,
        r#"<img src="http://127.0.0.1:9/p.png" alt="remote"><img src="data:image/svg+xml,x" alt="svg">"#
    );
    assert_eq!(
        html_preview::prepare(page).markdown,
        "![dot](data:image/png;base64,iVBORw0KGgo=)remotesvg\n"
    );
}

#[test]
fn an_svg_data_image_is_not_decoded() {
    for uri in [
        "data:image/svg+xml;base64,PHN2Zy8+",
        "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%2F%3E",
        "data:image/tiff;base64,SUkqAA==",
    ] {
        match html_preview::image_source(&SharedUri::from(uri.to_string())) {
            ImageSource::Image(image) => {
                assert!(image.bytes().is_empty(), "{uri} was decoded")
            }
            _ => panic!("{uri} should become an empty image"),
        }
    }
}

#[gpui_kit::test]
fn a_page_past_the_node_bound_previews_its_start_with_a_note(cx: &mut TestAppContext) {
    // Short lines, in Raw, so the body is in the editor and Preview is offered.
    let line = "<br>".repeat(60);
    let page = vec![line; html_preview::MAX_NODES / 60 + 20].join("\n");
    let (win, view) = host(cx, typed("text/html", &page));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.set_mode(apinata_app::body_view::BodyMode::Raw, w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    choose_format(cx, win, "Preview");
    assert!(present(cx, win, "body-html-preview"));
    let note = cx
        .update_window(win, |_, w, _| {
            w.find("body-banner").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(
        note.as_deref(),
        Some(apinata_app::body_view::PREVIEW_CUT_NOTE)
    );
}

#[gpui_kit::test]
fn the_find_bar_closes_when_a_body_lands_in_the_preview(cx: &mut TestAppContext) {
    let (win, view) = host(cx, typed("text/html", PAGE));
    choose_format(cx, win, "Preview");
    let json = typed("application/json", r#"{"a":1}"#);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.show(Some(json), w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| view.update(cx, |v, cx| v.open_find(w, cx)))
        .unwrap();
    assert!(cx.update(|cx| view.read(cx).find_open()));
    let html = typed("text/html", PAGE);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(html), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| view.read(cx).previewing()));
    assert!(!cx.update(|cx| view.read(cx).find_open()));
}
