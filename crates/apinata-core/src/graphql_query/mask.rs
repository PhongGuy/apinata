//! Placeholder masking and the classification of where each placeholder sits.

use std::ops::Range;

use apollo_parser::cst::CstNode;
use apollo_parser::{Parser, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxTree, TextRange};

/// Query text with every `{{placeholder}}` overwritten by underscores of the same length.
pub(crate) struct Masked {
    pub(crate) text: String,
    pub(crate) spans: Vec<Range<usize>>,
}

/// Masks each placeholder the resolver would substitute. Whole characters are replaced
/// with ASCII bytes, so offsets match the original and the text stays valid UTF-8.
pub(crate) fn mask(text: &str) -> Masked {
    let spans: Vec<Range<usize>> = crate::interpolate::placeholders(text)
        .into_iter()
        .map(|(span, _)| span)
        .collect();
    let mut bytes = text.as_bytes().to_vec();
    for span in &spans {
        bytes[span.clone()].fill(b'_');
    }
    let text = String::from_utf8(bytes).unwrap_or_else(|_| "_".repeat(text.len()));
    Masked { text, spans }
}

pub(crate) fn parse(masked: &str) -> SyntaxTree {
    Parser::new(masked).parse()
}

fn root(tree: &SyntaxTree) -> SyntaxNode {
    tree.document().syntax().clone()
}

/// True when the document holds nothing but whitespace, commas and comments.
pub(crate) fn is_blank(tree: &SyntaxTree) -> bool {
    root(tree)
        .descendants_with_tokens()
        .filter_map(SyntaxElement::into_token)
        .all(|tok| {
            matches!(
                tok.kind(),
                SyntaxKind::WHITESPACE | SyntaxKind::COMMA | SyntaxKind::COMMENT | SyntaxKind::EOF
            )
        })
}

/// True when some mask stands for query structure rather than a value. A mask is a value
/// when every token it overlaps is a string, a comment or the name of an enum value.
pub(crate) fn any_structural(tree: &SyntaxTree, spans: &[Range<usize>]) -> bool {
    let root = root(tree);
    let len = usize::from(root.text_range().end());
    spans.iter().any(|span| {
        let (s, e) = (span.start.min(len), span.end.min(len));
        let (Ok(s32), Ok(e32)) = (u32::try_from(s), u32::try_from(e)) else {
            return true;
        };
        let overlaps =
            |range: TextRange| usize::from(range.start()) < e && s < usize::from(range.end());
        // Walk only the smallest element covering the mask, never the whole tree.
        let tokens: Vec<_> = match root.covering_element(TextRange::new(s32.into(), e32.into())) {
            SyntaxElement::Token(tok) => vec![tok],
            SyntaxElement::Node(node) => node
                .descendants_with_tokens()
                .filter_map(SyntaxElement::into_token)
                .filter(|tok| overlaps(tok.text_range()))
                .collect(),
        };
        tokens.iter().any(|tok| match tok.kind() {
            SyntaxKind::STRING | SyntaxKind::COMMENT => false,
            SyntaxKind::IDENT => tok
                .parent()
                .and_then(|name| name.parent())
                .is_none_or(|grand| grand.kind() != SyntaxKind::ENUM_VALUE),
            _ => true,
        })
    })
}

/// Widens a byte range to char boundaries: the start floors, the end ceils, both clamped.
pub(crate) fn snap(text: &str, range: Range<usize>) -> Range<usize> {
    let mut start = range.start.min(text.len());
    let mut end = range.end.min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    while !text.is_char_boundary(end) {
        end += 1;
    }
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn structural(query: &str) -> bool {
        let m = mask(query);
        any_structural(&parse(&m.text), &m.spans)
    }

    #[test]
    fn masking_keeps_length_and_char_boundaries() {
        let text = "{ user(id: {{ø_id}}) { name } }";
        let m = mask(text);
        assert_eq!(m.text.len(), text.len());
        assert_eq!(m.spans, vec![11..20]);
        assert_eq!(&m.text[11..20], "_________");
        assert_eq!(&m.text[..11], &text[..11]);
        assert_eq!(&m.text[20..], &text[20..]);
        let adjacent = mask("{ a(x: {{a}}{{b}}) }");
        assert_eq!(adjacent.spans, vec![7..12, 12..17]);
        assert_eq!(adjacent.text, "{ a(x: __________) }");
    }

    #[test]
    fn value_positions_are_not_structural() {
        for q in [
            "{ user(id: {{id}}) { name } }",
            "{ user(id: \"{{id}}\") { name } }",
            "{ users(filter: {name: {{n}}}) { name } }",
            "{ users(filter: {{f}}) { name } }",
            "{ users(role: {{r}}) { name } }",
            "query Q($n: Int = {{n}}) { users(first: $n) { name } }",
            "{ users(first: [{{n}}]) { name } }",
            "{ user(id: {{a}}{{b}}) { name } }",
            "{ user(id: 1) @include(if: {{f}}) { name } }",
            "{ user(id: 1) { name } } # {{c}}",
            "{ user(id: 1) { name } }",
        ] {
            assert!(!structural(q), "{q}");
        }
    }

    #[test]
    fn structural_positions_are_found() {
        for q in [
            "{ user(id: 1) { {{fields}} } }",
            "{ user(id: 1) { name{{x}} } }",
            "{ user(id: 1) { ...{{frag}} } }",
            "{ user(id: 1) { ... on {{T}} { name } } }",
            "fragment {{F}} on User { name }",
            "query Q($v: {{T}}) { a }",
            "query Q($id: ID!) { user(id: ${{x}}) { name } }",
            "query Q(${{v}}: ID!) { a }",
            "{ user({{arg}}: 1) { name } }",
            "{ users(filter: { {{k}}: 1 }) { name } }",
            "{ user(id: 1) @{{dir}} { name } }",
        ] {
            assert!(structural(q), "{q}");
        }
    }

    #[test]
    fn blank_means_whitespace_commas_and_comments_only() {
        for text in ["", "  \n", ", ,\n", "# only\n", "  # a\n\n# b"] {
            assert!(is_blank(&parse(&mask(text).text)), "{text:?}");
        }
        for text in ["{ a }", "# c\n{ a }", "{{x}}"] {
            assert!(!is_blank(&parse(&mask(text).text)), "{text:?}");
        }
    }

    #[test]
    fn snapping_widens_to_char_boundaries() {
        assert_eq!(snap("aø", 2..2), 1..3);
        assert_eq!(snap("aø", 0..9), 0..3);
        assert_eq!(snap("abc", 1..2), 1..2);
    }
}
