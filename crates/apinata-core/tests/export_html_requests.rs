//! `export::html` request articles: URL, docs, params, headers, bodies, auth and notes.

use crate::export_support;

use export_support::*;

const HEAD_ROW: &str =
    "<thead><tr><th>Name</th><th>Value</th><th>Description</th></tr></thead>\n<tbody>\n";

#[test]
fn a_request_shows_its_parts_in_order_with_values_as_written() {
    let under = "  params:\n    - name: id\n      value: \"7\"\n      type: path\n    - name: page\n      value: \"2\"\n      type: query\n      description: Page number\n    - name: debug\n      value: \"1\"\n      type: query\n      disabled: true\n  headers:\n    - name: X-Trace\n      value: \"{{trace}}\"\n  body:\n    type: json\n    data: '{\"a\": \"<b>\"}'\n  auth:\n    type: bearer\n    token: tok-secret\n";
    let req = http("Get user", "POST", "{{baseUrl}}/users/:id?page=2", under)
        + "docs: Fetches *one* user.\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
    let page = html_page(&c);
    let want = [
        "<article id=\"r-1\">\n<h3><span class=\"method m-post\">POST</span> Get user</h3>\n",
        "<p class=\"url\"><code>{{baseUrl}}/users/:id?page=2</code></p>\n",
        "<div class=\"docs\">\n<p>Fetches <em>one</em> user.</p>\n</div>\n",
        "<table>\n<caption>Path params</caption>\n", HEAD_ROW,
        "<tr><td>id</td><td>7</td><td></td></tr>\n</tbody>\n</table>\n",
        "<table>\n<caption>Query params</caption>\n", HEAD_ROW,
        "<tr><td>page</td><td>2</td><td>Page number</td></tr>\n",
        "<tr class=\"disabled\"><td>debug <span class=\"tag\">disabled</span></td><td>1</td><td></td></tr>\n</tbody>\n</table>\n",
        "<table>\n<caption>Headers</caption>\n", HEAD_ROW,
        "<tr><td>X-Trace</td><td>{{trace}}</td><td></td></tr>\n</tbody>\n</table>\n",
        "<p class=\"part\">Body (json)</p>\n<pre>{&quot;a&quot;: &quot;&lt;b&gt;&quot;}</pre>\n",
        "<p class=\"auth\">Auth: Bearer token</p>\n</article>\n",
    ]
    .concat();
    assert!(page.contains(&want), "{page}");
    assert!(!page.contains("tok-secret"));
    assert!(html_notes(&c).is_empty());
}

#[test]
fn each_body_mode_has_its_own_shape() {
    let form = "<table>\n<caption>Body (form-urlencoded)</caption>\n".to_owned()
        + HEAD_ROW
        + "<tr><td>a</td><td>1</td><td></td></tr>\n<tr class=\"disabled\"><td>b <span class=\"tag\">disabled</span></td><td>x</td><td></td></tr>\n</tbody>\n</table>\n";
    let multipart = "<table>\n<caption>Body (multipart-form)</caption>\n".to_owned()
        + HEAD_ROW
        + "<tr><td>a</td><td>1</td><td></td></tr>\n<tr><td>up</td><td>file: /data/up.bin, /data/two.bin</td><td></td></tr>\n</tbody>\n</table>\n";
    let cases = [
        ("  body:\n    type: xml\n    data: <a/>\n", "<p class=\"part\">Body (xml)</p>\n<pre>&lt;a/&gt;</pre>\n".to_owned()),
        ("  body:\n    type: text\n    data: hi\n", "<p class=\"part\">Body (text)</p>\n<pre>hi</pre>\n".to_owned()),
        ("  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: \"1\"\n      - name: b\n        value: x\n        disabled: true\n", form),
        ("  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: \"1\"\n      - name: up\n        type: file\n        value: [/data/up.bin, /data/two.bin]\n", multipart),
        ("  body:\n    type: file\n    data:\n      - filePath: /data/a.png\n      - filePath: /data/b.png\n        selected: true\n",
            "<p class=\"part\">Body (file)</p>\n<ul>\n<li>/data/a.png</li>\n<li>/data/b.png <span class=\"tag\">selected</span></li>\n</ul>\n".to_owned()),
        ("  body:\n    type: graphql\n", "<p class=\"part\">Body (graphql)</p>\n<p class=\"auth\">".to_owned()),
        ("  body:\n    type: json\n    data: \"  \"\n", "</h3>\n<p class=\"url\"><code>https://x.test/r</code></p>\n<p class=\"auth\">".to_owned()),
    ];
    for (i, (body, want)) in cases.iter().enumerate() {
        let req = http("r", "POST", "https://x.test/r", &format!("{body}{INHERIT}"));
        let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
        assert!(
            html_page(&c).contains(want.as_str()),
            "case {i}: {}",
            html_page(&c)
        );
        let want_notes = match body.contains("graphql") {
            true => pairs(&[("r / body", "the graphql body cannot be shown")]),
            false => Vec::new(),
        };
        assert_eq!(html_notes(&c), want_notes, "case {i}");
    }
}

#[test]
fn auth_shows_the_mode_label_and_never_a_value() {
    let cases = [
        ("  auth: none\n", "No auth"),
        (
            "  auth:\n    type: basic\n    username: u-secret\n    password: p-secret\n",
            "Basic",
        ),
        (
            "  auth:\n    type: bearer\n    token: t-secret\n",
            "Bearer token",
        ),
        (
            "  auth:\n    type: apikey\n    key: X-Key\n    value: k-secret\n    placement: header\n",
            "API key",
        ),
        (
            "  auth:\n    type: digest\n    username: u-secret\n    password: p-secret\n",
            "Digest",
        ),
        (
            "  auth:\n    type: oauth1\n    consumerKey: ck-secret\n    consumerSecret: cs-secret\n",
            "OAuth 1.0",
        ),
        (
            "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: https://auth.test/token\n    credentials:\n      clientId: id-secret\n      clientSecret: cs-secret\n",
            "OAuth 2.0",
        ),
        (
            "  auth:\n    type: awsv4\n    accessKeyId: ak-secret\n    secretAccessKey: sk-secret\n",
            "AWS Signature v4",
        ),
        (
            "  auth:\n    type: ntlm\n    username: u-secret\n    password: p-secret\n    domain: d-secret\n",
            "NTLM",
        ),
        ("  auth:\n    type: hawk\n", "hawk"),
    ];
    for (auth, label) in cases {
        let (_d, c) = open(&[
            ("opencollection.yml", ROOT),
            ("r.yml", &http("r", "GET", "https://x.test/r", auth)),
        ]);
        let page = html_page(&c);
        assert!(
            page.contains(&format!(
                "<p class=\"auth\">Auth: {label}</p>\n</article>\n"
            )),
            "{label}: {page}"
        );
        assert!(!page.contains("-secret"), "{label}");
    }
}

#[test]
fn inherited_auth_names_the_mode_it_resolves_to() {
    let root = ROOT.to_owned() + "request:\n  auth:\n    type: bearer\n    token: t-secret\n";
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("a/folder.yml", &folder_yml("a", 1, "  auth: none\n")),
        ("a/r.yml", &http("ra", "GET", "https://x.test/a", INHERIT)),
        (
            "b/folder.yml",
            &folder_yml(
                "b",
                2,
                "  auth:\n    type: basic\n    username: u\n    password: p-secret\n",
            ),
        ),
        ("b/r.yml", &http("rb", "GET", "https://x.test/b", INHERIT)),
    ]);
    let page = html_page(&c);
    assert!(
        article(&page, "r-1-1").contains("<p class=\"auth\">Auth: Inherited (Bearer token)</p>"),
        "{page}"
    );
    assert!(
        article(&page, "r-2-1").contains("<p class=\"auth\">Auth: Inherited (Basic)</p>"),
        "{page}"
    );
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", "https://x.test/r", INHERIT)),
    ]);
    assert!(html_page(&c).contains("<p class=\"auth\">Auth: Inherited (No auth)</p>"));
}

#[test]
fn request_notes_follow_collection_and_folder_notes() {
    let root = ROOT.to_owned()
        + "request:\n  headers:\n    - name: Authorization\n      value: Basic abc\n";
    let under = "  headers:\n    - name: PROXY-AUTHORIZATION\n      value: Basic xyz\n    - name: Cookie\n      value: \"sid={{sid}}\"\n  body:\n    type: graphql\n  auth: inherit\n";
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", &folder_yml("f", 1, "")),
        ("f/r.yml", &http("r", "GET", "https://x.test/r", under)),
    ]);
    let tail = "holds a literal credential, written into the page";
    assert_eq!(
        html_notes(&c),
        vec![
            (
                "collection / headers".to_owned(),
                format!("header Authorization {tail}")
            ),
            (
                "f / r / headers".to_owned(),
                format!("header PROXY-AUTHORIZATION {tail}")
            ),
            (
                "f / r / body".to_owned(),
                "the graphql body cannot be shown".to_owned()
            ),
        ]
    );
    assert!(html_page(&c).contains("<td>Basic xyz</td>"));
}

#[test]
fn hostile_request_text_stays_text() {
    let req = format!(
        "info:\n  name: {H}\n  type: http\n  seq: 1\nhttp:\n  method: {H}\n  url: {H}\n  params:\n    - name: {H}\n      value: {H}\n      type: query\n      description: {H}\n  headers:\n    - name: {H}\n      value: {H}\n  body:\n    type: text\n    data: {H}\n  auth: inherit\n{HOSTILE_DOCS}",
        H = HOSTILE_YAML
    );
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
    let page = html_page(&c);
    assert_inert(&page);
    let e = HOSTILE_ESCAPED;
    for want in [
        format!("<h3><span class=\"method m-other\">{e}</span> {e}</h3>"),
        format!("<code>{e}</code>"),
        format!("<tr><td>{e}</td><td>{e}</td><td>{e}</td></tr>"),
        format!("<tr><td>{e}</td><td>{e}</td><td></td></tr>"),
        format!("<pre>{e}</pre>"),
        "&quot;&gt;&lt;img src=x onerror=alert(1)&gt;</p>".to_owned(),
    ] {
        assert!(page.contains(&want), "{want} in {page}");
    }
}

#[test]
fn a_graphql_request_shows_its_query_and_variables() {
    let g = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"https://x.test/graphql\"\n  body:\n    query: \"{ me }\"\n    variables: |-\n      {\n        \"a\": 1\n      }\n";
    let q = "info:\n  name: q\n  type: graphql\n  seq: 2\ngraphql:\n  url: \"https://x.test/graphql\"\n  body:\n    query: \"{ you }\"\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("g.yml", g), ("q.yml", q)]);
    let page = html_page(&c);
    assert!(page.contains("<p class=\"part\">Body (graphql)</p>\n<pre>{ me }</pre>\n<p class=\"part\">Variables</p>\n<pre>"), "{page}");
    assert!(page.contains("<pre>{ you }</pre>\n"), "{page}");
    assert_eq!(
        page.matches("<p class=\"part\">Variables</p>").count(),
        1,
        "blank variables are left out"
    );
    assert!(
        html_notes(&c).iter().all(|(s, _)| !s.ends_with("/ body")),
        "{:?}",
        html_notes(&c)
    );
    assert_inert(&page);
}

#[test]
fn footnotes_in_two_requests_keep_ids_unique_and_headings_in_order() {
    let note = "docs: \"x[^n]\\n\\n[^n]: note\"\n";
    let a = http_at("A", 1, "GET", "https://x.test/a", INHERIT) + note;
    let b = http_at("B", 2, "GET", "https://x.test/b", INHERIT) + note;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)]);
    let page = html_page(&c);
    // Every id is unique, and each footnote call points at its own label.
    let mut ids = Vec::new();
    for part in page.split(" id=\"").skip(1) {
        ids.push(part.split('"').next().unwrap().to_owned());
    }
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "duplicate ids in {ids:?}");
    assert!(!page.contains("\"footnote-label\""), "{page}");
    // The label sits one level below its request's h3 heading, never an h2.
    assert!(page.contains("<h4 id=\"d-r-1-footnote-label\""), "{page}");
    assert!(
        page.contains("aria-describedby=\"d-r-2-footnote-label\""),
        "{page}"
    );
    assert!(!page.contains("<h2 id=\"d-"), "{page}");
}

#[test]
fn a_literal_credential_query_param_gets_a_note() {
    let under = "  params:\n    - name: api_key\n      value: k-1\n      type: query\n    - name: token\n      value: \"{{tok}}\"\n      type: query\n    - name: page\n      value: \"2\"\n      type: query\n  auth: inherit\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", "https://x.test/r", under)),
    ]);
    assert_eq!(
        html_notes(&c),
        vec![(
            "r / params".to_owned(),
            "query param api_key holds a literal credential, written into the page".to_owned()
        )]
    );
}

#[test]
fn a_literal_credential_in_the_url_query_gets_a_note_when_params_do_not_list_it() {
    let under =
        "  params:\n    - name: page\n      value: \"2\"\n      type: query\n  auth: inherit\n";
    let url = "https://x.test/r?page=2&access_token=t-1&token={{t}}";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", url, under)),
    ]);
    assert_eq!(
        html_notes(&c),
        vec![(
            "r / url".to_owned(),
            "query param access_token holds a literal credential, written into the page".to_owned()
        )]
    );
}

#[test]
fn url_credentials_match_param_names_decoded_and_case_insensitively() {
    // A param of the same name with a templated value does not hide the literal one in the URL.
    let under = "  params:\n    - name: API_KEY\n      value: \"{{k}}\"\n      type: query\n  auth: inherit\n";
    let url = "https://x.test/r?api_key=k-1&%74oken=t-2";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", url, under)),
    ]);
    let tail = "holds a literal credential, written into the page";
    assert_eq!(
        html_notes(&c),
        vec![
            ("r / url".to_owned(), format!("query param api_key {tail}")),
            ("r / url".to_owned(), format!("query param token {tail}")),
        ]
    );
}

#[test]
fn a_literal_credential_in_both_url_and_params_differing_in_case_is_noted_once() {
    let under =
        "  params:\n    - name: Api_Key\n      value: k-1\n      type: query\n  auth: inherit\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &http("r", "GET", "https://x.test/r?api_key=k-1", under),
        ),
    ]);
    assert_eq!(html_notes(&c).len(), 1);
}
