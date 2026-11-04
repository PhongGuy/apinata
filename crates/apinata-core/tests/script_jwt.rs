//! jsonwebtoken in scripts: Node-identical tokens, verification, claims and errors.

use crate::script_support;

use apinata_core::Outcome;
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEYS: [(&str, &str); 8] = [
    ("RSA", include_str!("fixtures/jwt/rsa.pem")),
    ("RSA_PKCS1", include_str!("fixtures/jwt/rsa-pkcs1.pem")),
    ("RSA_PUB", include_str!("fixtures/jwt/rsa.pub.pem")),
    ("RSA_CRT", include_str!("fixtures/jwt/rsa.crt")),
    ("EC256", include_str!("fixtures/jwt/ec256-sec1.pem")),
    ("EC256_PUB", include_str!("fixtures/jwt/ec256.pub.pem")),
    ("EC384", include_str!("fixtures/jwt/ec384.pem")),
    ("EC384_PUB", include_str!("fixtures/jwt/ec384.pub.pem")),
];
const VECTORS: &str = include_str!("fixtures/jwt/vectors.json");
const SHOW: &str = "const show = (f) => { try { f(); return 'ok'; } catch (e) { return [e.name, e.message, e.expiredAt ? e.expiredAt.toISOString() : e.date ? e.date.toISOString() : null, e instanceof Error]; } };\nconst P = { sub: '1234567890', name: 'J\\u00f6rg \\u2713', admin: true, iat: 1700000000 };";

/// Runs `code` as a tests script with the keys, `V` (the Node vectors), `show` and `P` defined.
async fn run_jwt(code: &str) -> apinata_core::RequestResult {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let mut full: String = KEYS
        .iter()
        .map(|(n, k)| format!("const {n} = {};\n", serde_json::to_string(k).expect("json")))
        .collect();
    let vectors: serde_json::Value = serde_json::from_str(VECTORS).expect("vectors");
    full.push_str(&format!("const V = {vectors};\n{SHOW}\n{code}"));
    let text = scripted("R", &format!("{}/r", s.uri()), &script("tests", &full), "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", tests(&result));
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tokens_match_node_for_hs_and_rs() {
    let code = r#"console.log(typeof Object.getOwnPropertyDescriptor(globalThis, "jwt").get);
const out = [];
for (const alg of ["HS256", "HS384", "HS512"]) out.push(jwt.sign(P, "s3cr3t-key", { algorithm: alg }) === V[alg]);
out.push(jwt.sign(P, new Uint8Array(32).map((_, i) => i)) === V.HS256_buffer);
for (const alg of ["RS256", "RS384", "RS512"]) out.push(jwt.sign(P, RSA, { algorithm: alg }) === V[alg]);
out.push(jwt.sign(P, RSA_PKCS1, { algorithm: "RS256" }) === V.RS256_pkcs1);
out.push(jwt.sign({ scope: "read", iat: 1700000000 }, "s3cr3t-key", { expiresIn: "1h", notBefore: 60, audience: ["a", "b"], issuer: "apinata", subject: "u1", jwtid: "j1", keyid: "k1", header: { x5t: "abc" } }) === V.HS256_options);
out.push(jwt.sign({ a: 1 }, "s3cr3t-key", { noTimestamp: true }) === V.HS256_noTimestamp);
out.push(jwt.sign("hello", "s3cr3t-key") === V.HS256_string);
console.log(out.join(","));
console.log(require("jsonwebtoken") === jwt, typeof jwt.sign, typeof jwt.verify, typeof jwt.decode);
const exp = {};
for (const e of ["1h", "7d", "2 days", "1.5h", "90s", "60", 120, "1y", "-1h", "100ms"]) exp[JSON.stringify(e)] = jwt.decode(jwt.sign({ iat: 1700000000 }, "k", { expiresIn: e })).exp;
console.log(JSON.stringify(Object.keys(V.exp).filter((k) => exp[k] !== V.exp[k])));"#;
    let result = run_jwt(code).await;
    assert_eq!(
        console(&result),
        [
            "log:function",
            "log:true,true,true,true,true,true,true,true,true,true,true",
            "log:true function function function",
            "log:[]",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_accepts_node_tokens_for_every_family() {
    let code = r#"const keys = { HS256: "s3cr3t-key", RS256: RSA_PUB, RS384: RSA_CRT, RS512: RSA_PUB, PS256: RSA_CRT, PS384: RSA_PUB, PS512: RSA_PUB, ES256: EC256_PUB, ES384: EC384_PUB };
console.log(Object.entries(keys).map(([alg, key]) => jwt.verify(V[alg], key).admin).join(","));
const full = jwt.verify(jwt.sign({ a: 1 }, EC256, { algorithm: "ES256" }), EC256_PUB, { complete: true });
console.log(full.header.alg, full.payload.a, typeof full.signature, jwt.verify(jwt.sign({ a: 2 }, EC384, { algorithm: "ES384" }), EC384_PUB).a, jwt.verify(jwt.sign({ a: 3 }, RSA, { algorithm: "PS256" }), RSA_PUB).a);"#;
    let result = run_jwt(code).await;
    assert_eq!(
        console(&result),
        [
            "log:true,true,true,true,true,true,true,true,true",
            "log:ES256 1 string 2 3",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_errors_follow_node() {
    let code = r#"const rows = {
  malformed: show(() => jwt.verify("abc", "k")),
  bad_signature: show(() => jwt.verify(V.HS256, "wrong")),
  expired: show(() => jwt.verify(V.expiredToken, "k")),
  not_before: show(() => jwt.verify(V.futureToken, "k")),
  audience: show(() => jwt.verify(V.HS256_options, "s3cr3t-key", { audience: "c", ignoreExpiration: true })),
  issuer: show(() => jwt.verify(V.HS256_options, "s3cr3t-key", { issuer: "x", ignoreExpiration: true })),
  algorithm: show(() => jwt.verify(V.HS256, "s3cr3t-key", { algorithms: ["HS512"] })),
  rs_with_secret: show(() => jwt.verify(V.RS256, "s3cr3t-key")),
};
console.log(JSON.stringify(Object.keys(rows).filter((k) => JSON.stringify(rows[k].slice(0, 3)) !== JSON.stringify(V.verifyErrors[k]))));
console.log(rows.expired[3], rows.bad_signature[3]);
const near = jwt.sign({ exp: Math.floor(Date.now() / 1000) - 5 }, "k");
console.log(show(() => jwt.verify(near, "k"))[0], jwt.verify(near, "k", { clockTolerance: 60 }).exp > 0, jwt.verify(V.expiredToken, "k", { ignoreExpiration: true }).iat, show(() => jwt.verify(V.HS256_options, "s3cr3t-key", { subject: "u2", ignoreExpiration: true }))[1]);"#;
    let result = run_jwt(code).await;
    assert_eq!(
        console(&result),
        [
            "log:[]",
            "log:true true",
            "log:TokenExpiredError true 1700000000 jwt subject invalid. expected: u2",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_errors_follow_node_and_never_echo_the_key() {
    let code = r#"const rows = {
  bad_timespan: show(() => jwt.sign({}, "k", { expiresIn: "soon" })),
  claim_on_string: show(() => jwt.sign("x", "k", { expiresIn: 60 })),
  hs_with_rsa: show(() => jwt.sign({}, RSA, { algorithm: "HS256" })),
  rs_with_secret: show(() => jwt.sign({}, "k", { algorithm: "RS256" })),
  no_key: show(() => jwt.sign({}, "")),
  bad_alg: show(() => jwt.sign({}, "k", { algorithm: "HS999" })),
  exp_conflict: show(() => jwt.sign({ exp: 1 }, "k", { expiresIn: 60 })),
};
console.log(JSON.stringify(Object.keys(rows).filter((k) => JSON.stringify(rows[k].slice(0, 3)) !== JSON.stringify(V.signErrors[k]))));
console.log([show(() => jwt.sign({}, EC256, { algorithm: "ES512" })), show(() => jwt.sign({}, "k", { algorithm: "none" }))].map((r) => r[0] + ": " + r[1]).join(" | "));
console.log(show(() => jwt.sign({}, EC384, { algorithm: "ES256" }))[1], show(() => jwt.sign({}, RSA, { algorithm: "ES256" }))[1], show(() => jwt.sign({}, RSA_PUB, { algorithm: "RS256" }))[1]);
const leaks = [show(() => jwt.sign({}, "-----BEGIN PRIVATE KEY-----\nTOPSECRETKEYDATA\n-----END PRIVATE KEY-----", { algorithm: "RS256" })), show(() => jwt.verify(V.RS256, "-----BEGIN PUBLIC KEY-----\nTOPSECRETKEYDATA\n-----END PUBLIC KEY-----", { algorithms: ["RS256"] }))];
console.log(leaks.map((r) => r[1]).join(" | "), JSON.stringify(leaks).includes("TOPSECRET"));"#;
    let result = run_jwt(code).await;
    assert_eq!(
        console(&result),
        [
            "log:[]",
            "log:JsonWebTokenError: ES512 is not supported by Apinata | JsonWebTokenError: none is not supported by Apinata",
            r#"log:"alg" parameter "ES256" requires curve "prime256v1". "alg" parameter for "rsa" key type must be one of: RS256, PS256, RS384, PS384, RS512, PS512. Unsupported key: text with a PEM marker is never a secret, and this is no key that opens here (supported: RSA, ECDSA P-256, P-384 or P-521, and Ed25519)"#,
            "log:Unsupported key: text with a PEM marker is never a secret, and this is no key that opens here (supported: RSA, ECDSA P-256, P-384 or P-521, and Ed25519) | Unsupported key: text with a PEM marker is never a secret, and this is no key that opens here (supported: RSA, ECDSA P-256, P-384 or P-521, and Ed25519) false",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn callbacks_and_decode() {
    let code = r#"const r1 = jwt.sign({ a: 1 }, "s3cr3t-key", { noTimestamp: true }, (err, token) => console.log("sign", err, token === V.HS256_noTimestamp));
const r2 = jwt.verify(V.HS256, "wrong", (err, value) => console.log("verify", err.name, err.message, value));
const r3 = jwt.sign({ a: 1 }, "s3cr3t-key", (err, token) => console.log("sign2", err, typeof token));
console.log("returned", r1, r2, r3);
console.log(JSON.stringify(jwt.decode(V.HS256_options)), JSON.stringify(jwt.decode(V.HS256_string, { complete: true })), jwt.decode("abc"), jwt.decode("a.b.c"));"#;
    let result = run_jwt(code).await;
    assert_eq!(
        console(&result),
        [
            "log:verify JsonWebTokenError invalid signature undefined",
            "log:returned undefined undefined undefined",
            r#"log:{"scope":"read","iat":1700000000,"nbf":1700000060,"exp":1700003600,"aud":["a","b"],"iss":"apinata","sub":"u1","jti":"j1"} {"header":{"alg":"HS256"},"payload":"hello","signature":"T2IADWhys8eP1WJqU-a-aUbCJieLmzFMNWTTp1d9MLA"} null null"#,
            "log:sign null true",
            "log:sign2 null string",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_options_beyond_the_common_subset_follow_node() {
    let code = r#"const t = jwt.sign({ a: 1, iat: 1700000000, nonce: "n1" }, "k", { jwtid: "j1" });
const at = (clockTimestamp, extra) => Object.assign({ clockTimestamp }, extra);
const got = [];
jwt.verify(t, (h, cb) => cb(null, "k"), (err, p) => got.push(err, p.a));
jwt.verify(t, (h, cb) => cb(new Error("boom")), (err, p) => got.push(err.name, err.message, p));
const rows = [
  show(() => jwt.verify(t, "k", at(1700003600, { maxAge: "1h" }))),
  jwt.verify(t, "k", at(1700003599, { maxAge: "1h" })).a,
  show(() => jwt.verify(t, "k", { maxAge: "soon" }))[1],
  show(() => jwt.verify(t, "k", { nonce: "n2" }))[1],
  show(() => jwt.verify(t, "k", { nonce: " " }))[1],
  show(() => jwt.verify(t, "k", { jwtid: "j2" }))[1],
  show(() => jwt.verify(t, "k", { clockTimestamp: "x" }))[1],
  show(() => jwt.verify(t, (h, cb) => cb(null, "k")))[1],
  jwt.verify(t, "k", { nonce: "n1", jwtid: "j1" }).a,
  jwt.verify(V.futureToken, "k", { ignoreNotBefore: true }).nbf,
  show(() => jwt.verify(V.futureToken, "k"))[0],
];
console.log(JSON.stringify(rows));
console.log(JSON.stringify(got));"#;
    let result = run_jwt(code).await;
    assert_eq!(
        console(&result),
        [
            r#"log:[["TokenExpiredError","maxAge exceeded","2023-11-14T23:13:20.000Z",true],1,"\"maxAge\" should be a number of seconds or string representing a timespan eg: \"1d\", \"20h\", 60","jwt nonce invalid. expected: n2","nonce must be a non-empty string","jwt jwtid invalid. expected: j2","clockTimestamp must be a number","verify must be called asynchronous if secret or public key is provided as a callback",1,4102444800,"NotBeforeError"]"#,
            r#"log:[null,1,"JsonWebTokenError","error in secret or public key callback: boom",null]"#,
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_buffer_payload_signs_as_raw_bytes_like_node() {
    // Expected tokens and message from jsonwebtoken 9.0.3 on Node 24.16.0.
    let code = r#"console.log(jwt.sign(Buffer.from('{"a":1}'), "k"));
console.log(jwt.sign(Buffer.from("Jörg ✓"), "k"));
console.log(show(() => jwt.sign(Buffer.from("x"), "k", { expiresIn: 60 }))[1]);"#;
    let result = run_jwt(code).await;
    assert_eq!(
        console(&result),
        [
            "log:eyJhbGciOiJIUzI1NiJ9.eyJhIjoxfQ.OGHNXDgUnzgBVCe2z7nm7V9aIzY7ToZsw8H_3ZgS3L0",
            "log:eyJhbGciOiJIUzI1NiJ9.SsO2cmcg4pyT.N0GqjAc4c1pO8-Km10wpXp3lZWE8EUqRrKJ3wkUgLnY",
            "log:invalid expiresIn option for object payload",
        ]
    );
}
