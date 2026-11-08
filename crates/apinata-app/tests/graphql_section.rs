use crate::support;

use apinata_app::command::Edit;
use apinata_app::graphql_section::GraphqlSection;
use apinata_core::BodySpec;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

fn section(
    cx: &mut TestAppContext,
    spec: BodySpec,
) -> (AnyWindowHandle, Entity<SectionHost<GraphqlSection>>) {
    let (win, host) = host_section(cx, GraphqlSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, host)
}

fn texts(
    cx: &mut TestAppContext,
    host: &Entity<SectionHost<GraphqlSection>>,
) -> [(String, String); 2] {
    let s = host.read_with(cx, |h, _| h.view.clone());
    let (q, v) = s.read_with(cx, |s, _| (s.query().clone(), s.variables().clone()));
    let read = |e: &Entity<gpui_kit::component::input::EditorState>, cx: &mut TestAppContext| {
        e.read_with(cx, |c, _| {
            (c.value().to_string(), c.language_name().to_string())
        })
    };
    [read(&q, cx), read(&v, cx)]
}

#[gpui_kit::test]
fn loading_shows_both_fields_without_an_edit(cx: &mut TestAppContext) {
    let (_win, host) = section(
        cx,
        BodySpec::Graphql {
            query: "{ me }".into(),
            variables: "{}".into(),
        },
    );
    assert_eq!(
        texts(cx, &host),
        [
            ("{ me }".to_owned(), "graphql".to_owned()),
            ("{}".to_owned(), "json".to_owned())
        ]
    );
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
}

#[gpui_kit::test]
fn typing_variables_emits_the_whole_graphql_body(cx: &mut TestAppContext) {
    let (win, host) = section(
        cx,
        BodySpec::Graphql {
            query: "{ me }".into(),
            variables: String::new(),
        },
    );
    replace_code(cx, win, None, "graphql-variables", "{\"limit\": 10");
    cx.run_until_parked();
    let Some(Edit::Body(body)) = host.read_with(cx, |h, _| h.edits.last().cloned()) else {
        panic!("no body edit")
    };
    assert_eq!(
        body,
        BodySpec::Graphql {
            query: "{ me }".into(),
            variables: "{\"limit\": 10}".into()
        }
    );
}

// A query longer than the editor, scrolled part way by the wheel: typing at the bottom of
// the visible lines must keep the caret inside the editor's visible bounds.
#[gpui_kit::test]
fn typing_in_a_partly_scrolled_query_keeps_the_caret_in_view(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point, px};
    let long: String = (0..80).map(|i| format!("  field{i}\n")).collect();
    let (win, host) = section(
        cx,
        BodySpec::Graphql {
            query: format!("{{\n{long}}}"),
            variables: String::new(),
        },
    );
    let query = host.read_with(cx, |h, cx| h.view.read(cx).query().clone());
    // The caret's top and bottom on screen: the kit reports it without the vertical scroll.
    let caret = |cx: &mut TestAppContext| {
        query.read_with(cx, |q, _| {
            let (c, _) = q.cursor_layout().expect("a laid out caret");
            let scrolled = q.text_bounds().unwrap().origin.y - q.input_bounds().origin.y;
            (c.top() + scrolled, c.bottom() + scrolled, q.input_bounds())
        })
    };
    cx.update_window(win, |_, w, cx| {
        w.scroll(
            "graphql-query",
            ScrollDelta::Pixels(point(px(0.), px(-300.))),
            cx,
        );
        w.render_frame(cx);
        // The last visible line of the editor.
        w.click_at("graphql-query", point(px(200.), px(280.)), cx);
        w.render_frame(cx);
    })
    .unwrap();
    for n in 0..12 {
        cx.update_window(win, |_, w, cx| {
            w.press("enter", cx);
            w.input("x", cx);
            w.render_frame(cx);
        })
        .unwrap();
        let (top, bottom, view) = caret(cx);
        assert!(
            top >= view.top() - px(0.5) && bottom <= view.bottom() + px(0.5),
            "after {n} lines the caret {top:?}..{bottom:?} is outside the editor {view:?}"
        );
    }
}

// In a short window the section body scrolls around the two editors; typing at the bottom of
// the query, with the body scrolled part way, must keep the caret inside the body's viewport.
#[gpui_kit::test]
fn typing_in_a_query_under_a_scrolled_section_body_keeps_the_caret_in_view(
    cx: &mut TestAppContext,
) {
    use apinata_app::request_editor::Section;
    use gpui_kit::{px, size};
    let long: String = (0..80).map(|i| format!("      field{i}\n")).collect();
    let file = format!(
        "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: http://x/0\n  body:\n    query: |-\n      {{\n{long}      }}\n"
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("g.yml", file.as_str())]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "g.yml", false);
    gpui_kit::VisualTestContext::from_window(win, cx).simulate_resize(size(px(1000.), px(400.)));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx)
    })
    .unwrap();
    frame(cx, win);
    frame(cx, win);
    let query = ed.read_with(cx, |e, cx| e.graphql().read(cx).query().clone());
    cx.update_window(win, |_, w, cx| {
        w.click_at("graphql-query", gpui_kit::point(px(200.), px(40.)), cx);
        w.press(&key("end"), cx);
    })
    .unwrap();
    for n in 0..8 {
        cx.update_window(win, |_, w, cx| {
            w.press("enter", cx);
            w.input("x", cx);
            w.render_frame(cx);
            w.render_frame(cx);
        })
        .unwrap();
        let (top, bottom) = query.read_with(cx, |q, _| {
            let (c, _) = q.cursor_layout().expect("a laid out caret");
            let scrolled = q.text_bounds().unwrap().origin.y - q.input_bounds().origin.y;
            (c.top() + scrolled, c.bottom() + scrolled)
        });
        let body = cx
            .update_window(win, |_, w, _| w.find("section-body").bounds())
            .unwrap();
        assert!(
            top >= body.top() - px(0.5) && bottom <= body.bottom() + px(0.5),
            "after {n} lines the caret {top:?}..{bottom:?} is outside the body {body:?}"
        );
    }
    // Wheeling the body away from the caret afterwards is not undone by the follow.
    let caret_bottom = |cx: &mut TestAppContext| {
        query.read_with(cx, |q, _| {
            let (c, _) = q.cursor_layout().expect("a laid out caret");
            c.bottom() + q.text_bounds().unwrap().origin.y - q.input_bounds().origin.y
        })
    };
    let before = caret_bottom(cx);
    cx.update_window(win, |_, w, cx| {
        w.scroll(
            "section-body",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(60.))),
            cx,
        );
        for _ in 0..4 {
            w.render_frame(cx);
        }
    })
    .unwrap();
    assert!(
        caret_bottom(cx) > before,
        "the body was snapped back to the caret"
    );
}

// A builder tick with the caret on the last line and the query scrolled to its top must leave
// the section body where it was: the caret the editor scrolled away is not the body's to reveal.
#[gpui_kit::test]
fn a_builder_tick_leaves_the_section_body_still(cx: &mut TestAppContext) {
    use apinata_app::graphql_schema_store::SchemaStore;
    use apinata_app::request_editor::Section;
    use apinata_core::{FieldPath, Schema};
    use gpui_kit::{point, px, size};
    let long: String = (0..80).map(|i| format!("      # line {i}\n")).collect();
    let file = format!(
        "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: http://x/0\n  body:\n    query: |-\n      {{\n        users {{\n          id\n        }}\n{long}      }}\n"
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("g.yml", file.as_str())]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "g.yml", false);
    let schema = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../apinata-core/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    cx.update(|cx| {
        let store = cx.default_global::<SchemaStore>();
        let g = store.begin("http://x/0");
        store.finish("http://x/0", g, Ok(Schema::from_json(&schema).unwrap()));
    });
    gpui_kit::VisualTestContext::from_window(win, cx).simulate_resize(size(px(1000.), px(500.)));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx)
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("graphql-builder-toggle", cx))
        .unwrap();
    frame(cx, win);
    let (query, builder) = ed.read_with(cx, |e, cx| {
        let g = e.graphql().read(cx);
        (g.query().clone(), g.builder().clone())
    });
    for path in ["Query", "Query.users"] {
        builder.update(cx, |b, cx| {
            b.toggle_expanded(&FieldPath::new(path.split('.')), cx)
        });
    }
    // Caret on the last line, then the query scrolled back to its top.
    cx.update_window(win, |_, w, cx| {
        w.click_at("graphql-query", point(px(200.), px(40.)), cx);
        w.press(&key("end"), cx);
        for _ in 0..4 {
            w.render_frame(cx);
        }
    })
    .unwrap();
    query.update(cx, |q, cx| q.set_scroll_offset(point(px(0.), px(0.)), cx));
    for _ in 0..4 {
        frame(cx, win);
    }
    let top = |cx: &mut TestAppContext| query.read_with(cx, |q, _| q.input_bounds().top());
    let before = top(cx);
    let name = FieldPath::new("Query.users.name".split('.'));
    let ix = builder.read_with(cx, |b, _| b.rows().iter().position(|r| r.path == name));
    cx.update_window(win, |_, w, cx| {
        w.click(("graphql-builder-check", ix.expect("a name row")), cx);
        for _ in 0..4 {
            w.render_frame(cx);
        }
    })
    .unwrap();
    assert!(query.read_with(cx, |q, _| q.value().contains("name")));
    assert_eq!(top(cx), before, "the section body scrolled");
}
