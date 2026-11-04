// Key forms past a bare string: KeyObjects and `{ key, format, type, encoding }`, the 2048-bit
// rule, and what a key of the wrong family does once the option lets it through.
const T = K.tokens, D = K.der;
const der = (name) => Buffer.from(D[name], "base64");
const O = {
  rsa: crypto.createPrivateKey(K.RSA), ec: crypto.createPrivateKey(K.EC256), ed: crypto.createPrivateKey(K.ED),
  rsa_pub: crypto.createPublicKey(K.RSA_PUB), ec_pub: crypto.createPublicKey(K.EC256_PUB), ed_pub: crypto.createPublicKey(K.ED_PUB),
  derived: crypto.createPublicKey(K.RSA), secret: crypto.createSecretKey(Buffer.from("s3cr3t-key")), secret_text: crypto.createSecretKey("s3cr3t-key", "utf8"),
};
for (const [name, key] of Object.entries(O)) {
  for (const alg of ["HS256", "RS256", "PS256", "ES256"]) {
    S(`sign keyobject ${name} ${alg}`, key, alg);
    V(`verify keyobject ${name} ${alg}`, T[alg], key);
    V(`verify keyobject ${name} ${alg} named`, T[alg], key, { algorithms: [alg] });
  }
}
R("round keyobject rsa PS256", O.rsa, "PS256", O.rsa_pub);
R("round keyobject ec ES256", O.ec, "ES256", O.ec_pub);
const hex = (s) => Buffer.from(s).toString("hex");
const b64 = (s) => Buffer.from(s).toString("base64");
const jwk = (pem) => crypto.createPrivateKey(pem).export({ format: "jwk" });
const FORMS = {
  key_only: [{ key: K.RSA }, "RS256"],
  key_bytes: [{ key: Buffer.from(K.RSA) }, "RS256"],
  key_uint8: [{ key: new Uint8Array(Buffer.from(K.RSA)) }, "RS256"],
  pem: [{ key: K.RSA, format: "pem" }, "RS256"],
  pem_type: [{ key: K.RSA, format: "pem", type: "pkcs8" }, "RS256"],
  pem_pkcs1: [{ key: K.RSA_PKCS1, format: "pem", type: "pkcs1" }, "RS256"],
  pem_ec: [{ key: K.EC256, format: "pem" }, "ES256"],
  pem_ed: [{ key: K.ED }, "HS256"],
  der_pkcs8: [{ key: der("rsa_pkcs8"), format: "der", type: "pkcs8" }, "RS256"],
  der_pkcs1: [{ key: der("rsa_pkcs1"), format: "der", type: "pkcs1" }, "RS256"],
  der_ec: [{ key: der("ec_pkcs8"), format: "der", type: "pkcs8" }, "ES256"],
  der_sec1: [{ key: der("ec_sec1"), format: "der", type: "sec1" }, "ES256"],
  der_ed: [{ key: der("ed_pkcs8"), format: "der", type: "pkcs8" }, "RS256"],
  der_hex: [{ key: D.rsa_pkcs8, encoding: "base64", format: "der", type: "pkcs8" }, "RS256"],
  hex: [{ key: hex(K.RSA), encoding: "hex" }, "RS256"],
  base64: [{ key: b64(K.RSA), encoding: "base64" }, "RS256"],
  utf8: [{ key: K.RSA, encoding: "utf8" }, "RS256"],
  buffer_encoding: [{ key: K.RSA, encoding: "buffer" }, "RS256"],
  jwk: [{ key: jwk(K.RSA), format: "jwk" }, "RS256"],
  jwk_ec: [{ key: jwk(K.EC256), format: "jwk" }, "ES256"],
  der_no_type: [{ key: der("rsa_pkcs8"), format: "der" }, "RS256"],
  der_bad_type: [{ key: der("rsa_pkcs8"), format: "der", type: "spki" }, "RS256"],
  der_wrong_type: [{ key: der("rsa_pkcs8"), format: "der", type: "pkcs1" }, "RS256"],
  bad_format: [{ key: K.RSA, format: "bad" }, "RS256"],
  bad_encoding: [{ key: K.RSA, encoding: "bad" }, "RS256"],
  jwk_text: [{ key: K.RSA, format: "jwk" }, "RS256"],
  secret_text: [{ key: "a shared secret" }, "HS256"],
  secret_bytes: [{ key: Buffer.from("a shared secret") }, "HS256"],
  broken_pem: [{ key: "-----BEGIN PRIVATE KEY-----\nbm9wZQ==\n-----END PRIVATE KEY-----\n" }, "HS256"],
  public_pem: [{ key: K.RSA_PUB }, "RS256"],
  public_pem_hs: [{ key: K.RSA_PUB }, "HS256"],
  keyobject: [{ key: O.rsa }, "RS256"],
  public_keyobject: [{ key: O.rsa_pub }, "RS256"],
  no_key: [{}, "HS256"],
  undefined_key: [{ key: undefined }, "HS256"],
  null_key: [{ key: null }, "HS256"],
  array: [[], "HS256"],
  date: [new Date(0), "HS256"],
};
for (const [name, [key, alg]] of Object.entries(FORMS)) {
  S(`sign form ${name}`, key, alg);
  S(`sign form ${name} hs`, key, "HS256");
}
const VERIFY = {
  key_only: [{ key: K.RSA_PUB }, "RS256"],
  key_bytes: [{ key: Buffer.from(K.RSA_PUB) }, "RS256"],
  private_pem: [{ key: K.RSA }, "RS256"],
  pem: [{ key: K.RSA_PUB, format: "pem" }, "RS256"],
  pem_type: [{ key: K.RSA_PUB, format: "pem", type: "spki" }, "RS256"],
  certificate: [{ key: K.RSA_CRT }, "RS256"],
  der_spki: [{ key: der("rsa_spki"), format: "der", type: "spki" }, "RS256"],
  der_pkcs1: [{ key: der("rsa_pub_pkcs1"), format: "der", type: "pkcs1" }, "PS256"],
  der_private: [{ key: der("rsa_pkcs8"), format: "der", type: "pkcs8" }, "RS256"],
  der_ec: [{ key: der("ec_spki"), format: "der", type: "spki" }, "ES256"],
  der_ed: [{ key: der("ed_spki"), format: "der", type: "spki" }, "RS256"],
  hex: [{ key: hex(K.RSA_PUB), encoding: "hex" }, "RS256"],
  jwk: [{ key: O.rsa_pub.export({ format: "jwk" }), format: "jwk" }, "RS256"],
  jwk_private: [{ key: jwk(K.EC256), format: "jwk" }, "ES256"],
  keyobject_private: [{ key: O.rsa }, "RS256"],
  keyobject_public: [{ key: O.rsa_pub }, "RS256"],
  keyobject_secret: [{ key: O.secret }, "HS256"],
  der_no_type: [{ key: der("rsa_spki"), format: "der" }, "RS256"],
  bad_format: [{ key: K.RSA_PUB, format: "bad" }, "RS256"],
  secret_text: [{ key: "s3cr3t-key" }, "HS256"],
  broken_pem: [{ key: "-----BEGIN PUBLIC KEY-----\nbm9wZQ==\n-----END PUBLIC KEY-----\n" }, "RS256"],
  no_key: [{}, "HS256"],
};
for (const [name, [key, alg]] of Object.entries(VERIFY)) {
  V(`verify form ${name}`, T[alg], key);
  V(`verify form ${name} named`, T[alg], key, { algorithms: [alg] });
  V(`verify form ${name} hs`, T.HS256, key);
}
// OpenSSL scans past blocks it was not asked for, so a key in a bundle opens as a key.
const BUNDLES = { cert_key: K.RSA_CRT + K.RSA, public_key: K.RSA_PUB + K.RSA, key_cert: K.RSA + K.RSA_CRT, note_key: `issued 2026\n${K.RSA}` };
for (const [name, key] of Object.entries(BUNDLES)) {
  for (const alg of ["HS256", "RS256"]) {
    S(`bundle ${name} ${alg}`, key, alg);
    V(`bundle ${name} verify ${alg}`, T[alg], key);
  }
}
// The 2048-bit rule, on every size and form that reaches it.
const SIZED = {
  512: K.RSA512, 1024: K.RSA1024, "1024 pkcs1": K.RSA1024_PKCS1, "1024 keyobject": crypto.createPrivateKey(K.RSA1024), "1024 object": { key: K.RSA1024 },
  "1024 der": { key: Buffer.from(D.rsa1024_pkcs8, "base64"), format: "der", type: "pkcs8" }, 2047: K.RSA2047, 2048: K.RSA,
};
for (const [size, key] of Object.entries(SIZED)) {
  for (const alg of ["RS256", "RS384", "RS512", "PS256", "PS384", "PS512"]) {
    S(`size ${size} ${alg}`, key, alg);
    S(`size ${size} ${alg} false`, key, alg, { allowInsecureKeySizes: false });
    S(`size ${size} ${alg} allowed`, key, alg, { allowInsecureKeySizes: true });
  }
  S(`size ${size} HS256`, key, "HS256", { allowInsecureKeySizes: true });
  S(`size ${size} ES256`, key, "ES256");
}
R("round size 1024 PS256", K.RSA1024, "PS256", K.RSA1024_PUB, { allowInsecureKeySizes: true });
R("round size 2047 PS512", K.RSA2047, "PS512", K.RSA2047_PUB, { allowInsecureKeySizes: true });
S("size option number", K.RSA1024, "RS256", { allowInsecureKeySizes: 1 });
S("size option text", K.RSA, "RS256", { allowInsecureKeySizes: "yes" });
S("size option on an EC key", K.EC256, "ES256", { allowInsecureKeySizes: true });
line("size before payload", () => jwt.sign(undefined, K.RSA1024, { algorithm: "RS256" }));
line("size before options", () => jwt.sign({}, K.RSA1024, { algorithm: "RS256", unknown: 1 }));
jwt.sign(P, K.RSA1024, { algorithm: "RS256" }, (e, t) => console.log(`size callback: ${e && e.message} ${t}`));
const VERIFY_SIZES = [
  ["RS256_1024", K.RSA1024_PUB], ["RS512_1024", K.RSA1024_PUB], ["PS256_1024", K.RSA1024_PUB], ["RS256_2047", K.RSA2047_PUB], ["RS256_512", K.RSA512_PUB],
  ["RS256_1024", K.RSA1024], ["RS256_1024", crypto.createPublicKey(K.RSA1024_PUB)], ["RS256_1024", K.RSA512_PUB], ["RS256_1024", K.RSA_PUB],
];
VERIFY_SIZES.forEach(([token, key], i) => {
  const alg = token.slice(0, 5);
  V(`size verify ${i} ${token}`, T[token], key);
  V(`size verify ${i} ${token} named`, T[token], key, { algorithms: [alg] });
});
// The option skips the type check, so the crypto layer answers for the wrong family.
const LOOSE = { allowInvalidAsymmetricKeyTypes: true };
const BOTH = { ...LOOSE, allowInsecureKeySizes: true };
for (const [name, key] of Object.entries({ RSA: K.RSA, EC256: K.EC256, EC384: K.EC384, EC521: K.EC521, ED: K.ED })) {
  for (const alg of ["RS256", "RS512", "PS256", "ES256", "ES384"]) {
    line(`loose ${name} ${alg}`, () => typeof jwt.sign(P, key, { algorithm: alg, ...LOOSE }));
    line(`loose ${name} ${alg} sized`, () => typeof jwt.sign(P, key, { algorithm: alg, ...BOTH }));
  }
}
const long = { ...P, pad: "x".repeat(300) };
line("loose long payload", () => typeof jwt.sign(long, K.EC256, { algorithm: "RS256", ...LOOSE }));
line("loose round", () => { const t = jwt.sign(P, K.EC256, { algorithm: "RS256", ...BOTH }); return jwt.verify(t, K.EC256_PUB, { algorithms: ["RS256"], ...LOOSE }).sub; });
V("loose node token", T.RS256_ec, K.EC256_PUB, { algorithms: ["RS256"], ...LOOSE });
for (const [name, key] of Object.entries({ RSA_PUB: K.RSA_PUB, EC256_PUB: K.EC256_PUB, ED_PUB: K.ED_PUB })) {
  for (const alg of ["RS256", "PS256", "ES256", "ES384"]) {
    V(`loose verify ${name} ${alg}`, T[alg], key, { algorithms: [alg], ...LOOSE });
  }
}
