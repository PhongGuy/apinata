// Text with a PEM marker that does not open as a key must never become an HMAC secret, however it
// fails. Each entry says what Node did; the shim refuses by name where it will not guess.
const T = K.tokens;
const PW = "correct horse";
const labelled = (label, body) => `-----BEGIN ${label}-----\n${body}\n-----END ${label}-----\n`;
const BROKEN_PUB = labelled("PUBLIC KEY", "bm9wZQ==");
const BROKEN_ENC = labelled("ENCRYPTED PRIVATE KEY", "bm9wZQ==");
const TRUNCATED = K.RSA_PKCS1.slice(0, 300);
const forge = (key) => {
  const head = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url");
  const body = Buffer.from(JSON.stringify({ sub: "forged" })).toString("base64url");
  return `${head}.${body}.${crypto.createHmac("sha256", key).update(`${head}.${body}`).digest("base64url")}`;
};
const opened = (f, key) => { try { return f(key).asymmetricKeyType; } catch (_) { return "unreadable"; } };
const blankFirst = (p) => p.replace("-----\n", "-----\n\n");
const innerSpace = (p) => { const l = p.split("\n"); l[1] = `${l[1].slice(0, 10)} ${l[1].slice(10)}`; return l.join("\n"); };
const onMarker = (ch) => (p) => p.replace("-----\n", `-----${ch}\n`);
const junk = (n) => (p) => "x".repeat(n) + p;
const FORMS = { blank_first: blankFirst, inner_space: innerSpace, marker_vt: onMarker("\v"), marker_ctl: onMarker("\u0001"), marker_nul: onMarker("\0"), junk_254: junk(254), junk_508: junk(508) };
const CASES = {};
for (const [form, f] of Object.entries(FORMS)) {
  CASES[`${form} public`] = f(K.RSA_PUB);
  CASES[`${form} cert`] = f(K.RSA_CRT);
  CASES[`${form} private`] = f(K.RSA);
}
Object.assign(CASES, {
  cert_then_broken_public: K.RSA_CRT + BROKEN_PUB, broken_public_then_cert: BROKEN_PUB + K.RSA_CRT,
  legacy_ec_public_rsa: E.LEGACY_EC + K.EC256_PUB + K.RSA, legacy_rsa_public_ec: E.LEGACY_RSA + K.RSA_PUB + K.EC256, ec_legacy_public: K.EC256 + E.LEGACY_EC + K.ED_PUB, legacy_ec_then_rsa: E.LEGACY_EC + K.RSA, legacy_alone: E.LEGACY_RSA,
  broken_enc_then_rsa: BROKEN_ENC + K.RSA, truncated_then_pkcs1: `${TRUNCATED}\n${K.RSA_PKCS1}`, truncated_pkcs1_x25519: `${TRUNCATED}\n${K.RSA_PKCS1}${K.X25519}`, rsa_then_truncated: `${K.RSA}${TRUNCATED}\n`, unknown_then_rsa: labelled("FOO", "bm9wZQ==") + K.RSA,
  // Text no reader would call a key, which Node takes for a secret.
  marker_only: "-----BEGIN", marker_label: "-----BEGIN PRIVATE KEY-----", marker_in_text: "secret -----BEGIN and more", marker_end_only: "-----BEGIN X-----\n", glued: "-----BEGINFOO", public_text: K.RSA_PUB, certificate_text: K.RSA_CRT, encrypted_text: K.RSA_ENC, ssh_text: labelled("OPENSSH PRIVATE KEY", "b3BlbnNzaA=="),
});
for (const [name, key] of Object.entries(CASES)) {
  for (const [form, value] of [["string", key], ["bytes", Buffer.from(key)], ["uint8", new Uint8Array(Buffer.from(key))]]) {
    for (const alg of ["HS256", "RS256", "ES256"]) S(`closed ${name} ${form} ${alg}`, value, alg);
    V(`closed ${name} ${form} forged`, forge(key), value);
    V(`closed ${name} ${form} forged hs`, forge(key), value, { algorithms: ["HS256"] });
    V(`closed ${name} ${form} rs`, T.RS256, value);
    V(`closed ${name} ${form} es`, T.ES256, value, { algorithms: ["ES256"] });
  }
  console.log(`closed ${name} crypto: ${opened(crypto.createPrivateKey, key)} ${opened(crypto.createPublicKey, key)}`);
  S(`closed ${name} passphrase`, { key, passphrase: PW }, "RS256");
}
// Text that merely is not PEM stays a secret on both paths.
for (const key of ["a shared secret", "-----BEGI", "----BEGIN PRIVATE KEY-----", "-----begin private key-----"]) {
  S(`closed secret ${JSON.stringify(key)}`, key, "HS256");
  V(`closed secret ${JSON.stringify(key)} verify`, forge(key), key);
}
