//! `export::html`: page skeleton, collection header, escaping and template hygiene.

use crate::export_support;

use apinata_core::export::html;
use export_support::*;

const HEAD: &str = concat!(
    "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n",
    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src data:\">\n",
    "<title>c</title>\n<style>\n",
);
const TAIL: &str = "</style>\n</head>\n<body>\n<header>\n<h1>c</h1>\n</header>\n<nav>\n<h2>Contents</h2>\n<ul>\n</ul>\n</nav>\n<main>\n</main>\n</body>\n</html>\n";

#[test]
fn an_empty_collection_is_a_complete_page_with_an_empty_contents_list() {
    let (_d, c) = open(&[("opencollection.yml", ROOT)]);
    let out = html(&c);
    assert!(out.html.starts_with(HEAD), "{}", out.html);
    assert!(out.html.ends_with(TAIL), "{}", out.html);
    assert!(out.html.contains("@media (prefers-color-scheme: dark)"));
    assert!(out.notes.is_empty());
}

#[test]
fn collection_docs_render_as_markdown_under_the_title() {
    let root = format!("{ROOT}docs:\n  content: About *c*\n  type: text/markdown\n");
    let (_d, c) = open(&[("opencollection.yml", &root)]);
    let page = html_page(&c);
    let want = "<h1>c</h1>\n<div class=\"docs\">\n<p>About <em>c</em></p>\n</div>\n</header>\n";
    assert!(page.contains(want), "{page}");
    let root = format!("{ROOT}docs: \"  \"\n");
    let (_d, c) = open(&[("opencollection.yml", &root)]);
    assert!(html_page(&c).contains("<h1>c</h1>\n</header>\n"));
}

#[test]
fn a_hostile_collection_name_and_docs_stay_text() {
    let root = format!("opencollection: \"1.0.0\"\ninfo:\n  name: {HOSTILE_YAML}\n{HOSTILE_DOCS}");
    let (_d, c) = open(&[("opencollection.yml", &root)]);
    let page = html_page(&c);
    assert_inert(&page);
    assert!(
        page.contains(&format!("<title>{HOSTILE_ESCAPED}</title>")),
        "{page}"
    );
    assert!(
        page.contains(&format!("<h1>{HOSTILE_ESCAPED}</h1>")),
        "{page}"
    );
    assert!(
        page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "{page}"
    );
    assert!(
        page.contains("&quot;&gt;&lt;img src=x onerror=alert(1)&gt;</p>"),
        "{page}"
    );
    assert!(page.contains("<a href=\"\">x</a>"), "{page}");
}

/// One folder with one request with every part filled, and no `src=`, `href=` or the
/// reference app's name in any file.
fn plain_collection() -> (tempfile::TempDir, apinata_core::Collection) {
    let root = ROOT.to_owned()
        + "request:\n  headers:\n    - name: X-C\n      value: c\n  auth:\n    type: bearer\n    token: t\n";
    let folder = folder_yml("f", 1, "  headers:\n    - name: X-F\n      value: f\n");
    let under = "  params:\n    - name: q\n      value: \"1\"\n      type: query\n  headers:\n    - name: X-R\n      value: r\n      disabled: true\n  body:\n    type: json\n    data: '{\"a\": 1}'\n  auth: inherit\n";
    let req = http("r", "POST", "https://x.test/r?q=1", under)
        + "examples:\n  - name: ok\n    request:\n      url: https://x.test/r\n      method: POST\n    response:\n      status: 200\n      statusText: OK\n      headers:\n        - name: Content-Type\n          value: application/json\n      body:\n        type: json\n        data: '{\"id\": 1}'\n";
    open(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", &folder),
        ("f/r.yml", &req),
    ])
}

#[test]
fn the_template_fetches_nothing() {
    let (_d, c) = plain_collection();
    let page = html_page(&c);
    assert!(!page.contains("src="), "{page}");
    assert!(
        page.split("href=").skip(1).all(|h| h.starts_with("\"#")),
        "{page}"
    );
}

#[test]
fn the_template_never_names_the_reference_app() {
    let (_d, c) = plain_collection();
    assert!(!html_page(&c).to_ascii_lowercase().contains("bruno"));
}

#[test]
fn a_docs_heading_that_repeats_the_collection_name_is_not_written_twice() {
    for docs in ["# c\\n\\nAbout", "  ## C ##\\nAbout", "\\n# c\\nAbout"] {
        let root = format!("{ROOT}docs: \"{docs}\"\n");
        let (_d, c) = open(&[("opencollection.yml", &root)]);
        let page = html_page(&c);
        let want = "<h1>c</h1>\n<div class=\"docs\">\n<p>About</p>\n</div>\n</header>\n";
        assert!(page.contains(want), "{docs}: {page}");
    }
    let root = format!("{ROOT}docs: \"# c\"\n");
    let (_d, c) = open(&[("opencollection.yml", &root)]);
    assert!(html_page(&c).contains("<h1>c</h1>\n</header>\n"));
}

#[test]
fn a_docs_heading_that_differs_from_the_collection_name_stays() {
    for (docs, kept) in [
        ("# Overview\\n\\nAbout", "<h1>Overview</h1>"),
        ("# c api\\nAbout", "<h1>c api</h1>"),
        ("About\\n\\n# c", "<h1>c</h1>\n</div>"),
    ] {
        let root = format!("{ROOT}docs: \"{docs}\"\n");
        let (_d, c) = open(&[("opencollection.yml", &root)]);
        let page = html_page(&c);
        assert!(page.contains(kept), "{docs}: {page}");
    }
}
