//! JWT auth read from and written to a request file.

use apinata_core::{
    AuthSpec, JwtAlgorithm, JwtPlacement, JwtSpec, OAuth1KeyKind, OAuth1PrivateKey, RequestFile,
};
use std::fs;
use tempfile::TempDir;

const HEAD: &str =
    "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://x.test/\n";

const FULL: &str = "  auth:
    type: jwt
    algorithm: RS256
    secret: \"\"
    secretBase64: false
    privateKey:
      type: text
      value: \"{{jwt_key}}\"
    headers: '{\"kid\":\"k1\"}'
    payload: '{\"sub\":\"{{user}}\"}'
    expiresIn: 300
    placement: header
    headerPrefix: Bearer
    queryParamKey: token
";

fn jwt(auth: AuthSpec) -> JwtSpec {
    match auth {
        AuthSpec::Jwt(spec) => spec,
        other => panic!("not jwt: {other:?}"),
    }
}

fn file(auth: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("r.yml");
    fs::write(&path, format!("{HEAD}{auth}")).unwrap();
    (dir, path)
}

#[test]
fn every_key_reads_into_the_spec() {
    let (_d, path) = file(FULL);
    let spec = jwt(RequestFile::load(&path).unwrap().auth());
    assert_eq!(spec.algorithm, JwtAlgorithm::Rs256);
    assert_eq!(spec.secret, "");
    assert!(!spec.secret_base64);
    assert_eq!(
        spec.private_key,
        Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, "{{jwt_key}}"))
    );
    assert_eq!(spec.headers, "{\"kid\":\"k1\"}");
    assert_eq!(spec.payload, "{\"sub\":\"{{user}}\"}");
    assert_eq!(spec.expires_in, "300");
    assert_eq!(spec.placement, JwtPlacement::Header);
    assert_eq!(spec.header_prefix.as_deref(), Some("Bearer"));
    assert_eq!(spec.query_param_key, "token");
}

#[test]
fn an_unchanged_spec_leaves_the_file_byte_for_byte() {
    let (_d, path) = file(FULL);
    let mut r = RequestFile::load(&path).unwrap();
    r.set_auth(r.auth());
    r.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), format!("{HEAD}{FULL}"));
}

#[test]
fn a_new_spec_writes_keys_in_the_shape_order() {
    let (_d, path) = file("");
    let mut r = RequestFile::load(&path).unwrap();
    let mut spec = JwtSpec::default();
    spec.secret = "{{jwt_secret}}".into();
    spec.payload = "{\"sub\":\"1\"}".into();
    spec.expires_in = "60".into();
    spec.placement = JwtPlacement::Query;
    r.set_auth(AuthSpec::Jwt(spec.clone()));
    r.save().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let want = "  auth:
    type: jwt
    algorithm: HS256
    secret: \"{{jwt_secret}}\"
    secretBase64: false
    privateKey:
      type: text
      value: \"\"
    headers: \"\"
    payload: '{\"sub\":\"1\"}'
    expiresIn: 60
    placement: query
    headerPrefix: Bearer
    queryParamKey: token
";
    // The writer spaces top-level blocks apart once it rewrites the file.
    assert!(
        text.ends_with(&format!("url: https://x.test/\n{want}")),
        "{text}"
    );
    assert_eq!(jwt(RequestFile::load(&path).unwrap().auth()), spec);
}

#[test]
fn unknown_names_survive_and_an_empty_prefix_stays_empty() {
    let auth = "  auth:\n    type: jwt\n    algorithm: XS999\n    placement: cookie\n    headerPrefix: \"\"\n";
    let (_d, path) = file(auth);
    let spec = jwt(RequestFile::load(&path).unwrap().auth());
    assert_eq!(spec.algorithm, JwtAlgorithm::Other("XS999".into()));
    assert_eq!(spec.algorithm.as_str(), "XS999");
    assert_eq!(spec.placement, JwtPlacement::Other("cookie".into()));
    assert_eq!(spec.header_prefix.as_deref(), Some(""));
    let sparse = jwt(RequestFile::load(&file("  auth:\n    type: jwt\n").1)
        .unwrap()
        .auth());
    assert_eq!(sparse.algorithm, JwtAlgorithm::Hs256);
    assert_eq!(sparse.header_prefix, None);
    assert_eq!(sparse.private_key, None);
}

#[test]
fn algorithms_parse_and_name_their_family() {
    for name in [
        "HS256", "HS384", "HS512", "RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256",
        "ES384", "ES512",
    ] {
        let alg = JwtAlgorithm::parse(name);
        assert_eq!(alg.as_str(), name);
        assert!(!matches!(alg, JwtAlgorithm::Other(_)), "{name}");
        assert_eq!(alg.uses_secret(), name.starts_with("HS"), "{name}");
    }
}

#[test]
fn debug_hides_the_secret_and_inline_key() {
    let mut spec = JwtSpec::default();
    spec.secret = "hunter2-secret".into();
    spec.private_key = Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, "PEM-TEXT"));
    let shown = format!("{:?}", AuthSpec::Jwt(spec));
    assert!(!shown.contains("hunter2-secret"), "{shown}");
    assert!(!shown.contains("PEM-TEXT"), "{shown}");
}
