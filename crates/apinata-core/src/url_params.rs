//! Ports of Bruno's URL and query param helpers at 1301ea17: bruno-app `utils/url`
//! (`splitOnFirst`, `parsePathParams`) and bruno-common `utils/url` (`parseQueryParams`,
//! `buildQueryString`), without their decode and encode options.

use crate::js::js_trim;

/// Splits `s` at the first `ch` outside a `{{...}}` template; `None` when there is none.
pub(crate) fn split_on_first(s: &str, ch: char) -> (&str, Option<&str>) {
    // Bruno masks each `/\{\{.*?\}\}/g` match, so a template that closes before a line break
    // hides its contents; an unclosed `{{` hides nothing.
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if rest.starts_with("{{")
            && let Some(len) = template_len(rest)
        {
            i += len;
            continue;
        }
        let c = rest.chars().next().unwrap_or_default();
        if c == ch {
            return (&s[..i], Some(&s[i + c.len_utf8()..]));
        }
        i += c.len_utf8();
    }
    (s, None)
}

/// Byte length of the shortest `{{...}}` at the start of `text`, if it closes on the same line.
fn template_len(text: &str) -> Option<usize> {
    let body = &text[2..];
    let close = body.find("}}")?;
    let on_one_line = !body[..close].contains(['\n', '\r', '\u{2028}', '\u{2029}']);
    on_one_line.then_some(close + 4)
}

/// Splits a raw query into `(name, value)` pairs, dropping any fragment and empty names.
/// A value is `None` when the pair has no `=`; nothing is decoded.
pub(crate) fn parse_query_params(query: &str) -> Vec<(String, Option<String>)> {
    let query = query.split('#').next().unwrap_or_default();
    query
        .split('&')
        .filter_map(|pair| {
            let (name, value) = match pair.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (pair, None),
            };
            (!name.is_empty()).then(|| (name.to_owned(), value))
        })
        .collect()
}

/// Joins pairs as `name=value` with `&`, skipping names that are blank once trimmed.
pub(crate) fn build_query_string<'a>(params: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    params
        .filter(|(name, _)| !js_trim(name).is_empty())
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// Which side of a `name=value` pair a piece of query text is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryPart {
    Name,
    Value,
}

/// The delimiters that would split `part` inside a query, with their escapes.
fn delimiters(part: QueryPart) -> &'static [(char, &'static str)] {
    match part {
        QueryPart::Name => &[('&', "%26"), ('#', "%23"), ('=', "%3D")],
        QueryPart::Value => &[('&', "%26"), ('#', "%23")],
    }
}

/// The escape for a delimiter of `part` that `text` starts with, as the editor writes it.
pub(crate) fn delimiter_escape_at(text: &str, part: QueryPart) -> Option<&'static str> {
    delimiters(part)
        .iter()
        .map(|(_, escape)| *escape)
        .find(|escape| text.starts_with(escape))
}

/// Byte offsets in the URL template `url` of the query's delimiter escapes that the table's
/// `cells` (enabled query `(name, value)` text, in table order) account for.
/// Each pair claims the first unused cell that `escape_query_part` writes as exactly its text,
/// and only the escapes standing for a delimiter in that cell count; typed escape text does not.
/// A `#` ends the matching: the pair holding it and every later one are never claimed.
pub(crate) fn table_escape_spots(url: &str, cells: &[(&str, &str)]) -> Vec<usize> {
    let (head, Some(query)) = split_on_first(url, '?') else {
        return Vec::new();
    };
    let mut used = vec![false; cells.len()];
    let mut spots = Vec::new();
    let mut start = head.len() + 1;
    let mut rest = Some(query);
    while let Some(text) = rest {
        let (pair, next) = split_on_first(text, '&');
        rest = next;
        let pair_start = start;
        start += pair.len() + 1;
        if split_on_first(pair, '#').1.is_some() {
            break;
        }
        if pair.contains('#') {
            continue;
        }
        let (name, value) = split_on_first(pair, '=');
        let value = value.unwrap_or_default();
        let claimed = (0..cells.len()).find(|&i| {
            !used[i]
                && escape_query_part(cells[i].0, QueryPart::Name) == name
                && escape_query_part(cells[i].1, QueryPart::Value) == value
        });
        let Some(i) = claimed else {
            continue;
        };
        used[i] = true;
        mark_delimiters(cells[i].0, QueryPart::Name, pair_start, &mut spots);
        let value_start = pair_start + name.len() + 1;
        mark_delimiters(cells[i].1, QueryPart::Value, value_start, &mut spots);
    }
    spots
}

/// Walks `cell` beside the text `escape_query_part` wrote for it from byte `at`, pushing where
/// each delimiter's escape landed; templates are skipped whole, as the writer copies them.
fn mark_delimiters(cell: &str, part: QueryPart, mut at: usize, spots: &mut Vec<usize>) {
    let mut i = 0;
    while i < cell.len() {
        let rest = &cell[i..];
        if rest.starts_with("{{")
            && let Some(len) = template_len(rest)
        {
            i += len;
            at += len;
            continue;
        }
        let c = rest.chars().next().unwrap_or_default();
        match delimiters(part).iter().find(|(d, _)| *d == c) {
            Some((_, escape)) => {
                spots.push(at);
                at += escape.len();
            }
            None => at += c.len_utf8(),
        }
        i += c.len_utf8();
    }
}

/// `&` -> `%26`, `#` -> `%23`, and in a Name `=` -> `%3D`, outside `{{...}}` templates.
pub(crate) fn escape_query_part(text: &str, part: QueryPart) -> String {
    map_outside_templates(text, |rest| {
        let c = rest.chars().next()?;
        let (_, escape) = delimiters(part).iter().find(|(d, _)| *d == c)?;
        Some((*escape, c.len_utf8()))
    })
}

/// Copies `text`, letting `swap` replace the start of any rest outside a `{{...}}` template
/// with new text and the byte length it consumed; templates are copied whole.
fn map_outside_templates<S: AsRef<str>>(
    text: &str,
    swap: impl Fn(&str) -> Option<(S, usize)>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let take = if rest.starts_with("{{")
            && let Some(len) = template_len(rest)
        {
            len
        } else if let Some((with, len)) = swap(rest) {
            out.push_str(with.as_ref());
            i += len;
            continue;
        } else {
            rest.chars().next().map_or(1, char::len_utf8)
        };
        out.push_str(&rest[..take]);
        i += take;
    }
    out
}

/// Names of the path params in `url`, in first-appearance order: `:name` segments and the
/// `:name` keys of OData segments such as `Users(id=:id)`.
pub(crate) fn parse_path_params(url: &str) -> Vec<String> {
    if url.is_empty() {
        return Vec::new();
    }
    let uri = if url.starts_with("http://") || url.starts_with("https://") {
        url.to_owned()
    } else {
        format!("http://{url}")
    };
    // Node's `URL.pathname` percent-encodes `^` where the `url` crate keeps it; that is the
    // only printable ASCII difference.
    let path = match url::Url::parse(&uri) {
        Ok(parsed) => parsed.path().replace('^', "%5E"),
        Err(_) => uri,
    };
    let mut found: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        if !name.is_empty() && !found.iter().any(|seen| seen == name) {
            found.push(name.to_owned());
        }
    };
    for segment in path.split('/') {
        if let Some(name) = segment.strip_prefix(':') {
            add(name);
        } else if is_odata_segment(segment) {
            // A capture is `[a-zA-Z_]\w*`, so Bruno's quote and paren stripping never applies.
            for name in odata_keys(segment) {
                add(name);
            }
        }
    }
    found
}

/// `^[A-Za-z0-9_.-]+\([^)]*\)$`.
fn is_odata_segment(segment: &str) -> bool {
    let Some(open) = segment.find('(') else {
        return false;
    };
    let (entity, args) = segment.split_at(open);
    let entity_ok = !entity.is_empty()
        && entity
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    let inner = args[1..].strip_suffix(')');
    entity_ok && inner.is_some_and(|inner| !inner.contains(')'))
}

/// Every `:([a-zA-Z_]\w*)` capture in `segment`, scanned left to right without overlap.
fn odata_keys(segment: &str) -> Vec<&str> {
    let bytes = segment.as_bytes();
    let mut keys = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let starts_key = bytes[i] == b':'
            && bytes
                .get(i + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_');
        if !starts_key {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start + 1;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        keys.push(&segment[start..end]);
        i = end;
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_skips_the_separator_inside_templates() {
        assert_eq!(split_on_first("a?b?c", '?'), ("a", Some("b?c")));
        assert_eq!(
            split_on_first("{{a?b}}/x?q=1", '?'),
            ("{{a?b}}/x", Some("q=1"))
        );
        assert_eq!(split_on_first("no-query", '?'), ("no-query", None));
        assert_eq!(split_on_first("{{open?x", '?'), ("{{open", Some("x")));
        assert_eq!(split_on_first("", '?'), ("", None));
    }

    #[test]
    fn query_params_keep_raw_text_and_tell_missing_from_empty_values() {
        assert_eq!(
            parse_query_params("a=1&b&c=&=x&d=e=f#frag&g=2"),
            vec![
                ("a".to_owned(), Some("1".to_owned())),
                ("b".to_owned(), None),
                ("c".to_owned(), Some(String::new())),
                ("d".to_owned(), Some("e=f".to_owned())),
            ]
        );
        assert_eq!(parse_query_params(""), vec![]);
        assert_eq!(
            parse_query_params("q=%20{{x}}"),
            vec![("q".to_owned(), Some("%20{{x}}".to_owned()))]
        );
    }

    #[test]
    fn query_strings_skip_blank_names() {
        let built = build_query_string([("a", "1"), (" ", "x"), ("b", "")].into_iter());
        assert_eq!(built, "a=1&b=");
    }

    #[test]
    fn path_params_come_from_segments_and_odata_keys() {
        assert_eq!(
            parse_path_params("https://x.test/users/:id/posts/:postId"),
            ["id", "postId"]
        );
        assert_eq!(parse_path_params("{{host}}/users/:id/:id"), ["id"]);
        assert_eq!(
            parse_path_params("x.test/Users(id=:userId,name=':who')"),
            ["userId", "who"]
        );
        assert_eq!(
            parse_path_params("https://x.test/a:b/:"),
            Vec::<String>::new()
        );
        assert_eq!(parse_path_params(""), Vec::<String>::new());
    }

    #[test]
    fn path_params_are_read_from_the_encoded_pathname() {
        // WHATWG percent-encodes the space, as Node's `URL` does.
        assert_eq!(parse_path_params("https://x.test/:na me"), ["na%20me"]);
    }

    #[test]
    fn a_caret_is_percent_encoded_as_node_does() {
        assert_eq!(parse_path_params("x.test/:c^d"), ["c%5Ed"]);
        assert_eq!(parse_path_params("x.test/Users(id=:a^b)"), ["a"]);
    }

    #[test]
    fn an_unparseable_url_falls_back_to_splitting_the_text() {
        // `http://:x/y` has an empty host, so Bruno splits the string itself.
        assert_eq!(parse_path_params("http://:x/:y"), ["x", "y"]);
    }
}
