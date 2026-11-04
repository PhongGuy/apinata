//! Path-tracking readers for the internally tagged documents in tool arguments. serde reads
//! such a document through a buffer that loses the path, so an error inside one is re-found
//! here by reading the document, without its `type`, as the tagged variant's own struct.

use crate::dto::{MaxBodySize, Oauth1KeyDoc, Oauth2Doc, PlacementDoc};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// A path-tracked error.
pub(crate) type Error = serde_path_to_error::Error<serde_json::Error>;

/// The error inside `doc`, the object at field `key`, when `key` names a tagged document
/// and its `type` a variant; None when it names neither or the variant reads cleanly.
pub(crate) fn variant_error(key: &str, doc: &Map<String, Value>) -> Option<Error> {
    let mode = doc.get("type")?.as_str()?;
    let mut rest = doc.clone();
    rest.remove("type");
    let rest = Value::Object(rest);
    // `auth` and `token_placement` hold these documents in every tool's arguments.
    match (key, mode) {
        ("auth", "none" | "inherit") | ("token_placement", "unset") => read::<NoFields>(rest),
        ("auth", "basic" | "digest" | "wsse") => read::<UserPass>(rest),
        ("auth", "bearer") => read::<Bearer>(rest),
        ("auth", "apikey") => read::<ApiKey>(rest),
        ("auth", "awsv4") => read::<AwsV4>(rest),
        ("auth", "oauth1") => read::<OAuth1>(rest),
        ("auth", "oauth2") => read::<Oauth2Doc>(rest),
        ("auth", "ntlm") => read::<Ntlm>(rest),
        ("auth", "edgegrid") => read::<EdgeGrid>(rest),
        ("auth", "unsupported") => read::<Unsupported>(rest),
        ("token_placement", "header") => read::<Prefix>(rest),
        ("token_placement", "query") => read::<Key>(rest),
        _ => None,
    }
}

fn read<T: DeserializeOwned>(rest: Value) -> Option<Error> {
    serde_path_to_error::deserialize::<_, T>(&rest).err()
}

// Each struct below mirrors one variant of `AuthDoc` or `TokenPlacementDoc` in dto.rs.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoFields {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "read only to find an error")]
struct UserPass {
    username: String,
    password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "read only to find an error")]
struct Bearer {
    token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "read only to find an error")]
struct ApiKey {
    key: String,
    value: String,
    placement: PlacementDoc,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "read only to find an error")]
struct AwsV4 {
    access_key_id: String,
    secret_access_key: String,
    session_token: String,
    service: String,
    region: String,
    profile_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "read only to find an error")]
struct OAuth1 {
    consumer_key: Option<String>,
    consumer_secret: Option<String>,
    access_token: Option<String>,
    access_token_secret: Option<String>,
    callback_url: Option<String>,
    verifier: Option<String>,
    signature_method: String,
    private_key: Option<Oauth1KeyDoc>,
    timestamp: Option<String>,
    nonce: Option<String>,
    version: String,
    realm: Option<String>,
    placement: String,
    include_body_hash: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "read only to find an error")]
struct Ntlm {
    username: String,
    password: String,
    domain: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "read only to find an error")]
struct EdgeGrid {
    access_token: String,
    client_token: String,
    client_secret: String,
    nonce: String,
    timestamp: String,
    base_url: String,
    headers_to_sign: String,
    max_body_size: Option<MaxBodySize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "read only to find an error")]
struct Unsupported {
    mode: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "read only to find an error")]
struct Prefix {
    prefix: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code, reason = "read only to find an error")]
struct Key {
    key: String,
}

#[cfg(test)]
mod tests {
    use super::variant_error;
    use crate::dto::{AuthDoc, TokenPlacementDoc};
    use serde_json::{Value, json};

    /// Every variant's documents, each of which the mirror must also read cleanly.
    fn object(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().expect("object").clone()
    }

    #[test]
    fn every_valid_auth_document_reads_cleanly_through_its_mirror() {
        let docs = [
            json!({"type": "none"}),
            json!({"type": "inherit"}),
            json!({"type": "basic", "username": "u", "password": "p"}),
            json!({"type": "digest", "username": "u", "password": "p"}),
            json!({"type": "wsse", "username": "u", "password": "p"}),
            json!({"type": "bearer", "token": "t"}),
            json!({"type": "apikey", "key": "k", "value": "v", "placement": "header"}),
            json!({"type": "awsv4", "access_key_id": "", "secret_access_key": "",
                "session_token": "", "service": "", "region": "", "profile_name": ""}),
            json!({"type": "oauth1", "consumer_key": null, "consumer_secret": null,
                "access_token": null, "access_token_secret": null, "callback_url": null,
                "verifier": null, "signature_method": "HMAC-SHA1",
                "private_key": {"type": "text", "value": "k"}, "timestamp": null,
                "nonce": null, "version": "1.0", "realm": null, "placement": "header",
                "include_body_hash": false}),
            json!({"type": "oauth2", "flow": "implicit", "authorization_url": "",
                "access_token_url": "", "refresh_token_url": "", "callback_url": "",
                "client_id": "", "client_secret": "", "client_auth": null, "username": "",
                "password": "", "scope": "", "state": "", "pkce": false, "token_id": "",
                "token_placement": {"type": "unset"}, "token_source": "access_token",
                "auto_fetch": true, "auto_refresh": false, "params": []}),
            json!({"type": "ntlm", "username": "u", "password": "p", "domain": ""}),
            json!({"type": "edgegrid", "access_token": "", "client_token": "",
                "client_secret": "", "nonce": "", "timestamp": "", "base_url": "",
                "headers_to_sign": "", "max_body_size": "Infinity"}),
            json!({"type": "unsupported", "mode": "hawk"}),
        ];
        for doc in docs {
            serde_json::from_value::<AuthDoc>(doc.clone()).expect("AuthDoc reads it");
            assert!(
                variant_error("auth", &object(doc.clone())).is_none(),
                "{doc}"
            );
        }
        for doc in [
            json!({"type": "header", "prefix": "Bearer"}),
            json!({"type": "query", "key": "k"}),
            json!({"type": "unset"}),
        ] {
            serde_json::from_value::<TokenPlacementDoc>(doc.clone()).expect("reads");
            assert!(
                variant_error("token_placement", &object(doc.clone())).is_none(),
                "{doc}"
            );
        }
    }

    /// The backquoted names in an unknown-field message after the unknown key itself: the
    /// fields serde says the struct takes.
    fn expected_fields(message: &str) -> Vec<String> {
        let mut names: Vec<String> = message
            .split('`')
            .skip(1)
            .step_by(2)
            .skip(1)
            .map(str::to_owned)
            .collect();
        names.sort();
        names
    }

    // Each variant in the schema must have a mirror that takes exactly its properties, so a new
    // variant or field in dto.rs fails here instead of silently losing the inner path.
    fn assert_mirrors(key: &str, schema: schemars::Schema) {
        let schema = serde_json::to_value(schema).expect("schema json");
        let variants = schema["oneOf"].as_array().expect("oneOf");
        assert!(!variants.is_empty());
        for variant in variants {
            let mode = variant["properties"]["type"]["const"]
                .as_str()
                .expect("type const");
            let mut want: Vec<String> = (variant["properties"].as_object())
                .expect("properties")
                .keys()
                .filter(|k| *k != "type")
                .cloned()
                .collect();
            want.sort();
            let doc = object(json!({"type": mode, "zz_unknown": 0}));
            let error =
                variant_error(key, &doc).unwrap_or_else(|| panic!("{key} `{mode}` has no mirror"));
            let message = error.inner().to_string();
            assert!(
                message.starts_with("unknown field `zz_unknown`"),
                "{mode}: {message}"
            );
            assert_eq!(expected_fields(&message), want, "{key} `{mode}`: {message}");
        }
    }

    #[test]
    fn every_schema_variant_has_a_mirror_with_exactly_its_fields() {
        assert_mirrors("auth", schemars::schema_for!(AuthDoc));
        assert_mirrors("token_placement", schemars::schema_for!(TokenPlacementDoc));
    }
}
