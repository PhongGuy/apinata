use super::corpus::table;
use super::scan::parts;
use super::tokens::same;
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
    assert_eq!(report.plan_a, 167);
    assert_eq!(report.matched + report.deviated, 167);
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

#[track_caller]
fn check(src: &str, want: &str) {
    let out = translate(src).code;
    assert!(
        same(&out, want),
        "\n input: {src}\noutput: {out}\n  want: {want}"
    );
}

#[test]
fn parts_split_at_top_level_commas() {
    let src = "f(a, [b, c], {d: 1, e: 2}, 'x,y', `${g(1, 2)}`, /,/g, )";
    let got: Vec<&str> = parts(src, 2, src.len() - 1)
        .iter()
        .map(|&(s, e)| &src[s..e])
        .collect();
    assert_eq!(
        got,
        [
            "a",
            "[b, c]",
            "{d: 1, e: 2}",
            "'x,y'",
            "`${g(1, 2)}`",
            "/,/g"
        ]
    );
    assert!(parts("f()", 2, 2).is_empty());
}

#[test]
fn send_request_becomes_pm_send_request_whatever_its_arguments() {
    assert_eq!(t("bru.sendRequest();"), "pm.sendRequest();");
    assert_eq!(
        t("bru.sendRequest('https://x.test');"),
        "pm.sendRequest('https://x.test');"
    );
    assert_eq!(
        t("await bru.sendRequest(opts);"),
        "await pm.sendRequest(opts);"
    );
    assert_eq!(
        t("bru.sendRequest(make(), cb, extra);"),
        "pm.sendRequest(make(), cb);"
    );
}

#[test]
fn the_send_request_layout_is_pinned() {
    assert_eq!(
        t("bru.sendRequest({url: 'u', headers: {a: '1'}, data: {k: 'v'}});"),
        "pm.sendRequest({url: 'u', header: {a: '1'}, body: { mode: \"raw\", raw: {k: 'v'} }});"
    );
    assert_eq!(
        t(
            "bru.sendRequest({headers: {'content-type': 'application/x-www-form-urlencoded'}, data: {k: 'v', ...more}});"
        ),
        "pm.sendRequest({header: {'content-type': 'application/x-www-form-urlencoded'}, body: { mode: \"urlencoded\", urlencoded: [{ key: \"k\", value: 'v' }, ...Object.entries(more).map(([key, value]) => ({ key: key, value: value }))] }});"
    );
}

#[test]
fn the_body_mode_follows_a_plain_content_type_header() {
    let call =
        |h: &str, d: &str| format!("bru.sendRequest({{url: 'u', headers: {h}, data: {d}}});");
    let want = |h: &str, b: &str| format!("pm.sendRequest({{url: 'u', header: {h}, body: {b}}});");
    for (h, d, b) in [
        (
            "{'Content-Type': 'multipart/form-data; boundary=x'}",
            "{a: 1}",
            "{mode: 'formdata', formdata: [{key: 'a', value: 1}]}",
        ),
        (
            "{\"CONTENT-TYPE\": \"Application/X-WWW-Form-Urlencoded\"}",
            "{a: 1}",
            "{mode: 'urlencoded', urlencoded: [{key: 'a', value: 1}]}",
        ),
        (
            "{'Content-Type': `multipart/form-data`}",
            "{a: 1}",
            "{mode: 'raw', raw: {a: 1}}",
        ),
        (
            "{'Content-Type': 'application/x-www-form-urlencoded'}",
            "'a=1'",
            "{mode: 'raw', raw: 'a=1'}",
        ),
        (
            "{'Content-Type': 'application/json'}",
            "{a: 1}",
            "{mode: 'raw', raw: {a: 1}}",
        ),
        ("h", "{a: 1}", "{mode: 'raw', raw: {a: 1}}"),
    ] {
        check(&call(h, d), &want(h, b));
    }
    check(
        "bru.sendRequest({url: 'u', data: {a: 1}});",
        "pm.sendRequest({url: 'u', body: {mode: 'raw', raw: {a: 1}}});",
    );
    check(
        "const data = 'x';\nbru.sendRequest({url: 'u', data});",
        "const data = 'x';\npm.sendRequest({url: 'u', body: {mode: 'raw', raw: data}});",
    );
}

#[test]
fn data_properties_become_key_value_pairs() {
    check(
        "bru.sendRequest({headers: {'Content-Type': 'multipart/form-data'}, data: {'a-b': 1, c: x, 2: z, [k]: w, e, ...rest}});",
        "pm.sendRequest({header: {'Content-Type': 'multipart/form-data'}, body: {mode: 'formdata', formdata: [{key: 'a-b', value: 1}, {key: 'c', value: x}, {key: 2, value: z}, {key: k, value: w}, {key: 'e', value: e}, ...Object.entries(rest).map(([key, value]) => ({key: key, value: value}))]}});",
    );
}

#[test]
fn templated_and_quoted_send_requests() {
    assert_eq!(
        t("const s = `${bru.sendRequest('u')}`;"),
        "const s = `${pm.sendRequest('u')}`;"
    );
    let src = "'bru.sendRequest(1)'; // bru.sendRequest(2)";
    assert_eq!(translate(src).code, src);
    assert_eq!(translate(src).rewritten, 0);
}

#[test]
fn a_send_request_shape_the_rewrite_cannot_follow_is_kept_and_reported() {
    for src in [
        "bru.sendRequest({headers: {'Content-Type': 'multipart/form-data'}, data: {get a() { return 1; }}});",
        "bru . sendRequest('u');",
        "const f = bru.sendRequest;",
    ] {
        let out = translate(src);
        assert_eq!(out.code, src);
        assert_eq!(out.rewritten, 0, "{src}");
        assert_eq!(out.leftovers, ["bru.sendRequest"], "{src}");
    }
}

#[test]
fn send_request_input_stays_bounded() {
    assert_eq!(
        bounded(|| translate(&"bru.sendRequest('u');\n".repeat(20_000)).rewritten),
        20_000
    );
    let deep = format!(
        "{}1{}",
        "bru.sendRequest({data: ".repeat(5_000),
        "})".repeat(5_000)
    );
    assert!(bounded(move || translate(&deep).code.len()) > 0);
}

#[test]
fn the_callback_layout_is_pinned() {
    assert_eq!(
        t("bru.sendRequest(\"https://x.test/c\", (err, r) => { console.log(r.data); });"),
        "pm.sendRequest(\"https://x.test/c\", function(err, r) { console.log(r.json()); });"
    );
}

#[test]
fn callbacks_become_plain_functions_reading_the_postman_response() {
    for (cb, want) in [
        (
            "(e, r) => r.data.items",
            "function(e, r) { return r.json().items; }",
        ),
        (
            "async (e, r) => { await f(r.status); }",
            "async function(e, r) { await f(r.code); }",
        ),
        (
            "function done(e, r) { r.statusText; }",
            "function(e, r) { r.status; }",
        ),
        ("async function (e, r) {}", "async function(e, r) {}"),
        ("r => r.data", "function(r, response) { return r.data; }"),
        (
            "(e, {data}) => data",
            "function(e, response) { return data; }",
        ),
        ("() => 1", "function(error, response) { return 1; }"),
        (
            "(e, r) => [r.dataset, x.r.data, r?.data, r.headers, r\n.data]",
            "function(e, r) { return [r.dataset, x.r.data, r?.data, r.headers, r.json()]; }",
        ),
        ("cb", "cb"),
    ] {
        check(
            &format!("bru.sendRequest('u', {cb});"),
            &format!("pm.sendRequest('u', {want});"),
        );
    }
    check(
        "bru.sendRequest('u', (e, r) => { bru.setVar('a', r.data); res.getBody(); });",
        "pm.sendRequest('u', function(e, r) { pm.variables.set('a', r.json()); pm.response.json(); });",
    );
}

#[test]
fn a_config_passed_by_name_is_rewritten_where_it_is_declared() {
    check(
        "const base = {url: 'u', headers: {a: '1'}, data: 'x'};\nconst cfg = base;\nbru.sendRequest(cfg, function (e, r) { return r.status; });",
        "const base = {url: 'u', header: {a: '1'}, body: {mode: 'raw', raw: 'x'}};\nconst cfg = base;\npm.sendRequest(cfg, function(e, r) { return r.code; });",
    );
    check(
        "bru.sendRequest(c);\nvar c = {headers: {}};",
        "pm.sendRequest(c);\nvar c = {header: {}};",
    );
    check(
        "let a = b;\nlet b = a;\nbru.sendRequest(a);",
        "let a = b;\nlet b = a;\npm.sendRequest(a);",
    );
    check(
        "const headers = {a: '1'};\nbru.sendRequest({url: 'u', headers});",
        "const headers = {a: '1'};\npm.sendRequest({url: 'u', header: headers});",
    );
    check(
        "const other = {headers: {}};\nbru.sendRequest(cfg);",
        "const other = {headers: {}};\npm.sendRequest(cfg);",
    );
}

#[test]
fn a_send_request_inside_a_callback_is_rewritten_too() {
    check(
        "bru.sendRequest('a', (e, r) => { bru.sendRequest({data: r.data}); });",
        "pm.sendRequest('a', function(e, r) { pm.sendRequest({body: {mode: 'raw', raw: r.json()}}); });",
    );
}

#[test]
fn a_send_request_counts_once_and_its_arguments_count_their_own() {
    let out = translate("bru.sendRequest({data: res.getBody()}, (e, r) => r.data);");
    assert_eq!(out.rewritten, 2);
    assert!(out.leftovers.is_empty(), "{:?}", out.leftovers);
    let out = translate("bru.sendRequest('u', (e, res) => res.data);");
    assert_eq!(
        out.code,
        "pm.sendRequest('u', function(e, res) { return res.json(); });"
    );
    assert_eq!(out.leftovers, ["res.json"]);
}

#[test]
fn configs_by_name_stay_bounded() {
    let chain = (0..5_000)
        .map(|i| format!("const c{} = c{i};\n", i + 1))
        .collect::<String>()
        + "const c0 = {headers: {}};\n"
        + &"bru.sendRequest(c5000);\n".repeat(5_000);
    assert_eq!(bounded(move || translate(&chain).rewritten), 5_000);
}

fn iife(h: &str) -> String {
    format!(
        "(function() {{ const _headers = {h}; for (const key in _headers) {{ pm.request.headers.upsert({{ key: key, value: _headers[key] }}); }} }})()"
    )
}

#[test]
fn set_url_and_set_method_become_assignments() {
    assert_eq!(
        t("req.setUrl(\"https://x.test/b\");"),
        "pm.request.url = \"https://x.test/b\";"
    );
    assert_eq!(t("req.setMethod(m)"), "pm.request.method = m");
    assert_eq!(t("req.setUrl()"), "pm.request.url");
    assert_eq!(t("req.setUrl(a, b)"), "pm.request.url = a");
    assert_eq!(
        t("req.setUrl(bru.getVar('u'));"),
        "pm.request.url = pm.variables.get('u');"
    );
    assert_eq!(
        t("req.setUrl(req.setUrl(u))"),
        "pm.request.url = pm.request.url = u"
    );
}

#[test]
fn an_assignment_is_parenthesized_where_an_operator_would_bind_it() {
    for (src, want) in [
        ("if (x) req.setUrl(u);", "if (x) pm.request.url = u;"),
        ("{ req.setUrl(u) }", "{ pm.request.url = u }"),
        ("f(req.setUrl(u), 1)", "f(pm.request.url = u, 1)"),
        ("const v = req.setUrl(u);", "const v = pm.request.url = u;"),
        ("() => req.setUrl(u)", "() => pm.request.url = u"),
        ("ok ? req.setUrl(u) : 0", "ok ? pm.request.url = u : 0"),
        ("return req.setUrl(u);", "return pm.request.url = u;"),
        (
            "bar()\nreq.setUrl(u)\nfoo()",
            "bar()\npm.request.url = u\nfoo()",
        ),
        ("a + req.setUrl(u)", "a + (pm.request.url = u)"),
        ("a == req.setUrl(u)", "a == (pm.request.url = u)"),
        ("a ?? req.setUrl(u)", "a ?? (pm.request.url = u)"),
        ("!req.setUrl(u)", "!(pm.request.url = u)"),
        ("typeof req.setUrl(u)", "typeof (pm.request.url = u)"),
        ("req.setUrl(u).length", "(pm.request.url = u).length"),
        ("req.setUrl(u) + 1", "(pm.request.url = u) + 1"),
        (
            "const v = req.setUrl(u)\n.trim()",
            "const v = (pm.request.url = u)\n.trim()",
        ),
    ] {
        assert_eq!(t(src), want, "{src:?}");
    }
}

#[test]
fn set_header_and_set_body_take_postman_shapes() {
    assert_eq!(
        t("req.setHeader(\"h\", \"v\");"),
        "pm.request.headers.upsert({ key: \"h\", value: \"v\" });"
    );
    assert_eq!(t("req.setHeader(k)"), "pm.request.headers.upsert(k)");
    assert_eq!(t("req.setHeader()"), "pm.request.headers.upsert()");
    assert_eq!(
        t("req.setHeader(...pair)"),
        "pm.request.headers.upsert(...pair)"
    );
    assert_eq!(
        t("req.setHeader(k, v, extra)"),
        "pm.request.headers.upsert({ key: k, value: v })"
    );
    assert_eq!(
        t("req.setHeader('a', res.getHeader('b'))"),
        "pm.request.headers.upsert({ key: 'a', value: pm.response.headers.get('b') })"
    );
    assert_eq!(
        t("req.setBody({a: 1});"),
        "pm.request.body.update({ mode: \"raw\", raw: JSON.stringify({a: 1}) });"
    );
    assert_eq!(t("req.setBody()"), "pm.request.body");
    assert_eq!(
        t("req.setBody(res.getBody())"),
        "pm.request.body.update({ mode: \"raw\", raw: JSON.stringify(pm.response.json()) })"
    );
}

#[test]
fn set_headers_becomes_an_upsert_loop() {
    assert_eq!(
        t("req.setHeaders({h: \"v\"});"),
        format!("{};", iife("{h: \"v\"}"))
    );
    assert_eq!(t("req.setHeaders()"), "pm.request.headers");
    assert_eq!(
        t("req.setHeaders(res.headers, 2)"),
        iife("pm.response.headers")
    );
}

#[test]
fn a_loop_starting_a_line_after_an_unterminated_one_gets_a_semicolon() {
    let h = iife("h");
    for (src, want) in [
        ("bar()\nreq.setHeaders(h)", format!("bar()\n;{h}")),
        (
            "const a = b\n  req.setHeaders(h)",
            format!("const a = b\n  ;{h}"),
        ),
        ("x = [1]\nreq.setHeaders(h)", format!("x = [1]\n;{h}")),
        ("a;\nreq.setHeaders(h)", format!("a;\n{h}")),
        ("if (x)\n  req.setHeaders(h)", format!("if (x)\n  {h}")),
        (
            "while (x)\n  req.setHeaders(h)",
            format!("while (x)\n  {h}"),
        ),
        (
            "if (x) {} else\n  req.setHeaders(h)",
            format!("if (x) {{}} else\n  {h}"),
        ),
        ("{\nreq.setHeaders(h)\n}", format!("{{\n{h}\n}}")),
        (
            "const v =\n  req.setHeaders(h)",
            format!("const v =\n  {h}"),
        ),
        ("f(a)(req.setHeaders(h))", format!("f(a)({h})")),
        (
            "const o = {a: 1}\nreq.setHeaders(h)",
            format!("const o = {{a: 1}}\n;{h}"),
        ),
        (
            "const f = function() {}\nreq.setHeaders(h)",
            format!("const f = function() {{}}\n;{h}"),
        ),
        ("if (x) {}\nreq.setHeaders(h)", format!("if (x) {{}}\n;{h}")),
    ] {
        assert_eq!(t(src), want, "{src:?}");
    }
}

#[test]
fn stringified_response_body_becomes_response_text() {
    assert_eq!(
        t("const s = JSON.stringify(res.getBody());"),
        "const s = pm.response.text();"
    );
    assert_eq!(t("JSON.stringify(res.getBody('x'))"), "pm.response.text()");
    assert_eq!(
        t("JSON.stringify(res.getBody(), null, 2)"),
        "JSON.stringify(pm.response.json(), null, 2)"
    );
    assert_eq!(
        t("JSON.stringify(res.getBody().a)"),
        "JSON.stringify(pm.response.json().a)"
    );
    assert_eq!(t("JSON.stringify(x)"), "JSON.stringify(x)");
}

#[test]
fn a_spread_where_an_argument_is_reshaped_keeps_the_call() {
    for (src, chain) in [
        ("req.setHeader(...a, b)", "req.setHeader"),
        ("req.setUrl(...u)", "req.setUrl"),
        ("req.setBody(...b)", "req.setBody"),
        ("req.setHeaders(...h)", "req.setHeaders"),
    ] {
        let out = translate(src);
        assert_eq!(out.code, src);
        assert_eq!(out.leftovers, [chain]);
    }
}

#[test]
fn reshaped_calls_count_once_plus_their_arguments() {
    assert_eq!(
        translate("req.setHeader('a', res.getHeader('b'))").rewritten,
        2
    );
    assert_eq!(
        translate("req.setBody(JSON.stringify(res.getBody()))").rewritten,
        2
    );
    let out = translate("const f = req.setUrl;");
    assert_eq!(out.code, "const f = req.setUrl;");
    assert_eq!(out.leftovers, ["req.setUrl"]);
}

#[test]
fn reshaped_input_stays_bounded() {
    assert_eq!(
        bounded(|| translate(&"req.setUrl(u);\n".repeat(50_000)).rewritten),
        50_000
    );
    assert_eq!(
        bounded(|| translate(&"bar()\nreq.setHeaders(h)\n".repeat(20_000)).rewritten),
        20_000
    );
    let deep = format!("{}1{}", "req.setBody(".repeat(5_000), ")".repeat(5_000));
    let out = bounded(move || translate(&deep));
    assert!(!out.kept);
    assert_eq!(out.leftovers, ["req.setBody"]);
}

#[test]
fn a_parenthesized_assignment_starting_a_line_after_an_unterminated_one_gets_a_semicolon() {
    assert_eq!(
        t("x = [1]\nreq.setUrl(u)"),
        "x = [1]\n;(pm.request.url = u)"
    );
    assert_eq!(
        t("const s = 'a'\nreq.setUrl(u).trim()"),
        "const s = 'a'\n;(pm.request.url = u).trim()"
    );
    assert_eq!(t("a +\nreq.setUrl(u)"), "a +\n(pm.request.url = u)");
}
