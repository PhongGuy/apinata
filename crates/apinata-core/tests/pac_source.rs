//! The PAC source parser shared by every surface: URLs kept, paths turned into file URLs, and
//! credentials redacted wherever the source is shown.

use apinata_core::{GlobalProxy, NetworkSettings, PacSource, PacSourceError};

fn parse(input: &str) -> Result<String, PacSourceError> {
    PacSource::parse(input, None).map(|s| s.as_str().to_owned())
}

#[test]
fn urls_with_a_supported_scheme_are_kept_normalised() {
    assert_eq!(
        parse("HTTP://Example.COM/p.pac"),
        Ok("http://example.com/p.pac".to_owned())
    );
    assert_eq!(
        parse("  https://a.test/proxy.pac \n"),
        Ok("https://a.test/proxy.pac".to_owned())
    );
    assert_eq!(
        parse("file:///etc/proxy.pac"),
        Ok("file:///etc/proxy.pac".to_owned())
    );
}

#[test]
fn empty_overlong_and_unsupported_inputs_are_refused() {
    assert_eq!(parse(""), Err(PacSourceError::Empty));
    assert_eq!(parse(" \t\n"), Err(PacSourceError::Empty));
    let at_limit = format!("http://a.test/{}", "a".repeat(2048 - 14));
    assert_eq!(at_limit.len(), 2048);
    assert!(parse(&at_limit).is_ok());
    assert_eq!(parse(&format!("{at_limit}a")), Err(PacSourceError::TooLong));
    assert_eq!(
        parse("ftp://a.test/p.pac"),
        Err(PacSourceError::Scheme {
            scheme: "ftp".to_owned()
        })
    );
    assert_eq!(
        parse("javascript:alert(1)"),
        Err(PacSourceError::Scheme {
            scheme: "javascript".to_owned()
        })
    );
    assert!(matches!(
        parse("http://"),
        Err(PacSourceError::Invalid { .. })
    ));
}

#[cfg(unix)]
#[test]
fn paths_become_file_urls() {
    assert_eq!(
        parse("/etc/proxy.pac"),
        Ok("file:///etc/proxy.pac".to_owned())
    );
    assert_eq!(
        parse("/tmp/my pac/proxy.pac"),
        Ok("file:///tmp/my%20pac/proxy.pac".to_owned())
    );
    assert_eq!(
        parse("proxy.pac"),
        Err(PacSourceError::Relative {
            path: "proxy.pac".to_owned()
        })
    );
    let base = std::path::Path::new("/srv/collections");
    assert_eq!(
        PacSource::parse("pac/proxy.pac", Some(base)).map(|s| s.as_str().to_owned()),
        Ok("file:///srv/collections/pac/proxy.pac".to_owned())
    );
    // A drive-letter path is a relative file name here, never a URL with scheme `c`.
    assert_eq!(
        parse("C:\\pac\\proxy.pac"),
        Err(PacSourceError::Relative {
            path: "C:\\pac\\proxy.pac".to_owned()
        })
    );
}

#[cfg(windows)]
#[test]
fn windows_paths_become_file_urls() {
    assert_eq!(
        parse("C:\\pac\\proxy.pac"),
        Ok("file:///C:/pac/proxy.pac".to_owned())
    );
    assert_eq!(
        parse("proxy.pac"),
        Err(PacSourceError::Relative {
            path: "proxy.pac".to_owned()
        })
    );
}

#[cfg(unix)]
#[test]
fn a_relative_path_error_never_repeats_credentials() {
    for (input, shown) in [
        ("a:pw@host/p.pac", "***@host/p.pac"),
        ("u:pw@host:8080/p.pac", "***@host:8080/p.pac"),
        ("u@host/p.pac", "***@host/p.pac"),
        ("a:p/w@host/p.pac", "***@host/p.pac"),
        ("dir/a@b.pac", "dir/a@b.pac"),
    ] {
        let err = parse(input).expect_err(input);
        assert_eq!(
            err,
            PacSourceError::Relative {
                path: shown.to_owned()
            },
            "{input}"
        );
        let text = err.to_string();
        assert!(!text.contains("pw") && !text.contains("p/w"), "{err}");
    }
}

#[test]
fn a_host_and_port_without_a_scheme_asks_for_http() {
    for input in [
        "pac.test:8080/p.pac",
        "localhost:8080",
        "pac.test:3128?x=1",
        "127.0.0.1:8080",
        "[::1]:8080",
        "10.0.0.1:8080/p.pac",
    ] {
        let err = parse(input).expect_err(input);
        assert_eq!(err, PacSourceError::MissingScheme, "{input}");
        assert_eq!(
            err.to_string(),
            "the PAC source needs http:// or https:// in front of the host"
        );
    }
}

#[test]
fn credentials_are_kept_for_fetching_and_redacted_for_display() {
    let source =
        PacSource::parse("https://user:p4ss@pac.corp.test/proxy.pac", None).expect("parses");
    assert_eq!(source.as_str(), "https://user:p4ss@pac.corp.test/proxy.pac");
    assert_eq!(source.to_string(), "https://***@pac.corp.test/proxy.pac");
    let debug = format!("{source:?}");
    assert!(
        debug.contains("https://***@pac.corp.test/proxy.pac")
            && !debug.contains("p4ss")
            && !debug.contains("user"),
        "{debug}"
    );
    let user_only = PacSource::parse("http://user@pac.corp.test/p.pac", None).expect("parses");
    assert_eq!(user_only.to_string(), "http://***@pac.corp.test/p.pac");
    let plain = PacSource::parse("http://pac.corp.test/p.pac", None).expect("parses");
    assert_eq!(plain.to_string(), "http://pac.corp.test/p.pac");
}

#[test]
fn errors_read_as_sentences() {
    let cases = [
        (PacSourceError::Empty, "the PAC source is empty"),
        (
            PacSourceError::TooLong,
            "the PAC source is longer than 2048 bytes",
        ),
        (
            PacSourceError::Scheme {
                scheme: "ftp".into(),
            },
            "the PAC source must be an http, https or file URL, not ftp",
        ),
        (
            PacSourceError::Relative {
                path: "p.pac".into(),
            },
            "the PAC path p.pac is relative and there is no directory to resolve it against",
        ),
        (
            PacSourceError::Invalid {
                message: "empty host".into(),
            },
            "the PAC source is not a valid URL: empty host",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn the_global_layer_takes_a_pac_source_and_never_prints_its_credentials() {
    let source = PacSource::parse("https://u:secret@pac.test/p.pac", None).expect("parses");
    let settings = NetworkSettings::new().proxy(GlobalProxy::Pac {
        source: source.clone(),
    });
    assert_eq!(settings.proxy, GlobalProxy::Pac { source });
    assert!(!format!("{settings:?}").contains("secret"));
}
