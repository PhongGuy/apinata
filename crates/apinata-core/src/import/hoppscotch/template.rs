//! Hoppscotch's `<<name>>` variable templates.

/// `text` with each `<<name>>` as `{{name}}`, and the `$`-prefixed names met that core does not
/// generate, first-seen order, no repeats.
pub(super) fn rewrite(text: &str) -> (String, Vec<String>) {
    // Hoppscotch's pattern is `<<([^>]*)>>` with a non-empty inner part: from each `<<`, the
    // inner text runs to the first `>`, and only a `>>` there closes it.
    let mut out = String::with_capacity(text.len());
    let mut dynamic: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("<<") {
        let after = &rest[open + 2..];
        let close = after
            .find('>')
            .filter(|&at| at > 0 && after[at..].starts_with(">>"));
        let Some(close) = close else {
            // Not a template here; keep one `<` and try again from the next character.
            out.push_str(&rest[..open + 1]);
            rest = &rest[open + 1..];
            continue;
        };
        let name = &after[..close];
        let unknown = name
            .strip_prefix('$')
            .is_some_and(|n| !crate::dynamic::dynamic_variables().contains(&n));
        if unknown && !dynamic.iter().any(|d| d == name) {
            dynamic.push(name.to_owned());
        }
        out.push_str(&rest[..open]);
        out.push_str("{{");
        out.push_str(name);
        out.push_str("}}");
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    (out, dynamic)
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    #[test]
    fn rewrites_names_and_reports_dynamic_ones() {
        let cases: [(&str, &str, &[&str]); 8] = [
            ("plain", "plain", &[]),
            ("<<a>>", "{{a}}", &[]),
            ("x<<a.b>>y<<c>>", "x{{a.b}}y{{c}}", &[]),
            ("<< spaced >>", "{{ spaced }}", &[]),
            (
                "<<$guid>>-<<$guid>>-<<$ts>>",
                "{{$guid}}-{{$guid}}-{{$ts}}",
                &["$ts"],
            ),
            ("<<>>", "<<>>", &[]),
            ("<<open", "<<open", &[]),
            ("<<a>b>>", "<<a>b>>", &[]),
        ];
        for (input, text, names) in cases {
            let (got, found) = rewrite(input);
            assert_eq!(
                (
                    got.as_str(),
                    found.iter().map(String::as_str).collect::<Vec<_>>()
                ),
                (text, names.to_vec()),
                "{input}"
            );
        }
    }
}
