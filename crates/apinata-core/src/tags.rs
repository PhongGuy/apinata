//! Request tags: normalising them and filtering a run by the tags a request carries.

/// Keeps a request when `include` is empty or shares a tag with it, and `exclude` shares none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TagFilter {
    include: Vec<String>,
    exclude: Vec<String>,
}

impl TagFilter {
    /// A filter keeping requests with any tag of `include` and none of `exclude`.
    pub fn new(include: Vec<String>, exclude: Vec<String>) -> TagFilter {
        TagFilter { include, exclude }
    }

    /// Comma-separated input, each item trimmed, empty items dropped.
    pub fn parse_list(text: &str) -> Vec<String> {
        text.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Whether the filter keeps every request.
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    /// The filter in words for an error message, such as `tags a, b; excluding tags c`;
    /// `None` when the filter keeps every request.
    pub fn describe(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.include.is_empty() {
            parts.push(format!("tags {}", self.include.join(", ")));
        }
        if !self.exclude.is_empty() {
            parts.push(format!("excluding tags {}", self.exclude.join(", ")));
        }
        (!parts.is_empty()).then(|| parts.join("; "))
    }

    /// Included when `include` is empty or shares a tag, and `exclude` shares none; case-sensitive.
    pub fn matches(&self, effective: &[String]) -> bool {
        let included =
            self.include.is_empty() || self.include.iter().any(|t| effective.contains(t));
        included && !self.exclude.iter().any(|t| effective.contains(t))
    }
}

/// Tags trimmed, empties dropped and each kept once, at its first position; case-sensitive.
pub fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    normalize(tags)
}

/// [`normalize_tags`] over any iterator of owned tags.
pub(crate) fn normalize(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if !tag.is_empty() && !out.iter().any(|t| t == tag) {
            out.push(tag.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn describe_names_the_include_and_exclude_lists() {
        assert_eq!(TagFilter::default().describe(), None);
        let inc = TagFilter::new(list(&["nope"]), vec![]);
        assert_eq!(inc.describe().as_deref(), Some("tags nope"));
        let exc = TagFilter::new(vec![], list(&["a", "b"]));
        assert_eq!(exc.describe().as_deref(), Some("excluding tags a, b"));
        let both = TagFilter::new(list(&["x"]), list(&["y"]));
        assert_eq!(both.describe().as_deref(), Some("tags x; excluding tags y"));
    }
}
