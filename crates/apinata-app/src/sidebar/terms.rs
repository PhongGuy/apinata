//! The sidebar filter's terms: plain name text plus `tag:` and `method:` constraints.

/// A parsed filter box. Every constraint must hold (AND).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterTerms {
    /// Lowercased name text; empty when the filter holds only `tag:` and `method:` terms.
    name: String,
    /// Tags a request must carry, case-sensitive like core's `TagFilter`.
    tags: Vec<String>,
    /// Lowercased methods; a request has one, so several never match unless equal.
    methods: Vec<String>,
}

impl FilterTerms {
    /// Parse the box text, or None when it constrains nothing (blank, or only empty-valued
    /// `tag:` and `method:`). Text with no recognised prefix is kept whole as the name substring,
    /// exactly as before terms existed; otherwise the remaining words are joined by one space.
    pub fn parse(text: &str) -> Option<FilterTerms> {
        let mut tags = Vec::new();
        let mut methods = Vec::new();
        let mut words = Vec::new();
        let mut prefixed = false;
        for word in text.split_whitespace() {
            match split_prefix(word) {
                Some((Prefix::Tag, value)) => {
                    prefixed = true;
                    if !value.is_empty() {
                        tags.push(value.to_owned());
                    }
                }
                Some((Prefix::Method, value)) => {
                    prefixed = true;
                    if !value.is_empty() {
                        methods.push(value.to_lowercase());
                    }
                }
                None => words.push(word),
            }
        }
        let name = if prefixed {
            words.join(" ")
        } else {
            text.to_owned()
        };
        let name = name.to_lowercase();
        if text.trim().is_empty()
            || (name.trim().is_empty() && tags.is_empty() && methods.is_empty())
        {
            return None;
        }
        Some(FilterTerms {
            name,
            tags,
            methods,
        })
    }

    /// Whether a leaf passes. `tags` are the leaf's effective tags (its own plus every ancestor
    /// folder's); None when unknown, which fails any `tag:` term. A missing `method` fails any
    /// `method:` term.
    pub fn matches(&self, name: &str, method: Option<&str>, tags: Option<&[String]>) -> bool {
        name.to_lowercase().contains(&self.name)
            && self
                .methods
                .iter()
                .all(|m| method.is_some_and(|x| x.to_lowercase() == *m))
            && self
                .tags
                .iter()
                .all(|t| tags.is_some_and(|have| have.contains(t)))
    }
}

enum Prefix {
    Tag,
    Method,
}

/// `tag:value` or `method:value`, the prefix in any case.
fn split_prefix(word: &str) -> Option<(Prefix, &str)> {
    let (head, value) = word.split_once(':')?;
    match head.to_lowercase().as_str() {
        "tag" => Some((Prefix::Tag, value)),
        "method" => Some((Prefix::Method, value)),
        _ => None,
    }
}
