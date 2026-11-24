//! A GET snapshot for a URL: the draft a link from a response opens as.

use crate::tree_support;

use apinata_core::{AuthSpec, DraftSnapshot, ItemPath, KeyValue};
use tree_support::{ROOT, open};

#[test]
fn a_get_snapshot_opens_as_a_named_draft_with_the_url_headers_and_auth_given() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let snap = DraftSnapshot::http_get(
        "https://api.test.invalid/items?page=2",
        vec![KeyValue::new("X-Team", "{{team}}")],
        AuthSpec::Bearer {
            token: "lit".into(),
        },
    )
    .expect("a valid header");
    let draft = c.create_draft_from(&ItemPath::root(), &snap).expect("made");
    let r = c.request(&draft).expect("the draft");
    assert_eq!(r.name(), "Untitled request");
    assert_eq!(r.method(), "GET");
    assert_eq!(r.url(), "https://api.test.invalid/items?page=2");
    let params = r.params();
    assert_eq!(
        (params[0].name.as_str(), params[0].value.as_str()),
        ("page", "2")
    );
    let headers = r.headers();
    assert_eq!(headers.len(), 1);
    assert_eq!(
        (headers[0].name.as_str(), headers[0].value.as_str()),
        ("X-Team", "{{team}}")
    );
    // The auth is kept as given; only a stored snapshot of the draft blanks it.
    assert!(matches!(r.auth(), AuthSpec::Bearer { token } if token == "lit"));
}

#[test]
fn a_get_snapshot_with_no_headers_inherits_its_auth() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let snap = DraftSnapshot::http_get("http://127.0.0.1/next", Vec::new(), AuthSpec::Inherit)
        .expect("made");
    let draft = c.create_draft_from(&ItemPath::root(), &snap).expect("made");
    let r = c.request(&draft).expect("the draft");
    assert!(r.headers().is_empty());
    assert!(matches!(r.auth(), AuthSpec::Inherit));
}

#[test]
fn a_header_name_with_a_line_break_is_refused() {
    let bad = vec![KeyValue::new("X\nY", "v")];
    assert!(DraftSnapshot::http_get("http://127.0.0.1/", bad, AuthSpec::Inherit).is_err());
}
