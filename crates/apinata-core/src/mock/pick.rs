//! Chooses which saved example answers a matched route, and explains a miss.

use crate::mock::routes::{Entry, Route};

/// The `x-mock-response-name` and `x-mock-response-code` request headers.
pub(crate) struct Overrides {
    pub name: Option<String>,
    pub code: Option<u16>,
}

pub(crate) enum Pick<'a> {
    /// `fallback` is set when no example's recorded query fit and the first was served anyway.
    Hit {
        entry: &'a Entry,
        fallback: bool,
    },
    Miss {
        candidates: Vec<Candidate>,
    },
}

/// One example considered for a miss, with why it was not chosen.
pub(crate) struct Candidate {
    pub request: String,
    pub example: String,
    pub reason: String,
}

const MAX_CANDIDATES: usize = 20;

fn status_of(entry: &Entry) -> u16 {
    entry
        .example
        .response
        .as_ref()
        .and_then(|r| r.status)
        .unwrap_or(200)
}

/// Parses a raw query string into percent-decoded pairs, in order, repeats kept.
pub(crate) fn parse_query(raw: Option<&str>) -> Vec<(String, String)> {
    url::form_urlencoded::parse(raw.unwrap_or("").as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// Applies the override headers first, then prefers the example whose recorded query pairs all
/// appear in the incoming query (most pairs wins, file order breaks ties). With none eligible
/// the first kept example is served as a fallback.
pub(crate) fn pick<'a>(route: &'a Route, query: &[(String, String)], o: &Overrides) -> Pick<'a> {
    let kept: Vec<&Entry> = route
        .entries
        .iter()
        .filter(|e| o.name.as_ref().is_none_or(|n| &e.example.name == n))
        .filter(|e| o.code.is_none_or(|c| status_of(e) == c))
        .collect();
    if kept.is_empty() {
        let candidates = route
            .entries
            .iter()
            .take(MAX_CANDIDATES)
            .map(|e| {
                let reason = match (&o.name, o.code) {
                    (Some(n), _) if &e.example.name != n => {
                        format!("name is \"{}\", wanted \"{n}\"", e.example.name)
                    }
                    (_, Some(c)) => format!("status is {}, wanted {c}", status_of(e)),
                    _ => "excluded by override".to_string(),
                };
                Candidate {
                    request: e.request.clone(),
                    example: e.example.name.clone(),
                    reason,
                }
            })
            .collect();
        return Pick::Miss { candidates };
    }
    let mut best: Option<&Entry> = None;
    for e in &kept {
        let eligible = e.queries.iter().all(|r| query.contains(r));
        if eligible && best.is_none_or(|b| e.queries.len() > b.queries.len()) {
            best = Some(e);
        }
    }
    match best {
        Some(entry) => Pick::Hit {
            entry,
            fallback: false,
        },
        None => Pick::Hit {
            entry: kept[0],
            fallback: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::routes::RouteTable;
    use crate::mock::testkit::{collection, request};
    use crate::{ContextOptions, VarContext};

    fn route(examples: &str) -> (RouteTable, tempfile::TempDir) {
        let (dir, c) = collection(&[("r.yml", &request("r", 1, "GET", "/r", examples))]);
        let ctx = VarContext::new(&c, ContextOptions::new().env_var("q", "blue")).unwrap();
        (RouteTable::build(&c, &ctx).0, dir)
    }
    fn ex(name: &str, status: u16, params: &str) -> String {
        format!(
            "  - name: {name}\n    request:\n      url: /r\n      method: GET\n{params}    response:\n      status: {status}\n"
        )
    }
    fn q(name: &str, value: &str, extra: &str) -> String {
        format!(
            "      params:\n        - name: {name}\n          value: \"{value}\"\n          type: query\n{extra}"
        )
    }
    fn name(p: &Pick) -> (String, bool) {
        match p {
            Pick::Hit { entry, fallback } => (entry.example.name.clone(), *fallback),
            Pick::Miss { .. } => panic!("miss"),
        }
    }
    fn none() -> Overrides {
        Overrides {
            name: None,
            code: None,
        }
    }
    fn pairs(s: &str) -> Vec<(String, String)> {
        parse_query(Some(s))
    }

    #[test]
    fn parse_query_decodes_and_keeps_order_and_repeats() {
        assert_eq!(
            pairs("a=1&b=x%20y&a=2&c&d=p+q"),
            vec![
                ("a".into(), "1".into()),
                ("b".into(), "x y".into()),
                ("a".into(), "2".into()),
                ("c".into(), "".into()),
                ("d".into(), "p q".into())
            ]
        );
        assert!(parse_query(None).is_empty());
    }

    #[test]
    fn the_example_with_the_most_matching_recorded_params_wins() {
        let y = format!(
            "examples:\n{}{}{}",
            ex("plain", 200, ""),
            ex("blue", 201, &q("c", "blue", "")),
            ex("one", 202, &q("c", "red", ""))
        );
        let (t, _d) = route(&y);
        let r = &t.routes()[0];
        assert_eq!(
            name(&pick(r, &pairs("c=blue&extra=1"), &none())),
            ("blue".into(), false)
        );
        assert_eq!(
            name(&pick(r, &pairs("c=red"), &none())),
            ("one".into(), false)
        );
    }

    #[test]
    fn a_recordless_example_is_the_default_for_an_unmatched_query() {
        let y = format!(
            "examples:\n{}{}",
            ex("blue", 201, &q("c", "blue", "")),
            ex("plain", 200, "")
        );
        let (t, _d) = route(&y);
        let r = &t.routes()[0];
        assert_eq!(
            name(&pick(r, &pairs("c=green"), &none())),
            ("plain".into(), false)
        );
    }

    #[test]
    fn when_every_example_records_a_param_the_first_is_served_as_a_fallback() {
        let y = format!(
            "examples:\n{}{}",
            ex("blue", 201, &q("c", "blue", "")),
            ex("red", 202, &q("c", "red", ""))
        );
        let (t, _d) = route(&y);
        assert_eq!(
            name(&pick(&t.routes()[0], &pairs("c=green"), &none())),
            ("blue".into(), true)
        );
        assert_eq!(
            name(&pick(&t.routes()[0], &pairs(""), &none())),
            ("blue".into(), true)
        );
    }

    #[test]
    fn a_repeated_incoming_param_satisfies_when_any_pair_equals() {
        let y = format!(
            "examples:\n{}{}",
            ex("two", 201, &q("a", "2", "")),
            ex("plain", 200, "")
        );
        let (t, _d) = route(&y);
        assert_eq!(
            name(&pick(&t.routes()[0], &pairs("a=1&a=2"), &none())),
            ("two".into(), false)
        );
    }

    #[test]
    fn disabled_recorded_params_are_ignored_and_recorded_values_are_interpolated() {
        let off = q("c", "red", "          disabled: true\n");
        let y = format!(
            "examples:\n{}{}",
            ex("off", 201, &off),
            ex("var", 202, &q("c", "{{q}}", ""))
        );
        let (t, _d) = route(&y);
        let r = &t.routes()[0];
        assert_eq!(
            name(&pick(r, &pairs("c=blue"), &none())),
            ("var".into(), false)
        );
        assert_eq!(
            name(&pick(r, &pairs("c=zzz"), &none())),
            ("off".into(), false)
        );
    }

    #[test]
    fn an_uninterpolatable_recorded_value_is_dropped_from_the_constraint() {
        let y = format!("examples:\n{}", ex("u", 200, &q("c", "{{missing}}", "")));
        let (t, _d) = route(&y);
        assert_eq!(
            name(&pick(&t.routes()[0], &pairs("c=anything"), &none())),
            ("u".into(), false)
        );
    }

    #[test]
    fn override_headers_select_by_name_and_status_or_miss_with_candidates() {
        let y = format!("examples:\n{}{}", ex("ok", 200, ""), ex("gone", 410, ""));
        let (t, _d) = route(&y);
        let r = &t.routes()[0];
        let by_name = Overrides {
            name: Some("gone".into()),
            code: None,
        };
        assert_eq!(name(&pick(r, &[], &by_name)).0, "gone");
        let by_code = Overrides {
            name: None,
            code: Some(200),
        };
        assert_eq!(name(&pick(r, &[], &by_code)).0, "ok");
        let both = Overrides {
            name: Some("ok".into()),
            code: Some(410),
        };
        match pick(r, &[], &both) {
            Pick::Miss { candidates } => {
                assert_eq!(candidates.len(), 2);
                assert_eq!(candidates[0].request, "r.yml");
                assert!(!candidates[0].reason.is_empty());
            }
            _ => panic!("expected a miss"),
        }
    }

    #[test]
    fn examples_without_a_response_register_nothing() {
        let y = "examples:\n  - name: bare\n";
        let (t, _d) = route(y);
        assert!(t.routes().is_empty());
    }
}
