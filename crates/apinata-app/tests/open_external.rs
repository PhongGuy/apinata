//! Opening a body externally: a private temp file handed to the injected opener, and an HTML
//! or SVG body as an inert browser copy. Tests install a recorder, so nothing is ever launched.

use crate::support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use apinata_app::body_view::BodyView;
use apinata_app::body_view::{MASKED, OPEN_PAGE_TIP};
use apinata_app::open_external::{
    self, FileOpener, INERT_CSP, OPEN_DIR_PREFIX, extension, inert_html,
};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};
use support::*;

fn recorder() -> (FileOpener, Arc<Mutex<Vec<PathBuf>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let opener: FileOpener = Arc::new(move |path, _| log.lock().unwrap().push(path.to_path_buf()));
    (opener, seen)
}

fn pdf(body: Vec<u8>, truncated: bool) -> Arc<RequestResult> {
    let mut r = Response::new(200)
        .with_header("content-type", "application/pdf")
        .with_body(body, "");
    if truncated {
        let n = r.body().len() as u64;
        r = r.with_truncated(n);
    }
    Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(r),
        Vec::new(),
    ))
}

fn body() -> Vec<u8> {
    vec![0xff, 0x25, 0x50, 0x44, 0x46, 0x00, 0x9f]
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
    (win, view)
}

fn click_open(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("body-open", cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[test]
fn the_extension_follows_the_media_type() {
    assert_eq!(extension("application/pdf"), "pdf");
    assert_eq!(extension("Application/PDF; name=x"), "pdf");
    assert_eq!(extension("image/png"), "png");
    assert_eq!(extension("image/jpeg"), "jpg");
    assert_eq!(extension("application/x-unknown"), "bin");
    assert_eq!(extension(""), "bin");
    assert_eq!(extension("text/html; charset=utf-8"), "html");
    assert_eq!(extension("image/svg+xml"), "svg");
}

fn installed_dir(
    cx: &mut TestAppContext,
    base: &tempfile::TempDir,
) -> (PathBuf, Arc<Mutex<Vec<PathBuf>>>) {
    let (opener, seen) = recorder();
    let root = base.path().to_path_buf();
    cx.update(|cx| open_external::install(opener, root, cx));
    cx.run_until_parked();
    let dir = cx
        .update(|cx| open_external::session_dir(cx))
        .expect("a session dir");
    (dir, seen)
}

#[gpui_kit::test]
fn install_prunes_only_older_own_open_dirs(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let old = base.path().join(format!("{OPEN_DIR_PREFIX}old"));
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("body-old.bin"), b"old").unwrap();
    let other = base.path().join("someone-else");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("keep.bin"), b"keep").unwrap();
    let plain = base.path().join(format!("{OPEN_DIR_PREFIX}file"));
    std::fs::write(&plain, b"not a dir").unwrap();
    let (dir, _) = installed_dir(cx, &base);
    assert!(!old.exists(), "an older own open dir is pruned");
    assert!(
        other.join("keep.bin").exists(),
        "a foreign-named entry is untouched"
    );
    assert!(plain.exists(), "a file with the prefix is untouched");
    assert_eq!(dir.parent(), Some(base.path()));
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap();
    assert!(name.starts_with(OPEN_DIR_PREFIX) && name != "apinata-open-old");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

// The fake liveness check: only this test process counts as running.
fn only_this_process(pid: u32) -> bool {
    pid == std::process::id()
}

fn sibling(base: &tempfile::TempDir, pid: u32) -> PathBuf {
    let dir = base.path().join(format!("{OPEN_DIR_PREFIX}{pid}-sibling"));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("body-x.pdf"), b"x").unwrap();
    dir
}

fn install_checked(cx: &mut TestAppContext, base: &tempfile::TempDir) {
    let (opener, _) = recorder();
    let root = base.path().to_path_buf();
    cx.update(|cx| open_external::install_with_liveness(opener, root, only_this_process, cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_live_sibling_instance_keeps_its_open_dir(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let live = sibling(&base, std::process::id());
    let dead = sibling(&base, u32::MAX);
    install_checked(cx, &base);
    assert!(
        live.join("body-x.pdf").exists(),
        "a running instance's dir survives"
    );
    assert!(!dead.exists(), "a dir whose owner is gone is pruned");
    let mine = cx.update(|cx| open_external::session_dir(cx)).unwrap();
    let name = mine
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap()
        .to_owned();
    assert!(name.starts_with(&format!("{OPEN_DIR_PREFIX}{}-", std::process::id())));
}

#[gpui_kit::test]
fn a_stale_dir_is_pruned_even_when_its_pid_is_live(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let stale = sibling(&base, std::process::id());
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(25 * 60 * 60);
    std::fs::File::open(&stale)
        .unwrap()
        .set_modified(old)
        .unwrap();
    install_checked(cx, &base);
    assert!(!stale.exists(), "a dir older than the age limit is pruned");
}

#[gpui_kit::test]
fn open_writes_a_private_file_and_calls_the_opener(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let (dir, seen) = installed_dir(cx, &base);
    let (win, view) = host(cx, pdf(body(), false));
    assert!(cx.update(|cx| view.read(cx).can_open(cx)));
    click_open(cx, win);
    click_open(cx, win);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert_ne!(seen[0], seen[1], "each open gets its own file");
    for path in &seen {
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("pdf"));
        assert_eq!(std::fs::read(path).unwrap(), body());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}

#[gpui_kit::test]
fn without_an_opener_there_is_no_open_button(cx: &mut TestAppContext) {
    let (win, _view) = host(cx, pdf(body(), false));
    assert!(present(cx, win, "body-binary"));
    assert!(!present(cx, win, "body-open"));
}

#[gpui_kit::test]
fn a_truncated_body_cannot_be_opened(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let (_dir, seen) = installed_dir(cx, &base);
    let (win, view) = host(cx, pdf(body(), true));
    // The kit reports no disabled flag to test snapshots, so the view's own answer stands in.
    assert!(present(cx, win, "body-open"), "the button is still offered");
    assert!(!cx.update(|cx| view.read(cx).can_open(cx)));
    click_open(cx, win);
    assert!(seen.lock().unwrap().is_empty());
}

#[gpui_kit::test]
fn a_lost_session_dir_is_replaced_by_a_fresh_one(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let (old, seen) = installed_dir(cx, &base);
    std::fs::remove_dir_all(&old).unwrap();
    let (win, _view) = host(cx, pdf(body(), false));
    click_open(cx, win);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "the open still reaches the opener");
    let dir = seen[0].parent().unwrap().to_path_buf();
    assert_ne!(dir, old, "the old name is never recreated");
    assert!(!old.exists());
    assert_eq!(dir.parent(), Some(base.path()));
    assert_eq!(
        cx.update(|cx| open_external::session_dir(cx)),
        Some(dir.clone())
    );
    assert_eq!(std::fs::read(&seen[0]).unwrap(), body());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

#[gpui_kit::test]
fn a_recreated_dir_with_the_session_name_is_not_trusted(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let (old, seen) = installed_dir(cx, &base);
    // The impostor exists before the session dir goes, so it cannot reuse that inode.
    let impostor = base.path().join("impostor");
    std::fs::create_dir(&impostor).unwrap();
    std::fs::remove_dir_all(&old).unwrap();
    std::fs::rename(&impostor, &old).unwrap();
    let (win, _view) = host(cx, pdf(body(), false));
    click_open(cx, win);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_ne!(
        seen[0].parent(),
        Some(old.as_path()),
        "the impostor dir gets no body"
    );
    assert_eq!(std::fs::read_dir(&old).unwrap().count(), 0);
    assert_eq!(seen[0].parent().and_then(|p| p.parent()), Some(base.path()));
}

const SECRET: &str = "s3cr3t-value-123";

// The bytes every inert page starts with, ahead of any body markup.
fn prefix() -> String {
    format!(
        r#"<meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="{INERT_CSP}">"#
    )
}

// Everything between the inert head and the escaped body: the title, the frame-filling style
// and the opening of the sandboxed srcdoc frame.
const WRAPPER: &str = r#"<title>HTML response</title><style>html,body{margin:0;height:100%}iframe{display:block;border:0;width:100%;height:100%}</style><iframe sandbox="" title="HTML response" srcdoc=""#;

// The test's own copy of the srcdoc escaping, `&` first so no reference is escaped twice.
fn escaped(body: &str) -> String {
    body.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// The reverse, `&amp;` last so an escaped `&amp;lt;` comes back as `&lt;`, not `<`.
fn unescaped(value: &str) -> String {
    value
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn framed(body: &str) -> String {
    format!("{}{WRAPPER}{}\"></iframe>", prefix(), escaped(body))
}

// Checks that `page` frames `body` exactly: one frame, nothing escapable left in its srcdoc,
// and the srcdoc value unescapes back to the body.
fn assert_framed(page: &str, body: &str) {
    assert_eq!(page, framed(body), "{body:?}");
    let value = page
        .strip_prefix(&format!("{}{WRAPPER}", prefix()))
        .and_then(|rest| rest.strip_suffix("\"></iframe>"))
        .expect("head, wrapper, srcdoc, close");
    for c in ['"', '<', '>', '\''] {
        assert!(!value.contains(c), "{c} in {value:?}");
    }
    assert_eq!(page.matches("<iframe").count(), 1, "{page}");
    assert_eq!(page.matches("</iframe>").count(), 1, "{page}");
    assert!(page.ends_with("\"></iframe>"));
    assert_eq!(unescaped(value), body);
}

const HOSTILE: &[&str] = &[
    r#""><script>alert(1)</script>"#,
    "</iframe><script>x</script>",
    "' onload='x",
    "&quot;><img src=x onerror=y>",
    r#"<meta http-equiv="refresh" content="0;url=http://127.0.0.1:9/">"#,
    r#"<a href="http://127.0.0.1:9/" target="_top">x</a>"#,
    r#"<base href="http://127.0.0.1:9/">"#,
    "&amp;lt;",
    "a\r\nb \u{f8} \u{65e5}\u{672c} \u{1f642}",
    "<!-- x --><![CDATA[ y ]]>",
    "a\0b\rc",
];

#[test]
fn a_hostile_html_body_stays_inside_the_srcdoc_value() {
    for body in HOSTILE {
        assert_framed(&inert_html(body), body);
    }
}

#[gpui_kit::test]
fn a_hostile_html_body_opened_by_click_stays_framed(cx: &mut TestAppContext) {
    let body = HOSTILE[1];
    let base = tempfile::tempdir().unwrap();
    let (_dir, seen) = installed_dir(cx, &base);
    let (win, _view) = host(cx, typed("text/html", body.as_bytes(), false));
    click_open(cx, win);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_framed(&std::fs::read_to_string(&seen[0]).unwrap(), body);
}

#[test]
fn the_html_frame_is_sandboxed_with_no_allowance() {
    let page = inert_html(PAGE);
    assert_eq!(page.matches("sandbox=").count(), 1, "{page}");
    assert!(page.contains(r#"<iframe sandbox="" "#), "{page}");
    for allow in [
        "allow-scripts",
        "allow-top-navigation",
        "allow-popups",
        "allow-forms",
        "allow-same-origin",
        "allow-downloads",
    ] {
        assert!(!page.contains(allow), "{allow}");
    }
}

fn typed(ct: &str, body: &[u8], truncated: bool) -> Arc<RequestResult> {
    let text = String::from_utf8_lossy(body).into_owned();
    let mut r = Response::new(200)
        .with_header("content-type", ct)
        .with_body(body.to_vec(), text);
    if truncated {
        let n = r.body().len() as u64;
        r = r.with_truncated(n);
    }
    Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(r),
        Vec::new(),
    ))
}

fn open_label(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find("body-open").label().map(str::to_owned)
    })
    .unwrap()
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

const PAGE: &str = "<!doctype html><html><body><h1>Hi</h1><script>alert(1)</script></body></html>";

#[gpui_kit::test]
fn an_html_body_opens_as_an_inert_page(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let (dir, seen) = installed_dir(cx, &base);
    let (win, view) = host(
        cx,
        typed("text/html; charset=utf-8", PAGE.as_bytes(), false),
    );
    assert!(cx.update(|cx| view.read(cx).can_open(cx)));
    assert_eq!(open_label(cx, win).as_deref(), Some("Open in browser"));
    click_open(cx, win);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let path = &seen[0];
    assert_eq!(path.parent(), Some(dir.as_path()));
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("html"));
    #[cfg(unix)]
    assert_eq!(mode(path), 0o600);
    let written = std::fs::read_to_string(path).unwrap();
    assert!(written.starts_with(&prefix()));
    assert_eq!(written, framed(PAGE));
    assert!(!written.contains("<script"), "{written}");
    assert!(!written.contains("<h1"), "{written}");
}

#[gpui_kit::test]
fn a_binary_body_keeps_the_system_app_label(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    installed_dir(cx, &base);
    let (win, _view) = host(cx, pdf(body(), false));
    assert_eq!(open_label(cx, win).as_deref(), Some("Open with system app"));
}

// A real send whose `ct` response `body` echoes the environment's secret token, so the result
// knows the value to redact; the files and server stay alive with the returned guard.
fn secret_send(
    cx: &mut TestAppContext,
    ct: &str,
    body: String,
) -> (Arc<RequestResult>, impl Sized + use<>) {
    http_ready(cx);
    let server = mock_server(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_raw(body.into_bytes(), ct),
    );
    let a = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/ok?token={{{{token}}}}\"\n",
        server.uri()
    );
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "token"), SECRET).unwrap();
    let s = started_on(cx, &col, |_| {}, Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);
    (result, (server, col, s, model))
}

#[gpui_kit::test]
fn an_opened_html_page_holds_no_secret(cx: &mut TestAppContext) {
    let page = format!("<html><body><p>token {SECRET}</p></body></html>");
    let (result, _keep) = secret_send(cx, "text/html", page);
    assert!(result.response.as_ref().unwrap().text().contains(SECRET));
    let base = tempfile::tempdir().unwrap();
    let (_dir, seen) = installed_dir(cx, &base);
    let (win, view) = host(cx, result);
    // Revealing the body on screen does not reveal it on disk.
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_reveal(true, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    click_open(cx, win);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let written = std::fs::read_to_string(&seen[0]).unwrap();
    assert!(written.starts_with(&prefix()));
    assert!(!written.contains(SECRET), "{written}");
    assert!(
        written.contains(&escaped(&format!("token {MASKED}"))),
        "{written}"
    );
}

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4"/></svg>"#;

#[gpui_kit::test]
fn an_svg_body_opens_as_an_image_in_an_html_page(cx: &mut TestAppContext) {
    use base64::Engine as _;
    let base = tempfile::tempdir().unwrap();
    let (dir, seen) = installed_dir(cx, &base);
    let (win, view) = host(cx, typed("image/svg+xml", SVG.as_bytes(), false));
    assert!(cx.update(|cx| view.read(cx).can_open(cx)));
    assert_eq!(open_label(cx, win).as_deref(), Some("Open in browser"));
    click_open(cx, win);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].extension().and_then(|e| e.to_str()), Some("html"));
    #[cfg(unix)]
    assert_eq!(mode(&seen[0]), 0o600);
    let written = std::fs::read_to_string(&seen[0]).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(SVG);
    assert_eq!(
        written,
        format!(
            r#"{}<title>SVG response</title><img alt="SVG response" src="data:image/svg+xml;base64,{b64}">"#,
            prefix()
        )
    );
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(names.iter().all(|n| !n.ends_with(".svg")), "{names:?}");
}

#[gpui_kit::test]
fn text_xml_and_raster_bodies_have_no_open_action(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    installed_dir(cx, &base);
    for (ct, body) in [
        ("application/json", &br#"{"a":1}"#[..]),
        ("text/plain", b"hello"),
        ("application/xml", b"<a><b>1</b></a>"),
        ("image/png", PNG),
    ] {
        let (win, view) = host(cx, typed(ct, body, false));
        assert!(!cx.update(|cx| view.read(cx).can_open(cx)), "{ct}");
        assert!(!present(cx, win, "body-open"), "{ct}");
    }
}

#[gpui_kit::test]
fn a_truncated_html_body_cannot_be_opened(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let (_dir, seen) = installed_dir(cx, &base);
    let (win, view) = host(cx, typed("text/html", PAGE.as_bytes(), true));
    assert!(present(cx, win, "body-open"), "the button is still offered");
    assert!(!cx.update(|cx| view.read(cx).can_open(cx)));
    click_open(cx, win);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn the_inert_policy_blocks_script_network_forms_frames_and_base() {
    for part in [
        "default-src 'none'",
        "form-action 'none'",
        "base-uri 'none'",
        "frame-src 'none'",
    ] {
        assert!(INERT_CSP.contains(part), "{part}");
    }
    for allowance in ["script-src", "connect-src"] {
        assert!(!INERT_CSP.contains(allowance), "{allowance}");
    }
    assert_eq!(INERT_CSP.matches("frame-src").count(), 1, "{INERT_CSP}");
}

// The PNG signature and a 1x1 IHDR chunk: enough for the body to show as a PNG image.
const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0,
    1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89,
];

#[gpui_kit::test]
fn an_opened_svg_page_holds_no_secret(cx: &mut TestAppContext) {
    use base64::Engine as _;
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><text>{SECRET}</text></svg>"#);
    let (result, _keep) = secret_send(cx, "image/svg+xml", svg);
    let base = tempfile::tempdir().unwrap();
    let (_dir, seen) = installed_dir(cx, &base);
    let (win, view) = host(cx, result);
    assert!(cx.update(|cx| view.read(cx).can_open(cx)));
    click_open(cx, win);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let written = std::fs::read_to_string(&seen[0]).unwrap();
    let b64 = written
        .split("data:image/svg+xml;base64,")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a data: image");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    let decoded = String::from_utf8(decoded).unwrap();
    assert!(!decoded.contains(SECRET), "{decoded}");
    assert!(
        decoded.contains(&format!("<text>{MASKED}</text>")),
        "{decoded}"
    );
}

// Whatever the opener received, never as a page a browser would run.
fn no_active_file(seen: &[PathBuf], dir: &std::path::Path) {
    for path in seen {
        let ext = path.extension().and_then(|e| e.to_str());
        assert!(!matches!(ext, Some("html" | "svg")), "{path:?}");
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        assert!(
            !name.ends_with(".html") && !name.ends_with(".svg"),
            "{name}"
        );
    }
}

#[gpui_kit::test]
fn an_invalid_utf8_html_body_opens_only_as_raw_bytes(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let (dir, seen) = installed_dir(cx, &base);
    let (win, _view) = host(cx, typed("text/html", b"<script>x</script>\xff", false));
    assert!(present(cx, win, "body-open"), "offered as a binary body");
    click_open(cx, win);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    no_active_file(&seen, &dir);
    assert_eq!(seen[0].extension().and_then(|e| e.to_str()), Some("bin"));
}

#[gpui_kit::test]
fn a_non_utf8_svg_body_is_never_written_as_a_page(cx: &mut TestAppContext) {
    let base = tempfile::tempdir().unwrap();
    let (dir, seen) = installed_dir(cx, &base);
    let (win, view) = host(cx, typed("image/svg+xml", b"<svg>\xff</svg>", false));
    assert!(!cx.update(|cx| view.read(cx).can_open(cx)));
    assert!(!present(cx, win, "body-open"));
    let seen = seen.lock().unwrap().clone();
    assert!(seen.is_empty());
    no_active_file(&seen, &dir);
}

#[test]
fn the_page_tooltip_names_what_is_blocked_and_redacted() {
    for word in ["scripts", "network loads", "redacted"] {
        assert!(OPEN_PAGE_TIP.contains(word), "{word}");
    }
}
