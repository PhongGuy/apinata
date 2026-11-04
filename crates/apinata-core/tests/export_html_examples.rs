//! `export::html` saved examples: request and response halves, binary bodies, escaping.

use crate::export_support;

use export_support::*;

const EXAMPLES: &str = "examples:\n  - name: created\n    description: A new post\n    request:\n      url: https://x.test/p?draft=1\n      method: POST\n      headers:\n        - name: Content-Type\n          value: application/json\n      params:\n        - name: draft\n          value: \"1\"\n          type: query\n      body:\n        type: json\n        data: '{\"t\": 1}'\n    response:\n      status: 201\n      statusText: Created\n      headers:\n        - name: Content-Type\n          value: application/json\n      body:\n        type: json\n        data: '{\"id\": 7}'\n  - name: png\n    response:\n      status: 200\n      body:\n        type: binary\n        data: iVBORw0K\n  - name: bare\n";

#[test]
fn examples_show_both_halves_and_leave_binary_bodies_out() {
    let req = http("r", "POST", "https://x.test/p", INHERIT) + EXAMPLES;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
    let page = html_page(&c);
    let head = "<thead><tr><th>Name</th><th>Value</th><th>Description</th></tr></thead>\n<tbody>\n";
    let want = [
        "<p class=\"auth\">Auth: Inherited (No auth)</p>\n",
        "<div class=\"example\">\n<h4>Example: created</h4>\n<p>A new post</p>\n",
        "<p class=\"part\">Request</p>\n",
        "<p class=\"url\"><span class=\"method m-post\">POST</span> <code>https://x.test/p?draft=1</code></p>\n",
        "<table>\n<caption>Query params</caption>\n", head,
        "<tr><td>draft</td><td>1</td><td></td></tr>\n</tbody>\n</table>\n",
        "<table>\n<caption>Headers</caption>\n", head,
        "<tr><td>Content-Type</td><td>application/json</td><td></td></tr>\n</tbody>\n</table>\n",
        "<p class=\"part\">Body (json)</p>\n<pre>{&quot;t&quot;: 1}</pre>\n",
        "<p class=\"part\">Response</p>\n<p class=\"status\">201 Created</p>\n",
        "<table>\n<caption>Response headers</caption>\n<thead><tr><th>Name</th><th>Value</th></tr></thead>\n<tbody>\n",
        "<tr><td>Content-Type</td><td>application/json</td></tr>\n</tbody>\n</table>\n",
        "<p class=\"part\">Body (json)</p>\n<pre>{&quot;id&quot;: 7}</pre>\n</div>\n",
        "<div class=\"example\">\n<h4>Example: png</h4>\n<p class=\"part\">Response</p>\n<p class=\"status\">200</p>\n</div>\n",
        "<div class=\"example\">\n<h4>Example: bare</h4>\n</div>\n</article>\n",
    ]
    .concat();
    assert!(page.contains(&want), "{page}");
    assert!(!page.contains("iVBORw0K"));
    assert_eq!(
        html_notes(&c),
        pairs(&[(
            "r / examples",
            "the binary body of example png was left out"
        )])
    );
}

#[test]
fn a_status_text_alone_and_each_text_kind_show() {
    let ex = "examples:\n  - name: x\n    response:\n      statusText: Teapot\n      body:\n        type: xml\n        data: <t/>\n  - name: h\n    response:\n      body:\n        type: html\n        data: <p>hi</p>\n";
    let req = http("r", "GET", "https://x.test/r", INHERIT) + ex;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
    let page = html_page(&c);
    assert!(page.contains("<p class=\"status\">Teapot</p>\n<p class=\"part\">Body (xml)</p>\n<pre>&lt;t/&gt;</pre>\n"), "{page}");
    assert!(page.contains("<p class=\"part\">Response</p>\n<p class=\"part\">Body (html)</p>\n<pre>&lt;p&gt;hi&lt;/p&gt;</pre>\n"), "{page}");
}

#[test]
fn hostile_example_text_stays_text() {
    let ex = format!(
        "examples:\n  - name: {H}\n    description: {H}\n    request:\n      url: {H}\n      method: {H}\n      body:\n        type: text\n        data: {H}\n    response:\n      status: 200\n      statusText: {H}\n      headers:\n        - name: {H}\n          value: {H}\n      body:\n        type: html\n        data: {H}\n",
        H = HOSTILE_YAML
    );
    let req = http("r", "GET", "https://x.test/r", INHERIT) + &ex;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
    let page = html_page(&c);
    assert_inert(&page);
    let e = HOSTILE_ESCAPED;
    for want in [
        format!("<h4>Example: {e}</h4>\n<p>{e}</p>\n"),
        format!("<span class=\"method m-other\">{e}</span> <code>{e}</code>"),
        format!("<p class=\"status\">200 {e}</p>"),
        format!("<tr><td>{e}</td><td>{e}</td></tr>"),
        format!("<p class=\"part\">Body (html)</p>\n<pre>{e}</pre>"),
    ] {
        assert!(page.contains(&want), "{want} in {page}");
    }
}

#[test]
fn a_blank_response_body_is_omitted() {
    let ex = "examples:\n  - name: gone\n    response:\n      status: 204\n      body:\n        type: text\n        data: \"\"\n  - name: ws\n    response:\n      status: 200\n      body:\n        type: json\n        data: \" \\n\\t\"\n";
    let req = http("r", "GET", "https://x.test/r", INHERIT) + ex;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
    let page = html_page(&c);
    let want = [
        "<div class=\"example\">\n<h4>Example: gone</h4>\n<p class=\"part\">Response</p>\n<p class=\"status\">204</p>\n</div>\n",
        "<div class=\"example\">\n<h4>Example: ws</h4>\n<p class=\"part\">Response</p>\n<p class=\"status\">200</p>\n</div>\n",
    ]
    .concat();
    assert!(page.contains(&want), "{page}");
    assert!(html_notes(&c).is_empty());
}

#[test]
fn an_example_request_header_with_a_literal_credential_gets_a_note() {
    let ex = "examples:\n  - name: keyed\n    request:\n      url: https://x.test/p\n      method: GET\n      headers:\n        - name: X-Api-Key\n          value: abc123\n        - name: Cookie\n          value: \"{{session}}\"\n        - name: X-Trace\n          value: t1\n";
    let req = http("r", "GET", "https://x.test/p", INHERIT) + ex;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
    assert_eq!(
        html_notes(&c),
        pairs(&[(
            "r / examples",
            "example keyed: header X-Api-Key holds a literal credential, written into the page"
        )])
    );
}

#[test]
fn an_example_request_query_param_with_a_literal_credential_gets_a_note() {
    let ex = "examples:\n  - name: keyed\n    request:\n      url: https://x.test/p\n      method: GET\n      params:\n        - name: access_token\n          value: abc123\n          type: query\n        - name: token\n          value: \"{{t}}\"\n          type: query\n        - name: page\n          value: \"1\"\n          type: query\n";
    let req = http("r", "GET", "https://x.test/p", INHERIT) + ex;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
    assert_eq!(
        html_notes(&c),
        pairs(&[(
            "r / examples",
            "example keyed: query param access_token holds a literal credential, written into the page"
        )])
    );
}
