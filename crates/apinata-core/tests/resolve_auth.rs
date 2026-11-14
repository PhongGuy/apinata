//! Header merging and auth inheritance, with axios's case-insensitive header names.

use crate::common;

use apinata_core::{AuthSource, AuthSpec, Body, Collection, ItemPath, ResolveError, VarContext};
use common::{at, pairs, setup};

fn headers_at(c: &Collection, ctx: &VarContext, rel: &str) -> Vec<(String, String)> {
    at(c, ctx, rel).expect("resolves").headers().to_vec()
}

/// The collection root with shared variables; `extra` continues its `request` block.
fn root(extra: &str) -> String {
    format!(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: token\n      value: t0k\n    - name: user\n      value: aladdin\n    - name: pass\n      value: opensesame\n    - name: v\n      value: fv\n    - name: hname\n      value: X-Named\n{extra}"
    )
}

/// A request file; `under_http` is indented under `http:`.
fn http(url: &str, under_http: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{under_http}")
}

fn folder(block: &str) -> String {
    format!("info:\n  name: f\n  type: folder\nrequest:\n{block}")
}

const U: &str = "https://x.test/";

#[test]
fn headers_merge_case_insensitively_keeping_the_first_name() {
    let root = root(
        "  headers:\n    - name: X-A\n      value: collection\n    - name: X-Coll\n      value: \"1\"\n    - name: X-Gone\n      value: gone\n      disabled: true\n",
    );
    let f = folder(
        "  headers:\n    - name: x-a\n      value: folder\n    - name: X-Fold\n      value: \"{{v}}\"\n",
    );
    let r = http(
        U,
        "  headers:\n    - name: X-A\n      value: request\n    - name: \"{{hname}}\"\n      value: hv\n    - name: \"\"\n      value: x\n    - name: X-Coll\n      value: \"2\"\n      disabled: true\n",
    );
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", &f),
        ("f/r.yml", &r),
    ]);
    assert_eq!(
        headers_at(&c, &ctx, "f/r.yml"),
        pairs(&[
            // Bruno merges raw names case-sensitively, so the request's `X-A` overwrites the
            // collection's in place, and axios then folds the later folder `x-a` onto it.
            ("X-A", "folder"),
            ("X-Coll", "1"),
            ("X-Fold", "fv"),
            ("X-Named", "hv")
        ])
    );
}

#[test]
fn bearer_and_basic_set_the_authorization_header() {
    let bearer = http(
        U,
        "  headers:\n    - name: authorization\n      value: stale\n  auth:\n    type: bearer\n    token: \"{{token}}\"\n",
    );
    let basic = http(
        U,
        "  auth:\n    type: basic\n    username: \"{{user}}\"\n    password: \"{{pass}}\"\n",
    );
    let no_password = http(U, "  auth:\n    type: basic\n    username: aladdin\n");
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root("")),
        ("bearer.yml", &bearer),
        ("basic.yml", &basic),
        ("nopass.yml", &no_password),
    ]);
    assert_eq!(
        headers_at(&c, &ctx, "bearer.yml"),
        pairs(&[("authorization", "Bearer t0k")])
    );
    assert_eq!(
        headers_at(&c, &ctx, "basic.yml"),
        pairs(&[("Authorization", "Basic YWxhZGRpbjpvcGVuc2VzYW1l")])
    );
    assert_eq!(
        headers_at(&c, &ctx, "nopass.yml"),
        pairs(&[("Authorization", "Basic YWxhZGRpbjo=")])
    );
}

#[test]
fn apikey_goes_to_a_header_or_the_query() {
    let apikey =
        |url: &str, fields: &str| http(url, &format!("  auth:\n    type: apikey\n{fields}"));
    let header = apikey(
        U,
        "    key: X-Api-Key\n    value: \"{{token}}\"\n    placement: header\n",
    );
    let query = apikey(
        U,
        "    key: api_key\n    value: \"{{token}}\"\n    placement: query\n",
    );
    // `searchParams.set`: the first `api_key` takes the form-encoded value, later ones go.
    let replace = apikey(
        "https://x.test/p?a=1&api_key=old&b=2&api_key=x",
        "    key: api_key\n    value: \"a b&c\"\n    placement: query\n",
    );
    let unplaced = apikey(U, "    key: api_key\n    value: v\n");
    let empty = apikey(
        U,
        "    key: X-Api-Key\n    value: \"\"\n    placement: header\n",
    );
    let blank_header = apikey(
        U,
        "    key: \"{{blank}}\"\n    value: v\n    placement: header\n",
    );
    let blank_query = apikey(
        U,
        "    key: \"{{blank}}\"\n    value: v\n    placement: query\n",
    );
    let (_dir, c, ctx) = setup(&[
        (
            "opencollection.yml",
            &root("    - name: blank\n      value: \"\"\n"),
        ),
        ("h.yml", &header),
        ("q.yml", &query),
        ("r.yml", &replace),
        ("n.yml", &unplaced),
        ("e.yml", &empty),
        ("bh.yml", &blank_header),
        ("bq.yml", &blank_query),
    ]);
    let h = at(&c, &ctx, "h.yml").expect("resolves");
    assert_eq!(h.headers().to_vec(), pairs(&[("X-Api-Key", "t0k")]));
    assert_eq!(h.url(), U);
    let q = at(&c, &ctx, "q.yml").expect("resolves");
    assert!(q.headers().is_empty());
    assert_eq!(q.url(), "https://x.test/?api_key=t0k");
    let r = at(&c, &ctx, "r.yml").expect("resolves");
    assert_eq!(r.url(), "https://x.test/p?a=1&api_key=a+b%26c&b=2");
    for rel in ["n.yml", "e.yml", "bh.yml", "bq.yml"] {
        let r = at(&c, &ctx, rel).expect("resolves");
        assert!(r.headers().is_empty() && r.url() == U, "{rel}: {r:?}");
    }
}

#[test]
fn an_auth_names_the_header_it_sets_over_the_request_own() {
    let own = |name: &str, auth: &str| {
        http(
            U,
            &format!("  headers:\n    - name: {name}\n      value: by-hand\n  auth:\n{auth}"),
        )
    };
    let files = [
        (
            "basic.yml",
            own(
                "Authorization",
                "    type: basic\n    username: u\n    password: p\n",
            ),
            Some("Authorization"),
        ),
        (
            "bearer.yml",
            own("authorization", "    type: bearer\n    token: t\n"),
            Some("Authorization"),
        ),
        (
            "apikey.yml",
            own(
                "X-Key",
                "    type: apikey\n    key: X-Key\n    value: v\n    placement: header\n",
            ),
            Some("X-Key"),
        ),
        (
            "query.yml",
            own(
                "api_key",
                "    type: apikey\n    key: api_key\n    value: v\n    placement: query\n",
            ),
            None,
        ),
        ("none.yml", own("Authorization", "    type: none\n"), None),
    ];
    let mut tree = vec![("opencollection.yml", root(""))];
    tree.extend(files.iter().map(|(rel, text, _)| (*rel, text.clone())));
    let tree: Vec<(&str, &str)> = tree.iter().map(|(r, t)| (*r, t.as_str())).collect();
    let (_dir, c, ctx) = setup(&tree);
    for (rel, _, header) in &files {
        let auth = c
            .request(&ItemPath::new(rel).expect("a valid item path"))
            .expect("the request")
            .auth();
        assert_eq!(auth.sets_header(), *header, "{rel}");
        // The header it names is the one sent; the hand-set value is gone.
        if let Some(name) = header {
            let sent = headers_at(&c, &ctx, rel);
            let named: Vec<_> = sent
                .iter()
                .filter(|(n, _)| n.eq_ignore_ascii_case(name))
                .collect();
            assert_eq!(named.len(), 1, "{rel}: {sent:?}");
            assert_ne!(named[0].1, "by-hand", "{rel}");
        }
    }
}

#[test]
fn inherit_takes_the_nearest_folder_with_real_auth_else_the_collection() {
    let root = root("  auth:\n    type: bearer\n    token: coll\n");
    let none = folder("  auth: none\n");
    let basic =
        folder("  auth:\n    type: basic\n    username: aladdin\n    password: opensesame\n");
    let inherit = folder("  auth: inherit\n");
    let inherits = http(U, "  auth: inherit\n");
    let (omitted, explicit_none) = (http(U, ""), http(U, "  auth: none\n"));
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("a/folder.yml", &none),
        ("a/b/folder.yml", &basic),
        ("a/b/c/folder.yml", &inherit),
        ("a/r.yml", &inherits),
        ("a/b/c/r.yml", &inherits),
        ("a/n.yml", &omitted),
        ("a/x.yml", &explicit_none),
    ]);
    assert_eq!(
        headers_at(&c, &ctx, "a/r.yml"),
        pairs(&[("Authorization", "Bearer coll")])
    );
    assert_eq!(
        headers_at(&c, &ctx, "a/b/c/r.yml"),
        pairs(&[("Authorization", "Basic YWxhZGRpbjpvcGVuc2VzYW1l")])
    );
    assert!(headers_at(&c, &ctx, "a/n.yml").is_empty());
    assert!(headers_at(&c, &ctx, "a/x.yml").is_empty());
}

#[test]
fn a_collection_level_inherit_means_no_auth() {
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root("  auth: inherit\n")),
        ("r.yml", &http(U, "  auth: inherit\n")),
    ]);
    assert!(headers_at(&c, &ctx, "r.yml").is_empty());
}

#[test]
fn unsupported_auth_fails_only_when_it_applies_and_before_interpolation() {
    let root = root("  auth:\n    type: kerberos\n    username: u\n    password: p\n");
    let (inherits, own) = (
        http(U, "  auth: inherit\n"),
        http(U, "  auth:\n    type: bearer\n    token: x\n"),
    );
    let oauth = http("https://x.test/{{missing}}", "  auth:\n    type: oauth2\n");
    let weird = http(U, "  auth: weird\n");
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("i.yml", &inherits),
        ("own.yml", &own),
        ("o.yml", &oauth),
        ("w.yml", &weird),
    ]);
    let kind = |rel: &str| match at(&c, &ctx, rel) {
        Err(ResolveError::UnsupportedAuth { kind }) => kind,
        other => panic!("{rel}: expected UnsupportedAuth, got {other:?}"),
    };
    assert_eq!(kind("i.yml"), "kerberos");
    assert_eq!(kind("o.yml"), "oauth2");
    assert_eq!(kind("w.yml"), "weird");
    assert_eq!(
        headers_at(&c, &ctx, "own.yml"),
        pairs(&[("Authorization", "Bearer x")])
    );
}

#[test]
fn a_folder_auth_without_a_type_is_skipped_by_inherit() {
    let root = root("  auth:\n    type: bearer\n    token: coll\n");
    let typeless = folder("  auth:\n    username: u\n");
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("a/folder.yml", &typeless),
        ("a/r.yml", &http(U, "  auth: inherit\n")),
    ]);
    assert_eq!(
        headers_at(&c, &ctx, "a/r.yml"),
        pairs(&[("Authorization", "Bearer coll")])
    );
}

#[test]
fn debug_output_redacts_header_query_and_body_values() {
    let r = http(
        U,
        "  headers:\n    - name: X-Secret\n      value: hdr-s3cret\n  auth:\n    type: apikey\n    key: api_key\n    value: q-s3cret\n    placement: query\n  body:\n    type: text\n    data: body-s3cret\n",
    );
    let (_dir, c, ctx) = setup(&[("opencollection.yml", &root("")), ("r.yml", &r)]);
    let resolved = at(&c, &ctx, "r.yml").expect("resolves");
    assert_eq!(resolved.body(), &Body::Text("body-s3cret".to_owned()));
    let debug = format!("{resolved:?}");
    assert!(
        debug.contains("X-Secret") && debug.contains("api_key"),
        "{debug}"
    );
    assert!(debug.contains("11"), "body length shown: {debug}");
    assert!(!debug.contains("s3cret"), "{debug}");
}

#[test]
fn a_collection_auth_without_a_type_means_no_auth() {
    let root = root("  auth:\n    username: u\n");
    let (_dir, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        ("r.yml", &http(U, "  auth: inherit\n")),
    ]);
    assert!(headers_at(&c, &ctx, "r.yml").is_empty());
}

/// The auth `rel` inherits and the level that supplies it.
fn inherited(c: &Collection, rel: &str) -> (AuthSpec, AuthSource) {
    let got = c.inherited_auth(&ItemPath::new(rel).expect("a valid item path"));
    (got.auth().clone(), got.source().clone())
}

fn folder_at(rel: &str) -> AuthSource {
    AuthSource::Folder(ItemPath::new(rel).expect("a valid item path"))
}

#[test]
fn inherited_auth_names_the_level_that_supplies_it() {
    let root = root("  auth:\n    type: bearer\n    token: coll\n");
    let none = folder("  auth: none\n");
    let basic =
        folder("  auth:\n    type: basic\n    username: aladdin\n    password: opensesame\n");
    let inherit = folder("  auth: inherit\n");
    let typeless = folder("  auth:\n    username: u\n");
    let inherits = http(U, "  auth: inherit\n");
    let (_dir, c, _ctx) = setup(&[
        ("opencollection.yml", &root),
        ("a/folder.yml", &none),
        ("a/b/folder.yml", &basic),
        ("a/b/c/folder.yml", &inherit),
        ("a/b/c/d/folder.yml", &typeless),
        ("a/r.yml", &inherits),
        ("a/b/c/r.yml", &inherits),
        ("a/b/c/d/r.yml", &inherits),
        ("r.yml", &inherits),
    ]);
    let coll = AuthSpec::Bearer {
        token: "coll".to_owned(),
    };
    let aladdin = AuthSpec::Basic {
        username: "aladdin".to_owned(),
        password: "opensesame".to_owned(),
    };
    assert_eq!(
        inherited(&c, "r.yml"),
        (coll.clone(), AuthSource::Collection)
    );
    assert_eq!(
        inherited(&c, "a/r.yml"),
        (coll.clone(), AuthSource::Collection),
        "a folder's none passes inheritance up"
    );
    assert_eq!(
        inherited(&c, "a/b/c/r.yml"),
        (aladdin.clone(), folder_at("a/b"))
    );
    assert_eq!(
        inherited(&c, "a/b/c/d/r.yml"),
        (aladdin.clone(), folder_at("a/b")),
        "a typeless folder auth is skipped"
    );
    // A folder inherits from the levels above it, not from itself.
    assert_eq!(inherited(&c, "a/b/c"), (aladdin, folder_at("a/b")));
    assert_eq!(inherited(&c, "a/b"), (coll, AuthSource::Collection));
}

#[test]
fn inherited_auth_from_a_collection_without_one_is_no_auth() {
    for block in [
        "",
        "  auth: inherit\n",
        "  auth: none\n",
        "  auth:\n    username: u\n",
    ] {
        let (_dir, c, _ctx) = setup(&[
            ("opencollection.yml", &root(block)),
            ("r.yml", &http(U, "  auth: inherit\n")),
        ]);
        assert_eq!(
            inherited(&c, "r.yml"),
            (AuthSpec::None, AuthSource::Collection),
            "{block:?}"
        );
    }
    let ntlm = root("  auth:\n    type: ntlm\n    username: u\n    password: p\n");
    let (_dir, c, _ctx) = setup(&[
        ("opencollection.yml", &ntlm),
        ("r.yml", &http(U, "  auth: inherit\n")),
    ]);
    assert_eq!(
        inherited(&c, "r.yml"),
        (
            AuthSpec::Ntlm {
                username: "u".to_owned(),
                password: "p".to_owned(),
                domain: String::new()
            },
            AuthSource::Collection
        )
    );
    let kerberos = root("  auth:\n    type: kerberos\n");
    let (_dir, c, _ctx) = setup(&[
        ("opencollection.yml", &kerberos),
        ("r.yml", &http(U, "  auth: inherit\n")),
    ]);
    assert_eq!(
        inherited(&c, "r.yml"),
        (
            AuthSpec::Unsupported {
                mode: "kerberos".to_owned()
            },
            AuthSource::Collection
        )
    );
}

#[test]
fn a_folder_auth_typed_none_or_inherit_ends_the_walk_as_unsupported() {
    let root = root("  auth:\n    type: bearer\n    token: coll\n");
    for mode in ["none", "inherit"] {
        let typed = folder(&format!("  auth:\n    type: {mode}\n"));
        let (_dir, c, ctx) = setup(&[
            ("opencollection.yml", &root),
            ("a/folder.yml", &typed),
            ("a/r.yml", &http(U, "  auth: inherit\n")),
        ]);
        let unsupported = AuthSpec::Unsupported {
            mode: mode.to_owned(),
        };
        assert_eq!(
            inherited(&c, "a/r.yml"),
            (unsupported, folder_at("a")),
            "{mode}"
        );
        // A send rejects the same folder auth rather than passing inheritance up.
        assert!(
            matches!(
                at(&c, &ctx, "a/r.yml"),
                Err(ResolveError::UnsupportedAuth { .. })
            ),
            "{mode}"
        );
    }
}
