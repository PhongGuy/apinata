//! The "environment not found" text shared by every error that reports one: the names that do
//! exist and, when one is close to what was typed, a suggestion.

/// The closest of `names` to `wanted`: a case-insensitive match, else the unique name within
/// `max(1, len / 3)` edits. `None` when nothing is close or two names tie.
pub(crate) fn closest<'a>(wanted: &str, names: &'a [String]) -> Option<&'a str> {
    // A case-only difference is the likeliest slip, so it wins outright when it is unique.
    let mut folded = names
        .iter()
        .filter(|n| n.to_lowercase() == wanted.to_lowercase());
    if let (Some(only), None) = (folded.next(), folded.next()) {
        return Some(only);
    }
    let wanted: Vec<char> = wanted.to_lowercase().chars().collect();
    if wanted.is_empty() {
        return None;
    }
    let allowance = (wanted.len() / 3).max(1);
    let mut best: Option<(&str, usize)> = None;
    let mut tied = false;
    for name in names {
        let lowered: Vec<char> = name.to_lowercase().chars().collect();
        let distance = edit_distance(&wanted, &lowered);
        if distance > allowance {
            continue;
        }
        match best {
            Some((_, d)) if distance > d => {}
            Some((_, d)) if distance == d => tied = true,
            _ => {
                best = Some((name, distance));
                tied = false;
            }
        }
    }
    best.filter(|_| !tied).map(|(name, _)| name)
}

/// Levenshtein distance between two char slices, over two rolling rows.
fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(ca != cb);
            row.push(substitute.min(previous[j + 1] + 1).min(row[j] + 1));
        }
        previous = row;
    }
    previous[b.len()]
}

/// The not-found text for a collection's environments, or the global ones when `global`.
pub(crate) fn env_text(global: bool, name: &str, available: &[String]) -> String {
    if global {
        not_found_text(
            "global environment",
            "no global environments",
            name,
            available,
        )
    } else {
        not_found_text(
            "environment",
            "this collection has no environments",
            name,
            available,
        )
    }
}

/// `{kind} "{name}" not found; available: a, b; did you mean a?`, or `{none}` after the name
/// when `available` is empty.
pub(crate) fn not_found_text(kind: &str, none: &str, name: &str, available: &[String]) -> String {
    if available.is_empty() {
        return format!("{kind} \"{name}\" not found; {none}");
    }
    let mut text = format!(
        "{kind} \"{name}\" not found; available: {}",
        available.join(", ")
    );
    if let Some(near) = closest(name, available) {
        text.push_str(&format!("; did you mean {near}?"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(of: &[&str]) -> Vec<String> {
        of.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_case_difference_is_the_closest() {
        let n = names(&["dev", "staging"]);
        assert_eq!(closest("Dev", &n), Some("dev"));
        assert_eq!(closest("STAGING", &n), Some("staging"));
    }

    #[test]
    fn one_edit_is_close_and_a_far_name_is_not() {
        let n = names(&["dev", "staging", "prod"]);
        assert_eq!(closest("stagin", &n), Some("staging"));
        assert_eq!(closest("deb", &n), Some("dev"));
        assert_eq!(closest("nosuch", &n), None);
        assert_eq!(closest("de", &names(&["dev", "prod"])), Some("dev"));
    }

    #[test]
    fn the_allowance_grows_with_the_length() {
        let n = names(&["production"]);
        assert_eq!(closest("prodxxtiox", &n), Some("production"));
        assert_eq!(closest("prodxxxxxx", &n), None);
    }

    #[test]
    fn a_tie_between_names_suggests_nothing() {
        assert_eq!(closest("dex", &names(&["dev", "dew"])), None);
        assert_eq!(closest("", &names(&["a"])), None);
        assert_eq!(closest("x", &[]), None);
    }

    #[test]
    fn an_exact_case_variant_beats_an_edit_neighbour() {
        assert_eq!(closest("Dev", &names(&["dev", "den"])), Some("dev"));
    }

    #[test]
    fn the_text_lists_names_and_suggests() {
        let n = names(&["dev", "staging", "prod"]);
        assert_eq!(
            not_found_text(
                "environment",
                "this collection has no environments",
                "stagin",
                &n
            ),
            "environment \"stagin\" not found; available: dev, staging, prod; did you mean staging?"
        );
        assert_eq!(
            not_found_text(
                "environment",
                "this collection has no environments",
                "zzz",
                &n
            ),
            "environment \"zzz\" not found; available: dev, staging, prod"
        );
    }

    #[test]
    fn the_text_says_when_there_are_none() {
        assert_eq!(
            not_found_text(
                "environment",
                "this collection has no environments",
                "x",
                &[]
            ),
            "environment \"x\" not found; this collection has no environments"
        );
        assert_eq!(
            not_found_text("global environment", "no global environments", "x", &[]),
            "global environment \"x\" not found; no global environments"
        );
    }
}
