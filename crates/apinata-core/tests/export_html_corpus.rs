//! `export::html` over the export corpus: snapshots, self-containment and the size budget.

use crate::export_support;

use apinata_core::Collection;
use apinata_core::export::{HtmlExport, html};
use export_support::*;

/// The demo page's byte budget: about twice its length when its snapshot was first accepted.
const DEMO_MAX_BYTES: usize = 20_000;

/// The page, then one `source: detail` line per note.
fn rendered(out: &HtmlExport) -> String {
    let mut text = format!("{}--- notes\n", out.html);
    for n in &out.notes {
        text.push_str(&format!("{}: {}\n", n.source, n.detail));
    }
    text
}

#[test]
fn every_corpus_page_matches_its_snapshot() {
    for (label, root) in corpus() {
        let out = html(&Collection::open(&root).expect("opens"));
        insta::assert_snapshot!(format!("html-{label}"), rendered(&out));
    }
}

#[test]
fn every_corpus_page_is_self_contained() {
    let csp = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src data:\">\n";
    for (label, root) in corpus() {
        let page = html(&Collection::open(&root).expect("opens")).html;
        let lower = page.to_ascii_lowercase();
        for tag in ["<script", "<link", "<iframe", "<object", "<embed"] {
            assert!(!lower.contains(tag), "{label}: {tag}");
        }
        assert_eq!(page.matches(csp).count(), 1, "{label}");
        assert!(page.ends_with("</html>\n"), "{label}");
    }
}

#[test]
fn the_demo_page_stays_within_its_size_budget() {
    let (_, root) = corpus()
        .into_iter()
        .find(|(l, _)| *l == "demo")
        .expect("demo");
    let len = html(&Collection::open(&root).expect("opens")).html.len();
    assert!(len < DEMO_MAX_BYTES, "the demo page is {len} bytes");
}
