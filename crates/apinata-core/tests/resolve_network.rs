//! The collection's network block resolved into `ResolvedRequest::network()`: the proxy
//! expanded and validated, the proxy secret mark, and the client certificate chosen by domain.

use crate::network_support;

use apinata_core::{
    CertificateFiles, ClientCertificate, ContextOptions, RequestProxy, ResolveError,
    ResolvedRequest,
};
use network_support::{custom_proxy, pem_certificates, resolve_in, root_with};

const PLAIN: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn proxy_of(config: &str, options: ContextOptions) -> Result<RequestProxy, ResolveError> {
    let r = resolve_in(&root_with(config), "https://api.example.com/x", options);
    r.result.map(|req| req.network().proxy().clone())
}

fn custom(config: &str, options: ContextOptions) -> (apinata_core::ResolvedProxy, String) {
    match proxy_of(config, options).expect("resolves") {
        RequestProxy::Custom { proxy, bypass } => (proxy, bypass),
        other => panic!("{other:?}"),
    }
}

#[test]
fn no_block_inherits_and_off_expands_nothing() {
    let r = resolve_in(PLAIN, "https://a.test/", ContextOptions::new());
    assert_eq!(r.request().network().proxy(), &RequestProxy::Inherit);
    let off = "  proxy:\n    inherit: false\n    config:\n      hostname: \"{{nope}}\"\n    disabled: true\n";
    assert_eq!(proxy_of(off, ContextOptions::new()), Ok(RequestProxy::Off));
}

#[test]
fn custom_fields_are_expanded_and_normalised() {
    let config = custom_proxy(
        "{{scheme}}",
        "{{host}}",
        "{{port}}",
        Some(("u", "p w")),
        "localhost, {{skip}}",
    );
    let options = ContextOptions::new()
        .env_var("scheme", "SOCKS5")
        .env_var("host", "Proxy.Test")
        .env_var("port", "1081")
        .env_var("skip", ".internal");
    let (proxy, bypass) = custom(&config, options);
    assert_eq!(
        (proxy.scheme(), proxy.host(), proxy.port()),
        ("socks5", "proxy.test", 1081)
    );
    assert_eq!(proxy.to_string(), "socks5://proxy.test:1081");
    assert_eq!(bypass, "localhost, .internal");
    assert!(!format!("{proxy:?}").contains("p w"));
}

#[test]
fn an_empty_port_takes_the_scheme_default() {
    for (scheme, port) in [
        ("http", 80),
        ("https", 443),
        ("socks4", 1080),
        ("socks5", 1080),
    ] {
        let (proxy, _) = custom(
            &custom_proxy(scheme, "p.test", "", None, ""),
            ContextOptions::new(),
        );
        assert_eq!(proxy.port(), port, "{scheme}");
    }
}

#[test]
fn bad_fields_are_invalid_proxy_errors_that_quote_no_value() {
    let cases = [
        ("gopherz", "p.test", "8080", "gopherz"),
        ("http", "", "8080", ""),
        ("http", "p.test", "70000", "70000"),
        ("http", "p.test", "x1y", "x1y"),
        ("http", "a b.test", "8080", "a b.test"),
        ("http", "u@p.test", "8080", "u@p.test"),
        ("http", "p.test/x", "8080", "p.test/x"),
        ("http", "p.test\\\\x", "8080", "p.test"),
    ];
    for (scheme, host, port, bad) in cases {
        let err = proxy_of(
            &custom_proxy(scheme, host, port, None, ""),
            ContextOptions::new(),
        )
        .expect_err("invalid");
        let ResolveError::InvalidProxy { reason } = &err else {
            panic!("{err:?}")
        };
        assert!(bad.is_empty() || !reason.contains(bad), "{reason}");
    }
}

#[test]
fn an_undefined_variable_in_a_proxy_field_fails() {
    let err = proxy_of(
        &custom_proxy("http", "{{nope}}", "1", None, ""),
        ContextOptions::new(),
    );
    assert_eq!(
        err,
        Err(ResolveError::UndefinedVariable {
            name: "nope".into(),
            hint: None
        })
    );
}

fn with_proxy(
    host: &str,
    port: &str,
    auth: Option<(&str, &str)>,
    options: ContextOptions,
) -> network_support::Resolved {
    resolve_in(
        &root_with(&custom_proxy("http", host, port, auth, "")),
        "https://api.example.com/x",
        options,
    )
}

#[test]
fn proxy_credentials_join_the_sensitive_values() {
    let r = with_proxy(
        "p.test",
        "8080",
        Some(("lit-user", "{{process.env.PW}}")),
        ContextOptions::new().process_env([("PW", "pw-from-env")]),
    );
    let values = r.request().sensitive_values();
    assert!(values.iter().any(|v| v == "lit-user"), "{values:?}");
    assert!(values.iter().any(|v| v == "pw-from-env"), "{values:?}");
}

#[test]
fn a_sensitive_value_in_the_proxy_host_or_port_marks_the_proxy() {
    let secret = || ContextOptions::new().process_env([("H", "p.test"), ("P", "8080")]);
    let marked = |r: network_support::Resolved| r.request().network().secret_in_proxy();
    assert!(marked(with_proxy(
        "{{process.env.H}}",
        "8080",
        None,
        secret()
    )));
    assert!(marked(with_proxy(
        "p.test",
        "{{process.env.P}}",
        None,
        secret()
    )));
    // The proxy's own password being the same secret does not excuse the host.
    assert!(marked(with_proxy(
        "{{process.env.H}}",
        "8080",
        Some(("u", "{{process.env.H}}")),
        secret()
    )));
    assert!(!marked(with_proxy(
        "p.test",
        "8080",
        Some(("u", "{{process.env.H}}")),
        secret()
    )));
    assert!(!marked(with_proxy(
        "{{h}}",
        "8080",
        None,
        ContextOptions::new().env_var("h", "p.test")
    )));
}

fn chosen(config: &str, url: &str, options: ContextOptions) -> Option<(String, CertificateFiles)> {
    let r = resolve_in(&root_with(config), url, options);
    let req: &ResolvedRequest = r.request();
    req.network()
        .certificate()
        .map(|c| (c.domain().to_owned(), c.files().clone()))
}

#[test]
fn the_first_enabled_matching_certificate_wins_and_relative_paths_join_the_root() {
    let abs = std::env::temp_dir()
        .join("abs-one.key")
        .display()
        .to_string();
    let certs = format!(
        "  clientCertificates:\n    - domain: \"*.example.com\"\n      type: pem\n      certificateFilePath: off.pem\n      privateKeyFilePath: off.key\n      disabled: true\n    - domain: api.example.com\n      type: pem\n      certificateFilePath: c/one.pem\n      privateKeyFilePath: '{abs}'\n    - domain: \"*.example.com\"\n      type: pkcs12\n      pkcs12FilePath: two.p12\n"
    );
    let r = resolve_in(
        &root_with(&certs),
        "https://api.example.com/x",
        ContextOptions::new(),
    );
    let cert = r.request().network().certificate().expect("a match");
    assert_eq!(cert.domain(), "api.example.com");
    let CertificateFiles::Pem {
        certificate,
        private_key,
    } = cert.files()
    else {
        panic!("pem")
    };
    assert_eq!(certificate.read, r.root.join("c/one.pem"));
    assert_eq!(certificate.written, "c/one.pem");
    assert_eq!(private_key.read, std::path::PathBuf::from(&abs));
    let (_, files) =
        chosen(&certs, "https://www.example.com/", ContextOptions::new()).expect("pkcs12 entry");
    assert!(matches!(files, CertificateFiles::Pkcs12 { .. }));
}

#[test]
fn domains_match_as_brunos_prefix_regex() {
    let cases = [
        ("api.example.com", "https://api.example.com/x", true),
        ("api.example.com", "http://api.example.com/x", false),
        (
            "api.example.com",
            "https://api.example.com.evil.test/",
            true,
        ),
        ("api.example.com", "https://apixexample.com/", false),
        ("*.example.com", "https://a.b.example.com/", true),
        ("*.example.com", "https://example.com/", false),
        (
            "api.example.com:8443",
            "https://api.example.com:8443/",
            true,
        ),
        ("api.example.com:8443", "https://api.example.com/", false),
        ("*", "https://anything.test/", true),
        ("bad(", "https://bad(.test/", false),
    ];
    for (domain, url, matches) in cases {
        let config = pem_certificates(&[(domain, "c.pem", "k.pem", "")]);
        assert_eq!(
            chosen(&config, url, ContextOptions::new()).is_some(),
            matches,
            "{domain} {url}"
        );
    }
}

#[test]
fn templates_expand_and_the_passphrase_is_sensitive() {
    let config = pem_certificates(&[("{{d}}", "{{dir}}/c.pem", "k.pem", "{{process.env.PP}}")]);
    let options = ContextOptions::new()
        .env_var("d", "api.example.com")
        .env_var("dir", "certs")
        .process_env([("PP", "pass-phrase")]);
    let r = resolve_in(&root_with(&config), "https://api.example.com/", options);
    let req = r.request();
    let CertificateFiles::Pem { certificate, .. } =
        req.network().certificate().expect("match").files()
    else {
        panic!("pem")
    };
    assert_eq!(certificate.read, r.root.join("certs/c.pem"));
    assert_eq!(certificate.written, "{{dir}}/c.pem");
    assert!(req.sensitive_values().iter().any(|v| v == "pass-phrase"));
    assert!(!format!("{req:?}").contains("pass-phrase"));
}

#[test]
fn context_certificates_come_after_the_collections() {
    let config = pem_certificates(&[("api.example.com", "coll.pem", "coll.key", "")]);
    let extra = vec![
        ClientCertificate::pem("api.example.com", "cli.pem", "cli.key"),
        ClientCertificate::pem("other.test", "o.pem", "o.key"),
    ];
    let written = |url: &str| match chosen(
        &config,
        url,
        ContextOptions::new().client_certificates(extra.clone()),
    ) {
        Some((_, CertificateFiles::Pem { certificate, .. })) => certificate.written,
        other => panic!("{other:?}"),
    };
    assert_eq!(written("https://api.example.com/"), "coll.pem");
    assert_eq!(written("https://other.test/"), "o.pem");
}
