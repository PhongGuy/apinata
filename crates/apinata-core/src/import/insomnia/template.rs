//! Insomnia's Nunjucks templates in values.

use super::chain::Chains;
use super::tags::{Converted, convert};
use super::{Budget, Crumbs};
use crate::import::{DocumentError, ImportNote, NoteKind};

/// A value with its member-path placeholders and template tags rewritten.
pub(super) struct Rewritten {
    pub(super) text: String,
    /// Every other `{{ }}` and `{% %}` construct, in order.
    pub(super) kept: Vec<Kept>,
    /// Each response tag converted to a chained variable, in order.
    pub(super) chained: Vec<Chained>,
}

/// A response tag now read from a variable its source request sets.
pub(super) struct Chained {
    pub(super) written: String,
    pub(super) variable: String,
    /// The source request's name.
    pub(super) source: String,
    /// Whether the tag asked for the source to be sent again before reading it.
    pub(super) resent: bool,
}

/// A construct with no equivalent.
pub(super) struct Kept {
    /// The construct as written in the input.
    pub(super) written: String,
    /// The variable a tag was written as; `None` when the construct stays as written.
    pub(super) variable: Option<String>,
    /// Why a tag has no equivalent, when one is known.
    pub(super) reason: Option<String>,
}

/// Rewrite member-path placeholders to `{{path}}` and convert each named `{% tag ... %}` that
/// has an equivalent, linking response tags through `chains`. Any other named tag becomes
/// `{{tag}}`, so sending stops on the undefined variable until the user gives it a value. Every
/// other `{{ }}` and `{% %}` stays as written. Each construct but a member path and a converted
/// tag is listed in `kept`, in order. An unclosed `{{` or `{%` is plain text.
pub(super) fn rewrite(text: &str, chains: &mut Chains) -> Rewritten {
    let mut out = String::with_capacity(text.len());
    let mut kept = Vec::new();
    let mut chained = Vec::new();
    let mut rest = text;
    // Once a closer is missing from the rest, no later opener of that kind can close, so each
    // kind searches for its closer at most once without consuming text.
    let (mut braces_open, mut tags_open) = (true, true);
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let close = if rest.starts_with("{{") {
            "}}"
        } else if rest.starts_with("{%") {
            "%}"
        } else {
            out.push('{');
            rest = &rest[1..];
            continue;
        };
        let can_close = if close == "}}" {
            &mut braces_open
        } else {
            &mut tags_open
        };
        // An opener without its closer is literal text; scanning resumes after it.
        let found = if *can_close {
            rest[2..].find(close)
        } else {
            None
        };
        let Some(end) = found else {
            *can_close = false;
            out.push_str(&rest[..2]);
            rest = &rest[2..];
            continue;
        };
        let whole = &rest[..end + 4];
        let body = whole[2..end + 2].trim();
        let path = body.strip_prefix("_.").unwrap_or(body);
        // A bare `_` is the whole render context, which has no counterpart.
        if close == "}}" && body != "_" && is_member_path(path) {
            out.push_str("{{");
            out.push_str(path);
            out.push_str("}}");
        } else {
            let variable = (close == "%}").then(|| tag_name(body)).flatten();
            let converted = variable.map(|name| convert(name, &body[name.len()..], chains));
            let reason = match converted {
                Some(Converted::Value(value)) => {
                    out.push_str(&value);
                    rest = &rest[end + 4..];
                    continue;
                }
                Some(Converted::Chain {
                    variable,
                    source,
                    resent,
                }) => {
                    out.push_str(&format!("{{{{{variable}}}}}"));
                    chained.push(Chained {
                        written: whole.to_owned(),
                        variable,
                        source,
                        resent,
                    });
                    rest = &rest[end + 4..];
                    continue;
                }
                Some(Converted::Kept(reason)) => Some(reason),
                None => None,
            };
            match variable {
                Some(name) => {
                    out.push_str("{{");
                    out.push_str(name);
                    out.push_str("}}");
                }
                None => out.push_str(whole),
            }
            kept.push(Kept {
                written: whole.to_owned(),
                variable: variable.map(str::to_owned),
                reason,
            });
        }
        rest = &rest[end + 4..];
    }
    out.push_str(rest);
    Rewritten {
        text: out,
        kept,
        chained,
    }
}

/// `rewrite(text).text`, pushing one `Converted` note sourced `trail / at` naming each chained
/// response tag, and one `NeedsInput` note naming every kept construct and the variable to set
/// for each tag. A computed value longer than its tag is charged to the budget.
pub(super) fn apply(
    text: &str,
    trail: &[String],
    at: &str,
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<String, DocumentError> {
    let input_len = text.len();
    let Rewritten {
        text,
        kept,
        chained,
    } = rewrite(text, &mut budget.chains);
    budget.charge(text.len().saturating_sub(input_len))?;
    if !chained.is_empty() {
        let each: Vec<String> = chained
            .iter()
            .map(|c| {
                let again = if c.resent {
                    "; it is not sent again before the read"
                } else {
                    ""
                };
                format!(
                    "`{}` reads `{{{{{}}}}}`, which request `{}` sets from its response, so send \
                     that request first{again}",
                    c.written, c.variable, c.source
                )
            })
            .collect();
        notes.push(budget.note(trail, at, NoteKind::Converted, each.join("; "))?);
    }
    let mut parts = Vec::new();
    let literal: Vec<String> = kept
        .iter()
        .filter(|k| k.variable.is_none())
        .map(|k| format!("`{}`", k.written))
        .collect();
    if !literal.is_empty() {
        parts.push(format!(
            "{} has no equivalent and is kept as written; replace it before sending",
            literal.join(", ")
        ));
    }
    let (mut written, mut names) = (Vec::new(), Vec::<String>::new());
    for (k, name) in kept.iter().filter_map(|k| Some((k, k.variable.as_ref()?))) {
        let why = k
            .reason
            .as_ref()
            .map_or(String::new(), |r| format!(" ({r})"));
        written.push(format!("`{}` as `{{{{{name}}}}}`{why}", k.written));
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    if !written.is_empty() {
        let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
        parts.push(format!(
            "a template tag could not be converted, so each is written as a variable: {}; give {} a value before sending",
            written.join(", "),
            quoted.join(", ")
        ));
    }
    if !parts.is_empty() {
        let detail = parts.join("; ");
        let mut note = budget.note(trail, at, NoteKind::NeedsInput, detail)?;
        note.asks_for = names;
        notes.push(note);
    }
    Ok(text)
}

/// The leading name of a tag body such as `now 'iso-8601'`, when it can name a plain variable:
/// it starts with a letter or `_`, so it never reads as a generated `$` variable.
fn tag_name(body: &str) -> Option<&str> {
    let len = body
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    let name = &body[..len];
    let starts_well = name
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_');
    starts_well.then_some(name)
}

/// `ident ( '.' ident | '[' digits ']' )*`, with `ident = [A-Za-z_$][A-Za-z0-9_$]*`.
fn is_member_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    let ident_start = |b: u8| b.is_ascii_alphabetic() || b == b'_' || b == b'$';
    let ident_rest = |b: u8| ident_start(b) || b.is_ascii_digit();
    let ident = |i: usize| -> Option<usize> {
        let first = *bytes.get(i)?;
        if !ident_start(first) {
            return None;
        }
        let len = bytes[i + 1..]
            .iter()
            .take_while(|&&b| ident_rest(b))
            .count();
        Some(i + 1 + len)
    };
    let Some(mut i) = ident(0) else {
        return false;
    };
    while i < bytes.len() {
        i = match bytes[i] {
            b'.' => match ident(i + 1) {
                Some(next) => next,
                None => return false,
            },
            b'[' => {
                let digits = bytes[i + 1..]
                    .iter()
                    .take_while(|b| b.is_ascii_digit())
                    .count();
                if digits == 0 || bytes.get(i + 1 + digits) != Some(&b']') {
                    return false;
                }
                i + digits + 2
            }
            _ => return false,
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::chain::Chains;
    use super::Rewritten;
    use std::collections::HashSet;

    /// `rewrite` with no requests to chain to.
    fn rewrite(text: &str) -> Rewritten {
        super::rewrite(text, &mut Chains::default())
    }

    #[test]
    fn member_paths_become_plain_placeholders() {
        for (input, want) in [
            ("{{ _.base_url }}/a", "{{base_url}}/a"),
            ("{{base_url}}", "{{base_url}}"),
            ("{{ a.b[0].c }}", "{{a.b[0].c}}"),
            ("{{ _.arr[12] }}", "{{arr[12]}}"),
            ("{{ my_.x }}", "{{my_.x}}"),
            ("{{ _._x }}", "{{_x}}"),
            ("{{ _.a }}-{{ b }}", "{{a}}-{{b}}"),
            ("{{\n  _.a\n}}", "{{a}}"),
            ("x {{ a", "x {{ a"),
            ("no templates", "no templates"),
        ] {
            let out = rewrite(input);
            assert_eq!((out.text.as_str(), out.kept.len()), (want, 0), "{input}");
        }
    }

    fn written(out: &Rewritten) -> Vec<(&str, Option<&str>)> {
        out.kept
            .iter()
            .map(|k| (k.written.as_str(), k.variable.as_deref()))
            .collect()
    }

    #[test]
    fn filters_and_expressions_stay_literal_and_are_listed() {
        for input in [
            "{{ x | upper }}",
            "{{ _['a-b'] }}",
            "{{ a + 1 }}",
            "{{ _.a.b. }}",
            "{{ 1a }}",
            "{{ }}",
        ] {
            let out = rewrite(input);
            assert_eq!(out.text, input);
            assert_eq!(written(&out), [(input, None)]);
        }
        assert!(rewrite("{% unclosed").kept.is_empty());
    }

    #[test]
    fn tags_become_variables_named_after_the_tag() {
        let out = rewrite("{{ _.a }}/{% now 'millis', '' %}/{{ x|lower }}/{%prompt%}");
        assert_eq!(out.text, "{{a}}/{{now}}/{{ x|lower }}/{{prompt}}");
        assert_eq!(
            written(&out),
            [
                ("{% now 'millis', '' %}", Some("now")),
                ("{{ x|lower }}", None),
                ("{%prompt%}", Some("prompt")),
            ]
        );
        // A tag without a name a variable could carry stays as written.
        for input in ["{% %}", "{% 1x %}", "{% $guid %}", "{%%}"] {
            let out = rewrite(input);
            assert_eq!(out.text, input);
            assert_eq!(written(&out), [(input, None)]);
        }
    }

    #[test]
    fn many_unclosed_openers_are_plain_text() {
        for input in ["{{".repeat(500_000), "{{ {%".repeat(200_000)] {
            let out = rewrite(&input);
            assert_eq!(out.text, input);
            assert!(out.kept.is_empty());
        }
    }

    #[test]
    fn the_whole_context_is_kept_and_listed() {
        let out = rewrite("{{ _ }}");
        assert_eq!(out.text, "{{ _ }}");
        assert_eq!(written(&out), [("{{ _ }}", None)]);
        assert_eq!(rewrite("{{ _._ }}").text, "{{_}}");
    }

    /// Each input rewritten to `want` with nothing kept.
    fn converts(cases: &[(&str, &str)]) {
        for (input, want) in cases {
            let out = rewrite(input);
            assert_eq!(
                (out.text.as_str(), written(&out)),
                (*want, vec![]),
                "{input}"
            );
        }
    }

    /// Each input kept as a variable named after its tag, with a reason.
    fn keeps(cases: &[(&str, &str)]) {
        for (input, name) in cases {
            let out = rewrite(input);
            assert_eq!(out.text, format!("{{{{{name}}}}}"), "{input}");
            assert_eq!(written(&out), [(*input, Some(*name))], "{input}");
            assert!(out.kept[0].reason.is_some(), "{input}");
        }
    }

    #[test]
    fn faker_uuid_and_now_become_dynamic_values() {
        converts(&[
            ("{% faker 'randomFirstName' %}", "{{$randomFirstName}}"),
            ("{% faker \"randomEmail\" %}", "{{$randomEmail}}"),
            ("{% faker 'guid' %}", "{{$guid}}"),
            ("{% uuid 'v4' %}", "{{$randomUUID}}"),
            ("{%uuid%}", "{{$randomUUID}}"),
            ("{% uuid 'v1' %}", "{{$randomUUID}}"),
            ("{% now %}", "{{$isoTimestamp}}"),
            ("{% now 'iso-8601', '' %}", "{{$isoTimestamp}}"),
            ("{% now 'ISO-8601' %}", "{{$isoTimestamp}}"),
            ("{% now 'unix' %}", "{{$timestamp}}"),
            ("{% now 's' %}", "{{$timestamp}}"),
            ("{% now 'seconds', '' %}", "{{$timestamp}}"),
        ]);
        keeps(&[
            ("{% faker 'randomAvatarImage' %}", "faker"),
            ("{% faker _.name %}", "faker"),
            ("{% faker %}", "faker"),
            ("{% now 'millis', '' %}", "now"),
            ("{% now 'custom', 'YYYY' %}", "now"),
        ]);
    }

    #[test]
    fn literal_base64_and_hash_are_computed() {
        converts(&[
            (
                "{% base64 'encode', 'normal', 'hi there' %}",
                "aGkgdGhlcmU=",
            ),
            ("{% base64 'encode', 'normal' %}", ""),
            ("{% base64 'encode', 'url', '??>' %}", "Pz8-"),
            ("{% base64 'encode', 'hex', '4869' %}", "SGk="),
            ("{% base64 'decode', 'normal', 'aGk=' %}", "hi"),
            ("{% base64 'decode', 'hex', 'aGk=' %}", "6869"),
            ("{% base64 'encode', 'normal', 'b64::aGk=::46b' %}", "aGk="),
            ("{% base64 'encode', 'normal', 'it\\'s' %}", "aXQncw=="),
            (
                "a{% hash 'md5', 'hex', 'abc' %}b",
                "a900150983cd24fb0d6963f7d28e17f72b",
            ),
            (
                "{% hash 'sha256', 'hex', 'abc' %}",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                "{% hash 'SHA1', 'base64', 'abc' %}",
                "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=",
            ),
        ]);
        keeps(&[
            ("{% base64 'encode', 'normal', _.secret %}", "base64"),
            ("{% base64 'encode', 'hex', 'xyz' %}", "base64"),
            ("{% base64 'decode', 'normal', '!!' %}", "base64"),
            ("{% base64 'decode', 'normal', '/w==' %}", "base64"),
            // A computed value that reads as a template would be interpolated again.
            ("{% base64 'decode', 'normal', 'e3t4fX0=' %}", "base64"),
            ("{% base64 'decode', 'normal', 'e3skZ3VpZH19' %}", "base64"),
            ("{% hash 'md5', 'hex', 'b64::e3t4fX0=::46b' %}", "hash"),
            ("{% base64 'rot13', 'normal', 'a' %}", "base64"),
            ("{% hash 'sha3', 'hex', 'a' %}", "hash"),
            ("{% hash 'md5', 'binary', 'a' %}", "hash"),
            ("{% hash 'md5', 'hex', _.password %}", "hash"),
        ]);
    }

    fn login() -> Chains {
        let sources = [("req_1".to_owned(), "Log in".to_owned())].into();
        Chains::new(sources, HashSet::from(["res_log_in_id".to_owned()]))
    }

    fn links(chains: &Chains) -> Vec<(&str, &str, &str)> {
        chains
            .links()
            .iter()
            .map(|l| (l.source.as_str(), l.name.as_str(), l.expression.as_str()))
            .collect()
    }

    #[test]
    fn a_response_tag_reads_a_variable_its_source_sets() {
        let mut chains = login();
        let token = "{% response 'body', 'req_1', 'b64::JC50b2tlbg==::46b', 'never', 60 %}";
        let input = format!(
            "{token}/{token}/{{% response 'header', 'req_1', 'X-Token' %}}/\
             {{% response 'body', 'req_1', '$.items[0][\"id\"]', 'always' %}}/\
             {{% response 'body', 'req_1', '$.id' %}}"
        );
        let out = super::rewrite(&input, &mut chains);
        assert_eq!(
            out.text,
            "{{res_log_in_token}}/{{res_log_in_token}}/{{res_log_in_x_token}}/\
             {{res_log_in_items_0_id}}/{{res_log_in_id_2}}"
        );
        assert!(out.kept.is_empty());
        assert_eq!(
            links(&chains),
            [
                ("req_1", "res_log_in_token", "res.body.token"),
                ("req_1", "res_log_in_x_token", "res.headers['x-token']"),
                ("req_1", "res_log_in_items_0_id", "res.body.items[0]['id']"),
                ("req_1", "res_log_in_id_2", "res.body.id"),
            ]
        );
        let resent: Vec<bool> = out.chained.iter().map(|c| c.resent).collect();
        assert_eq!(resent, [false, false, false, true, false]);
        assert!(out.chained.iter().all(|c| c.source == "Log in"));
    }

    #[test]
    fn a_response_tag_without_a_simple_equivalent_is_kept() {
        let mut chains = login();
        for input in [
            "{% response 'body', 'req_9', '$.token' %}",
            "{% response 'body', _.id, '$.token' %}",
            "{% response 'raw', 'req_1', '' %}",
            "{% response 'url', 'req_1', '' %}",
            "{% response 'body', 'req_1', '' %}",
            "{% response 'body', 'req_1', '$..token' %}",
            "{% response 'body', 'req_1', '$.items[*]' %}",
            "{% response 'body', 'req_1', '$.a[?(@.b)]' %}",
            "{% response 'body', 'req_1', '//token' %}",
            "{% response 'header', 'req_1', 'a\\'b' %}",
            "{% response 'header', 'req_1', '' %}",
        ] {
            let out = super::rewrite(input, &mut chains);
            assert_eq!(out.text, "{{response}}", "{input}");
            assert!(out.kept[0].reason.is_some(), "{input}");
        }
        assert!(chains.links().is_empty());
    }

    #[test]
    fn other_tags_keep_a_variable_and_say_why() {
        keeps(&[
            ("{% prompt 'p', '', '' %}", "prompt"),
            ("{% cookie 'http://x.test', 'sid' %}", "cookie"),
            ("{% file '/tmp/a' %}", "file"),
            ("{% request 'name', '', '' %}", "request"),
            ("{% os 'arch' %}", "os"),
            ("{% jsonpath '{}', '$.a' %}", "jsonpath"),
            ("{% vault 'aws', '', '{}' %}", "vault"),
        ]);
    }
}
