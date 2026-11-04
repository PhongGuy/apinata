// Writes keys.json, the keys and Node-signed tokens plain.js and forms.js read: every PEM beside
// this file, the sized RSA keys, throwaway keys of each type Node opens, and DER encodings.
// Run with Node 24 and jsonwebtoken@9.0.3 on NODE_PATH: node keys.js > keys.json
const jwt = require("jsonwebtoken");
const crypto = require("crypto");
const fs = require("fs");
const path = require("path");
const read = (...p) => fs.readFileSync(path.join(__dirname, ...p), "utf8");
const K = {
  RSA: read("rsa.pem"), RSA_PKCS1: read("rsa-pkcs1.pem"), RSA_PUB: read("rsa.pub.pem"), RSA_CRT: read("rsa.crt"),
  EC256: read("ec256-sec1.pem"), EC256_PUB: read("ec256.pub.pem"), EC384: read("ec384.pem"), EC384_PUB: read("ec384.pub.pem"),
  RSA512: read("../rsa-sizes/rsa512.pem"), RSA512_PUB: read("../rsa-sizes/rsa512.pub.pem"),
  RSA1024: read("../rsa-sizes/rsa1024.pem"), RSA1024_PKCS1: read("../rsa-sizes/rsa1024-pkcs1.pem"), RSA1024_PUB: read("../rsa-sizes/rsa1024.pub.pem"),
  // The RFC 8032 section 7.1 test 1 key as PKCS#8 and SPKI.
  ED: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----\n",
  ED_PUB: "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=\n-----END PUBLIC KEY-----\n",
};
const pair = (name, type, options) => {
  const { privateKey, publicKey } = crypto.generateKeyPairSync(type, options);
  K[name] = privateKey.export({ type: "pkcs8", format: "pem" });
  K[`${name}_PUB`] = publicKey.export({ type: "spki", format: "pem" });
};
pair("RSA2047", "rsa", { modulusLength: 2047 });
pair("EC521", "ec", { namedCurve: "secp521r1" });
pair("K256", "ec", { namedCurve: "secp256k1" });
pair("ED448", "ed448");
pair("X25519", "x25519");
pair("DSA", "dsa", { modulusLength: 2048, divisorLength: 256 });
pair("PSS", "rsa-pss", { modulusLength: 2048 });
pair("PSS_SHA256", "rsa-pss", { modulusLength: 2048, hashAlgorithm: "sha256", mgf1HashAlgorithm: "sha256", saltLength: 32 });
K.K256_SEC1 = crypto.createPrivateKey(K.K256).export({ type: "sec1", format: "pem" });
// An RSA PEM whose integers parse but whose private exponent no longer belongs to its modulus.
const rsaParts = crypto.createPrivateKey(K.RSA).export({ format: "jwk" });
K.RSA_INCONSISTENT = crypto.createPrivateKey({ key: { ...rsaParts, d: rsaParts.dp }, format: "jwk" }).export({ type: "pkcs1", format: "pem" });
K.RSA_ENC = crypto.createPrivateKey(K.RSA).export({ type: "pkcs8", format: "pem", cipher: "aes-256-cbc", passphrase: "correct horse" });
const der = (pem, type) => crypto.createPrivateKey(pem).export({ type, format: "der" }).toString("base64");
const pub = (pem, type) => crypto.createPublicKey(pem).export({ type, format: "der" }).toString("base64");
K.der = {
  rsa_pkcs8: der(K.RSA, "pkcs8"), rsa_pkcs1: der(K.RSA, "pkcs1"), ec_pkcs8: der(K.EC256, "pkcs8"), ec_sec1: der(K.EC256, "sec1"),
  ed_pkcs8: der(K.ED, "pkcs8"), rsa_spki: pub(K.RSA_PUB, "spki"), rsa_pub_pkcs1: pub(K.RSA_PUB, "pkcs1"), ec_spki: pub(K.EC256_PUB, "spki"),
  ed_spki: pub(K.ED_PUB, "spki"), rsa1024_pkcs8: der(K.RSA1024, "pkcs8"),
};
const claims = { sub: "x", iat: 1700000000 };
const sign = (key, algorithm, extra) => jwt.sign(claims, key, { algorithm, ...extra });
const insecure = { allowInsecureKeySizes: true };
K.tokens = {
  HS256: sign("s3cr3t-key", "HS256"), HS512: sign("s3cr3t-key", "HS512"),
  RS256: sign(K.RSA, "RS256"), RS384: sign(K.RSA, "RS384"), RS512: sign(K.RSA, "RS512"), PS256: sign(K.RSA, "PS256"), PS512: sign(K.RSA, "PS512"),
  ES256: sign(K.EC256, "ES256"), ES384: sign(K.EC384, "ES384"),
  RS256_1024: sign(K.RSA1024, "RS256", insecure), RS512_1024: sign(K.RSA1024, "RS512", insecure), PS256_1024: sign(K.RSA1024, "PS256", insecure),
  RS256_512: sign(K.RSA512, "RS256", insecure), RS256_2047: sign(K.RSA2047, "RS256", insecure),
  RS256_ec: sign(K.EC256, "RS256", { ...insecure, allowInvalidAsymmetricKeyTypes: true }),
};
console.log(JSON.stringify(K, null, 1));
