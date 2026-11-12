//! Prepared example responses, the miss body and the hit log.

use super::MockHit;
use super::pick::Candidate;
use crate::{ExampleBodyKind, ExampleResponse};
use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

/// An example response ready to replay.
pub(crate) struct Prepared {
    pub status: u16,
    pub reason: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Binary bodies are replayed byte for byte, never templated.
    pub binary: bool,
}

/// Headers the server frames itself, so a recorded value would corrupt the reply, and headers
/// that describe the original response and go stale when it is replayed.
const SKIPPED: [&str; 8] = [
    "content-length",
    "transfer-encoding",
    "connection",
    "content-encoding",
    "keep-alive",
    "date",
    "etag",
    "alt-svc",
];

/// Status defaults to 200; framing and stale headers are dropped silently, and headers invalid as HTTP
/// are dropped with a warning each.
pub(crate) fn prepare(example: &ExampleResponse) -> (Prepared, Vec<String>) {
    let response = example.to_response();
    let mut warnings = Vec::new();
    let mut headers = Vec::new();
    // `to_response` lowercases names, so headers come from the example to keep their case.
    for (name, value) in &example.headers {
        if SKIPPED.iter().any(|s| name.eq_ignore_ascii_case(s)) {
            continue;
        }
        let valid = http::HeaderName::from_bytes(name.as_bytes()).is_ok()
            && http::HeaderValue::from_str(value).is_ok();
        if valid {
            headers.push((name.clone(), value.clone()));
        } else {
            warnings.push(format!("dropped invalid header `{name}`"));
        }
    }
    let status = match response.status() {
        0 => 200,
        s => s,
    };
    let reason = Some(response.status_text().to_string()).filter(|t| !t.is_empty());
    let prepared = Prepared {
        status,
        reason,
        headers,
        body: response.body().to_vec(),
        binary: example
            .body
            .as_ref()
            .is_some_and(|b| b.kind == ExampleBodyKind::Binary),
    };
    (prepared, warnings)
}

/// Replaces each `{{name}}` with its path param value in one left-to-right pass, so a value that
/// itself holds `{{x}}` is not expanded again. Unknown names and stray braces stay literal.
pub(crate) fn fill_params(text: &str, params: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let found = after.find("}}").and_then(|end| {
            let name = &after[..end];
            let value = params.iter().find(|(n, _)| *n == name)?.1;
            Some((value, end + 2))
        });
        match found {
            Some((value, used)) => {
                out.push_str(value);
                rest = &after[used..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Body of the 404 miss response.
pub(crate) fn miss_body(method: &str, path: &str, candidates: &[Candidate]) -> Vec<u8> {
    let candidates: Vec<_> = candidates
        .iter()
        .map(
            |c| serde_json::json!({"request": c.request, "example": c.example, "reason": c.reason}),
        )
        .collect();
    let body = serde_json::json!({
        "error": "no matching mock",
        "method": method,
        "path": path,
        "candidates": candidates,
    });
    serde_json::to_vec_pretty(&body).unwrap_or_default()
}

const LOG_CAP: usize = 1000;

/// The most recent hits, oldest first.
pub(crate) struct HitLog {
    hits: Mutex<VecDeque<MockHit>>,
}

impl HitLog {
    pub(crate) fn new() -> Self {
        HitLog {
            hits: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn push(&self, hit: MockHit) {
        let mut hits = self.hits.lock().unwrap_or_else(PoisonError::into_inner);
        if hits.len() == LOG_CAP {
            hits.pop_front();
        }
        hits.push_back(hit);
    }

    pub(crate) fn snapshot(&self) -> Vec<MockHit> {
        let hits = self.hits.lock().unwrap_or_else(PoisonError::into_inner);
        hits.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::pick::Candidate;
    use crate::{ExampleBody, ExampleBodyKind, ExampleResponse};

    fn resp() -> ExampleResponse {
        let mut r = ExampleResponse::new(201);
        r.status_text = Some("Made".into());
        r.headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Content-Length".into(), "999".into()),
            ("Content-Encoding".into(), "gzip".into()),
            ("Transfer-Encoding".into(), "chunked".into()),
            ("Connection".into(), "close".into()),
            ("Keep-Alive".into(), "5".into()),
            ("X-Ok".into(), "1".into()),
            ("bad name".into(), "x".into()),
            ("X-Bad".into(), "line\nbreak".into()),
        ];
        r.body = Some(ExampleBody::new(ExampleBodyKind::Json, "{\"a\":1}"));
        r
    }

    #[test]
    fn prepare_replays_status_headers_and_body_minus_hop_headers() {
        let (p, warnings) = prepare(&resp());
        assert_eq!(p.status, 201);
        assert_eq!(p.reason.as_deref(), Some("Made"));
        assert_eq!(p.body, b"{\"a\":1}");
        let names: Vec<&str> = p.headers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Content-Type", "X-Ok"]);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn prepare_leaves_out_headers_that_go_stale_on_replay() {
        let mut r = ExampleResponse::new(200);
        r.headers = vec![
            ("Date".into(), "Mon, 01 Jan 2024 00:00:00 GMT".into()),
            ("ETAG".into(), "\"abc\"".into()),
            ("alt-svc".into(), "h3=\":443\"".into()),
            ("X-Kept".into(), "1".into()),
        ];
        let (p, _) = prepare(&r);
        let names: Vec<&str> = p.headers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["X-Kept"]);
    }

    #[test]
    fn prepare_defaults_the_status_and_decodes_binary_bodies() {
        let mut r = ExampleResponse::new(200);
        r.status = None;
        r.body = Some(ExampleBody::new(ExampleBodyKind::Binary, "AAEC/w=="));
        let (p, w) = prepare(&r);
        assert_eq!(p.status, 200);
        assert_eq!(p.body, [0u8, 1, 2, 255]);
        assert!(w.is_empty());
    }

    #[test]
    fn fill_params_substitutes_known_names_once_and_leaves_the_rest() {
        let p = [("id", "{{x}}"), ("n", "7")];
        assert_eq!(
            fill_params("a {{id}} {{n}} {{zz}} {{", &p),
            "a {{x}} 7 {{zz}} {{"
        );
        assert_eq!(fill_params("{{n}}{{n}}", &p), "77");
    }

    #[test]
    fn fill_params_treats_a_spaced_name_as_a_different_name() {
        assert_eq!(fill_params("{{ n }}", &[("n", "7")]), "{{ n }}");
    }

    #[test]
    fn miss_body_is_json_with_method_path_and_candidates() {
        let c = vec![Candidate {
            request: "a.yml".into(),
            example: "ok".into(),
            reason: "method is POST".into(),
        }];
        let v: serde_json::Value = serde_json::from_slice(&miss_body("GET", "/x", &c)).unwrap();
        assert_eq!(v["error"], "no matching mock");
        assert_eq!(v["method"], "GET");
        assert_eq!(v["path"], "/x");
        assert_eq!(v["candidates"][0]["request"], "a.yml");
        assert_eq!(v["candidates"][0]["example"], "ok");
        assert_eq!(v["candidates"][0]["reason"], "method is POST");
    }

    fn hit(i: u16) -> MockHit {
        MockHit::new("GET", &format!("/{i}"), 200, Some(("r.yml", "ok")), false)
    }

    #[test]
    fn the_hit_log_keeps_the_last_thousand_in_order() {
        let log = HitLog::new();
        for i in 0..1005 {
            log.push(hit(i));
        }
        let all = log.snapshot();
        assert_eq!(all.len(), 1000);
        assert_eq!(all[0].path, "/5");
        assert_eq!(all[999].path, "/1004");
    }

    #[test]
    fn the_log_line_names_request_and_example_or_no_match() {
        let l = hit(1).line();
        assert!(l.contains("GET /1 -> 200 r.yml / ok"), "{l}");
        let miss = MockHit::new("PUT", "/z", 404, None, false).line();
        assert!(miss.ends_with("PUT /z -> 404 (no match)"), "{miss}");
        let big = MockHit::new("POST", "/z", 413, None, false).line();
        assert!(big.ends_with("POST /z -> 413 (rejected)"), "{big}");
        assert!(
            MockHit::new("GET", "/", 200, Some(("r.yml", "ok")), true)
                .line()
                .ends_with("(fallback)")
        );
    }
}
