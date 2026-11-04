//! Spaces around a URL are ignored, and an empty URL has its own error.

use crate::common;

use apinata_core::{CheckOptions, FindingKind, ResolveError};
use common::{at, check_tree, setup};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: base\n      value: http://127.0.0.1:18925/api\n    - name: blank\n      value: \"\"\n    - name: spaces\n      value: \"   \"\n";

/// A request whose YAML-quoted `url` is written as given, so `\n` and `\t` are escapes.
fn req(url: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

/// The URL `url` resolves to, or the error.
fn resolve_url(url: &str) -> Result<String, ResolveError> {
    let text = req(url);
    let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    at(&c, &ctx, "r.yml").map(|r| r.url().to_owned())
}

#[test]
fn spaces_tabs_and_newlines_around_a_url_are_ignored() {
    for url in [
        "  http://127.0.0.1:18925/x",
        "http://127.0.0.1:18925/x  ",
        "\\t127.0.0.1:18925/x\\n",
        "\\r\\n http://127.0.0.1:18925/x \\t\\r\\n",
    ] {
        assert_eq!(
            resolve_url(url).as_deref(),
            Ok("http://127.0.0.1:18925/x"),
            "{url:?}"
        );
    }
}

#[test]
fn an_interpolated_url_with_spaces_around_it_is_trimmed_after_the_variable_is_filled() {
    assert_eq!(
        resolve_url("  {{base}}/x  ").as_deref(),
        Ok("http://127.0.0.1:18925/api/x")
    );
}

#[test]
fn a_trailing_space_after_an_explicit_scheme_url_is_dropped() {
    assert_eq!(
        resolve_url("https://127.0.0.1:18925/a?b=c ").as_deref(),
        Ok("https://127.0.0.1:18925/a?b=c")
    );
}

#[test]
fn an_empty_or_blank_url_is_empty_url() {
    for url in [
        "",
        "   ",
        "\\t\\n",
        "{{blank}}",
        "{{spaces}}",
        " {{blank}} ",
    ] {
        assert!(
            matches!(resolve_url(url), Err(ResolveError::EmptyUrl)),
            "{url:?}"
        );
    }
    assert_eq!(ResolveError::EmptyUrl.to_string(), "the URL is empty");
}

#[test]
fn a_space_inside_the_url_is_still_invalid() {
    match resolve_url("  http://exa mple.com/  ") {
        Err(ResolveError::InvalidUrl { url }) => assert_eq!(url, "http://exa mple.com/"),
        other => panic!("expected InvalidUrl, got {other:?}"),
    }
}

#[test]
fn check_reports_an_empty_url_in_plain_words() {
    let a = req("");
    let b = req("   ");
    let (_d, report) = check_tree(
        &[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)],
        &CheckOptions::default(),
    );
    assert_eq!(report.findings().len(), 2);
    for f in report.findings() {
        assert_eq!(f.kind(), FindingKind::InvalidRequest);
        assert!(f.message().contains("the URL is empty"), "{}", f.message());
    }
}

#[test]
fn the_file_keeps_the_text_as_written() {
    let text = req("  {{base}}/x  ");
    let (_dir, c, _ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    assert_eq!(common::request(&c, "r.yml").url(), "  {{base}}/x  ");
}
