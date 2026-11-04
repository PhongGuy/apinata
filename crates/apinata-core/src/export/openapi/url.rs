//! Splitting a request URL into a server origin and an OpenAPI path template.

pub(super) const NO_ORIGIN: &str = "the URL has no origin OpenAPI can express";
pub(super) const BAD_TEMPLATE: &str = "the URL holds a template name OpenAPI cannot express";

/// The origin as written (`{{x}}` kept); the path with `:name` segments and `{{name}}` as `{name}`.
#[derive(Debug, PartialEq)]
pub(super) struct Split {
    pub(super) origin: String,
    pub(super) path: String,
}

/// Splits `raw` after cutting its query and fragment; the error is the note's reason.
pub(super) fn split_url(raw: &str) -> Result<Split, &'static str> {
    let raw = raw.split(['?', '#']).next().unwrap_or_default();
    let (origin, rest) = origin(raw).ok_or(NO_ORIGIN)?;
    let path = if rest.is_empty() { "/" } else { rest };
    // Colon segments become templates first, so `{{x}}` inside one is still caught below.
    let path = path
        .split('/')
        .map(|seg| match seg.strip_prefix(':') {
            Some(name) => format!("{{{name}}}"),
            None => seg.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("/");
    let path = single_braces(&path);
    if !well_formed(&single_braces(origin)) || !well_formed(&path) {
        return Err(BAD_TEMPLATE);
    }
    Ok(Split {
        origin: origin.to_owned(),
        path,
    })
}

/// `url` without the `user:password@` part of its authority, or `None` when it has none.
pub(super) fn strip_userinfo(url: &str) -> Option<String> {
    let (scheme, after) = url.split_once("://")?;
    let end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let at = after[..end].rfind('@')?;
    Some(format!("{scheme}://{}", &after[at + 1..]))
}

/// The origin and the rest: a leading `{{x}}` ending at `/` or the end, else a literal
/// `scheme://authority` with a non-empty authority.
fn origin(raw: &str) -> Option<(&str, &str)> {
    if let Some(close) = raw.strip_prefix("{{").and_then(|r| r.find("}}")) {
        let end = close + 4;
        if raw[end..].is_empty() || raw[end..].starts_with('/') {
            return Some(raw.split_at(end));
        }
    }
    let (scheme, after) = raw.split_once("://")?;
    let mut chars = scheme.chars();
    let literal = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    let authority = after.find('/').unwrap_or(after.len());
    (literal && authority > 0).then(|| raw.split_at(scheme.len() + 3 + authority))
}

/// Every closed `{{name}}` as `{name}`; an unclosed `{{` is copied through.
fn single_braces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open + 2..].find("}}") else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push('{');
        out.push_str(&rest[open + 2..open + 2 + close]);
        out.push('}');
        rest = &rest[open + 4 + close..];
    }
    out.push_str(rest);
    out
}

/// Whether every `{` opens a `{name}` with a non-empty name free of `{` and `/`, and no `}`
/// stands alone.
fn well_formed(s: &str) -> bool {
    let mut rest = s;
    loop {
        match rest.find(['{', '}']) {
            None => return true,
            Some(i) if rest.as_bytes()[i] == b'}' => return false,
            Some(i) => {
                let Some(len) = rest[i + 1..].find('}') else {
                    return false;
                };
                let name = &rest[i + 1..i + 1 + len];
                if name.is_empty() || name.contains(['{', '/']) {
                    return false;
                }
                rest = &rest[i + 2 + len..];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_origin_and_path_or_names_the_reason() {
        let ok = |o: &str, p: &str| -> Result<Split, &'static str> {
            Ok(Split {
                origin: o.into(),
                path: p.into(),
            })
        };
        let cases = [
            ("{{baseUrl}}/users/:id", ok("{{baseUrl}}", "/users/{id}")),
            (
                "https://x.test/v{{version}}/items/{{itemId}}?q=1#top",
                ok("https://x.test", "/v{version}/items/{itemId}"),
            ),
            (
                "http://localhost:{{port}}",
                ok("http://localhost:{{port}}", "/"),
            ),
            ("{{baseUrl}}?page=2", ok("{{baseUrl}}", "/")),
            (
                "https://x.test/a/:b/c.json",
                ok("https://x.test", "/a/{b}/c.json"),
            ),
            ("{{a}}{{b}}/x", Err(NO_ORIGIN)),
            ("/relative/path", Err(NO_ORIGIN)),
            ("{{host}}:8080/x", Err(NO_ORIGIN)),
            ("{{proto}}://x.test/a", Err(NO_ORIGIN)),
            ("https:///x", Err(NO_ORIGIN)),
            ("https://x.test/a/:/b", Err(BAD_TEMPLATE)),
            ("https://x.test/a{b", Err(BAD_TEMPLATE)),
            ("{{base}}/x/{{a/b}}", Err(BAD_TEMPLATE)),
            ("https://x.test/{{open", Err(BAD_TEMPLATE)),
            ("{{}}/x", Err(BAD_TEMPLATE)),
        ];
        for (raw, want) in cases {
            assert_eq!(split_url(raw), want, "{raw}");
        }
    }
}
