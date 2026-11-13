//! Any RFC 9110 token is a usable HTTP method, and `normalize_method` is the one gate.

use crate::common::{at, executor, setup};
use crate::parity::file_with;

use apinata_core::{RequestFile, normalize_method};
use std::fs;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn doc(method: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: {method}\n  url: \"http://127.0.0.1/\"\n"
    )
}

#[test]
fn tokens_are_accepted_and_uppercased() {
    for (given, want) in [
        ("GET", "GET"),
        ("PROPFIND", "PROPFIND"),
        ("M-SEARCH", "M-SEARCH"),
        ("propfind", "PROPFIND"),
        ("Query", "QUERY"),
    ] {
        assert_eq!(normalize_method(given).as_deref(), Ok(want), "{given}");
    }
}

#[test]
fn non_tokens_are_refused_with_a_message() {
    for bad in [
        "",
        "GE T",
        "GET/",
        " GET",
        "GET ",
        "PRÖPFIND",
        "A\tB",
        "(X)",
    ] {
        let err = normalize_method(bad).expect_err(bad);
        assert!(!err.to_string().is_empty(), "{bad:?}");
    }
}

// The refusal is read in the app, the CLI and the MCP server alike, so it names the rule in
// plain words rather than listing the token characters.
#[test]
fn the_refusal_reads_in_plain_words() {
    let err = normalize_method("GE T").expect_err("a space is refused");
    assert_eq!(
        err.to_string(),
        "`GE T` is not a valid HTTP method: use letters, digits, - or _, with no spaces"
    );
}

#[test]
fn the_validator_agrees_with_the_sender() {
    let server = tokio::runtime::Runtime::new().expect("rt");
    for token in [
        "GET", "PROPFIND", "M-SEARCH", "propfind", "GE T", "", "GET/", "PRÖ",
    ] {
        let valid = normalize_method(token).is_ok();
        let text = doc(&format!("\"{token}\""));
        let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
        let req = at(&c, &ctx, "r.yml").expect("resolves");
        let sent = server.block_on(executor().send(&req));
        let refused = matches!(&sent, Err(apinata_core::ExecError::InvalidRequest { .. }));
        assert_eq!(valid, !refused, "{token:?}");
    }
}

#[tokio::test]
async fn a_custom_verb_reaches_the_wire() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(207))
        .mount(&server)
        .await;
    let text = doc("PROPFIND").replace("http://127.0.0.1/", &format!("{}/", server.uri()));
    let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    executor()
        .send(&at(&c, &ctx, "r.yml").expect("resolves"))
        .await
        .expect("sent");
    let seen = server.received_requests().await.expect("recorded");
    assert_eq!(seen[0].method.as_str(), "PROPFIND");
}

#[test]
fn custom_verbs_survive_a_save_byte_for_byte_and_a_set() {
    for verb in ["PROPFIND", "QUERY"] {
        let original = doc(verb);
        let (_dir, path) = file_with(&original);
        let mut r = RequestFile::load(&path).expect("loads");
        assert_eq!(r.method(), verb);
        r.save().expect("saves");
        assert_eq!(fs::read_to_string(&path).expect("read"), original);
    }
    let (_dir, path) = file_with(&doc("GET"));
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_method("PROPFIND");
    r.save().expect("saves");
    assert_eq!(
        RequestFile::load(&path).expect("loads").method(),
        "PROPFIND"
    );
}
