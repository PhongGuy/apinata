//! XPath filtering of XML response bodies.

use apinata_core::{FilterError, filter_xml};

const BODY: &str = r#"<?xml version="1.0"?>
<store><book id="1" lang="en"><title>Alpha</title><price>12.5</price></book><book id="2"><title>Beta &amp; Gamma</title><price>3</price></book><!--note--></store>"#;

const ATOM: &str = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:x="urn:x"><title>Feed</title><entry x:id="e1"><title>One</title></entry></feed>"#;

fn ok(expression: &str) -> String {
    filter_xml(BODY, expression).unwrap_or_else(|e| panic!("{expression}: {e}"))
}

#[test]
fn attributes_come_back_as_their_values_in_document_order() {
    assert_eq!(ok("//book/@id"), "1\n2");
}

#[test]
fn text_nodes_come_back_unescaped() {
    assert_eq!(ok("//title/text()"), "Alpha\nBeta & Gamma");
}

#[test]
fn a_comment_comes_back_as_its_text() {
    assert_eq!(ok("//comment()"), "note");
}

#[test]
fn scalars_use_their_xpath_string_values() {
    assert_eq!(ok("count(//book)"), "2");
    assert_eq!(ok("sum(//price)"), "15.5");
    assert_eq!(ok("count(//book) > 1"), "true");
    assert_eq!(ok("string(//title)"), "Alpha");
    assert_eq!(ok("1 div 0"), "Infinity");
}

#[test]
fn no_match_is_an_empty_result() {
    assert_eq!(ok("//missing"), "");
}

#[test]
fn the_expression_is_trimmed() {
    assert_eq!(ok("  //book/@id \n"), "1\n2");
}

#[test]
fn an_empty_filter_is_an_expression_error_at_the_start() {
    assert_eq!(
        filter_xml(BODY, "  "),
        Err(FilterError::Expression {
            message: "the filter is empty".to_owned(),
            position: 0,
        })
    );
}

#[test]
fn an_invalid_expression_says_so_without_a_position() {
    let e = filter_xml(BODY, "//book[").unwrap_err();
    assert!(matches!(e, FilterError::Invalid { .. }), "{e:?}");
    assert!(e.to_string().starts_with("Invalid filter: "), "{e}");
}

#[test]
fn a_malformed_body_is_not_xml() {
    let e = filter_xml("<a><b></a>", "//a").unwrap_err();
    assert!(matches!(e, FilterError::Xml { .. }), "{e:?}");
    assert!(e.to_string().starts_with("The body is not XML: "), "{e}");
}

#[test]
fn a_json_body_is_not_xml() {
    assert!(matches!(
        filter_xml(r#"{"a":1}"#, "//a"),
        Err(FilterError::Xml { .. })
    ));
}

#[test]
fn a_leading_byte_order_mark_is_skipped() {
    assert_eq!(filter_xml("\u{feff}<a x=\"1\"/>", "/a/@x").unwrap(), "1");
}

#[test]
fn entity_expansion_bombs_fail_closed() {
    let mut body = String::from("<!DOCTYPE l [<!ENTITY l0 \"ha\">");
    for i in 1..=30 {
        body.push_str(&format!("<!ENTITY l{i} \"&l{p};&l{p};\">", p = i - 1));
    }
    body.push_str("]><l>&l30;</l>");
    assert!(matches!(
        filter_xml(&body, "string(/l)"),
        Err(FilterError::Xml { .. })
    ));
}

#[test]
fn external_entities_are_never_fetched() {
    let body = r#"<!DOCTYPE a [<!ENTITY x SYSTEM "http://127.0.0.1:9/x">]><a>&x;</a>"#;
    assert!(matches!(
        filter_xml(body, "string(/a)"),
        Err(FilterError::Xml { .. })
    ));
}

#[test]
fn deep_nesting_fails_closed() {
    let body = format!("{}{}", "<a>".repeat(200), "</a>".repeat(200));
    assert!(matches!(
        filter_xml(&body, "count(//a)"),
        Err(FilterError::Xml { .. })
    ));
}

#[test]
fn prefixes_declared_on_the_root_are_bound() {
    assert_eq!(
        filter_xml(ATOM, "//*[local-name()='entry']/@x:id").unwrap(),
        "e1"
    );
}

#[test]
fn unprefixed_names_do_not_match_a_default_namespace() {
    assert_eq!(filter_xml(ATOM, "//title").unwrap(), "");
    assert_eq!(
        filter_xml(ATOM, "//*[local-name()='title']/text()").unwrap(),
        "Feed\nOne"
    );
}

#[test]
fn an_unbound_prefix_matches_nothing() {
    assert_eq!(filter_xml(ATOM, "//y:title").unwrap(), "");
}

fn rows(n: usize) -> String {
    let rows: String = (0..n)
        .map(|i| format!("<item id=\"{i}\"><v>{i}</v></item>"))
        .collect();
    format!("<items>{rows}</items>")
}

#[test]
fn a_large_body_fits_the_work_budget() {
    // A predicate over 20,000 rows needs more node visits than uppsala's default of 100,000.
    assert_eq!(
        filter_xml(&rows(20_000), "count(//item[v > 5])").unwrap(),
        "19994"
    );
}

#[test]
fn a_runaway_expression_hits_the_work_limit() {
    let e = filter_xml(&rows(20_000), "//item[count(//item) > 0]").unwrap_err();
    assert_eq!(e, FilterError::Limit);
    assert_eq!(
        e.to_string(),
        "The filter needs more work than allowed on this body"
    );
}

#[test]
fn an_element_comes_back_as_its_xml() {
    assert_eq!(
        ok("//title"),
        "<title>Alpha</title>\n<title>Beta &amp; Gamma</title>"
    );
}

#[test]
fn an_element_keeps_its_attributes_children_and_escapes() {
    let body = r#"<r><e a="x &quot;y&quot; &lt;z&gt;" b='1'>t &lt; u &gt; v<c/><!--k--><?p d?><![CDATA[<raw>]]></e></r>"#;
    assert_eq!(
        filter_xml(body, "/r/e").unwrap(),
        r#"<e a="x &quot;y&quot; &lt;z&gt;" b="1">t &lt; u &gt; v<c/><!--k--><?p d?><![CDATA[<raw>]]></e>"#
    );
}

#[test]
fn the_root_node_comes_back_as_the_document_element() {
    assert_eq!(
        filter_xml("<!--c--><a><b/></a>", "/").unwrap(),
        "<a><b/></a>"
    );
}

#[test]
fn source_whitespace_is_kept() {
    assert_eq!(
        filter_xml("<r>\n  <a>x</a>\n</r>", "/r").unwrap(),
        "<r>\n  <a>x</a>\n</r>"
    );
}

#[test]
fn a_matched_element_declares_the_namespaces_it_uses() {
    assert_eq!(
        filter_xml(ATOM, "//*[local-name()='entry']").unwrap(),
        r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:x="urn:x" x:id="e1"><title>One</title></entry>"#
    );
}

#[test]
fn unused_namespaces_are_not_declared() {
    let body = r#"<r xmlns:u="urn:u" xmlns:v="urn:v"><a><v:b/></a></r>"#;
    assert_eq!(
        filter_xml(body, "/r/a").unwrap(),
        r#"<a xmlns:v="urn:v"><v:b/></a>"#
    );
}

#[test]
fn declarations_inside_the_match_stay_where_they_were() {
    let body = r#"<r xmlns="urn:d"><a xmlns:p="urn:p"><p:b/><c xmlns=""/></a></r>"#;
    assert_eq!(
        filter_xml(body, "/*/*").unwrap(),
        r#"<a xmlns="urn:d" xmlns:p="urn:p"><p:b/><c xmlns=""/></a>"#
    );
}

#[test]
fn whitespace_character_references_survive_a_round_trip() {
    let body = r#"<r><e a="x&#9;y&#10;z"/><t>a&#13;b</t></r>"#;
    assert_eq!(filter_xml(body, "/r/e/@a").unwrap(), "x\ty\nz");
    assert_eq!(filter_xml(body, "/r/t/text()").unwrap(), "a\rb");
    assert_eq!(
        filter_xml(body, "/r/e").unwrap(),
        r#"<e a="x&#9;y&#10;z"/>"#
    );
    assert_eq!(filter_xml(body, "/r/t").unwrap(), "<t>a&#13;b</t>");
}
