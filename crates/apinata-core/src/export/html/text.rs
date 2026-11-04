//! Text safety for the HTML page: the one escaper and the markdown renderer for docs.

use crate::js::js_trim;
use markdown::{CompileOptions, Options, ParseOptions};

/// `text` with `& < > " '` as `&amp; &lt; &gt; &quot; &#39;`, safe in text and quoted attributes.
pub(super) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// `text` rendered as GFM inside `<div class="docs">\n…\n</div>\n`, footnote ids prefixed
/// `d-<anchor>-`, and the hidden footnote heading an `h<label_level>` with the same prefix;
/// empty when `text` is blank.
pub(super) fn docs(text: &str, anchor: &str, label_level: u8) -> String {
    if js_trim(text).is_empty() {
        return String::new();
    }
    // The safe defaults escape raw HTML and empty unsafe link targets; the per-owner footnote
    // prefix keeps footnote ids unique across the page.
    let options = Options {
        parse: ParseOptions::gfm(),
        compile: CompileOptions {
            gfm_footnote_clobber_prefix: Some(format!("d-{anchor}-")),
            gfm_footnote_label_tag_name: Some(format!("h{label_level}")),
            ..CompileOptions::gfm()
        },
    };
    // Only MDX parsing can fail, which `gfm()` never enables; fall back to preformatted text.
    let body = match markdown::to_html_with_options(text, &options) {
        // The renderer always writes the fixed label id; prefix it so it is unique per owner.
        Ok(html) => html.trim_end().replace(
            "\"footnote-label\"",
            &format!("\"d-{anchor}-footnote-label\""),
        ),
        Err(_) => format!("<pre>{}</pre>", escape(text)),
    };
    format!("<div class=\"docs\">\n{body}\n</div>\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_covers_text_and_attribute_context() {
        assert_eq!(escape("a&b<c>d\"e'f"), "a&amp;b&lt;c&gt;d&quot;e&#39;f");
        assert_eq!(escape("plain {{x}}"), "plain {{x}}");
    }

    #[test]
    fn docs_render_commonmark_and_gfm() {
        let text =
            "# T\n\n- a\n- b\n\n```\nx < y\n```\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n~~gone~~";
        let out = docs(text, "c", 2);
        assert!(
            out.starts_with("<div class=\"docs\">\n<h1>T</h1>\n"),
            "{out}"
        );
        assert!(out.ends_with("\n</div>\n"), "{out}");
        for part in [
            "<li>a</li>",
            "<pre><code>x &lt; y\n</code></pre>",
            "<th>a</th>",
            "<td>2</td>",
            "<del>gone</del>",
        ] {
            assert!(out.contains(part), "{part} in {out}");
        }
    }

    #[test]
    fn raw_html_is_escaped_and_unsafe_links_lose_their_target() {
        let out = docs(
            "<b>x</b>\n\n[a](javascript:alert(1)) [b](https://x.test/)",
            "c",
            2,
        );
        assert!(out.contains("&lt;b&gt;x&lt;/b&gt;"), "{out}");
        assert!(out.contains("<a href=\"\">a</a>"), "{out}");
        assert!(out.contains("<a href=\"https://x.test/\">b</a>"), "{out}");
    }

    #[test]
    fn footnote_ids_carry_the_owner_anchor() {
        let out = docs("x[^n]\n\n[^n]: note", "r-2-1", 4);
        assert!(out.contains("href=\"#d-r-2-1-fn-n\""), "{out}");
        assert!(out.contains("id=\"d-r-2-1-fnref-n\""), "{out}");
        assert!(!out.contains("user-content-"), "{out}");
    }

    #[test]
    fn blank_docs_render_nothing() {
        assert_eq!(docs(" \n\t", "c", 2), "");
    }
}
