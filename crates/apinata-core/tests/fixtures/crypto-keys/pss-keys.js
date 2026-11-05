// Writes pss-keys.json, the RSA-PSS keys and Node-made signatures pss.js reads:
// node pss-keys.js > pss-keys.json. Run from this directory with Node 24.
const crypto = require("crypto");
const PW = "correct horse", m = Buffer.from("msg");
const P = { sigs: {} };
// The PKCS#1 key inside an RSA-PSS PKCS#8, opened as a plain RSA key: it signs any salt.
const rsaView = (pem) => {
  const d = crypto.createPrivateKey(pem).export({ type: "pkcs8", format: "der" });
  const at = (i) => { let len = d[i + 1], head = 2; if (len & 0x80) { const n = len & 0x7f; len = 0; for (let j = 0; j < n; j++) len = len * 256 + d[i + 2 + j]; head += n; } return [i + head, len]; };
  const [body] = at(0), [alg, algLen] = at(body + 3), [key, keyLen] = at(alg + algLen);
  return crypto.createPrivateKey({ key: d.subarray(key, key + keyLen), format: "der", type: "pkcs1" });
};
const KEYS = {
  none: {},
  s256: { hashAlgorithm: "sha256", mgf1HashAlgorithm: "sha256", saltLength: 32 },
  s512: { hashAlgorithm: "sha512", saltLength: 20 },
  mgf: { hashAlgorithm: "sha256", mgf1HashAlgorithm: "sha1", saltLength: 10 },
  big: { hashAlgorithm: "sha256", saltLength: 40 },
  dflt: { hashAlgorithm: "sha1" },
  s224: { hashAlgorithm: "sha224" },
};
for (const [name, options] of Object.entries(KEYS)) {
  const { privateKey, publicKey } = crypto.generateKeyPairSync("rsa-pss", { modulusLength: 2048, ...options });
  P[name] = privateKey.export({ type: "pkcs8", format: "pem" });
  P[`${name}_pub`] = publicKey.export({ type: "spki", format: "pem" });
  const rsa = rsaView(P[name]);
  P[`${name}_rsa_pub`] = crypto.createPublicKey(rsa).export({ type: "spki", format: "pem" });
  if (name === "s224") continue;
  const digest = options.hashAlgorithm ?? "sha256", mgf = options.mgf1HashAlgorithm ?? digest;
  P.sigs[name] = { default: crypto.sign(digest, m, privateKey).toString("base64") };
  // Salts the key's own parameters refuse to sign, made through the plain RSA view where it can.
  for (const salt of [0, 5, 32, 33, 64, 190]) {
    const sign = () => (mgf === digest ? crypto.sign(digest, m, { key: rsa, padding: 6, saltLength: salt }) : crypto.sign(digest, m, { key: privateKey, saltLength: salt }));
    try { P.sigs[name][salt] = sign().toString("base64"); } catch (_) {}
  }
}
P.s256_enc = crypto.createPrivateKey(P.s256).export({ type: "pkcs8", format: "pem", cipher: "aes-256-cbc", passphrase: PW });
console.log(JSON.stringify(P, null, 1));
