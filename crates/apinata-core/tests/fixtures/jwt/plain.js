// Bare strings and bytes as keys. A PEM that opens is typed as its key; anything else is a secret.
const T = K.tokens;
const PRIVATE = { RSA: K.RSA, RSA_PKCS1: K.RSA_PKCS1, EC256: K.EC256, EC384: K.EC384, EC521: K.EC521, ED: K.ED };
for (const [name, key] of Object.entries(PRIVATE)) {
  for (const alg of ["HS256", "HS384", "HS512", "RS256", "RS512", "PS256", "PS512", "ES256", "ES384"]) {
    S(`sign ${name} ${alg}`, key, alg);
    S(`sign ${name} bytes ${alg}`, Buffer.from(key), alg);
  }
}
R("round RSA PS256", K.RSA, "PS256", K.RSA_PUB);
R("round EC256 ES256", K.EC256, "ES256", K.EC256_PUB);
R("round EC384 ES384", K.EC384, "ES384", K.EC384_PUB);
const KEYS = { RSA_PUB: K.RSA_PUB, RSA_CRT: K.RSA_CRT, RSA: K.RSA, RSA_PKCS1: K.RSA_PKCS1, EC256_PUB: K.EC256_PUB, EC256: K.EC256, EC384_PUB: K.EC384_PUB, EC384: K.EC384, EC521_PUB: K.EC521_PUB, EC521: K.EC521, ED_PUB: K.ED_PUB, ED: K.ED };
for (const [name, key] of Object.entries(KEYS)) {
  for (const alg of ["HS256", "RS256", "PS256", "ES256", "ES384"]) {
    V(`verify ${name} ${alg}`, T[alg], key);
    V(`verify ${name} ${alg} named`, T[alg], key, { algorithms: [alg] });
    V(`verify ${name} bytes ${alg}`, T[alg], Buffer.from(key));
  }
}
// Text that is not a key, however much it looks like one, is a secret on both paths.
const BROKEN = "-----BEGIN PRIVATE KEY-----\nbm9wZQ==\n-----END PRIVATE KEY-----\n";
const BROKEN_PUB = BROKEN.replaceAll("PRIVATE", "PUBLIC");
const TEXT = {
  plain: "a shared secret", spaces: "  spaced  ", unicode: "Jörg ✓", odd: "-----BEGIN -----END -----",
  broken: BROKEN, broken_pub: BROKEN_PUB, truncated: K.RSA.slice(0, 300), no_end: K.RSA.replace("-----END PRIVATE KEY-----", ""),
  bad_base64: "-----BEGIN PRIVATE KEY-----\n!!!!\n-----END PRIVATE KEY-----\n", unknown_label: K.RSA.replaceAll("PRIVATE KEY", "SECRET THING"),
  encrypted: K.RSA_ENC, ssh: "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaA==\n-----END OPENSSH PRIVATE KEY-----\n",
};
for (const [name, key] of Object.entries(TEXT)) {
  for (const alg of ["HS256", "RS256", "ES256"]) S(`secret ${name} ${alg}`, key, alg);
  S(`secret ${name} bytes HS256`, Buffer.from(key), "HS256");
  line(`secret ${name} verify`, () => jwt.verify(jwt.sign(P, key), key).sub);
  V(`secret ${name} verify RS256`, T.RS256, key, { algorithms: ["RS256"] });
  V(`secret ${name} verify wrong`, T.HS256, key);
}
// A public key cannot open as a private one, so signing reads it as a secret; verify types it.
for (const [name, key] of Object.entries({ RSA_PUB: K.RSA_PUB, RSA_CRT: K.RSA_CRT, EC256_PUB: K.EC256_PUB, ED_PUB: K.ED_PUB })) {
  for (const alg of ["HS256", "RS256", "PS256", "ES256"]) S(`public ${name} ${alg}`, key, alg);
}
S("secret uint8array", new Uint8Array([1, 2, 3]), "HS256");
S("secret arraybuffer", new Uint8Array([1, 2, 3]).buffer, "HS256");
S("secret dataview", new DataView(new Uint8Array([1, 2, 3]).buffer), "HS256");
S("secret empty bytes", Buffer.alloc(0), "HS256");
S("secret number", 123, "HS256");
S("secret true", true, "HS256");
S("secret function", () => 1, "HS256");
V("verify number", T.HS256, 123);
V("verify true", T.HS256, true);
// Keys Node opens and the shim cannot hold. Each must stop rather than become a secret.
const FOREIGN = { PSS: K.PSS, PSS_SHA256: K.PSS_SHA256, K256: K.K256, K256_SEC1: K.K256_SEC1, ED448: K.ED448, X25519: K.X25519, DSA: K.DSA, RSA_INCONSISTENT: K.RSA_INCONSISTENT };
for (const [name, key] of Object.entries(FOREIGN)) {
  for (const alg of ["HS256", "RS256", "PS256", "ES256"]) S(`foreign ${name} ${alg}`, key, alg);
  S(`foreign ${name} bytes HS256`, Buffer.from(key), "HS256");
  V(`foreign ${name} verify HS256`, T.HS256, key);
}
for (const [name, key] of Object.entries({ PSS_PUB: K.PSS_PUB, K256_PUB: K.K256_PUB, ED448_PUB: K.ED448_PUB, X25519_PUB: K.X25519_PUB, DSA_PUB: K.DSA_PUB })) {
  S(`foreign ${name} sign HS256`, key, "HS256");
  V(`foreign ${name} verify HS256`, T.HS256, key);
  V(`foreign ${name} verify RS256`, T.RS256, key);
}
S("sign callback", K.ED, "HS256");
jwt.sign(P, K.ED, { algorithm: "HS256" }, (e, t) => console.log(`callback: ${e && e.message} ${t}`));
