//! insta snapshots of every corpus collection's export; each snapshot is reviewed by hand
//! against the export rules before it is accepted.

use crate::export_support;

use apinata_core::Collection;
use apinata_core::export::{
    PostmanExport, PostmanOptions, postman, postman_environment, postman_with,
};
use export_support::corpus;

/// The JSON, then one `source: detail` line per note.
fn rendered(out: &PostmanExport) -> String {
    let mut text = format!("{}--- notes\n", out.json);
    for n in &out.notes {
        text.push_str(&format!("{}: {}\n", n.source, n.detail));
    }
    text
}

#[test]
fn every_corpus_collection_matches_its_snapshot() {
    for (label, root) in corpus() {
        let c = Collection::open(&root).expect("opens");
        insta::assert_snapshot!(format!("postman-{label}"), rendered(&postman(&c)));
    }
}

#[test]
fn kept_scripts_match_the_untranslated_snapshots() {
    let keep = PostmanOptions::new().keep_scripts(true);
    for (label, root) in corpus() {
        let c = Collection::open(&root).expect("opens");
        insta::assert_snapshot!(
            format!("postman-kept-{label}"),
            rendered(&postman_with(&c, &keep))
        );
    }
}

#[test]
fn postman_is_postman_with_the_default_options() {
    assert_eq!(PostmanOptions::default(), PostmanOptions::new());
    assert!(!PostmanOptions::new().keep_scripts);
    assert!(PostmanOptions::new().keep_scripts(true).keep_scripts);
    for (_, root) in corpus() {
        let c = Collection::open(&root).expect("opens");
        assert_eq!(postman(&c), postman_with(&c, &PostmanOptions::default()));
    }
}

#[test]
fn environments_match_their_snapshots() {
    let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (label, rel, env) in [
        ("demo", "../../examples/demo", "JSONPlaceholder"),
        (
            "env-inheritance",
            "tests/fixtures/bruno/env-inheritance",
            "dev",
        ),
        ("bruno-tests", "tests/fixtures/bruno/bruno-tests", "Local"),
    ] {
        let c = Collection::open(core.join(rel)).expect("opens");
        let out = postman_environment(&c, env).expect(env);
        insta::assert_snapshot!(format!("environment-{label}-{env}"), rendered(&out));
    }
}
