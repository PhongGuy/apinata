//! GraphQL subscriptions over WebSocket: detection of a subscription body and its transport.

pub(crate) mod handshake;
pub(crate) mod socket;

use std::time::Duration;

use apollo_parser::Parser;

use crate::stream::StreamLimits;
use apollo_parser::cst::Definition;

/// The `subscribe` payload of a GraphQL body whose one operation is a subscription.
pub(crate) struct Plan {
    /// `{"query": <string>}` plus `"variables"` when the body has that key; nothing else.
    pub(crate) payload: serde_json::Value,
}

/// Reads a sent GraphQL body. `Ok(None)`: not a subscription (not a JSON object, no string
/// `query`, or no subscription operation). `Err("a subscription request must hold one
/// operation")`: a subscription shares the document with another operation.
pub(crate) fn detect(body: &str) -> Result<Option<Plan>, String> {
    let Ok(serde_json::Value::Object(object)) = serde_json::from_str(body) else {
        return Ok(None);
    };
    let Some(query) = object.get("query").and_then(|q| q.as_str()) else {
        return Ok(None);
    };
    // Parse errors do not matter: count the operations the tree holds, fragments aside.
    let tree = Parser::new(query).parse();
    let (mut operations, mut subscriptions) = (0, 0);
    for definition in tree.document().definitions() {
        if let Definition::OperationDefinition(op) = definition {
            operations += 1;
            if op
                .operation_type()
                .and_then(|t| t.subscription_token())
                .is_some()
            {
                subscriptions += 1;
            }
        }
    }
    match (subscriptions, operations) {
        (0, _) => Ok(None),
        (1, 1) => {
            let mut payload = serde_json::Map::new();
            payload.insert("query".to_owned(), query.into());
            if let Some(variables) = object.get("variables") {
                payload.insert("variables".to_owned(), variables.clone());
            }
            Ok(Some(Plan {
                payload: payload.into(),
            }))
        }
        _ => Err("a subscription request must hold one operation".to_owned()),
    }
}

/// How long `connection_ack` may take: the request timeout when set, else 10 s.
pub(crate) fn ack_wait(timeout: Option<Duration>) -> Duration {
    timeout.unwrap_or(Duration::from_secs(10))
}

/// The limits of a send that does not stream: min(request timeout, 30 s) as the duration bound.
pub(crate) fn unstreamed_limits(timeout: Option<Duration>) -> StreamLimits {
    let ceiling = Duration::from_secs(30);
    StreamLimits::new().max_duration(Some(timeout.map_or(ceiling, |t| t.min(ceiling))))
}

#[cfg(test)]
mod tests {
    use super::detect;
    use serde_json::json;

    fn payload(body: serde_json::Value) -> Option<serde_json::Value> {
        detect(&body.to_string())
            .expect("not refused")
            .map(|p| p.payload)
    }

    #[test]
    fn one_subscription_is_detected_with_its_variables() {
        let q = "subscription Ticks($n: Int) { tick(n: $n) }";
        assert_eq!(
            payload(json!({"query": q, "variables": {"n": 2}})),
            Some(json!({"query": q, "variables": {"n": 2}}))
        );
    }

    #[test]
    fn fragments_are_not_operations() {
        let q = "subscription { tick { ...F } } fragment F on Tick { n }";
        assert_eq!(payload(json!({"query": q})), Some(json!({"query": q})));
    }

    #[test]
    fn only_query_and_variables_are_sent() {
        let q = "subscription { a }";
        let body = json!({"query": q, "operationName": "X", "extensions": {}});
        assert_eq!(payload(body), Some(json!({"query": q})));
    }

    #[test]
    fn other_operations_and_shapes_stay_http() {
        for body in [
            json!({"query": "query { a }"}),
            json!({"query": "{ a }"}),
            json!({"query": "mutation { a }"}),
            json!({"query": 5}),
            json!({"variables": {}}),
            json!("subscription { a }"),
            json!(null),
        ] {
            assert_eq!(payload(body.clone()), None, "{body}");
        }
        assert!(detect("subscription { a }").expect("not refused").is_none());
    }

    #[test]
    fn a_subscription_beside_another_operation_is_refused() {
        for q in [
            "subscription A { a } query B { b }",
            "query B { b } subscription A { a }",
            "subscription A { a } subscription B { b }",
        ] {
            let refused = detect(&json!({"query": q}).to_string()).err();
            assert_eq!(
                refused.as_deref(),
                Some("a subscription request must hold one operation"),
                "{q}"
            );
        }
    }

    #[test]
    fn several_operations_without_a_subscription_stay_http() {
        let body = json!({"query": "query A { a } mutation B { b }"});
        assert_eq!(payload(body), None);
    }

    #[test]
    fn the_ack_wait_is_the_request_timeout_or_ten_seconds() {
        use std::time::Duration;
        assert_eq!(super::ack_wait(None), Duration::from_secs(10));
        assert_eq!(
            super::ack_wait(Some(Duration::from_millis(300))),
            Duration::from_millis(300)
        );
    }

    #[test]
    fn an_unstreamed_send_is_bounded_by_its_timeout_and_thirty_seconds() {
        use std::time::Duration;
        let bound = |t| super::unstreamed_limits(t).max_duration;
        assert_eq!(bound(None), Some(Duration::from_secs(30)));
        assert_eq!(
            bound(Some(Duration::from_millis(400))),
            Some(Duration::from_millis(400))
        );
        assert_eq!(
            bound(Some(Duration::from_secs(60))),
            Some(Duration::from_secs(30))
        );
    }
}
