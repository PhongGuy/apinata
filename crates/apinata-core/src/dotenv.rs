//! Port of Node dotenv 16.6.1 `parse`, the .env reader Bruno uses.

use std::sync::LazyLock;

use regress::Regex;

use crate::js::js_trim;

/// dotenv 16.6.1 `LINE` (lib/main.js:9), run with the same `mg` flags.
const LINE: &str = r#"(?:^|^)\s*(?:export\s+)?([\w.-]+)(?:\s*=\s*?|:\s+?)(\s*'(?:\\'|[^'])*'|\s*"(?:\\"|[^"])*"|\s*`(?:\\`|[^`])*`|[^#\r\n]+)?\s*(?:#.*)?(?:$|$)"#;
// A fixed pattern, so a failure to compile is a bug every run hits, not an input error.
#[allow(clippy::expect_used)]
static LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::with_flags(LINE, "mg").expect("dotenv LINE compiles"));

/// Parses `.env` text into pairs in file order; a later duplicate wins when inserted into a map.
pub(crate) fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut pairs = Vec::new();
    for m in LINE_RE.find_iter(&text) {
        let Some(key) = m.group(1) else { continue };
        let mut value = js_trim(m.group(2).map_or("", |r| &text[r])).to_owned();
        let quote = value.chars().next();
        // dotenv's `^(['"`])([\s\S]*)\1$` strip: a matching pair of quotes goes, an unterminated one stays.
        if let Some(q) = quote.filter(|q| "'\"`".contains(*q))
            && value.len() >= 2
            && value.ends_with(q)
        {
            value = value[1..value.len() - 1].to_owned();
        }
        if quote == Some('"') {
            value = value.replace("\\n", "\n").replace("\\r", "\r");
        }
        pairs.push((text[key].to_owned(), value));
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::parse_dotenv;

    #[test]
    fn parses_like_node_dotenv_16_6_1() {
        let text = "A=1\nexport B = two # c\nC='single $X'\nD=\"dq\\nline\"\nE=`bt`\nF=pa$$word\n# comment\nG=\nH: colon\nI=\"multi\nline\"\r\nJ=  spaced  \nK.dot-x=v\n";
        let expected = [
            ("A", "1"),
            ("B", "two"),
            ("C", "single $X"),
            ("D", "dq\nline"),
            ("E", "bt"),
            ("F", "pa$$word"),
            ("G", ""),
            ("H", "colon"),
            ("I", "multi\nline"),
            ("J", "spaced"),
            ("K.dot-x", "v"),
        ];
        let parsed = parse_dotenv(text);
        let parsed: Vec<(&str, &str)> = parsed
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn crlf_lines_and_duplicates_keep_file_order() {
        let pairs = |items: &[(&str, &str)]| -> Vec<(String, String)> {
            items
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        assert_eq!(
            parse_dotenv("X=1\r\nX=2\r\n"),
            pairs(&[("X", "1"), ("X", "2")])
        );
    }

    #[test]
    fn values_trim_like_javascript() {
        let parsed = parse_dotenv("B=x\u{feff}\nC=y\u{85}\n");
        let parsed: Vec<(&str, &str)> = parsed
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(parsed, [("B", "x"), ("C", "y\u{85}")]);
    }
}
