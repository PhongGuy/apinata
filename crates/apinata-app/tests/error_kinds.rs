//! The friendly error block's classification, one row of the table per case.

use apinata_app::response_pane::{
    ErrorKind, ErrorTone, ErrorView, describe_error, describe_failure,
};
use apinata_core::{ConnectCause, ExecError, OAuth2Error, ResolveError, RunError, UndefinedHint};

fn exec(e: ExecError) -> RunError {
    RunError::Exec(e)
}

fn connect(url: &str, cause: ConnectCause) -> RunError {
    exec(ExecError::connect(url, "m", cause))
}

fn undefined(name: &str) -> RunError {
    RunError::Resolve(ResolveError::UndefinedVariable {
        name: name.into(),
        hint: None,
    })
}

/// (kind, title, sentence, retry, add_variable)
fn row(v: ErrorView) -> (ErrorKind, &'static str, String, bool, Option<String>) {
    (
        v.kind,
        v.title,
        v.sentence.unwrap_or_default(),
        v.retry,
        v.add_variable,
    )
}

fn s(text: &str) -> String {
    text.to_owned()
}

#[test]
fn connect_causes_name_the_host() {
    let d = "details";
    assert_eq!(
        row(describe_error(
            &connect("http://127.0.0.1:1/", ConnectCause::Refused),
            d,
            None
        )),
        (
            ErrorKind::Refused,
            "Connection refused",
            s("Nothing is listening at 127.0.0.1:1. Is the server running?"),
            true,
            None
        )
    );
    assert_eq!(
        row(describe_error(
            &connect("https://api.example.test/x", ConnectCause::Dns),
            d,
            None
        )),
        (
            ErrorKind::Dns,
            "DNS lookup failed",
            s("The host api.example.test could not be found. Check the URL for typos."),
            true,
            None
        )
    );
    assert_eq!(
        row(describe_error(
            &connect("https://self.test/", ConnectCause::Tls),
            d,
            None
        )),
        (
            ErrorKind::Tls,
            "TLS error",
            s("The secure connection to self.test failed."),
            true,
            None
        )
    );
    assert_eq!(
        row(describe_error(
            &connect("http://h.test:8080/", ConnectCause::Other),
            d,
            None
        )),
        (
            ErrorKind::Connect,
            "Could not connect",
            s("The connection to h.test failed."),
            true,
            None
        )
    );
    let v = describe_error(
        &connect("http://127.0.0.1:1/", ConnectCause::Refused),
        "a `b`",
        None,
    );
    assert_eq!(v.tone, ErrorTone::Danger);
    assert_eq!(
        v.message.as_deref(),
        Some(apinata_app::ui::plain_quotes("a `b`").as_str())
    );
}

#[test]
fn transport_layer_errors_have_their_own_rows() {
    let d = "details";
    let timeout = exec(ExecError::timeout("http://h.test/"));
    assert_eq!(
        row(describe_error(&timeout, d, None)),
        (
            ErrorKind::Timeout,
            "Timed out",
            s("No response within the request timeout."),
            true,
            None
        )
    );
    let proxy = exec(ExecError::proxy(
        "http://h.test/",
        "http://proxy.test:8080",
        "m",
    ));
    assert_eq!(
        row(describe_error(&proxy, d, None)),
        (
            ErrorKind::Proxy,
            "Proxy error",
            s("The proxy http://proxy.test:8080 could not be reached or refused the request."),
            true,
            None
        )
    );
    let cert = exec(ExecError::certificate("c.pem", "m"));
    assert_eq!(
        row(describe_error(&cert, d, None)),
        (
            ErrorKind::Certificate,
            "Client certificate error",
            s("The client certificate could not be loaded."),
            true,
            None
        )
    );
    let host = exec(ExecError::host_not_allowed(0, "evil.test"));
    let via = exec(ExecError::proxy_not_allowed(1, "p.test", "http://p.test:1"));
    assert_eq!(
        row(describe_error(&host, d, None)),
        (
            ErrorKind::HostNotAllowed,
            "Host not allowed",
            s("Sending to evil.test is not allowed here."),
            true,
            None
        )
    );
    assert_eq!(
        row(describe_error(&via, d, None)).2,
        "Sending to p.test is not allowed here."
    );
}

#[test]
fn errors_before_sending_say_request_not_sent_without_retry() {
    let secret = exec(ExecError::SecretInProxy);
    assert_eq!(
        row(describe_error(&secret, &secret.to_string(), None)),
        (
            ErrorKind::NotSent,
            "Request not sent",
            s("A secret cannot be used in a proxy host or port."),
            false,
            None
        )
    );
    let file = exec(ExecError::file("body.bin", "gone"));
    assert_eq!(
        row(describe_error(&file, "x", None)),
        (
            ErrorKind::NotSent,
            "Request not sent",
            s("A body file could not be read."),
            false,
            None
        )
    );
    let cyclic = RunError::Resolve(ResolveError::CyclicVariable { name: "a".into() });
    let text = cyclic.to_string();
    let mut want = text[..1].to_uppercase() + &text[1..];
    if !want.ends_with(['.', '!', '?']) {
        want.push('.');
    }
    let want = apinata_app::ui::plain_quotes(&want);
    assert!(text.contains('`') && !want.contains('`'));
    assert_eq!(row(describe_error(&cyclic, &text, None)).2, want);
    assert_eq!(
        row(describe_failure("a.yml was not found")),
        (
            ErrorKind::NotSent,
            "Request not sent",
            s("a.yml was not found"),
            false,
            None
        )
    );
    assert_eq!(describe_failure("x").message, None);
}

#[test]
fn anything_else_is_request_failed_with_its_first_line() {
    let t = exec(ExecError::transport("http://h.test/", "reset"));
    assert_eq!(
        row(describe_error(&t, "http://h.test/: reset\nmore", None)),
        (
            ErrorKind::Failed,
            "Request failed",
            s("http://h.test/: reset"),
            true,
            None
        )
    );
}

#[test]
fn undefined_variables_have_three_wordings() {
    let d = "variable `x` is not defined";
    assert_eq!(
        row(describe_error(&undefined("nope"), d, Some("dev"))),
        (
            ErrorKind::UndefinedVariable,
            "Undefined variable",
            s("nope is not defined in dev."),
            false,
            Some(s("nope"))
        )
    );
    assert_eq!(
        row(describe_error(&undefined("nope"), d, None)).2,
        "nope is not defined, and no environment is selected."
    );
    assert_eq!(
        row(describe_error(&undefined("a.b"), d, Some("dev"))),
        (
            ErrorKind::UndefinedVariable,
            "Undefined variable",
            s("a.b is not defined in dev."),
            false,
            Some(s("a.b"))
        )
    );
    assert_eq!(
        row(describe_error(
            &undefined("process.env.API_KEY"),
            d,
            Some("dev")
        )),
        (
            ErrorKind::UndefinedVariable,
            "Undefined variable",
            s("API_KEY is not set in the process environment or the collection's .env file."),
            false,
            None
        )
    );
    for odd in ["$weird", "?label", "has space", "a{b"] {
        assert_eq!(
            row(describe_error(&undefined(odd), d, Some("dev"))),
            (
                ErrorKind::UndefinedVariable,
                "Undefined variable",
                format!("{odd} is not defined."),
                false,
                None
            ),
            "{odd}"
        );
    }
    let oauth = RunError::OAuth2(OAuth2Error::Resolve(ResolveError::UndefinedVariable {
        name: "tokenUrl".into(),
        hint: None,
    }));
    assert_eq!(
        row(describe_error(&oauth, d, Some("dev"))),
        (
            ErrorKind::UndefinedVariable,
            "Undefined variable",
            s("tokenUrl is not defined in dev."),
            false,
            Some(s("tokenUrl"))
        )
    );
}

#[test]
fn request_failed_with_an_empty_first_line_has_no_sentence() {
    let t = exec(ExecError::transport("http://h.test/", ""));
    for details in ["", "\nmore"] {
        let v = describe_error(&t, details, None);
        assert_eq!(
            (v.title, v.sentence),
            ("Request failed", None),
            "{details:?}"
        );
    }
}

#[test]
fn every_tls_cause_is_a_tls_error_with_the_shared_sentence() {
    let d = "details";
    let cases = [
        (
            ConnectCause::TlsUnknownIssuer,
            "The certificate of self.test is not trusted (unknown issuer or self-signed). \
             Trust its CA or turn certificate verification off.",
        ),
        (
            ConnectCause::TlsWrongProtocol,
            "self.test did not answer TLS. The URL may need http:// instead of https://.",
        ),
        (
            ConnectCause::TlsExpired,
            "The certificate of self.test has expired.",
        ),
    ];
    for (cause, sentence) in cases {
        assert_eq!(
            row(describe_error(
                &connect("https://self.test/", cause),
                d,
                None
            )),
            (ErrorKind::Tls, "TLS error", s(sentence), true, None),
            "{cause:?}"
        );
    }
}

#[test]
fn an_undefined_variable_hint_is_worded_for_the_app() {
    let secret = UndefinedHint::SecretWithoutValue {
        environment: "dev".into(),
    };
    let hinted = |name: &str, hint: &UndefinedHint| {
        RunError::Resolve(ResolveError::UndefinedVariable {
            name: name.into(),
            hint: Some(hint.clone()),
        })
    };
    let view = describe_error(
        &hinted("tok", &secret),
        "variable `tok` is not defined",
        Some("dev"),
    );
    assert_eq!(
        view.sentence.as_deref(),
        Some(
            "tok is not defined in dev. tok is a secret with no value in environment dev; set it in the environment editor."
        )
    );
    let one = UndefinedHint::DefinedElsewhere {
        environments: vec!["dev".into()],
    };
    let view = describe_error(&hinted("base", &one), "x", None);
    assert!(
        view.sentence
            .unwrap()
            .ends_with("base is defined in environment dev; switch to it.")
    );
    let two = UndefinedHint::DefinedElsewhere {
        environments: vec!["dev".into(), "prod".into()],
    };
    let view = describe_error(&hinted("base", &two), "x", None);
    assert!(
        view.sentence
            .unwrap()
            .ends_with("base is defined in environments dev, prod; switch to one of them.")
    );
    let oauth = RunError::OAuth2(OAuth2Error::Resolve(ResolveError::UndefinedVariable {
        name: "tok".into(),
        hint: Some(secret),
    }));
    let view = describe_error(&oauth, "x", Some("dev"));
    assert!(
        view.sentence
            .unwrap()
            .contains("set it in the environment editor")
    );
}
