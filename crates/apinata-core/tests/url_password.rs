//! The one url password rule: the request-url and free-text readings, and the snapshot pass.

use crate::tree_support;

use apinata_core::{DraftSnapshot, text_without_password, url_without_password};
use tree_support::{ROOT, ip, open};

#[test]
fn an_encoded_colon_splits_the_userinfo_as_a_send_reads_it() {
    for (url, kept) in [
        ("http://u%3Ap@h/x", "http://u@h/x"),
        ("http://u%3ap@h/x", "http://u@h/x"),
        ("http://u%3Ap:q@h", "http://u@h"),
        ("http://u:p%3Aq@h", "http://u@h"),
    ] {
        assert_eq!(url_without_password(url), kept, "{url}");
        assert_eq!(text_without_password(url), kept, "{url}");
    }
}

#[test]
fn a_twice_encoded_colon_is_not_a_colon() {
    let url = "http://u%253Ap@h/x";
    assert_eq!(url_without_password(url), url);
    assert_eq!(text_without_password(url), url);
}

#[test]
fn an_empty_password_after_the_split_is_left_as_is() {
    for url in [
        "http://u%3A@h/x",
        "http://u:@h/x",
        "http://u@h/x",
        "http://h/x",
    ] {
        assert_eq!(url_without_password(url), url, "{url}");
        assert_eq!(text_without_password(url), url, "{url}");
    }
}

#[test]
fn a_lone_placeholder_password_after_the_split_stays() {
    for url in ["http://{{a}}%3A{{b}}@h/x", "http://u:{{pw}}@h/x"] {
        assert_eq!(url_without_password(url), url, "{url}");
        assert_eq!(text_without_password(url), url, "{url}");
    }
}

#[test]
fn an_empty_user_with_a_password_drops_the_whole_userinfo() {
    for (url, kept) in [
        ("http://:pw@h/x", "http://h/x"),
        ("http://%3Apw@h/x", "http://h/x"),
        ("http://:pw@h", "http://h"),
    ] {
        assert_eq!(url_without_password(url), kept, "{url}");
        assert_eq!(text_without_password(url), kept, "{url}");
    }
}

#[test]
fn a_schemeless_request_url_is_read_as_the_executor_sends_it() {
    assert_eq!(url_without_password("u%3Ap@h/x"), "u@h/x");
    assert_eq!(url_without_password("u:p@h/x"), "u@h/x");
    assert_eq!(
        url_without_password("{{proto}}://u:p@h/x"),
        "{{proto}}://u@h/x"
    );
}

#[test]
fn free_text_without_a_scheme_is_untouched() {
    for text in [
        r#"{"login":"joe:pw@x"}"#,
        "joe:pw@x",
        "joe%3Apw@x",
        "ask joe:pw at http://h/x",
        "see http://h for details",
    ] {
        assert_eq!(text_without_password(text), text, "{text}");
    }
}

#[test]
fn free_text_with_a_placeholder_scheme_loses_its_password() {
    assert_eq!(
        text_without_password("{{proto}}://u%3Ap@h/x"),
        "{{proto}}://u@h/x"
    );
}

#[test]
fn free_text_the_parser_reads_with_a_password_loses_it() {
    let kept = text_without_password("http://h is down, ask joe:pw@h");
    assert!(!kept.contains("pw"), "{kept:?}");
}

#[test]
fn odd_slashes_and_parser_skipped_whitespace_keep_no_password() {
    for url in [
        "http:/u:pwsecret@h/x",
        "http:\\\\u:pwsecret@h/x",
        "http:/u%3Apwsecret@h/x",
        "http://u:pwsecret@h\n",
        "http://u:pwsecret@h\n/x",
        "http://u\t:pwsecret@h/x",
        "http://u:pwsecret@h\r/x",
        "http://my user:pwsecret@h",
    ] {
        for kept in [text_without_password(url), url_without_password(url)] {
            assert!(!kept.contains("pwsecret"), "{url:?} kept {kept:?}");
            // The request reading keeps the user's bytes, whitespace the parser skips included.
            let user: String = kept.chars().filter(|c| !c.is_whitespace()).collect();
            assert!(
                user.contains("u@h") || user.contains("user@h"),
                "{url:?} kept {kept:?}"
            );
        }
    }
}

#[test]
fn leading_space_or_control_is_kept_and_the_password_dropped() {
    for lead in [" ", "\t", "\u{1}"] {
        let url = format!("{lead}http://u%3Apwsecret@h/x");
        assert_eq!(text_without_password(&url), format!("{lead}http://u@h/x"));
    }
}

// The snapshot of `http` (a request's `http` mapping as JSON) after the pass, and its answer.
fn snapshot_after(kind: &str, request: serde_json::Value) -> (serde_json::Value, bool) {
    let json = serde_json::json!({"info": {"name": "r", "type": kind}, kind: request});
    let mut snap = DraftSnapshot::from_json(&json.to_string()).unwrap();
    let changed = snap.drop_url_passwords();
    (serde_json::from_str(&snap.to_json()).unwrap(), changed)
}

#[test]
fn a_snapshot_reads_its_url_as_a_request_and_other_strings_as_free_text() {
    let request = serde_json::json!({
        "method": "GET",
        "url": "u%3Ap@h/x",
        "headers": [{"name": "Referer", "value": "http://v:pw@k/y"}],
        "body": {"type": "json", "data": r#"{"login":"joe:pw@x"}"#},
    });
    let (after, changed) = snapshot_after("http", request);
    assert!(changed);
    assert_eq!(after["http"]["url"], "u@h/x");
    assert_eq!(after["http"]["headers"][0]["value"], "http://v@k/y");
    assert_eq!(after["http"]["body"]["data"], r#"{"login":"joe:pw@x"}"#);
    assert_eq!(after["http"]["method"], "GET");
}

#[test]
fn a_graphql_snapshot_url_loses_its_password() {
    let request = serde_json::json!({"method": "POST", "url": "http://:pw@h/q"});
    let (after, changed) = snapshot_after("graphql", request);
    assert!(changed);
    assert_eq!(after["graphql"]["url"], "http://h/q");
}

#[test]
fn a_snapshot_with_nothing_to_drop_reports_no_change() {
    let request = serde_json::json!({"method": "GET", "url": "http://u:{{pw}}@h/x"});
    let (after, changed) = snapshot_after("http", request);
    assert!(!changed);
    assert_eq!(after["http"]["url"], "http://u:{{pw}}@h/x");
}

#[test]
fn a_request_snapshot_drops_an_encoded_colon_password_and_flags_it() {
    let text = "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://u%3Ap@h/x\"\n";
    let (_dir, c) = open(&[("opencollection.yml", ROOT), ("r.yml", text)]);
    let (snap, blanked) = c.request(&ip("r.yml")).unwrap().snapshot_and_blanks();
    let json: serde_json::Value = serde_json::from_str(&snap.to_json()).unwrap();
    assert_eq!(json["http"]["url"], "http://u@h/x");
    assert!(blanked);
}

#[test]
fn a_password_after_a_backslash_in_the_authority_text_is_hidden() {
    let url = "http://h\\u:pwsecret@x";
    for kept in [url_without_password(url), text_without_password(url)] {
        assert!(!kept.contains("pwsecret"), "{url:?} kept {kept:?}");
    }
}

#[test]
fn a_placeholder_before_a_backslash_never_shields_a_later_password() {
    let text = "postgres://u:{{p}}@h\\x:secret@k";
    let kept = text_without_password(text);
    assert!(!kept.contains("secret"), "{text:?} kept {kept:?}");
}
