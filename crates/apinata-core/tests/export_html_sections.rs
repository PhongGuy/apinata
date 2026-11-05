//! `export::html_with`: the opt-in scripts and environments sections.

use crate::export_support;

use apinata_core::export::{HtmlOptions, html, html_with};
use export_support::*;

const RUNTIME: &str = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n    - expression: res.body.x\n      operator: isEmpty\n      disabled: true\n  scripts:\n    - type: before-request\n      code: |-\n        const pre = 1;\n    - type: tests\n      code: |-\n        test(\"<script>alert(1)</script>\", () => {});\n";

fn scripted() -> (tempfile::TempDir, apinata_core::Collection) {
    let req = http("r", "GET", "https://x.test/r", "") + RUNTIME;
    open(&[("opencollection.yml", ROOT), ("r.yml", &req)])
}

#[test]
fn default_options_give_the_same_bytes_as_html() {
    let (_d, c) = scripted();
    assert_eq!(html_with(&c, &HtmlOptions::new()), html(&c));
}

#[test]
fn scripts_stay_out_unless_asked_for() {
    let (_d, c) = scripted();
    let page = html_page(&c);
    assert!(!page.contains("const pre"), "{page}");
    assert!(!page.contains("res.status"), "{page}");
}

#[test]
fn include_scripts_shows_assertions_and_scripts_escaped() {
    let (_d, c) = scripted();
    let page = html_with(&c, &HtmlOptions::new().include_scripts(true)).html;
    let want = [
        "<p class=\"part\">Assertions</p>\n<ul>\n",
        "<li><code>res.status eq 200</code></li>\n",
        "<li><code>res.body.x isEmpty</code> <span class=\"tag\">disabled</span></li>\n",
        "<p class=\"part\">Script (pre-request)</p>\n<pre>const pre = 1;</pre>\n",
        "<p class=\"part\">Script (tests)</p>\n<pre>test(&quot;&lt;script&gt;alert(1)&lt;/script&gt;&quot;, () =&gt; {});</pre>\n",
    ];
    for piece in want {
        assert!(page.contains(piece), "missing {piece:?} in {page}");
    }
    assert_inert(&page);
    assert!(!page.contains("Environments"), "{page}");
}

const ENV: &str = "name: dev\nvariables:\n  - name: host\n    value: \"{{base}}/<b>\"\n  - name: token\n    secret: true\n    value: hunter2-from-file\n  - name: off\n    value: gone\n    disabled: true\n";

fn with_env() -> (tempfile::TempDir, apinata_core::Collection) {
    let req = http("r", "GET", "https://x.test/r", "") + RUNTIME;
    open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &req),
        ("environments/dev.yml", ENV),
        (".env", "token=hunter2-from-dotenv\n"),
    ])
}

#[test]
fn include_environments_lists_values_as_stored_and_masks_secrets() {
    let (_d, c) = with_env();
    let page = html_with(&c, &HtmlOptions::new().include_environments(true)).html;
    let want = [
        "<a href=\"#environments\">Environments</a>",
        "<section id=\"environments\">\n<h2>Environments</h2>\n",
        "<caption>dev</caption>",
        "<tr><td>host</td><td>{{base}}/&lt;b&gt;</td></tr>\n",
        "<tr><td>token</td><td><span class=\"tag\">secret</span></td></tr>\n",
    ];
    for piece in want {
        assert!(page.contains(piece), "missing {piece:?} in {page}");
    }
    assert!(!page.contains("hunter2"), "{page}");
    assert!(!page.contains("gone"), "{page}");
    assert!(!page.contains("res.status"), "{page}");
    assert_inert(&page);
}

#[test]
fn both_flags_together_keep_the_csp_and_fetch_nothing() {
    let (_d, c) = with_env();
    let page = html_with(
        &c,
        &HtmlOptions::new()
            .include_scripts(true)
            .include_environments(true),
    )
    .html;
    let plain = html_page(&c);
    let csp = |p: &str| {
        p.lines()
            .find(|l| l.contains("Content-Security-Policy"))
            .map(str::to_owned)
    };
    assert_eq!(csp(&page), csp(&plain));
    assert!(
        !page.contains("src=") && !page.contains("href=\"http"),
        "{page}"
    );
    assert!(!page.contains("hunter2"), "{page}");
}

fn position(page: &str, needle: &str) -> usize {
    page.find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?} in {page}"))
}

#[test]
fn environment_rows_follow_file_order_with_inherited_rows_last() {
    let base = "name: base\nvariables:\n  - name: aaa_inherited\n    value: i\n  - name: kept\n    value: base\n";
    let dev = "name: dev\nextends: base\nvariables:\n  - name: zeta\n    value: z\n  - name: alpha\n    value: a\n  - name: kept\n    value: dev\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("environments/base.yml", base),
        ("environments/dev.yml", dev),
    ]);
    let page = html_with(&c, &HtmlOptions::new().include_environments(true)).html;
    let dev_part = &page[position(&page, "<caption>dev</caption>")..];
    let order: Vec<usize> = [
        "<td>zeta</td>",
        "<td>alpha</td>",
        "<td>kept</td>",
        "<td>aaa_inherited</td>",
    ]
    .iter()
    .map(|n| position(dev_part, n))
    .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{dev_part}");
    assert!(dev_part.contains("<td>kept</td><td>dev</td>"), "{dev_part}");
}

#[test]
fn an_inheritance_warning_is_noted_once() {
    let a = "name: a\nextends: lost\nvariables:\n  - name: x\n    value: 1\n";
    let b = "name: b\nextends: a\nvariables:\n  - name: y\n    value: 2\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("environments/a.yml", a),
        ("environments/b.yml", b),
    ]);
    let out = html_with(&c, &HtmlOptions::new().include_environments(true));
    let lost: Vec<_> = out
        .notes
        .iter()
        .filter(|n| n.detail.contains("lost"))
        .collect();
    assert_eq!(lost.len(), 1, "{:?}", out.notes);
}
