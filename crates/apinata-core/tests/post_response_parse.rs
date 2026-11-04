//! Which actions are post-response variables, how their fields read, and which still warn.

use crate::common;

use apinata_core::Warning;
use common::{at, request, setup};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const HEAD: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"http://h/\"\n";
const CAPTURE: &str = "    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.id\n        method: jsonq\n      variable:\n        name: id\n        scope: runtime\n";
const EARLY: &str = "    - type: set-variable\n      phase: before-request\n      selector:\n        expression: res.status\n      variable:\n        name: early\n";

#[test]
fn only_after_response_set_variable_actions_are_read_with_strings_coerced() {
    let more = "    - type: set-variable\n      phase: after-response\n      selector:\n        expression: 42\n      variable:\n        name: 7\n      disabled: true\n    - type: set-variable\n      phase: after-response\n      variable:\n        scope: request\n    - type: other\n      phase: after-response\n";
    let text = format!("{HEAD}runtime:\n  actions:\n{CAPTURE}{EARLY}{more}");
    let (_dir, c, _ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let vars = request(&c, "r.yml").post_response_vars();
    let shapes: Vec<_> = vars
        .iter()
        .map(|v| (v.name(), v.expression(), v.enabled()))
        .collect();
    assert_eq!(
        shapes,
        [
            ("id", "res.body.id", true),
            ("7", "42", false),
            ("", "", true)
        ]
    );
}

#[test]
fn post_response_actions_at_any_level_no_longer_warn_but_other_actions_do() {
    let root = format!("{ROOT}request:\n  actions:\n{CAPTURE}");
    let folder = format!("info:\n  name: f\n  type: folder\nrequest:\n  actions:\n{CAPTURE}");
    let own = format!("{HEAD}runtime:\n  actions:\n{CAPTURE}");
    let early = format!("{HEAD}runtime:\n  actions:\n{CAPTURE}{EARLY}");
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", &folder),
        ("f/own.yml", &own),
        ("early.yml", &early),
    ]);
    let w = |rel: &str| at(&c, &ctx, rel).expect(rel).warnings().to_vec();
    assert!(w("f/own.yml").is_empty());
    assert_eq!(w("early.yml"), [Warning::ActionsNotRun]);
    assert_eq!(
        Warning::ActionsNotRun.to_string(),
        "actions other than post-response variables are not executed"
    );
}
