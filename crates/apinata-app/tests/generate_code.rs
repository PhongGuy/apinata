//! The Generate Code dialog: palette and sidebar entry, target switch, copy, secrets, errors.

use crate::support;

use apinata_app::generate_code::current;
use apinata_app::sidebar::menu::MenuEntry;
use apinata_app::sidebar::model::NodeKind;
use apinata_core::Target;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, ClipboardItem, Focusable as _, TestAppContext};
use support::*;

fn focus_shell(cx: &mut TestAppContext, s: &Started) {
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx)
    })
    .unwrap();
}
fn palette(cx: &mut TestAppContext, s: &Started, text: &str) {
    focus_shell(cx, s);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.input(text, cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
}
fn with_request(file: &str, extra: &[(&str, &str)]) -> tempfile::TempDir {
    let mut files = vec![("opencollection.yml", ROOT), ("a.yml", file)];
    files.extend_from_slice(extra);
    write_tree(&files)
}
fn shown(cx: &mut TestAppContext) -> Option<(Target, String)> {
    cx.update(|cx| current(cx).map(|v| (v.read(cx).target(), v.read(cx).text().to_owned())))
}
fn close(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.close_dialog(cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_palette_opens_curl_and_a_switch_regenerates(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    palette(cx, &s, "generate code");
    assert_eq!(
        shown(cx),
        Some((
            Target::Curl,
            "curl --request 'GET' \\\n  --url 'http://x.test/ok'".to_owned()
        ))
    );
    let view = cx.update(|cx| current(cx)).unwrap();
    cx.update_window(s.win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_target(Target::PythonRequests, w, cx))
    })
    .unwrap();
    let (target, text) = shown(cx).unwrap();
    assert_eq!(target, Target::PythonRequests);
    assert!(
        text.starts_with("import requests\n") && text.contains("'http://x.test/ok'"),
        "{text}"
    );
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click("generate-code-copy", cx)
    })
    .unwrap();
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some(text));
}

#[gpui_kit::test]
fn the_sidebar_menu_opens_it_with_the_last_target(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/m"), &[]);
    let s = started(cx, &col);
    choose_menu(
        cx,
        s.win,
        "a.yml",
        NodeKind::Request,
        MenuEntry::GenerateCode,
    );
    let view = cx.update(|cx| current(cx)).expect("the dialog is open");
    cx.update_window(s.win, |_, w, cx| {
        view.update(cx, |v, cx| v.set_target(Target::GoNative, w, cx))
    })
    .unwrap();
    close(cx, &s);
    choose_menu(
        cx,
        s.win,
        "a.yml",
        NodeKind::Request,
        MenuEntry::GenerateCode,
    );
    let (target, text) = shown(cx).expect("open again");
    assert_eq!(target, Target::GoNative);
    assert!(text.starts_with("package main\n"), "{text}");
}

#[gpui_kit::test]
fn secrets_wait_for_confirmation_before_the_dialog(cx: &mut TestAppContext) {
    let file = format!(
        "{}  headers:\n    - name: Authorization\n      value: \"Bearer {{{{process.env.APINATA_GEN_TOKEN}}}}\"\n",
        request("a", "http://x.test/s")
    );
    let col = with_request(&file, &[(".env", "APINATA_GEN_TOKEN=s3cret\n")]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    palette(cx, &s, "generate code");
    assert!(confirm_open(cx));
    assert!(shown(cx).is_none());
    answer_confirm(cx, "confirm-cancel");
    assert!(shown(cx).is_none());
    palette(cx, &s, "generate code");
    answer_confirm(cx, "confirm-show");
    assert!(shown(cx).is_some_and(|(_, t)| t.contains("Bearer s3cret")));
}

#[gpui_kit::test]
fn an_undefined_variable_opens_nothing_and_says_why(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/{{nope}}"), &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let before = cx
        .update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap();
    palette(cx, &s, "generate code");
    assert!(!confirm_open(cx));
    assert!(shown(cx).is_none());
    let after = cx
        .update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap();
    assert_eq!(after, before + 1);
}

#[gpui_kit::test]
fn copy_as_curl_still_copies_the_curl_text(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/c"), &[]);
    let s = started(cx, &col);
    cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
    choose_menu(cx, s.win, "a.yml", NodeKind::Request, MenuEntry::CopyAsCurl);
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("curl --request 'GET' \\\n  --url 'http://x.test/c'")
    );
}

#[gpui_kit::test]
fn every_new_target_generates_in_the_dialog(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    palette(cx, &s, "generate code");
    let view = cx.update(|cx| current(cx)).unwrap();
    for (target, start) in [
        (Target::PowerShellRestMethod, "#Requires -Version 7"),
        (Target::PhpCurl, "<?php"),
        (Target::RustReqwest, "// Cargo.toml: reqwest"),
        (Target::JavaHttpClient, "import "),
        (Target::CSharpHttpClient, "using "),
    ] {
        cx.update_window(s.win, |_, w, cx| {
            view.update(cx, |v, cx| v.set_target(target, w, cx))
        })
        .unwrap();
        let (shown_target, text) = shown(cx).expect("dialog open");
        assert_eq!(shown_target, target);
        assert!(text.starts_with(start), "{}: {text}", target.id());
        assert!(text.contains("http://x.test/ok"), "{}: {text}", target.id());
    }
    close(cx, &s);
}

#[test]
fn every_target_gets_a_known_grammar() {
    use apinata_app::generate_code::highlighter;
    use gpui_kit::component::highlighter::Language;
    for target in Target::ALL {
        let name = highlighter(*target);
        // The kit has no PowerShell grammar; the app registers its own under this name.
        if *target == Target::PowerShellRestMethod {
            assert_eq!(name, "powershell");
        } else {
            assert_ne!(
                Language::from_str(name),
                Language::Plain,
                "{}: {name} has no grammar",
                target.id()
            );
        }
    }
}

#[gpui_kit::test]
fn every_highlighted_target_has_a_highlights_query(cx: &mut TestAppContext) {
    use apinata_app::generate_code::highlighter;
    use gpui_kit::component::highlighter::LanguageRegistry;
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let _s = started(cx, &col);
    for target in Target::ALL {
        let name = highlighter(*target);
        let config = LanguageRegistry::singleton()
            .language(name)
            .unwrap_or_else(|| panic!("{name} is registered"));
        assert!(
            !config.highlights.is_empty(),
            "{}: {name} has no highlights query",
            target.id()
        );
    }
}

#[gpui_kit::test]
fn the_csharp_query_compiles_and_paints_a_snippet(cx: &mut TestAppContext) {
    use gpui_kit::component::Rope;
    use gpui_kit::component::highlighter::{HighlightTheme, SyntaxHighlighter};
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let _s = started(cx, &col);
    // A query that fails to compile makes the kit fall back to plain "text".
    let code = "using System;\nvar s = \"hi\";\n";
    let mut h = SyntaxHighlighter::new("csharp");
    assert_eq!(h.language().as_ref(), "csharp");
    h.update(None, &Rope::from(code), None);
    let theme = HighlightTheme::default_dark();
    let styles = h.styles(&(0..code.len()), &*theme);
    assert!(
        styles.iter().any(|(_, style)| style.color.is_some()),
        "csharp paints nothing: {styles:?}"
    );
}

#[gpui_kit::test]
fn the_powershell_query_compiles_and_paints_a_snippet(cx: &mut TestAppContext) {
    use gpui_kit::component::Rope;
    use gpui_kit::component::highlighter::{HighlightTheme, SyntaxHighlighter};
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let _s = started(cx, &col);
    // A query that fails to compile makes the kit fall back to plain "text".
    let code = "$headers = @{ 'Accept' = 'application/json' }\nInvoke-RestMethod -Uri 'http://x.test/ok' -Headers $headers\n";
    let mut h = SyntaxHighlighter::new("powershell");
    assert_eq!(h.language().as_ref(), "powershell");
    h.update(None, &Rope::from(code), None);
    let theme = HighlightTheme::default_dark();
    let styles = h.styles(&(0..code.len()), &*theme);
    assert!(
        styles.iter().any(|(_, style)| style.color.is_some()),
        "powershell paints nothing: {styles:?}"
    );
}

#[gpui_kit::test]
fn copy_never_covers_the_code(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    use gpui_kit::{VisualTestContext, px, size};
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    // The two stacked cases the issue names, then a short window that puts the preview beside
    // the target.
    for (height, zoom) in [(800., 1.0), (800., 2.0), (400., 2.0)] {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1280.), px(height)));
        cx.update(|cx| {
            let settings = cx.global::<AppState>().settings.clone();
            settings.update(cx, |p, cx| p.update(cx, |s| s.zoom = zoom));
        });
        palette(cx, &s, "generate code");
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, _| {
            use gpui_kit::test::TestWindowExt as _;
            let preview = w.find("generate-code-preview").bounds();
            let copy = w.find("generate-code-copy").bounds();
            assert!(
                !copy.intersects(&preview),
                "{height}px at {zoom}: {copy:?} over {preview:?}"
            );
            assert!(w.find("generate-code-copy").visible());
            let target = w.find("generate-code-target").bounds();
            let beside = target.right() <= preview.left();
            assert_eq!(beside, height < 500., "{height}px at {zoom}: the layout");
        })
        .unwrap();
        close(cx, &s);
    }
}

#[gpui_kit::test]
fn copy_leaves_room_for_its_focus_ring_inside_the_dialog_body(cx: &mut TestAppContext) {
    use apinata_app::app::AppState;
    use apinata_app::style::FOCUS_RING_ROOM;
    use gpui_kit::{VisualTestContext, px, size};
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    // Both stacked cases, then the short side-by-side layout, which must keep its room.
    for (height, zoom) in [(800., 1.0), (800., 2.0), (400., 2.0)] {
        VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1280.), px(height)));
        cx.update(|cx| {
            let settings = cx.global::<AppState>().settings.clone();
            settings.update(cx, |p, cx| p.update(cx, |s| s.zoom = zoom));
        });
        palette(cx, &s, "generate code");
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, _| {
            use gpui_kit::test::TestWindowExt as _;
            // The kit's dialog body clips with no padding, so the view's own box is the clip.
            let body = w.within("dialog").find("generate-code").bounds();
            for id in ["generate-code-copy", "generate-code-target"] {
                let b = w.find(id).bounds();
                let why = format!("{height}px at {zoom}: {id} {b:?} in {body:?}");
                assert!(b.top() - FOCUS_RING_ROOM >= body.top(), "{why}");
                assert!(b.left() - FOCUS_RING_ROOM >= body.left(), "{why}");
                assert!(b.right() + FOCUS_RING_ROOM <= body.right(), "{why}");
            }
        })
        .unwrap();
        close(cx, &s);
    }
}

#[gpui_kit::test]
fn enter_on_the_focused_copy_button_copies_the_code(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    palette(cx, &s, "generate code");
    let (_, text) = shown(cx).expect("the dialog is open");
    enter_on(cx, s.win, "generate-code-copy");
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some(text));
    assert!(shown(cx).is_some(), "the dialog stays open");
}

fn target_open(cx: &mut TestAppContext, s: &Started) -> Option<bool> {
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.render_frame(cx);
        w.find("generate-code-target").expanded()
    })
    .unwrap()
}

#[gpui_kit::test]
fn enter_on_the_focused_target_select_opens_it_and_keeps_the_dialog(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    palette(cx, &s, "generate code");
    enter_on(cx, s.win, "generate-code-target");
    frame(cx, s.win);
    assert!(shown(cx).is_some(), "the dialog stays open");
    assert_eq!(target_open(cx, &s), Some(true), "Enter opens the targets");
}

#[gpui_kit::test]
fn enter_on_a_target_in_the_open_select_picks_it_and_keeps_the_dialog(cx: &mut TestAppContext) {
    let col = with_request(&request("a", "http://x.test/ok"), &[]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    palette(cx, &s, "generate code");
    enter_on(cx, s.win, "generate-code-target");
    frame(cx, s.win);
    for key in ["down", "enter"] {
        cx.simulate_keystrokes(s.win, key);
        cx.run_until_parked();
    }
    frame(cx, s.win);
    let (target, _) = shown(cx).expect("the dialog stays open");
    assert_ne!(target, Target::Curl, "Enter picked the next target");
}
