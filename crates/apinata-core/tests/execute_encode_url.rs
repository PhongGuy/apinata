//! Params typed in the editor reach the server intact when `encodeUrl` is on.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, Param, ParamKind, RequestFile, VarContext, resolve,
};
use common::{executor, request, write_tree};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// Sends one request whose query is set to `name=value` through the editing API, and
/// returns the query pairs the server decoded.
async fn received_query(name: &str, value: &str) -> Vec<(String, String)> {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let text = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{}/echo\"\nsettings:\n  encodeUrl: true\n",
        server.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let mut file = RequestFile::load(dir.path().join("r.yml")).expect("loads");
    file.set_params(vec![Param::new(name, value, ParamKind::Query)]);
    file.save().expect("saves");
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let resolved = resolve(request(&c, "r.yml"), &ctx).expect("resolves");
    executor().send(&resolved).await.expect("sent");
    let received = server.received_requests().await.expect("recorded");
    received[0]
        .url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

#[tokio::test]
async fn query_delimiters_typed_as_data_arrive_decoded_once() {
    for (name, value) in [
        ("b", "1 & 2"),
        ("b", "a#b"),
        ("b", "x=y"),
        ("k&=#", "v"),
        ("b", "100%"),
    ] {
        assert_eq!(
            received_query(name, value).await,
            [(name.to_owned(), value.to_owned())],
            "{name}={value}"
        );
    }
}

#[tokio::test]
async fn escapes_from_a_variable_value_are_encoded_like_any_other_text() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let root = format!(
        "{ROOT}request:\n  variables:\n    - name: cb\n      value: \"https://a/?x=1%262#y\"\n"
    );
    let text = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{}/echo?redirect={{{{cb}}}}&b=1 %26 2\"\n  params:\n    - name: redirect\n      value: \"{{{{cb}}}}\"\n      type: query\n    - name: b\n      value: \"1 & 2\"\n      type: query\nsettings:\n  encodeUrl: true\n",
        server.uri()
    );
    let dir = write_tree(&[("opencollection.yml", &root), ("r.yml", &text)]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let resolved = resolve(request(&c, "r.yml"), &ctx).expect("resolves");
    executor().send(&resolved).await.expect("sent");
    let received = server.received_requests().await.expect("recorded");
    let pairs: Vec<(String, String)> = received[0]
        .url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    // The reference double-encodes expanded text, so the server sees the value's escapes as written.
    assert_eq!(
        pairs,
        [
            ("redirect".to_owned(), "https://a/?x=1%262#y".to_owned()),
            ("b".to_owned(), "1 & 2".to_owned()),
        ]
    );
}
