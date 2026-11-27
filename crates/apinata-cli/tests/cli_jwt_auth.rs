//! `apinata run` signs a JWT bearer token through core.

use crate::common;

use common::{ROOT, check, column, run_json, write_tree};
use wiremock::matchers::{header_regex, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "cli-jwt-secret-1";

fn request(url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: J\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n  auth:\n    type: jwt\n    algorithm: HS256\n    secret: \"{{{{jwt_secret}}}}\"\n    payload: '{{\"sub\":\"me\"}}'\n    expiresIn: 60\n{extra}runtime:\n  assertions:\n{}",
        check("res.status", "eq", "200")
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_sends_a_signed_token_in_the_header_and_the_query() {
    let s = MockServer::start().await;
    let token = r"^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$";
    Mock::given(path("/h"))
        .and(header_regex(
            "authorization",
            &format!("^Bearer {}", &token[1..]),
        ))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    Mock::given(path("/q"))
        .and(query_param("keep", "1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let header = request(&format!("{}/h", s.uri()), "");
    let query = request(
        &format!("{}/q?keep=1&jwt=old", s.uri()),
        "    placement: query\n    queryParamKey: jwt\n",
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("h.yml", &header),
        ("q.yml", &query),
    ]);
    let var = format!("jwt_secret={SECRET}");
    let (code, report) = run_json(dir.path(), &["--no-scripts", "--env-var", &var]);
    assert_eq!(
        (code, column(&report, "outcome")),
        (0, vec!["passed".to_owned(), "passed".to_owned()]),
        "{report}"
    );
    assert!(!report.to_string().contains(SECRET), "{report}");
    let sent = s.received_requests().await.unwrap_or_default();
    let q = sent
        .iter()
        .find(|r| r.url.path() == "/q")
        .expect("query send");
    let jwt: Vec<String> = q
        .url
        .query_pairs()
        .filter(|(k, _)| k == "jwt")
        .map(|(_, v)| v.into_owned())
        .collect();
    assert_eq!(jwt.len(), 1, "{jwt:?}");
    assert!(regex_like(&jwt[0]), "{}", jwt[0]);
    // Neither token, the header one nor the one in the query, reaches the report.
    let h = sent
        .iter()
        .find(|r| r.url.path() == "/h")
        .expect("header send");
    let bearer = h.headers.get("authorization").unwrap().to_str().unwrap();
    let header_token = bearer.strip_prefix("Bearer ").unwrap();
    let text = report.to_string();
    for token in [header_token, jwt[0].as_str()] {
        assert!(!text.contains(token), "{text}");
    }
}

/// Whether `token` has three base64url parts, the first two non-empty.
fn regex_like(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3
        && parts[..2].iter().all(|p| !p.is_empty())
        && parts.iter().all(|p| {
            p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}
