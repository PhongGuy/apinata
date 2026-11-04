// Generates the JWT test vectors with jsonwebtoken@9.0.3 (npm install --save-exact jsonwebtoken@9.0.3)
// over the throwaway keys beside this file: node gen.js > vectors.json. Run from this directory.
const jwt = require("jsonwebtoken");
const fs = require("fs");
const k = (f) => fs.readFileSync(f, "utf8");
const P = { sub: "1234567890", name: "Jörg ✓", admin: true, iat: 1700000000 };
const out = {};
for (const alg of ["HS256", "HS384", "HS512"]) out[alg] = jwt.sign(P, "s3cr3t-key", { algorithm: alg });
out.HS256_buffer = jwt.sign(P, Buffer.from(Array.from({ length: 32 }, (_, i) => i)), { algorithm: "HS256" });
out.HS256_options = jwt.sign({ scope: "read", iat: 1700000000 }, "s3cr3t-key", {
  expiresIn: "1h", notBefore: 60, audience: ["a", "b"], issuer: "apinata", subject: "u1", jwtid: "j1", keyid: "k1", header: { x5t: "abc" },
});
out.HS256_noTimestamp = jwt.sign({ a: 1 }, "s3cr3t-key", { noTimestamp: true });
out.HS256_string = jwt.sign("hello", "s3cr3t-key");
for (const alg of ["RS256", "RS384", "RS512"]) out[alg] = jwt.sign(P, k("rsa.pem"), { algorithm: alg });
out.RS256_pkcs1 = jwt.sign(P, k("rsa-pkcs1.pem"), { algorithm: "RS256" });
for (const alg of ["PS256", "PS384", "PS512"]) out[alg] = jwt.sign(P, k("rsa.pem"), { algorithm: alg });
out.ES256 = jwt.sign(P, k("ec256-sec1.pem"), { algorithm: "ES256" });
out.ES384 = jwt.sign(P, k("ec384.pem"), { algorithm: "ES384" });
// Node-side checks that each asymmetric token verifies with the public key and the certificate.
for (const alg of ["RS256", "PS256"]) jwt.verify(out[alg], k("rsa.crt"));
jwt.verify(out.ES256, k("ec256.pub.pem")); jwt.verify(out.ES384, k("ec384.pub.pem"));
out.exp = {};
for (const e of ["1h", "7d", "2 days", "1.5h", "90s", "60", 120, "1y", "-1h", "100ms"]) {
  out.exp[JSON.stringify(e)] = jwt.decode(jwt.sign({ iat: 1700000000 }, "k", { expiresIn: e })).exp;
}
const err = (f) => { try { f(); return "no error"; } catch (e) { return [e.name, e.message, e.expiredAt || e.date || null]; } };
out.signErrors = {
  bad_timespan: err(() => jwt.sign({}, "k", { expiresIn: "soon" })),
  claim_on_string: err(() => jwt.sign("x", "k", { expiresIn: 60 })),
  hs_with_rsa: err(() => jwt.sign({}, k("rsa.pem"), { algorithm: "HS256" })),
  rs_with_secret: err(() => jwt.sign({}, "k", { algorithm: "RS256" })),
  no_key: err(() => jwt.sign({}, "")),
  bad_alg: err(() => jwt.sign({}, "k", { algorithm: "HS999" })),
  exp_conflict: err(() => jwt.sign({ exp: 1 }, "k", { expiresIn: 60 })),
};
const expired = jwt.sign({ iat: 1700000000, exp: 1700003600 }, "k");
const future = jwt.sign({ nbf: 4102444800 }, "k");
out.verifyErrors = {
  malformed: err(() => jwt.verify("abc", "k")),
  bad_signature: err(() => jwt.verify(out.HS256, "wrong")),
  expired: err(() => jwt.verify(expired, "k")),
  not_before: err(() => jwt.verify(future, "k")),
  audience: err(() => jwt.verify(out.HS256_options, "s3cr3t-key", { audience: "c", ignoreExpiration: true })),
  issuer: err(() => jwt.verify(out.HS256_options, "s3cr3t-key", { issuer: "x", ignoreExpiration: true })),
  algorithm: err(() => jwt.verify(out.HS256, "s3cr3t-key", { algorithms: ["HS512"] })),
  rs_with_secret: err(() => jwt.verify(out.RS256, "s3cr3t-key")),
};
out.expiredToken = expired; out.futureToken = future;
console.log(JSON.stringify(out, null, 1));
