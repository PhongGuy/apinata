//! `export::html`: walk order, positional anchors, the contents list, folder sections,
//! collection and folder headers and auth, and left-out items.

use crate::export_support;

use export_support::*;

#[test]
fn sections_follow_the_postman_walk_with_positional_anchors() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "x.yml",
            &http_at("late", 2, "GET", "https://x.test/late", INHERIT),
        ),
        (
            "y.yml",
            &http_at("early", 1, "POST", "https://x.test/early", INHERIT),
        ),
        ("g.yml", "info:\n  name: g\n  type: grpc\n  seq: 3\n"),
        ("a/folder.yml", &folder_yml("a", 2, "")),
        ("b/folder.yml", &folder_yml("b", 1, "")),
        ("b/inner/folder.yml", &folder_yml("inner", 1, "")),
        (
            "b/inner/r.yml",
            &http_at("deep", 1, "DELETE", "https://x.test/d", INHERIT),
        ),
        (
            "b/r.yml",
            &http_at("mid", 1, "GET", "https://x.test/m", INHERIT),
        ),
    ]);
    let page = html_page(&c);
    let nav = concat!(
        "<nav>\n<h2>Contents</h2>\n<ul>\n",
        "<li><a href=\"#f-1\">b</a>\n<ul>\n",
        "<li><a href=\"#f-1-1\">inner</a>\n<ul>\n",
        "<li><a href=\"#r-1-1-1\"><span class=\"method m-delete\">DELETE</span> deep</a></li>\n",
        "</ul>\n</li>\n",
        "<li><a href=\"#r-1-2\"><span class=\"method m-get\">GET</span> mid</a></li>\n",
        "</ul>\n</li>\n",
        "<li><a href=\"#f-2\">a</a></li>\n",
        "<li><a href=\"#r-3\"><span class=\"method m-post\">POST</span> early</a></li>\n",
        "<li><a href=\"#r-4\"><span class=\"method m-get\">GET</span> late</a></li>\n",
        "</ul>\n</nav>\n",
    );
    assert!(page.contains(nav), "{page}");
    let ids: Vec<&str> = page
        .split(" id=\"")
        .skip(1)
        .map(|s| &s[..s.find('"').unwrap()])
        .collect();
    assert_eq!(
        ids,
        ["f-1", "f-1-1", "r-1-1-1", "r-1-2", "f-2", "r-3", "r-4"]
    );
    let nested = "<section id=\"f-1\">\n<h2>b</h2>\n<section id=\"f-1-1\">\n<h2>inner</h2>\n<article id=\"r-1-1-1\">\n<h3><span class=\"method m-delete\">DELETE</span> deep</h3>\n";
    assert!(page.contains(nested), "{page}");
    assert!(
        page.contains("</article>\n</section>\n<article id=\"r-1-2\">\n"),
        "{page}"
    );
    assert!(
        page.contains("<section id=\"f-2\">\n<h2>a</h2>\n</section>\n"),
        "{page}"
    );
    assert!(page.contains("</article>\n</main>\n"), "{page}");
    let detail = "only HTTP requests are exported; this gRPC item was left out";
    assert_eq!(html_notes(&c), pairs(&[("g", detail)]));
}

#[test]
fn collection_and_folder_sections_show_docs_headers_and_the_auth_mode_only() {
    let root = ROOT.to_owned()
        + "request:\n  headers:\n    - name: X-C\n      value: c\n      description: Collection header\n    - name: X-Off\n      value: \"0\"\n      disabled: true\n  auth:\n    type: bearer\n    token: tok-secret\n";
    let f = folder_yml(
        "f",
        1,
        "  headers:\n    - name: X-F\n      value: f\n  auth:\n    type: basic\n    username: user-secret\n    password: pw-secret\n",
    ) + "docs:\n  content: About f\n  type: text/markdown\n";
    let (_d, c) = open(&[("opencollection.yml", &root), ("f/folder.yml", &f)]);
    let page = html_page(&c);
    let header = concat!(
        "<h1>c</h1>\n<table>\n<caption>Headers</caption>\n",
        "<thead><tr><th>Name</th><th>Value</th><th>Description</th></tr></thead>\n<tbody>\n",
        "<tr><td>X-C</td><td>c</td><td>Collection header</td></tr>\n",
        "<tr class=\"disabled\"><td>X-Off <span class=\"tag\">disabled</span></td><td>0</td><td></td></tr>\n",
        "</tbody>\n</table>\n<p class=\"auth\">Auth: Bearer token</p>\n</header>\n",
    );
    assert!(page.contains(header), "{page}");
    let folder = "<section id=\"f-1\">\n<h2>f</h2>\n<div class=\"docs\">\n<p>About f</p>\n</div>\n<table>\n<caption>Headers</caption>\n";
    assert!(page.contains(folder), "{page}");
    let tail = "<tr><td>X-F</td><td>f</td><td></td></tr>\n</tbody>\n</table>\n<p class=\"auth\">Auth: Basic</p>\n</section>\n";
    assert!(page.contains(tail), "{page}");
    for secret in ["tok-secret", "user-secret", "pw-secret"] {
        assert!(!page.contains(secret), "{secret}");
    }
}

#[test]
fn pass_through_auth_and_empty_headers_show_nothing() {
    for under in ["  auth: inherit\n", "  auth: none\n", "  headers: []\n"] {
        let root = format!("{ROOT}request:\n{under}");
        let f = folder_yml("f", 1, under);
        let (_d, c) = open(&[("opencollection.yml", &root), ("f/folder.yml", &f)]);
        let page = html_page(&c);
        assert!(!page.contains("class=\"auth\""), "{under}: {page}");
        assert!(!page.contains("<caption>"), "{under}: {page}");
    }
}

#[test]
fn literal_credential_headers_are_noted_where_they_are_written() {
    let root = ROOT.to_owned()
        + "request:\n  headers:\n    - name: X-API-Key\n      value: k-1\n    - name: Authorization\n      value: \"{{token}}\"\n";
    let f = folder_yml(
        "f",
        1,
        "  headers:\n    - name: cookie\n      value: a=1\n      disabled: true\n    - name: Api-Key\n      value: \" \"\n    - name: X-Other\n      value: plain\n",
    );
    let (_d, c) = open(&[("opencollection.yml", &root), ("f/folder.yml", &f)]);
    let tail = "holds a literal credential, written into the page";
    assert_eq!(
        html_notes(&c),
        vec![
            (
                "collection / headers".to_owned(),
                format!("header X-API-Key {tail}")
            ),
            ("f / headers".to_owned(), format!("header cookie {tail}")),
        ]
    );
    assert!(html_page(&c).contains("<td>k-1</td>"));
}

#[test]
fn hostile_folder_names_docs_headers_and_auth_modes_stay_text() {
    let f = format!(
        "info:\n  name: {HOSTILE_YAML}\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: {HOSTILE_YAML}\n      value: {HOSTILE_YAML}\n  auth:\n    type: {HOSTILE_YAML}\n{HOSTILE_DOCS}"
    );
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("f/folder.yml", &f)]);
    let page = html_page(&c);
    assert_inert(&page);
    let e = HOSTILE_ESCAPED;
    assert!(
        page.contains(&format!("<a href=\"#f-1\">{e}</a>")),
        "{page}"
    );
    assert!(page.contains(&format!("<h2>{e}</h2>")), "{page}");
    assert!(
        page.contains(&format!("<tr><td>{e}</td><td>{e}</td><td></td></tr>")),
        "{page}"
    );
    assert!(
        page.contains(&format!("<p class=\"auth\">Auth: {e}</p>")),
        "{page}"
    );
    assert!(
        page.contains("&quot;&gt;&lt;img src=x onerror=alert(1)&gt;</p>"),
        "{page}"
    );
}
