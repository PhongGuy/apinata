//! The route table: each request with examples becomes a method plus a path template, and
//! incoming paths are matched against it by segment.

use super::respond::{Prepared, prepare};
use crate::interpolate::interpolate;
use crate::{Collection, Example, Folder, ParamKind, VarContext};
use percent_encoding::percent_decode_str;

/// Splits a URL's path into percent-decoded segments, dropping host, query, fragment and one
/// trailing slash. A URL with no path is the root, an empty list. Other empty segments stay,
/// so `/a//` cannot match a `:param`.
pub(crate) fn path_segments(url: &str) -> Vec<String> {
    let url = url.split(['?', '#']).next().unwrap_or("");
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("", |i| &rest[i..]),
        None if url.starts_with('/') => url,
        None => url.find('/').map_or("", |i| &url[i..]),
    };
    let path = path.strip_prefix('/').unwrap_or(path);
    let mut segments: Vec<&str> = path.split('/').collect();
    if segments.last() == Some(&"") {
        segments.pop();
    }
    segments
        .into_iter()
        .map(|s| percent_decode_str(s).decode_utf8_lossy().into_owned())
        .collect()
}

/// One request's example, with the recorded query pairs the picker matches on.
pub(crate) struct Entry {
    /// The request's item path, e.g. `users/get.yml`.
    pub request: String,
    pub example: Example,
    pub queries: Vec<(String, String)>,
    pub prepared: Prepared,
}

/// A method and path template, with every entry that shares them in tree order.
pub(crate) struct Route {
    pub method: String,
    pub segments: Vec<String>,
    pub entries: Vec<Entry>,
}

impl Route {
    fn literals(&self) -> usize {
        self.segments.iter().filter(|s| !is_param(s)).count()
    }

    /// Whether the decoded incoming segments fit the template; a param needs a non-empty one.
    fn fits(&self, incoming: &[String]) -> bool {
        self.segments.len() == incoming.len()
            && self
                .segments
                .iter()
                .zip(incoming)
                .all(|(t, i)| if is_param(t) { !i.is_empty() } else { t == i })
    }
}

fn is_param(segment: &str) -> bool {
    segment.len() > 1 && segment.starts_with(':')
}

/// The immutable table the server answers from.
pub(crate) struct RouteTable {
    routes: Vec<Route>,
}

impl RouteTable {
    /// Routes plus startup warnings (one per request whose URL has an undefined variable).
    pub(crate) fn build(collection: &Collection, ctx: &VarContext) -> (RouteTable, Vec<String>) {
        let mut table = RouteTable { routes: Vec::new() };
        let mut warnings = Vec::new();
        table.add_folder(collection, ctx, collection.tree(), &mut warnings);
        (table, warnings)
    }

    // A folder's own requests come first, then its subfolders, depth first.
    fn add_folder(
        &mut self,
        collection: &Collection,
        ctx: &VarContext,
        folder: &Folder,
        warnings: &mut Vec<String>,
    ) {
        for req in folder.requests() {
            let examples: Vec<Example> = req
                .examples()
                .into_iter()
                .filter(|e| e.response.is_some())
                .collect();
            if examples.is_empty() {
                continue;
            }
            let item = collection
                .item_path(req.path())
                .map(|p| p.as_str().to_string())
                .unwrap_or_default();
            let url = match interpolate(req.url(), &ctx.vars_for(req)) {
                Ok(url) => url,
                Err(e) => {
                    warnings.push(format!("{item}: {e}"));
                    continue;
                }
            };
            let vars = ctx.vars_for(req);
            let method = req.method().to_ascii_uppercase();
            let segments = path_segments(&url);
            let idx = match self
                .routes
                .iter()
                .position(|r| r.method == method && r.segments == segments)
            {
                Some(i) => i,
                None => {
                    self.routes.push(Route {
                        method,
                        segments,
                        entries: Vec::new(),
                    });
                    self.routes.len() - 1
                }
            };
            for example in examples {
                // Recorded query pairs steer the picker; one that cannot be resolved is dropped.
                let queries = example
                    .request
                    .iter()
                    .flat_map(|r| &r.params)
                    .filter(|p| p.enabled && p.kind == ParamKind::Query)
                    .filter_map(|p| Some((p.name.clone(), interpolate(&p.value, &vars).ok()?)))
                    .collect();
                let (prepared, notes) = match &example.response {
                    Some(r) => prepare(r),
                    None => continue,
                };
                warnings.extend(
                    notes
                        .into_iter()
                        .map(|n| format!("{item} / {}: {n}", example.name)),
                );
                self.routes[idx].entries.push(Entry {
                    request: item.clone(),
                    example,
                    queries,
                    prepared,
                });
            }
        }
        for sub in folder.folders() {
            self.add_folder(collection, ctx, sub, warnings);
        }
    }

    /// The matching route with the most literal segments; ties go to the earlier one.
    pub(crate) fn find(&self, method: &str, path: &str) -> Option<&Route> {
        let method = method.to_ascii_uppercase();
        let incoming = path_segments(path);
        let mut best: Option<&Route> = None;
        for r in self
            .routes
            .iter()
            .filter(|r| r.method == method && r.fits(&incoming))
        {
            if best.is_none_or(|b| r.literals() > b.literals()) {
                best = Some(r);
            }
        }
        best
    }

    /// Routes whose path matches under any other method (for the miss trace).
    pub(crate) fn find_other_methods(&self, method: &str, path: &str) -> Vec<&Route> {
        let method = method.to_ascii_uppercase();
        let incoming = path_segments(path);
        self.routes
            .iter()
            .filter(|r| r.method != method && r.fits(&incoming))
            .collect()
    }

    pub(crate) fn routes(&self) -> &[Route] {
        &self.routes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::testkit::{collection, request};
    use crate::{ContextOptions, VarContext};

    const OK: &str = "examples:\n  - name: ok\n    response:\n      status: 200\n";

    fn table(files: &[(&str, &str)], vars: &[(&str, &str)]) -> (RouteTable, Vec<String>) {
        let (_dir, c) = collection(files);
        let mut o = ContextOptions::new();
        for (k, v) in vars {
            o = o.env_var(*k, *v);
        }
        let ctx = VarContext::new(&c, o).unwrap();
        RouteTable::build(&c, &ctx)
    }

    #[test]
    fn path_segments_strips_host_query_fragment_and_trailing_slash() {
        assert_eq!(
            path_segments("https://h.test:8080/api/users/?a=1#f"),
            ["api", "users"]
        );
        assert_eq!(path_segments("http://h.test"), Vec::<String>::new());
        assert_eq!(path_segments("/a/b"), ["a", "b"]);
        assert_eq!(path_segments("h.test/a"), ["a"]);
        assert_eq!(path_segments("/a//b"), ["a", "", "b"]);
    }

    #[test]
    fn path_segments_percent_decodes_each_segment_without_splitting_on_encoded_slash() {
        assert_eq!(path_segments("/a%2Fb/c%20d"), ["a/b", "c d"]);
    }

    #[test]
    fn build_keeps_a_base_url_prefix_and_ignores_the_host() {
        let g = request("g", 1, "GET", "{{baseUrl}}/users", OK);
        let (t, w) = table(&[("g.yml", &g)], &[("baseUrl", "https://x.test/api")]);
        assert!(w.is_empty());
        assert_eq!(t.routes()[0].segments, ["api", "users"]);
    }

    #[test]
    fn build_skips_an_undefined_variable_with_a_warning_and_requests_without_examples() {
        let a = request("a", 1, "GET", "{{nope}}/x", OK);
        let b = request("b", 2, "GET", "/none", "");
        let (t, w) = table(&[("a.yml", &a), ("b.yml", &b)], &[]);
        assert!(t.routes().is_empty());
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("a.yml") && w[0].contains("nope"), "{w:?}");
    }

    #[test]
    fn same_method_and_template_share_one_route_in_tree_order() {
        let a = request("a", 1, "GET", "/u/:id", OK);
        let b = request("b", 2, "get", "/u/:id/", OK);
        let (t, _) = table(&[("a.yml", &a), ("b.yml", &b)], &[]);
        assert_eq!(t.routes().len(), 1);
        let names: Vec<&str> = t.routes()[0]
            .entries
            .iter()
            .map(|e| e.request.as_str())
            .collect();
        assert_eq!(names, ["a.yml", "b.yml"]);
    }

    #[test]
    fn the_route_with_more_literal_segments_wins_then_tree_order() {
        let a = request("a", 1, "GET", "/u/:id", OK);
        let b = request("b", 2, "GET", "/u/me", OK);
        let c = request("c", 3, "GET", "/:x/:y", OK);
        let (t, _) = table(&[("a.yml", &a), ("b.yml", &b), ("c.yml", &c)], &[]);
        assert_eq!(t.find("GET", "/u/me").unwrap().segments, ["u", "me"]);
        assert_eq!(t.find("GET", "/u/7").unwrap().segments, ["u", ":id"]);
        assert_eq!(t.find("GET", "/z/7").unwrap().segments, [":x", ":y"]);
    }

    #[test]
    fn matching_is_case_sensitive_and_method_exact_and_params_need_a_non_empty_segment() {
        let a = request("a", 1, "GET", "/Users/:id", OK);
        let (t, _) = table(&[("a.yml", &a)], &[]);
        assert!(t.find("GET", "/users/1").is_none());
        assert!(t.find("POST", "/Users/1").is_none());
        assert!(t.find("GET", "/Users/").is_none());
        assert!(t.find("GET", "/Users//").is_none());
        assert!(t.find("GET", "/Users/1/").is_some());
        assert!(t.find("GET", "/Users/1/2").is_none());
        assert_eq!(t.find_other_methods("POST", "/Users/1").len(), 1);
    }

    #[test]
    fn encoded_template_segments_compare_after_decoding() {
        let a = request("a", 1, "GET", "/a%20b", OK);
        let (t, _) = table(&[("a.yml", &a)], &[]);
        assert!(t.find("GET", "/a%20b").is_some());
    }
}
