//! curl 8.22.0's option table and the scan that splits curl's arguments into flags and URLs.

use std::sync::LazyLock;

use super::CurlError;
use super::shell::Word;

/// One curl option: its short letter, long name and whether it takes a value.
#[derive(Debug)]
pub(crate) struct CurlOption {
    pub short: Option<char>,
    pub long: &'static str,
    pub takes_value: bool,
}

// Generated from `curl --help all`.
static TABLE: &str = include_str!("curl-options-8.22.0.txt");

// The table is a vendored file with a long name on every line, so a bad line is a bug every run hits.
#[allow(clippy::expect_used)]
static OPTIONS: LazyLock<Vec<CurlOption>> = LazyLock::new(|| {
    TABLE
        .lines()
        .map(|line| {
            let mut parts = line.split(' ');
            let short = parts
                .next()
                .and_then(|s| s.strip_prefix('-'))
                .and_then(|s| s.chars().next());
            let long = parts
                .next()
                .and_then(|l| l.strip_prefix("--"))
                .expect("table line has a long name");
            let takes_value = parts.next() == Some("value");
            CurlOption {
                short,
                long,
                takes_value,
            }
        })
        .collect()
});

/// Every option curl 8.22.0 lists in `--help all`, in the table's order.
pub(crate) fn options() -> &'static [CurlOption] {
    &OPTIONS
}

/// "-H", "--header" or "--no-location" (a negated boolean); None when curl 8.22.0 has no such option.
#[cfg(test)]
pub(crate) fn lookup(written: &str) -> Option<&'static CurlOption> {
    if let Some(name) = written.strip_prefix("--") {
        return resolve_long(name).map(|(option, _)| option);
    }
    let mut chars = written.strip_prefix('-')?.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => short(c),
        _ => None,
    }
}

fn short(c: char) -> Option<&'static CurlOption> {
    options().iter().find(|o| o.short == Some(c))
}

/// The option a long name means, and whether it was the `--no-` form of a boolean.
fn resolve_long(name: &str) -> Option<(&'static CurlOption, bool)> {
    if let Some(option) = options().iter().find(|o| o.long == name) {
        return Some((option, false));
    }
    let base = name.strip_prefix("no-")?;
    options()
        .iter()
        .find(|o| o.long == base && !o.takes_value)
        .map(|option| (option, true))
}

/// The options whose meaning the importer maps onto a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Known {
    UserAgent,
    Header,
    Data,
    DataUrlencode,
    DataRaw,
    DataBinary,
    Json,
    User,
    Method,
    Cookie,
    Form,
    Url,
    Head,
    Compressed,
    Insecure,
    Digest,
    Ntlm,
    Get,
    Bearer,
    AwsSigv4,
    MaxTime,
    MaxRedirs,
    Location,
}

/// The options mapped onto a request, keyed by curl's long name: Bruno's FLAG_CATEGORIES
/// (parse-curl.js), `url`, and the auth and settings options Apinata can express.
pub(crate) fn known(long: &str) -> Option<Known> {
    Some(match long {
        "user-agent" => Known::UserAgent,
        "header" => Known::Header,
        "data" | "data-ascii" => Known::Data,
        "data-urlencode" => Known::DataUrlencode,
        "data-raw" => Known::DataRaw,
        "data-binary" => Known::DataBinary,
        "json" => Known::Json,
        "user" => Known::User,
        "request" => Known::Method,
        "cookie" => Known::Cookie,
        "form" => Known::Form,
        "url" => Known::Url,
        "head" => Known::Head,
        "compressed" => Known::Compressed,
        "insecure" => Known::Insecure,
        "digest" => Known::Digest,
        "ntlm" => Known::Ntlm,
        "get" => Known::Get,
        "oauth2-bearer" => Known::Bearer,
        "aws-sigv4" => Known::AwsSigv4,
        "max-time" => Known::MaxTime,
        "max-redirs" => Known::MaxRedirs,
        "location" => Known::Location,
        _ => return None,
    })
}

/// One option as written, with the value curl would give it.
#[derive(Debug)]
pub(crate) struct Flag {
    pub written: String,
    pub offset: usize,
    pub option: Option<&'static CurlOption>,
    /// None for options the importer does not map, and for a negated form.
    pub known: Option<Known>,
    /// Whether it was written as the `--no-` form of a boolean.
    pub negated: bool,
    pub value: Option<Word>,
}

/// A word after `curl`: an option, or a positional argument (a URL).
#[derive(Debug)]
pub(crate) enum Arg {
    Flag(Flag),
    Positional(Word),
}

/// The words after `curl`. A value is the next word whatever it looks like; a missing one is MissingValue.
pub(crate) fn scan(words: &[Word]) -> Result<Vec<Arg>, CurlError> {
    let mut args = Vec::new();
    let mut rest = words.iter();
    while let Some(word) = rest.next() {
        let text = word.text.as_str();
        if !text.starts_with('-') || text.len() == 1 {
            args.push(Arg::Positional(word.clone()));
            continue;
        }
        if let Some(name) = text.strip_prefix("--") {
            // Since curl 8.16.0 `--name=value` attaches the value; a boolean ignores it.
            let attached = resolve_long(name)
                .is_none()
                .then(|| name.split_once('='))
                .flatten()
                .and_then(|(base, value)| Some((resolve_long(base)?, base, value)));
            let (resolved, written) = match attached {
                Some((resolved, base, _)) => (Some(resolved), &text[..2 + base.len()]),
                // An unknown `--name=value` is written as `--name`, so notes never carry its value.
                None => (
                    resolve_long(name),
                    name.split_once('=')
                        .map_or(text, |(base, _)| &text[..2 + base.len()]),
                ),
            };
            let option = resolved.map(|(option, _)| option);
            let known = resolved
                .filter(|(_, negated)| !negated)
                .and_then(|(o, _)| known(o.long));
            let negated = resolved.is_some_and(|(_, negated)| negated);
            let value = match (option.is_some_and(|o| o.takes_value), attached) {
                (false, _) => None,
                (true, Some((_, _, value))) => Some(Word {
                    text: value.to_owned(),
                    offset: word.offset + written.len() + 1,
                }),
                (true, None) => Some(next_value(&mut rest, text, word.offset)?),
            };
            args.push(Arg::Flag(Flag {
                written: written.to_owned(),
                offset: word.offset,
                option,
                known,
                negated,
                value,
            }));
            continue;
        }
        // A cluster like `-sSL` or `-XPOST`: each letter is an option until one takes a value,
        // which then takes the rest of the word, or the next word when nothing is left. curl
        // rejects a word with an unknown letter, so that letter is kept and the rest is not read.
        for (index, c) in text.char_indices().skip(1) {
            let option = short(c);
            let written = format!("-{c}");
            let mut value = None;
            let takes_value = option.is_some_and(|o| o.takes_value);
            if takes_value {
                let after = index + c.len_utf8();
                value = Some(if after < text.len() {
                    Word {
                        text: text[after..].to_owned(),
                        offset: word.offset + after,
                    }
                } else {
                    next_value(&mut rest, &written, word.offset)?
                });
            }
            let known = option.and_then(|o| known(o.long));
            args.push(Arg::Flag(Flag {
                written,
                offset: word.offset,
                option,
                known,
                negated: false,
                value,
            }));
            if takes_value || option.is_none() {
                break;
            }
        }
    }
    Ok(args)
}

fn next_value<'a>(
    rest: &mut impl Iterator<Item = &'a Word>,
    flag: &str,
    offset: usize,
) -> Result<Word, CurlError> {
    rest.next().cloned().ok_or_else(|| CurlError::MissingValue {
        flag: flag.to_owned(),
        offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::CurlError;

    fn show(texts: &[&str]) -> Vec<String> {
        let words: Vec<Word> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| Word {
                text: (*t).to_owned(),
                offset: i * 10,
            })
            .collect();
        scan(&words)
            .expect("scans")
            .into_iter()
            .map(|arg| match arg {
                Arg::Positional(word) => format!("url {}", word.text),
                Arg::Flag(f) => {
                    let kind = match (f.known, f.option) {
                        (Some(k), _) => format!("{k:?}"),
                        (None, Some(_)) => "curl".to_owned(),
                        (None, None) => "unknown".to_owned(),
                    };
                    match f.value {
                        Some(v) => format!("{} {kind} = {}", f.written, v.text),
                        None => format!("{} {kind}", f.written),
                    }
                }
            })
            .collect()
    }

    #[test]
    fn the_table_is_curl_8_22_0s() {
        assert_eq!(options().len(), 274);
        assert_eq!(options().iter().filter(|o| o.takes_value).count(), 146);
        assert_eq!(options().iter().filter(|o| o.short.is_some()).count(), 59);
        let header = lookup("-H").expect("-H");
        assert_eq!((header.long, header.takes_value), ("header", true));
        assert_eq!(lookup("--oauth2-bearer").map(|o| o.takes_value), Some(true));
        assert_eq!(lookup("-L").map(|o| o.long), Some("location"));
        assert_eq!(lookup("--no-location").map(|o| o.takes_value), Some(false));
        assert!(lookup("--no-header").is_none(), "only booleans negate");
        assert!(lookup("--frobnicate").is_none());
        assert!(lookup("-W").is_none());
    }

    #[test]
    fn brunos_flags_agree_with_curl_on_values() {
        let bruno = [
            ("user-agent", true),
            ("header", true),
            ("data", true),
            ("data-ascii", true),
            ("data-urlencode", true),
            ("json", true),
            ("user", true),
            ("request", true),
            ("cookie", true),
            ("form", true),
            ("data-raw", true),
            ("data-binary", true),
            ("url", true),
            ("head", false),
            ("compressed", false),
            ("insecure", false),
            ("digest", false),
            ("ntlm", false),
            ("get", false),
        ];
        for (long, takes_value) in bruno {
            let option = lookup(&format!("--{long}")).expect(long);
            assert_eq!(option.takes_value, takes_value, "{long}");
            assert!(known(option.long).is_some(), "{long}");
        }
        // The options after Bruno's table are Apinata's own mappings.
        for long in ["oauth2-bearer", "aws-sigv4", "max-time", "max-redirs"] {
            let option = lookup(&format!("--{long}")).expect(long);
            assert!(option.takes_value && known(option.long).is_some(), "{long}");
        }
        assert_eq!(
            options().iter().filter(|o| known(o.long).is_some()).count(),
            bruno.len() + 5
        );
        assert!(
            !lookup("--location").expect("location").takes_value && known("location").is_some()
        );
    }

    #[test]
    fn clusters_and_attached_values_split_like_curl() {
        assert_eq!(
            show(&["-sSL", "-XPOST", "-HAccept: x", "-kI", "https://x.test"]),
            [
                "-s curl",
                "-S curl",
                "-L Location",
                "-X Method = POST",
                "-H Header = Accept: x",
                "-k Insecure",
                "-I Head",
                "url https://x.test",
            ]
        );
        assert_eq!(
            show(&["-sd", "a=1", "-G"]),
            ["-s curl", "-d Data = a=1", "-G Get"]
        );
    }

    #[test]
    fn unknown_flags_consume_the_values_curl_says_they_take() {
        assert_eq!(
            show(&[
                "--connect-timeout",
                "10",
                "-o",
                "out.json",
                "--no-progress-meter",
                "--frob",
                "x.test"
            ]),
            [
                "--connect-timeout curl = 10",
                "-o curl = out.json",
                "--no-progress-meter curl",
                "--frob unknown",
                "url x.test",
            ]
        );
        assert_eq!(show(&["-H", "-v"]), ["-H Header = -v"]);
        assert_eq!(show(&["-", "--"]), ["url -", "-- unknown"]);
    }

    #[test]
    fn a_flag_without_its_value_is_an_error() {
        let words = [
            Word {
                text: "-o".into(),
                offset: 0,
            },
            Word {
                text: "-sH".into(),
                offset: 3,
            },
        ];
        assert_eq!(
            scan(&words[1..]).expect_err("needs a value"),
            CurlError::MissingValue {
                flag: "-H".into(),
                offset: 3
            }
        );
        assert_eq!(
            scan(&words[..1]).expect_err("needs a value"),
            CurlError::MissingValue {
                flag: "-o".into(),
                offset: 0
            }
        );
    }
    #[test]
    fn long_options_take_a_value_after_an_equals_sign() {
        assert_eq!(
            show(&[
                "--data=a=1",
                "--header=",
                "--compressed=x",
                "--frob=1",
                "x.test"
            ]),
            [
                "--data Data = a=1",
                "--header Header = ",
                "--compressed Compressed",
                "--frob unknown",
                "url x.test",
            ]
        );
    }
}
