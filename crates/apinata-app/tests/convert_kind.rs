//! Convert to GraphQL and Convert to HTTP, from the sidebar menu and the palette, and the
//! GraphQL detection of the Import cURL dialog.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::shell::{ImportCurl, Shell};
use apinata_app::sidebar::menu::{MenuEntry, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use apinata_app::tabs::TabKind;
use apinata_core::{BodySpec, ItemPath, RequestKind};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
use support::*;

const HTTP: &str = "info:\n  name: a\n  type: http\n  seq: 1\n\nhttp:\n  method: POST\n  url: http://x.test/graphql\n  headers:\n    - name: X-A\n      value: \"1\"\n  body:\n    type: json\n    data: '{\"query\": \"{ me { id } }\", \"variables\": {\"id\": 1}}'\n";

const GRAPHQL: &str = "info:\n  name: g\n  type: graphql\n  seq: 2\n\ngraphql:\n  method: POST\n  url: http://x.test/graphql\n  body:\n    query: \"{ me }\"\n";

const TEXT: &str = "info:\n  name: t\n  type: http\n  seq: 3\n\nhttp:\n  method: POST\n  url: http://x.test/t\n  body:\n    type: text\n    data: hello\n";

fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", HTTP),
        ("g.yml", GRAPHQL),
        ("t.yml", TEXT),
    ])
}

fn file(s: &Started, rel: &str) -> String {
    std::fs::read_to_string(s.root.join(rel)).unwrap()
}

fn kind(cx: &mut TestAppContext, s: &Started, rel: &str) -> RequestKind {
    s.ws.read_with(cx, |w, cx| w.request(&s.root, &ip(rel), cx))
        .expect("the request")
        .kind
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

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
    cx.update_window(s.win, |_, w, cx| w.input(text, cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
    frame(cx, s.win);
}

// The label of the row menu's convert entry, which names the kind it converts to.
fn convert_label(cx: &mut TestAppContext, s: &Started, rel: &str) -> String {
    let ix = menu_entries(NodeKind::Request)
        .iter()
        .position(|e| *e == MenuEntry::ConvertToGraphql)
        .expect("offered");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.right_click(row(rel), cx)
    })
    .unwrap();
    frame(cx, s.win);
    let label = cx
        .update_window(s.win, |_, w, _| {
            w.within("popup-menu").find(ix).label().map(str::to_owned)
        })
        .unwrap();
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    label.expect("a labelled entry")
}

#[gpui_kit::test]
fn each_request_row_offers_conversion_to_the_other_kind(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    assert_eq!(convert_label(cx, &s, "a.yml"), "Convert to GraphQL");
    assert_eq!(convert_label(cx, &s, "g.yml"), "Convert to HTTP");
}

#[gpui_kit::test]
fn the_menu_converts_an_http_request_and_its_open_tab_follows(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    choose_menu(
        cx,
        s.win,
        "a.yml",
        NodeKind::Request,
        MenuEntry::ConvertToGraphql,
    );
    frame(cx, s.win);
    // Nothing is lost, so nothing is asked.
    assert!(!confirm_open(cx));
    let text = file(&s, "a.yml");
    assert!(text.contains("type: graphql\n"), "{text}");
    assert!(text.contains("\ngraphql:\n"), "{text}");
    assert!(text.contains("name: X-A"), "{text}");
    assert_eq!(kind(cx, &s, "a.yml"), RequestKind::Graphql);
    let query = cx.read(|cx| {
        let area = s.shell.read(cx).tab_area().read(cx);
        let editor = area.editor(tab).expect("the tab's editor").read(cx);
        let graphql = editor.graphql().read(cx);
        graphql.query().read(cx).value().to_string()
    });
    assert_eq!(query, "{ me { id } }");
}

#[gpui_kit::test]
fn the_palette_converts_the_active_graphql_request(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "g.yml");
    frame(cx, s.win);
    // The palette offers only the conversion to the other kind.
    palette(cx, &s, "convert to graphql");
    assert_eq!(file(&s, "g.yml"), GRAPHQL);
    palette(cx, &s, "convert to http");
    assert_eq!(kind(cx, &s, "g.yml"), RequestKind::Http);
    let view =
        s.ws.read_with(cx, |w, cx| w.request(&s.root, &ip("g.yml"), cx))
            .unwrap();
    assert_eq!(
        view.body,
        BodySpec::Json("{\n  \"query\": \"{ me }\",\n  \"variables\": {}\n}".into())
    );
}

#[gpui_kit::test]
fn unsaved_edits_block_the_conversion_with_a_toast(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    let edit = Command::Edit {
        target: Target::Request(ip("a.yml")),
        edit: Edit::Name("renamed".into()),
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, edit, cx));
    cx.run_until_parked();
    let before = toasts(cx, &s);
    choose_menu(
        cx,
        s.win,
        "a.yml",
        NodeKind::Request,
        MenuEntry::ConvertToGraphql,
    );
    frame(cx, s.win);
    assert_eq!(file(&s, "a.yml"), HTTP);
    assert_eq!(kind(cx, &s, "a.yml"), RequestKind::Http);
    assert_eq!(toasts(cx, &s), before + 1);
}

#[gpui_kit::test]
fn the_toast_undoes_the_conversion(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    choose_menu(
        cx,
        s.win,
        "g.yml",
        NodeKind::Request,
        MenuEntry::ConvertToGraphql,
    );
    frame(cx, s.win);
    assert_eq!(kind(cx, &s, "g.yml"), RequestKind::Http);
    cx.update_window(s.win, |_, w, cx| w.click("undo-convert", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    assert_eq!(file(&s, "g.yml"), GRAPHQL);
    assert_eq!(kind(cx, &s, "g.yml"), RequestKind::Graphql);
}

fn import_curl(cx: &mut TestAppContext, win: AnyWindowHandle, shell: &Entity<Shell>, curl: &str) {
    let shell = shell.clone();
    cx.update_window(win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(ImportCurl), cx);
    })
    .unwrap();
    frame(cx, win);
    replace_code(cx, win, None, "import-curl-input", curl);
    cx.update_window(win, |_, w, cx| w.click("import-curl-import", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

fn active_draft_view(
    cx: &mut TestAppContext,
    s: &Started,
) -> std::sync::Arc<apinata_app::snapshot::RequestView> {
    let item: ItemPath =
        s.ws.read_with(cx, |w, _| match w.tabs().active().map(|t| t.kind.clone()) {
            Some(TabKind::Request(item)) => item,
            other => panic!("a request tab is active: {other:?}"),
        });
    assert!(item.is_draft(), "{item:?}");
    s.ws.read_with(cx, |w, cx| w.request(&s.root, &item, cx))
        .expect("the draft")
}

#[gpui_kit::test]
fn a_pasted_graphql_payload_imports_as_a_graphql_request(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    import_curl(
        cx,
        s.win,
        &s.shell,
        r#"curl http://x.test/api -H 'Content-Type: application/json' --data-raw '{"query":"{ me { id } }","variables":{"id":1}}'"#,
    );
    let view = active_draft_view(cx, &s);
    assert_eq!(view.kind, RequestKind::Graphql);
    assert_eq!(
        view.body,
        BodySpec::Graphql {
            query: "{ me { id } }".into(),
            variables: "{\n  \"id\": 1\n}".into(),
        }
    );
}

#[gpui_kit::test]
fn a_pasted_plain_json_body_stays_http(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    import_curl(
        cx,
        s.win,
        &s.shell,
        r#"curl http://x.test/graphql -H 'Content-Type: application/json' --data-raw '{"query":"shoes","page":2}'"#,
    );
    let view = active_draft_view(cx, &s);
    assert_eq!(view.kind, RequestKind::Http);
    assert_eq!(
        view.body,
        BodySpec::Json("{\n  \"query\": \"shoes\",\n  \"page\": 2\n}".into())
    );
}

#[gpui_kit::test]
fn a_graphql_payload_pasted_into_a_graphql_url_takes_the_query(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "g.yml");
    frame(cx, s.win);
    let editor = cx.read(|cx| {
        let area = s.shell.read(cx).tab_area().read(cx);
        area.editor(tab).expect("the tab's editor").clone()
    });
    let curl = r#"curl http://x.test/v2 -H 'Content-Type: application/json' --data-raw '{"query":"{ you }"}'"#;
    cx.update_window(s.win, |_, w, cx| {
        editor.update(cx, |e, cx| assert!(e.paste_url(curl, w, cx)))
    })
    .unwrap();
    cx.run_until_parked();
    let view =
        s.ws.read_with(cx, |w, cx| w.request(&s.root, &ip("g.yml"), cx))
            .unwrap();
    assert_eq!(view.url, "http://x.test/v2");
    assert_eq!(
        view.body,
        BodySpec::Graphql {
            query: "{ you }".into(),
            variables: String::new(),
        }
    );
}

#[gpui_kit::test]
fn a_conversion_that_drops_the_body_asks_first(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    let convert = |cx: &mut TestAppContext| {
        choose_menu(
            cx,
            s.win,
            "t.yml",
            NodeKind::Request,
            MenuEntry::ConvertToGraphql,
        );
        frame(cx, s.win);
    };
    convert(cx);
    assert!(
        confirm_open(cx),
        "the loss is named before anything changes"
    );
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(file(&s, "t.yml"), TEXT);
    assert_eq!(kind(cx, &s, "t.yml"), RequestKind::Http);
    convert(cx);
    answer_confirm(cx, "confirm-convert");
    frame(cx, s.win);
    assert_eq!(kind(cx, &s, "t.yml"), RequestKind::Graphql);
    assert!(!file(&s, "t.yml").contains("hello"));
}

#[gpui_kit::test]
fn a_file_that_changes_while_the_dialog_is_open_is_not_converted(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    choose_menu(
        cx,
        s.win,
        "t.yml",
        NodeKind::Request,
        MenuEntry::ConvertToGraphql,
    );
    frame(cx, s.win);
    assert!(confirm_open(cx));
    // The file gains saved examples, so confirming would now drop more than was shown.
    let changed = format!("{TEXT}\nexamples:\n  - name: ok\n    response:\n      status: 200\n");
    std::fs::write(s.root.join("t.yml"), &changed).unwrap();
    let path = s.root.join("t.yml");
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, Command::Reload(vec![path.clone()]), cx)
    });
    cx.run_until_parked();
    answer_confirm(cx, "confirm-convert");
    frame(cx, s.win);
    assert_eq!(file(&s, "t.yml"), changed);
    assert_eq!(kind(cx, &s, "t.yml"), RequestKind::Http);
    assert_eq!(toasts(cx, &s), 1, "the refusal is toasted");
}
