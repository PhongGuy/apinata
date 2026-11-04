//! The one header-name list, and the session log using it.

use crate::common;
use crate::session_log_support;

use apinata_core::{ExecOptions, is_credential_header, mask_response_header};
use common::resolved_one;
use session_log_support::{header, logged, only};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn the_list_covers_every_credential_header_in_any_case() {
    for name in [
        "authorization",
        "Proxy-Authorization",
        "COOKIE",
        "Set-Cookie",
        "X-WSSE",
        "x-amz-security-token",
    ] {
        assert!(is_credential_header(name), "{name}");
    }
    for name in ["content-type", "x-api-key", "x-wsse-extra", ""] {
        assert!(!is_credential_header(name), "{name}");
    }
}

#[test]
fn a_response_header_is_masked_by_name() {
    assert_eq!(
        mask_response_header("Set-Cookie", " sid=abc123; Path=/"),
        "sid=[redacted]"
    );
    assert_eq!(mask_response_header("set-cookie", "junk"), "[redacted]");
    assert_eq!(
        mask_response_header("Authorization", "Bearer t"),
        "[redacted]"
    );
    assert_eq!(
        mask_response_header("content-type", "text/plain"),
        "text/plain"
    );
}

#[test]
fn a_set_cookie_pair_without_an_equals_sign_is_hidden_whole() {
    // The pair is the text before the first `;`; with no `=` in it the whole pair is the value.
    for value in [
        "sekrit-token",
        "sekrit-token; Path=/",
        "sekrit-token;HttpOnly",
        "=sekrit-token",
        " sekrit-token ; Max-Age=0",
    ] {
        let shown = mask_response_header("Set-Cookie", value);
        assert!(!shown.contains("sekrit"), "{value:?} showed {shown:?}");
        assert_eq!(shown, "[redacted]", "{value:?}");
    }
}

#[tokio::test]
async fn a_literal_session_token_header_is_hidden_by_name_in_the_log() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let under = "  headers:\n    - name: X-Amz-Security-Token\n      value: literal-session-9921\n";
    let (exec, log) = logged(ExecOptions::new());
    exec.send(&resolved_one("GET", &format!("{}/v", s.uri()), under, ""))
        .await
        .expect("sent");
    assert_eq!(
        header(&only(&log).hops[0].request_headers, "x-amz-security-token"),
        Some("[redacted]")
    );
}

#[test]
fn a_location_style_header_loses_its_userinfo() {
    for (name, value, kept) in [
        (
            "Location",
            "http://joe:hop-pa55@127.0.0.1/next",
            "http://127.0.0.1/next",
        ),
        (
            "content-location",
            "https://joe:hop-pa55@h.test/a",
            "https://h.test/a",
        ),
        ("REFRESH", "5; url=http://joe:hop-pa55@h/", "h/"),
        // A scheme-relative url carries userinfo the same way.
        ("Location", "//joe:hop-pa55@h/next", "//h/next"),
        ("Content-Location", "//joe:hop-pa55@h.test/a", "//h.test/a"),
        ("Refresh", "0; url=//joe:hop-pa55@h/", "0; url=//h/"),
        ("Refresh", "0; URL='//joe:hop-pa55@h/'", "0; URL='//h/'"),
        ("Refresh", "0; url=\"//joe:hop-pa55@h/\"", "0; url=\"//h/\""),
    ] {
        let shown = mask_response_header(name, value);
        assert!(!shown.contains("hop-pa55"), "{name}: {shown}");
        assert!(shown.contains(kept), "{name}: {shown}");
    }
    // Shapes a url parser reads credentials out of though they lack `scheme://` or `//`.
    for shape in [
        "\\\\joe:hop-pa55@h/next",
        "/\\joe:hop-pa55@h/next",
        "https:joe:hop-pa55@h/next",
        "https:/joe:hop-pa55@h/next",
        "https:\\\\joe:hop-pa55@h/next",
    ] {
        for (name, value) in [
            ("Location", shape.to_owned()),
            ("Content-Location", shape.to_owned()),
            ("Refresh", format!("0; url={shape}")),
        ] {
            let shown = mask_response_header(name, &value);
            assert!(!shown.contains("hop-pa55"), "{name} {value:?}: {shown}");
        }
    }
}

/// Every way of writing `scheme`, slashes and userinfo that a url parser accepts.
fn url_shapes() -> Vec<String> {
    let schemes = ["", "http:", "HTTPS:", "hTtP:"];
    let slashes = ["", "/", "//", "\\\\", "/\\", "\\/", "///"];
    let userinfo = ["u:Pw9zq@", ":Pw9zq@", "u%3APw9zq@", "u:Pw9zq%40x@"];
    let mut shapes = Vec::new();
    for scheme in schemes {
        for slash in slashes {
            // No scheme and at most one slash is a path on the current host, not an authority.
            if scheme.is_empty() && slash.len() < 2 {
                continue;
            }
            for info in userinfo {
                shapes.push(format!("{scheme}{slash}{info}h.test/next"));
            }
        }
    }
    for info in userinfo {
        shapes.push(format!("htt\tps://{info}h.test/next"));
        shapes.push(format!("h\tttps:{info}h.test/next"));
        shapes.push(format!("   //{info}h.test/next"));
        shapes.push(format!("  https:\\\\{info}h.test/next"));
    }
    shapes
}

#[test]
fn a_url_header_in_any_parser_shape_never_shows_its_password() {
    for shape in url_shapes() {
        for (name, value) in [
            ("Location", shape.clone()),
            ("content-location", shape.clone()),
            ("Refresh", format!("0; url={shape}")),
            ("refresh", format!("5;URL = '{shape}'")),
            ("Refresh", format!("1, \"{shape}\"")),
        ] {
            let shown = mask_response_header(name, &value);
            assert!(!shown.contains("Pw9zq"), "{name} {value:?}: {shown}");
        }
    }
}

#[test]
fn a_url_header_without_credentials_is_unchanged() {
    for (name, value) in [
        ("Location", "/a//b@c"),
        ("Location", "//h/a?e=a@b"),
        ("Location", "/next"),
        ("Location", "https://h/p"),
        ("Location", "u:p@h/next"),
        ("Location", "/u:p@h/next"),
        ("Content-Location", "http://h.test/a?q=1#f"),
        ("Refresh", "0; url=/x"),
        ("Refresh", "5"),
        ("Refresh", "0; URL='https://h/p'"),
    ] {
        assert_eq!(mask_response_header(name, value), value, "{name}");
    }
}

#[test]
fn a_double_slash_inside_a_path_is_not_a_scheme_relative_url() {
    assert_eq!(mask_response_header("Location", "/a//b@c"), "/a//b@c");
    assert_eq!(
        mask_response_header("Location", "//h/a?e=a@b"),
        "//h/a?e=a@b"
    );
}

#[tokio::test]
async fn the_session_log_never_keeps_a_redirect_url_password() {
    for (shape, follow) in [
        ("http://joe:hop-pa55@127.0.0.1:PORT/next", false),
        ("/\\joe:hop-pa55@127.0.0.1:PORT/next", false),
        ("https:joe:hop-pa55@127.0.0.1:PORT/next", false),
        ("\\\\joe:hop-pa55@127.0.0.1:PORT/next", true),
        ("http:\\\\joe:hop-pa55@127.0.0.1:PORT/next", true),
    ] {
        let s = MockServer::start().await;
        let port = s.uri().rsplit(':').next().expect("port").to_owned();
        let location = shape.replace("PORT", &port);
        Mock::given(wiremock::matchers::path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", location.as_str())
                    .insert_header("refresh", format!("0; url={location}").as_str()),
            )
            .mount(&s)
            .await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200))
            .mount(&s)
            .await;
        let top = if follow {
            ""
        } else {
            "settings:\n  followRedirects: false\n"
        };
        let (exec, log) = logged(ExecOptions::new());
        exec.send(&resolved_one("GET", &format!("{}/start", s.uri()), "", top))
            .await
            .expect("sent");
        let entry = only(&log);
        assert_eq!(entry.hops.len(), if follow { 2 } else { 1 }, "{shape}");
        for hop in &entry.hops {
            let pairs = hop.response_headers.iter().chain(&hop.request_headers);
            for (name, value) in pairs {
                assert!(!value.contains("hop-pa55"), "{shape}: {name}: {value}");
            }
            assert!(!hop.url.contains("hop-pa55"), "{shape}: {}", hop.url);
        }
    }
}

#[test]
fn a_refresh_url_keeps_its_url_prefix_when_its_userinfo_goes() {
    for (value, shown) in [
        (
            "0; url=joe:pw@127.0.0.1:8080/next",
            "0; url=127.0.0.1:8080/next",
        ),
        ("0; URL='joe:pw@h:80/next'", "0; URL='h:80/next'"),
        ("5;url=\"joe:pw@h:80/next\"", "5;url=\"h:80/next\""),
    ] {
        assert_eq!(mask_response_header("Refresh", value), shown, "{value}");
    }
}

#[test]
fn a_refresh_key_ends_at_its_first_equals_so_no_username_part_shows() {
    for (value, shown) in [
        ("0; url=jo=e:pw@h:1/x", "0; url=h:1/x"),
        ("0; url=\"jo=e:pw@h:1/x\"", "0; url=\"h:1/x\""),
        ("0; url=u:p@h:1/x", "0; url=h:1/x"),
        ("0; jo:p=w@h:1/x", "0; h:1/x"),
        ("0; jo\"e:pw@h:1/x", "0; h:1/x"),
        ("0; 'jo=e:pw@h:1/x'", "0; 'h:1/x'"),
        ("0; jo=e:pw@h:1/x", "0; h:1/x"),
        ("0;URL=jo=e:pw@h:1/x", "0;URL=h:1/x"),
    ] {
        assert_eq!(mask_response_header("Refresh", value), shown, "{value}");
    }
}

#[test]
fn a_url_key_inside_a_username_is_part_of_the_username() {
    for (value, shown) in [
        ("0; jo;url=e:pw@h:1/x", "0; h:1/x"),
        ("a;url=e:pw@h:1/x", "h:1/x"),
        ("0;url=jo;url=e:pw@h:1/x", "0;url=h:1/x"),
        ("url=jo;url=e:pw@h:1/x", "url=h:1/x"),
        ("1.5;'url=jo:pw@h:1/x'", "1.5;'url=h:1/x'"),
        ("5;url=\"joe:pw@h:80/next\"", "5;url=\"h:80/next\""),
        ("0; url=jo=e:pw@h:1/x", "0; url=h:1/x"),
    ] {
        assert_eq!(mask_response_header("Refresh", value), shown, "{value}");
    }
}

#[test]
fn a_url_header_keeps_a_path_or_query_at_sign_that_is_not_userinfo() {
    for value in [
        "http://127.0.0.1:8080/r/joe:pw@127.0.0.1:8080/next",
        "http://127.0.0.1:8080/a//b@c",
        "https://h/p?next=//a@b",
    ] {
        assert_eq!(mask_response_header("Location", value), value);
    }
}

#[test]
fn a_url_header_loses_only_the_userinfo_of_its_authority() {
    for (value, shown) in [
        ("//u:p@h?x=//a@b", "//h?x=//a@b"),
        ("https://u:p@h?x=a@b", "https://h?x=a@b"),
        ("https://u:p@h/r#a@b", "https://h/r#a@b"),
    ] {
        assert_eq!(mask_response_header("Location", value), shown, "{value}");
    }
}

#[test]
fn a_password_in_a_url_header_query_url_stays_hidden() {
    for value in [
        "https://h/p?next=//a:Pw9zq@b",
        "https://u:Pw9zq@h?x=https://a:Pw9zq@b",
        "//u:Pw9zq@h?x=//a:Pw9zq@b",
        "https://u:Pw9zq#w@h/x",
        "https://u:Pw9zq?w@h/x",
    ] {
        let shown = mask_response_header("Location", value);
        assert!(!shown.contains("Pw9zq"), "{value}: {shown}");
    }
}
