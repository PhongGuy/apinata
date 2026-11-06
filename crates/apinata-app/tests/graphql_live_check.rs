//! Live query checks: the typing pause, the schema used, placeholders, reloads.

use crate::support;

use std::ops::Range;
use std::path::Path;
use std::time::Duration;

use apinata_app::graphql_check::{CHECK_DELAY, CheckBasis};
use apinata_app::graphql_schema_store::{Lookup, SchemaStore, compiled};
use apinata_app::graphql_section::GraphqlSection;
use apinata_core::{BodySpec, Schema};
use gpui_kit::base::input::DiagnosticSeverity;
use gpui_kit::component::input::Position;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

const URL: &str = "http://check.test/graphql";
const SDL_A: &str = "type Query { me(id: ID): User }\ntype User { name: String }\n";
const SDL_B: &str = "type Query { me(id: ID): User }\ntype User { nmae: String }\n";

type Squiggle = (Range<Position>, DiagnosticSeverity, String);

fn store_schema(cx: &mut TestAppContext, sdl: Result<&str, &str>) {
    cx.update(|cx| {
        let s = cx.default_global::<SchemaStore>();
        let g = s.begin(URL);
        s.finish(
            URL,
            g,
            sdl.map(|t| Schema::from_sdl(t).unwrap())
                .map_err(str::to_owned),
        );
    });
    cx.update(|cx| compiled(URL, cx));
    cx.run_until_parked();
}

fn section(
    cx: &mut TestAppContext,
    sdl: Option<&str>,
    query: &str,
) -> (AnyWindowHandle, Entity<GraphqlSection>) {
    let (win, host) = host_section(cx, GraphqlSection::new);
    if let Some(sdl) = sdl {
        store_schema(cx, Ok(sdl));
        assert!(matches!(
            cx.update(|cx| compiled(URL, cx)),
            Lookup::Ready { .. }
        ));
    }
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, _, cx| {
        s.update(cx, |s, cx| s.set_schema_url(URL.into(), cx))
    })
    .unwrap();
    load(cx, win, &s, query);
    (win, s)
}

fn load(cx: &mut TestAppContext, win: AnyWindowHandle, s: &Entity<GraphqlSection>, query: &str) {
    let spec = BodySpec::Graphql {
        query: query.into(),
        variables: String::new(),
    };
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    cx.run_until_parked();
}

fn pause(cx: &mut TestAppContext, by: Duration) {
    cx.executor().advance_clock(by);
    cx.run_until_parked();
}

fn squiggles(cx: &mut TestAppContext, s: &Entity<GraphqlSection>) -> Vec<Squiggle> {
    let q = s.read_with(cx, |s, _| s.query().clone());
    q.read_with(cx, |e, _| {
        e.diagnostics()
            .map(|set| {
                set.iter()
                    .map(|d| {
                        (
                            d.diagnostic.range.clone(),
                            d.diagnostic.severity,
                            d.diagnostic.message.to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn basis(cx: &mut TestAppContext, s: &Entity<GraphqlSection>) -> Option<CheckBasis> {
    s.read_with(cx, |s, _| s.check_basis().cloned())
}

#[gpui_kit::test]
fn a_typo_is_flagged_once_typing_pauses(cx: &mut TestAppContext) {
    let (win, s) = section(cx, Some(SDL_A), "{ me { nmae } }");
    pause(cx, CHECK_DELAY - Duration::from_millis(1));
    assert!(squiggles(cx, &s).is_empty());
    pause(cx, Duration::from_millis(1));
    assert_eq!(
        squiggles(cx, &s),
        [(
            Position::new(0, 7)..Position::new(0, 11),
            DiagnosticSeverity::Error,
            "type `User` does not have a field `nmae`".to_owned()
        )]
    );
    assert_eq!(
        basis(cx, &s),
        Some(CheckBasis::Schema { source: URL.into() })
    );
    assert!(present(cx, win, "graphql-check-status"));
}

#[gpui_kit::test]
fn without_a_schema_only_syntax_is_checked(cx: &mut TestAppContext) {
    let (win, s) = section(cx, None, "{ me { nmae } }");
    pause(cx, CHECK_DELAY);
    assert!(squiggles(cx, &s).is_empty());
    assert_eq!(basis(cx, &s), Some(CheckBasis::NoSchema));
    load(cx, win, &s, "{ me { id }");
    pause(cx, CHECK_DELAY);
    let got = squiggles(cx, &s);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1, DiagnosticSeverity::Error);
    assert_eq!(got[0].2, "syntax error: expected R_CURLY, got EOF");
}

#[gpui_kit::test]
fn placeholders_are_not_flagged(cx: &mut TestAppContext) {
    let (win, s) = section(cx, Some(SDL_A), "{ me(id: {{id}}) { name } }");
    pause(cx, CHECK_DELAY);
    assert!(squiggles(cx, &s).is_empty());
    assert_eq!(
        basis(cx, &s),
        Some(CheckBasis::Schema { source: URL.into() })
    );
    load(cx, win, &s, "{ me(id: 1) { {{fields}} } }");
    pause(cx, CHECK_DELAY);
    assert!(squiggles(cx, &s).is_empty());
    assert_eq!(basis(cx, &s), Some(CheckBasis::Skipped));
}

#[gpui_kit::test]
fn a_reload_revalidates_against_the_new_schema(cx: &mut TestAppContext) {
    let (_win, s) = section(cx, Some(SDL_A), "{ me { nmae } }");
    pause(cx, CHECK_DELAY);
    assert_eq!(squiggles(cx, &s).len(), 1);
    store_schema(cx, Ok(SDL_B));
    pause(cx, CHECK_DELAY);
    assert!(squiggles(cx, &s).is_empty());
    store_schema(cx, Err("the server answered 500 Internal Server Error"));
    pause(cx, CHECK_DELAY);
    assert_eq!(basis(cx, &s), Some(CheckBasis::NoSchema));
}

#[gpui_kit::test]
fn an_edit_while_loading_is_rechecked_when_the_load_settles_on_the_same_key(
    cx: &mut TestAppContext,
) {
    let (win, s) = section(cx, None, "{ me { id } }");
    pause(cx, CHECK_DELAY);
    assert_eq!(basis(cx, &s), Some(CheckBasis::NoSchema));
    let g = cx.update(|cx| cx.default_global::<SchemaStore>().begin(URL));
    replace_code(cx, win, None, "graphql-query", "query Q");
    pause(cx, CHECK_DELAY);
    assert!(
        squiggles(cx, &s).is_empty(),
        "the check waits while the schema loads"
    );
    cx.update(|cx| {
        cx.default_global::<SchemaStore>()
            .finish(URL, g, Err("boom".into()))
    });
    cx.run_until_parked();
    pause(cx, CHECK_DELAY);
    assert_eq!(basis(cx, &s), Some(CheckBasis::NoSchema));
    let got = squiggles(cx, &s);
    assert!(
        !got.is_empty() && got.iter().all(|d| d.1 == DiagnosticSeverity::Error),
        "{got:?}"
    );
}

#[gpui_kit::test]
fn typing_clears_old_squiggles_and_restarts_the_pause(cx: &mut TestAppContext) {
    let (win, s) = section(cx, Some(SDL_A), "{ me { nmae } }");
    pause(cx, CHECK_DELAY);
    assert_eq!(squiggles(cx, &s).len(), 1);
    replace_code(cx, win, None, "graphql-query", "query Q");
    assert!(squiggles(cx, &s).is_empty(), "an edit drops the old set");
    pause(cx, CHECK_DELAY);
    let got = squiggles(cx, &s);
    assert!(
        !got.is_empty() && got.iter().all(|d| d.1 == DiagnosticSeverity::Error),
        "{got:?}"
    );
}

#[gpui_kit::test]
fn a_schema_file_is_named_only_while_its_load_succeeded(cx: &mut TestAppContext) {
    let (_win, s) = section(cx, None, "{ me { name } }");
    pause(cx, CHECK_DELAY);
    let file = Path::new("/schemas/check.graphql");
    cx.update(|cx| {
        let store = cx.default_global::<SchemaStore>();
        let g = store.begin_file(URL, file);
        store.finish(URL, g, Ok(Schema::from_sdl(SDL_A).unwrap()));
    });
    cx.update(|cx| compiled(URL, cx));
    cx.run_until_parked();
    pause(cx, CHECK_DELAY);
    assert_eq!(
        basis(cx, &s),
        Some(CheckBasis::Schema {
            source: file.display().to_string()
        })
    );
    // A failed file load keeps the file's name in the store, but nothing was checked against it.
    cx.update(|cx| {
        let store = cx.default_global::<SchemaStore>();
        let g = store.begin_file(URL, file);
        store.finish(URL, g, Err("not a schema".into()));
    });
    cx.run_until_parked();
    pause(cx, CHECK_DELAY);
    let got = basis(cx, &s).expect("a check ran");
    assert_eq!(got, CheckBasis::NoSchema);
    assert!(!got.label().contains("check.graphql"), "{}", got.label());
}
