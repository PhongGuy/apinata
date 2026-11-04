//! Insomnia's Nunjucks templates in values.

use super::{Budget, Crumbs};
use crate::import::{DocumentError, ImportNote, NoteKind};

/// A value with its member-path placeholders rewritten.
pub(super) struct Rewritten {
    pub(super) text: String,
    /// Every other `{{ }}` and `{% %}` construct, as written and in order.
    pub(super) kept: Vec<String>,
}

/// Rewrite member-path placeholders to `{{path}}`; every other `{{ }}` and `{% %}` stays as
/// written and is listed in `kept`, in order. An unclosed `{{` or `{%` is plain text.
/// The output is never longer than the input.
pub(super) fn rewrite(text: &str) -> Rewritten {
    let mut out = String::with_capacity(text.len());
    let mut kept = Vec::new();
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
            out.push_str(whole);
            kept.push(whole.to_owned());
        }
        rest = &rest[end + 4..];
    }
    out.push_str(rest);
    Rewritten { text: out, kept }
}

/// `rewrite(text).text`, pushing one `NeedsInput` note sourced `trail / at` naming every kept
/// construct when there is any.
pub(super) fn apply(
    text: &str,
    trail: &[String],
    at: &str,
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<String, DocumentError> {
    let Rewritten { text, kept } = rewrite(text);
    if !kept.is_empty() {
        let listed: Vec<String> = kept.iter().map(|k| format!("`{k}`")).collect();
        let detail = format!(
            "{} has no equivalent and is kept as written; replace it before sending",
            listed.join(", ")
        );
        notes.push(budget.note(trail, at, NoteKind::NeedsInput, detail)?);
    }
    Ok(text)
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
    use super::rewrite;

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

    #[test]
    fn filters_expressions_and_tags_stay_literal_and_are_listed() {
        for input in [
            "{{ x | upper }}",
            "{{ _['a-b'] }}",
            "{{ a + 1 }}",
            "{% uuid 'v4' %}",
            "{{ _.a.b. }}",
            "{{ 1a }}",
            "{{ }}",
        ] {
            let out = rewrite(input);
            assert_eq!(out.text, input);
            assert_eq!(out.kept, [input]);
        }
        let out = rewrite("{{ _.a }}/{% now 'millis', '' %}/{{ x|lower }}");
        assert_eq!(out.text, "{{a}}/{% now 'millis', '' %}/{{ x|lower }}");
        assert_eq!(out.kept, ["{% now 'millis', '' %}", "{{ x|lower }}"]);
        assert!(rewrite("{% unclosed").kept.is_empty());
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
        assert_eq!(out.kept, ["{{ _ }}"]);
        assert_eq!(rewrite("{{ _._ }}").text, "{{_}}");
    }
}
