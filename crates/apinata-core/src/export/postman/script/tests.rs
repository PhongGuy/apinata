use super::corpus::table;
use super::{CALLS, SIMPLE, translate};

fn t(src: &str) -> String {
    translate(src).code
}

fn bounded<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(20))
        .expect("finishes in time")
}

#[test]
fn every_simple_row_follows_the_reference_table() {
    let table = table();
    let ours: Vec<(String, String)> = SIMPLE
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect();
    assert_eq!(ours, table.simple);
    for (from, to) in &table.simple {
        let out = translate(&format!("{from}(1);"));
        assert_eq!(out.code, format!("{to}(1);"), "{from}");
        assert_eq!(out.rewritten, 1, "{from}");
        assert!(out.leftovers.is_empty(), "{from}: {:?}", out.leftovers);
    }
}

#[test]
fn the_longest_matching_chain_wins_and_the_rest_stays() {
    assert_eq!(t("req.headerList.get('a')"), "pm.request.headers.get('a')");
    assert_eq!(t("req.url.host"), "pm.request.url.host");
    assert_eq!(t("res.getBody().items"), "pm.response.json().items");
    assert_eq!(t("res.getBody()?.user?.id"), "pm.response.json()?.user?.id");
    assert_eq!(t("req.url = 'x';"), "pm.request.url = 'x';");
    assert_eq!(
        t("bru.setVar('b', res.status);"),
        "pm.variables.set('b', pm.response.code);"
    );
    assert_eq!(t("f(...res.headers)"), "f(...pm.response.headers)");
    assert_eq!(t("expect.fail('x')"), "pm.expect.fail('x')");
}

#[test]
fn only_free_standing_heads_are_rewritten() {
    for src in [
        "x.bru.getVar('a')",
        "mybru.getVar('a')",
        "$res.status",
        "res",
        "expect(1)",
    ] {
        let out = translate(src);
        assert_eq!(out.code, src);
        assert_eq!(out.rewritten, 0, "{src}");
    }
}

#[test]
fn strings_comments_and_regexes_are_left_alone() {
    for src in [
        "'bru.getVar(1)'",
        "\"res.status\"",
        "// bru.getVar(1)",
        "/* req.url */",
        "/bru.getVar/.test(s)",
    ] {
        let out = translate(src);
        assert_eq!(out.code, src);
        assert_eq!(out.rewritten, 0, "{src}");
        assert!(out.leftovers.is_empty(), "{src}");
    }
}

#[test]
fn template_holes_are_translated_and_template_text_is_not() {
    assert_eq!(
        t("`${bru.getVar(\"a\")} and bru.getVar`"),
        "`${pm.variables.get(\"a\")} and bru.getVar`"
    );
    assert_eq!(
        t("`${ {a: bru.getVar(\"x\")}.a }`"),
        "`${ {a: pm.variables.get(\"x\")}.a }`"
    );
    assert_eq!(
        t("`a ${`b ${res.status}`}`"),
        "`a ${`b ${pm.response.code}`}`"
    );
}

#[test]
fn division_is_not_read_as_a_regex() {
    let out = translate("const r = (a) / 2; const s = b / (c); bru.getVar('x');");
    assert_eq!(
        out.code,
        "const r = (a) / 2; const s = b / (c); pm.variables.get('x');"
    );
    assert!(!out.kept);
}

#[test]
fn leftovers_are_the_distinct_chains_in_first_seen_order() {
    let src = "bru.sleep(10);\nbru.sleep(5);\nres.setBody({});\nreq.getTags();\nconst s = `${bru.cwd()}`;\nconst q = 'bru.runRequest';\nobj.res.foo();";
    let out = translate(src);
    assert_eq!(
        out.leftovers,
        ["bru.sleep", "res.setBody", "req.getTags", "bru.cwd"]
    );
    assert_eq!(out.rewritten, 0);
    assert_eq!(out.code, src);
}

#[test]
fn rewrites_are_counted() {
    assert_eq!(
        translate("bru.getVar('a'); bru.setVar('b', res.status);").rewritten,
        3
    );
    assert_eq!(translate("").rewritten, 0);
    assert_eq!(t(""), "");
}

#[test]
fn a_script_whose_brackets_do_not_balance_is_kept_as_written() {
    for src in [
        "bru.getVar('a'); if (x) {",
        "bru.getVar('a'));",
        "f(bru.getVar('a']",
    ] {
        let out = translate(src);
        assert_eq!(out.code, src);
        assert!(out.kept, "{src}");
        assert_eq!(out.rewritten, 0, "{src}");
        assert_eq!(out.leftovers, ["bru.getVar"], "{src}");
    }
    assert!(!translate("bru.getVar('(');").kept);
}

#[test]
fn hostile_input_stays_bounded() {
    assert_eq!(
        bounded(|| translate(&"bru.getVar(1);\n".repeat(50_000)).rewritten),
        50_000
    );
    assert!(bounded(|| translate(&"res.status(".repeat(50_000)).kept));
    let deep = format!("{}x", "`${".repeat(10_000));
    let out = bounded({
        let deep = deep.clone();
        move || translate(&deep)
    });
    assert!(out.kept);
    assert_eq!(out.code, deep);
    assert_eq!(
        bounded(|| translate(&format!("bru{}", ".a".repeat(50_000)))
            .leftovers
            .len()),
        1
    );
}

#[test]
fn every_call_to_property_row_follows_the_reference_table() {
    let table = table();
    let ours: Vec<(String, String)> = CALLS[..13]
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect();
    assert_eq!(ours, table.call_to_property);
    assert_eq!(CALLS.len(), 14);
    for (from, to) in &table.call_to_property {
        let out = translate(&format!("const v = {from}();"));
        assert_eq!(out.code, format!("const v = {to};"), "{from}");
        assert_eq!(out.rewritten, 1, "{from}");
        assert!(out.leftovers.is_empty(), "{from}");
    }
}

#[test]
fn a_call_becomes_the_property_whatever_its_arguments() {
    assert_eq!(t("req.getUrl('x')"), "pm.request.url");
    assert_eq!(t("req.getUrl ( )"), "pm.request.url");
    assert_eq!(t("req.getUrl(\")\")"), "pm.request.url");
    assert_eq!(t("req.getUrl().host"), "pm.request.url.host");
    assert_eq!(
        t("if (res.getStatus() === 200) {}"),
        "if (pm.response.code === 200) {}"
    );
    let nested = translate("req.getUrl(bru.getVar('a'))");
    assert_eq!(nested.code, "pm.request.url");
    assert_eq!(nested.rewritten, 1);
}

#[test]
fn an_uncalled_getter_is_a_leftover() {
    let out = translate("const f = req.getUrl;");
    assert_eq!(out.code, "const f = req.getUrl;");
    assert_eq!(out.leftovers, ["req.getUrl"]);
}

#[test]
fn stop_execution_becomes_set_next_request_null() {
    assert_eq!(
        t("bru.runner.stopExecution();"),
        "pm.execution.setNextRequest(null);"
    );
    assert_eq!(
        t("bru.runner.skipRequest();"),
        "pm.execution.skipRequest();"
    );
}

#[test]
fn plan_a_cases_match_the_reference() {
    let report = super::corpus::run(&|s: &str| translate(s).code);
    eprintln!("{}", report.summary());
    assert!(
        report.differing.is_empty(),
        "differing: {:?}",
        report.differing
    );
    assert!(
        report.stale.is_empty(),
        "deviations that now match: {:?}",
        report.stale
    );
    assert_eq!(report.plan_a, 123);
    assert_eq!(report.matched + report.deviated, 123);
}

#[test]
fn an_unclosed_template_or_hole_keeps_the_script_and_reports_what_follows() {
    for src in [
        "`${ f( }` ; bru.getVar('a')",
        "`${ bru.getVar('a')",
        "const s = `a ${b} bru.getVar('a')",
    ] {
        let out = translate(src);
        assert!(out.kept, "{src}");
        assert_eq!(out.code, src);
        assert_eq!(out.rewritten, 0, "{src}");
        assert_eq!(out.leftovers, ["bru.getVar"], "{src}");
    }
}

// Wraps `inner` in `levels` templates, each holding the next in its only hole.
fn nested(levels: usize, inner: &str) -> String {
    (0..levels).fold(inner.to_owned(), |s, _| format!("`${{{s}}}`"))
}

#[test]
fn templates_nested_past_the_depth_cap_are_kept_with_their_leftovers() {
    let out = translate(&nested(31, "bru.getVar('a')"));
    assert!(!out.kept);
    assert_eq!(out.code, nested(31, "pm.variables.get('a')"));
    assert_eq!(out.rewritten, 1);
    assert!(out.leftovers.is_empty());
    for levels in [32, 33, 40] {
        let src = nested(levels, "bru.getVar('a')");
        let out = translate(&src);
        assert!(out.kept, "{levels}");
        assert_eq!(out.code, src);
        assert_eq!(out.leftovers, ["bru.getVar"], "{levels}");
    }
}

#[test]
fn spaced_and_optional_chains_are_reported_as_leftovers() {
    let src = "bru\n  .getVar('a');\nreq .getUrl();\nres?.getStatus();\nbru?.\n  setVar(1);\nconst n = res ? .5 : res?.5:1;";
    let out = translate(src);
    assert_eq!(out.code, src);
    assert_eq!(out.rewritten, 0);
    assert_eq!(
        out.leftovers,
        ["bru.getVar", "req.getUrl", "res.getStatus", "bru.setVar"]
    );
}
