//! The first directive of a PAC answer, mapped to the route a hop takes.

use super::PacWarning;
use crate::network::ResolvedProxy;

/// The longest answer a warning repeats, in characters.
const ANSWER_LIMIT: usize = 200;

/// The route of `answer`'s first directive: `Ok(None)` is DIRECT.
pub(crate) fn first_directive(answer: &str) -> Result<Option<ResolvedProxy>, PacWarning> {
    parse(answer).ok_or_else(|| PacWarning::Answer {
        answer: mask_userinfo(answer).chars().take(ANSWER_LIMIT).collect(),
    })
}

/// `answer` with everything up to the last `@` of each word replaced by `***`, so a proxy's
/// user name and password never reach a warning. Masked before the cut, so no fragment leaks.
fn mask_userinfo(answer: &str) -> String {
    let mut out = String::with_capacity(answer.len());
    let mut rest = answer;
    while let Some(start) = rest.find(|c: char| !(c.is_whitespace() || c == ';')) {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == ';')
            .unwrap_or(rest.len());
        let word = &rest[..end];
        match word.rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&word[at..]);
            }
            None => out.push_str(word),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// `None` when the first directive is not a usable route.
fn parse(answer: &str) -> Option<Option<ResolvedProxy>> {
    let directive = answer.split(';').map(str::trim).find(|d| !d.is_empty())?;
    let mut tokens = directive.split_ascii_whitespace();
    let keyword = tokens.next()?.to_ascii_uppercase();
    let scheme = match keyword.as_str() {
        "DIRECT" => return Some(None),
        "PROXY" | "HTTP" => "http",
        "HTTPS" => "https",
        "SOCKS" | "SOCKS5" => "socks5",
        "SOCKS4" => "socks4",
        "SOCKS4A" => "socks4a",
        "SOCKS5H" => "socks5h",
        _ => return None,
    };
    let (host, port) = split_host_port(tokens.next()?)?;
    // The constructor accepts port 0, which no proxy listens on.
    if !port.is_empty() && port.parse::<u16>().ok()? == 0 {
        return None;
    }
    // The remote-DNS path, so SOCKS4A and SOCKS5H keep name resolution at the proxy.
    ResolvedProxy::from_system(scheme, host, port).map(Some)
}

/// A bracketed IPv6 host runs to `]` and may take a `:port`; any other host must not contain `:`.
fn split_host_port(target: &str) -> Option<(&str, &str)> {
    if target.starts_with('[') {
        let end = target.find(']')? + 1;
        let (host, rest) = target.split_at(end);
        let port = match rest {
            "" => "",
            _ => rest.strip_prefix(':').filter(|p| !p.is_empty())?,
        };
        return Some((host, port));
    }
    let (host, port) = target.rsplit_once(':').unwrap_or((target, ""));
    if host.contains(':') || (target.contains(':') && port.is_empty()) {
        return None;
    }
    Some((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routed(answer: &str) -> String {
        match first_directive(answer) {
            Ok(Some(proxy)) => proxy.to_string(),
            Ok(None) => "DIRECT".to_owned(),
            Err(warning) => format!("warning: {warning}"),
        }
    }

    fn check(cases: &[(&str, &str)]) {
        for (answer, want) in cases {
            assert_eq!(routed(answer), *want, "{answer:?}");
        }
    }

    #[test]
    fn keywords_map_to_proxy_schemes() {
        check(&[
            ("PROXY 127.0.0.1:3128", "http://127.0.0.1:3128"),
            ("proxy a.test:8080", "http://a.test:8080"),
            ("HTTP a.test:8080", "http://a.test:8080"),
            ("HTTPS a.test:8443", "https://a.test:8443"),
            ("SOCKS a.test:1081", "socks5://a.test:1081"),
            ("SOCKS5 a.test:1081", "socks5://a.test:1081"),
            ("socks4 a.test:1081", "socks4://a.test:1081"),
            ("SOCKS4A a.test:1081", "socks4a://a.test:1081"),
            ("socks5h a.test:1081", "socks5h://a.test:1081"),
            ("DIRECT", "DIRECT"),
            ("direct", "DIRECT"),
            ("PROXY [::1]:3128", "http://[::1]:3128"),
            ("PROXY A.TEST:1", "http://a.test:1"),
        ]);
    }

    #[test]
    fn a_port_less_host_gets_the_schemes_default_port() {
        check(&[
            ("PROXY a.test", "http://a.test:80"),
            ("HTTP a.test", "http://a.test:80"),
            ("HTTPS a.test", "https://a.test:443"),
            ("SOCKS a.test", "socks5://a.test:1080"),
            ("SOCKS5 a.test", "socks5://a.test:1080"),
            ("SOCKS4 a.test", "socks4://a.test:1080"),
            ("SOCKS4A a.test", "socks4a://a.test:1080"),
            ("SOCKS5H a.test", "socks5h://a.test:1080"),
            ("PROXY [::1]", "http://[::1]:80"),
        ]);
    }

    #[test]
    fn only_the_first_directive_counts() {
        check(&[
            ("PROXY a.test:1; DIRECT", "http://a.test:1"),
            ("DIRECT; PROXY a.test:1", "DIRECT"),
            (" ; ;PROXY a.test:1 ;", "http://a.test:1"),
            ("PROXY  a.test:1", "http://a.test:1"),
        ]);
    }

    #[test]
    fn unusable_answers_warn_with_the_answer() {
        for answer in [
            "",
            "  ;  ",
            "PROXY",
            "PROXY a.test:0",
            "PROXY a.test:65536",
            "PROXY a.test:x",
            "PROXY a.test:1:2",
            "FTP a.test:1",
            "PROXY [::1",
            "PROXY [::1]:",
            "PROXY a.test:",
            "SOCKS6 a.test:1",
            "SOCKSX a.test:1",
        ] {
            assert_eq!(
                first_directive(answer),
                Err(PacWarning::Answer {
                    answer: answer.to_owned()
                }),
                "{answer:?}"
            );
        }
    }

    #[test]
    fn a_warning_masks_credentials_in_every_directive() {
        let answer = "PROXY user:pw@h.test:1:2; SOCKS5 u2:p2@[::1]:3 ;PROXY a@b.test:1";
        assert_eq!(
            first_directive(answer),
            Err(PacWarning::Answer {
                answer: "PROXY ***@h.test:1:2; SOCKS5 ***@[::1]:3 ;PROXY ***@b.test:1".to_owned()
            })
        );
    }

    #[test]
    fn a_long_answer_is_cut_to_200_characters() {
        match first_directive(&"é".repeat(300)) {
            Err(PacWarning::Answer { answer }) => assert_eq!(answer.chars().count(), 200),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn warnings_read_as_their_cause() {
        let cases = [
            (
                PacWarning::Fetch {
                    message: "file:///x.pac: not found".into(),
                },
                "could not be fetched: file:///x.pac: not found",
            ),
            (
                PacWarning::Script {
                    message: "boom".into(),
                },
                "script failed: boom",
            ),
            (PacWarning::Timeout, "ran past its time limit"),
            (
                PacWarning::Answer {
                    answer: "FTP a:1".into(),
                },
                "answered \"FTP a:1\", which is not a usable proxy",
            ),
        ];
        for (warning, text) in cases {
            assert_eq!(warning.to_string(), text);
        }
    }
}
