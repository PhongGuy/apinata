//! Marking a draft unsaved, and whether a snapshot blanked credentials.

use crate::tree_support;

use apinata_core::{AuthSpec, ItemPath};
use tree_support::{ROOT, ip, open, request};

#[test]
fn a_draft_marked_unsaved_stays_dirty_through_a_revert_until_saved() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    let snap = c.request_snapshot(&ip("a.yml")).expect("a request");
    let d = c.create_draft_from(&ItemPath::root(), &snap).unwrap();
    assert!(
        !c.request(&d).unwrap().is_dirty(),
        "create_draft_from starts clean"
    );
    assert!(c.mark_draft_unsaved(&d));
    assert!(c.request(&d).unwrap().is_dirty());
    let path = d.to_path(c.root());
    c.revert(&path).unwrap();
    let url = c.request(&ip("a.yml")).unwrap().url().to_owned();
    let draft = c.request(&d).unwrap();
    assert!(draft.is_dirty(), "a revert keeps a restored draft unsaved");
    assert_eq!(draft.url(), url, "and keeps its content");
    let saved = c.save_draft(&d, &ItemPath::root(), "kept").unwrap();
    assert!(!c.request(&saved).unwrap().is_dirty());
}

#[test]
fn only_a_draft_can_be_marked_unsaved() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    assert!(!c.mark_draft_unsaved(&ip("a.yml")));
    assert!(!c.request(&ip("a.yml")).unwrap().is_dirty());
    assert!(!c.mark_draft_unsaved(&ip("untitled-9.draft")));
}

#[test]
fn blanks_credentials_says_whether_the_snapshot_blanked_anything() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    let d = c.create_draft(&ItemPath::root()).unwrap();
    assert!(!c.request(&d).unwrap().blanks_credentials());
    let bearer = |token: &str| AuthSpec::Bearer {
        token: token.into(),
    };
    c.request_mut(&d).unwrap().set_auth(bearer("{{token}}"));
    assert!(
        !c.request(&d).unwrap().blanks_credentials(),
        "a lone placeholder is kept"
    );
    c.request_mut(&d)
        .unwrap()
        .set_auth(bearer("literal-token-1"));
    let r = c.request(&d).unwrap();
    assert!(r.blanks_credentials());
    assert!(!r.snapshot().to_json().contains("literal-token-1"));
}
