use apinata_core::{FormKind, FormPolicy, WireForm, host_forms, replace_ascii_ci, wire_forms};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use percent_encoding::{AsciiSet, CONTROLS, NON_ALPHANUMERIC, utf8_percent_encode};

const AWKWARD: &str = "Ab1 \"q\"/x+y";
const UNI: &str = "p&ss/wörd<x>+ 1";
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
const QUERY: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'\'');
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'?')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'%');

fn p() -> FormPolicy {
    FormPolicy::new(4, 6)
}
fn texts(v: &str, policy: FormPolicy) -> Vec<String> {
    wire_forms(v, policy).into_iter().map(|f| f.text).collect()
}
fn has(forms: &[String], f: &str) -> bool {
    forms.iter().any(|x| x == f)
}
fn pct(v: &str, set: &'static AsciiSet) -> String {
    utf8_percent_encode(v, set).to_string()
}

#[test]
fn the_whatwg_and_uri_component_sets_are_listed() {
    let forms = texts(AWKWARD, p());
    for (name, f) in [
        ("query", pct(AWKWARD, QUERY)),
        ("segment", pct(AWKWARD, SEGMENT)),
        ("query again", pct(&pct(AWKWARD, QUERY), UNRESERVED)),
        ("segment again", pct(&pct(AWKWARD, SEGMENT), UNRESERVED)),
        ("encodeURIComponent", "Ab1%20%22q%22%2Fx%2By".to_owned()),
        (
            "encodeURIComponent lower",
            "Ab1%20%22q%22%2fx%2by".to_owned(),
        ),
        ("php slash", "Ab1 \\\"q\\\"\\/x+y".to_owned()),
    ] {
        assert!(has(&forms, &f), "{name}: {f} missing");
    }
    let mut u = url::Url::parse("http://h.test/").unwrap();
    u.set_password(Some(AWKWARD)).unwrap();
    assert!(has(&forms, u.password().unwrap()), "url password");
    let mut u = url::Url::parse("http://h.test/").unwrap();
    u.set_path(AWKWARD);
    assert!(has(&forms, u.path().trim_start_matches('/')), "url path");
}

#[test]
fn go_html_and_slash_rescapes_come_in_both_hex_cases() {
    let forms = texts(UNI, p());
    for f in [
        "p\\u0026ss\\/w\\u00f6rd\\u003cx\\u003e+ 1",
        "p\\u0026ss\\/w\\u00F6rd\\u003Cx\\u003E+ 1",
        "p\\u0026ss/w\\u00f6rd\\u003cx\\u003e+ 1",
    ] {
        assert!(has(&forms, f), "{f} missing");
    }
}

#[test]
fn the_unicode_escape_covers_every_character() {
    let forms = texts(AWKWARD, p());
    assert!(has(
        &forms,
        "\\u0041\\u0062\\u0031\\u0020\\u0022\\u0071\\u0022\\u002F\\u0078\\u002B\\u0079"
    ));
    assert!(has(
        &forms,
        "\\u0041\\u0062\\u0031\\u0020\\u0022\\u0071\\u0022\\u002f\\u0078\\u002b\\u0079"
    ));
}

#[test]
fn kinds_follow_how_each_form_matches() {
    let forms = wire_forms(AWKWARD, p());
    let kind_of = |t: &str| forms.iter().find(|f| f.text == t).map(|f| f.kind);
    assert_eq!(kind_of(AWKWARD), Some(FormKind::Text));
    assert_eq!(kind_of(&STANDARD.encode(AWKWARD)), Some(FormKind::Base64));
    let fragment = &apinata_core::base64_fragment_forms("hunter22")[0];
    let forms = wire_forms("hunter22", p());
    assert!(
        forms
            .iter()
            .any(|f| &f.text == fragment && f.kind == FormKind::Base64Fragment)
    );
}

#[test]
fn basic_forms_ignore_the_floors_and_extended_ones_obey_them() {
    let short = wire_forms("A/b", p());
    let t: Vec<&str> = short.iter().map(|f| f.text.as_str()).collect();
    assert!(t.contains(&"A/b") && t.contains(&"A%2Fb"), "{t:?}");
    assert!(
        !t.contains(&"a/b"),
        "lowercase is Extended and under 6 characters"
    );
    assert!(short.iter().all(|f| f.kind == FormKind::Text));
    // A value under min_value_chars gets Basic forms only; a high form floor drops base64.
    let none = wire_forms("Ab1_", FormPolicy::new(5, 1));
    assert!(
        none.iter()
            .all(|f| f.kind == FormKind::Text && !f.text.contains("\\u"))
    );
    let high = wire_forms("Ab1_", FormPolicy::new(4, 100));
    assert!(
        high.iter()
            .all(|f| f.kind == FormKind::Text && !f.text.contains("\\u"))
    );
    assert!(
        wire_forms("Ab1_", FormPolicy::new(4, 1))
            .iter()
            .any(|f| f.kind == FormKind::Base64Fragment)
    );
}

#[test]
fn the_list_is_deduplicated_longest_first_and_never_empty_text() {
    assert!(wire_forms("", p()).is_empty());
    for v in [AWKWARD, UNI, "x", "a b"] {
        let forms: Vec<WireForm> = wire_forms(v, p());
        assert!(forms.iter().all(|f| !f.text.is_empty()), "{v}");
        assert!(
            forms.windows(2).all(|w| w[0].text.len() >= w[1].text.len()),
            "{v}"
        );
        let mut keys: Vec<(&str, FormKind)> =
            forms.iter().map(|f| (f.text.as_str(), f.kind)).collect();
        let n = keys.len();
        keys.sort_by_key(|k| (k.0.to_owned(), k.1 as u8));
        keys.dedup();
        assert_eq!(keys.len(), n, "{v}");
    }
}

#[test]
fn host_forms_give_lowercase_punycode_and_unicode() {
    let forms = host_forms("Bücher.EXAMPLE");
    assert!(forms.contains(&"bücher.example".to_owned()), "{forms:?}");
    assert!(
        forms.contains(&"xn--bcher-kva.example".to_owned()),
        "{forms:?}"
    );
}

#[test]
fn replace_ascii_ci_is_left_to_right_and_non_overlapping() {
    assert_eq!(replace_ascii_ci("aXbxAXc", "ax", "-"), "-bx-c");
    assert_eq!(replace_ascii_ci("aaa", "aa", "-"), "-a");
    assert_eq!(replace_ascii_ci("é Ab é", "ab", "[x]"), "é [x] é");
    assert_eq!(replace_ascii_ci("abc", "", "-"), "abc");
}

#[test]
fn html_and_non_ascii_escapes_vary_in_hex_case_independently() {
    let forms = texts(UNI, p());
    for f in [
        "p\\u0026ss\\/w\\u00F6rd\\u003cx\\u003e+ 1",
        "p\\u0026ss\\/w\\u00f6rd\\u003Cx\\u003E+ 1",
    ] {
        assert!(has(&forms, f), "{f} missing");
    }
    let forms = texts("\u{1F600}emoji&<>x", p());
    for f in [
        "\\uD83D\\uDE00emoji\\u0026\\u003c\\u003ex",
        "\\ud83d\\ude00emoji\\u0026\\u003C\\u003Ex",
    ] {
        assert!(has(&forms, f), "{f} missing");
    }
}

#[test]
fn host_forms_decode_percent_escapes_like_the_url_crate() {
    assert!(host_forms("p%41ss0").contains(&"pass0".to_owned()));
}

#[test]
fn every_legacy_list_is_a_subset_of_the_generator() {
    use crate::secret_forms_legacy as old;
    let corpus = [
        AWKWARD,
        UNI,
        "Sk Live/9é😀Q",
        "k>?~~?x",
        "hunter22",
        "Key/a b~*é",
        "a b",
        "x",
    ];
    for v in corpus {
        let shell = texts(v, p());
        let mcp = texts(v, FormPolicy::new(4, 1));
        let miss = |legacy: Vec<String>, list: &[String], who: &str| {
            for f in legacy.into_iter().filter(|f| !f.is_empty()) {
                assert!(has(list, &f), "{who}: {f:?} of {v:?} not generated");
            }
        };
        miss(old::session_forms(v), &shell, "session log");
        if v.chars().count() >= 4 {
            miss(old::core_forms(v), &shell, "saved examples");
            miss(old::history_forms(v), &shell, "history");
            miss(old::mcp_forms(v), &mcp, "mcp");
        }
    }
}
