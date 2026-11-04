// Writes edge.json for edges.js: keys only the openssl CLI writes, keys Node keeps and no crate
// reads, and tokens with changed signatures. Run from an empty directory with Node 24, openssl and
// jsonwebtoken@9.0.3 on NODE_PATH: node edge-keys.js > edge.json
const { execFileSync } = require("child_process");
const crypto = require("crypto");
const fs = require("fs");
const path = require("path");
const jwt = require("jsonwebtoken");
const K = JSON.parse(fs.readFileSync(path.join(__dirname, "keys.json"), "utf8"));
const traditional = (pem) => {
  fs.writeFileSync("in.pem", pem);
  return execFileSync("openssl", ["pkey", "-in", "in.pem", "-traditional"], { encoding: "utf8" });
};
const E = {};
E.DSA_TRADITIONAL = traditional(K.DSA);
fs.writeFileSync("ec.pem", K.EC256);
E.EC_CERT = execFileSync("openssl", ["req", "-x509", "-new", "-key", "ec.pem", "-subj", "/CN=ec", "-days", "3650"], { encoding: "utf8" });
E.LEGACY_RSA = execFileSync("openssl", ["pkey", "-traditional", "-aes128", "-passout", "pass:correct horse"], { input: K.RSA, encoding: "utf8" });
E.LEGACY_EC = execFileSync("openssl", ["ec", "-aes128", "-passout", "pass:correct horse"], { input: K.EC256, encoding: "utf8", stdio: ["pipe", "pipe", "ignore"] });
fs.writeFileSync("three.pem", execFileSync("openssl", ["genrsa", "-primes", "3", "2048"], { stdio: ["ignore", "pipe", "ignore"] }));
E.RSA_THREE_PRIME = traditional(fs.readFileSync("three.pem", "utf8"));
// An EC key whose stored point belongs to another key, and one whose scalar is zero.
const own = crypto.createPrivateKey(K.EC256).export({ format: "jwk" });
const other = crypto.generateKeyPairSync("ec", { namedCurve: "P-256" }).publicKey.export({ format: "jwk" });
const mismatched = crypto.createPrivateKey({ key: { ...own, x: other.x, y: other.y }, format: "jwk" });
E.EC_MISMATCH_SEC1 = mismatched.export({ type: "sec1", format: "pem" });
E.EC_MISMATCH_PKCS8 = mismatched.export({ type: "pkcs8", format: "pem" });
const lines = (der) => der.toString("base64").match(/.{1,64}/g).join("\n");
const zero = Buffer.from(`30310201010420${"00".repeat(32)}a00a06082a8648ce3d030107`, "hex");
E.EC_ZERO = `-----BEGIN EC PRIVATE KEY-----\n${lines(zero)}\n-----END EC PRIVATE KEY-----\n`;

const b64u = (b) => Buffer.from(b).toString("base64url");
const toBig = (b) => BigInt(`0x${Buffer.from(b).toString("hex") || "0"}`);
const toBytes = (v, size) => Buffer.from(v.toString(16).padStart(size * 2, "0"), "hex");
const withSig = (t, sig) => `${t.split(".").slice(0, 2).join(".")}.${b64u(sig)}`;
const claims = { sub: "x", iat: 1700000000 };
const insecure = { allowInsecureKeySizes: true };
const tampered = {};
const common = (name, t, sig) => {
  tampered[`${name} intact`] = t;
  tampered[`${name} empty last bit`] = t.slice(0, -1) + "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"[(("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_".indexOf(t.at(-1)) ^ 1))];
  tampered[`${name} flipped`] = withSig(t, Buffer.concat([sig.subarray(0, 8), Buffer.from([sig[8] ^ 1]), sig.subarray(9)]));
  tampered[`${name} short`] = withSig(t, sig.subarray(0, sig.length - 1));
  tampered[`${name} leading zero`] = withSig(t, Buffer.concat([Buffer.from([0]), sig]));
  tampered[`${name} appended`] = withSig(t, Buffer.concat([sig, Buffer.from([0])]));
  tampered[`${name} zero`] = withSig(t, Buffer.alloc(sig.length));
};
// RSA: find a signature with room for the modulus in the same length, then add it.
const rsa = (name, key, pub, algorithm) => {
  const n = toBig(Buffer.from(crypto.createPublicKey(pub).export({ format: "jwk" }).n, "base64url"));
  const size = (n.toString(16).length + 1) >> 1;
  for (let i = 0; ; i += 1) {
    const t = jwt.sign({ ...claims, jti: String(i) }, key, { algorithm, ...insecure });
    const sig = Buffer.from(t.split(".")[2], "base64url");
    const s = toBig(sig);
    if ((s + n).toString(16).length > size * 2) continue;
    common(name, t, sig);
    tampered[`${name} plus n`] = withSig(t, toBytes(s + n, size));
    tampered[`${name} n`] = withSig(t, toBytes(n, size));
    tampered[`${name} one`] = withSig(t, toBytes(1n, size));
    tampered[`${name} n minus one`] = withSig(t, toBytes(n - 1n, size));
    return;
  }
};
rsa("RS256", K.RSA, K.RSA_PUB, "RS256");
rsa("RS384", K.RSA, K.RSA_PUB, "RS384");
rsa("PS256", K.RSA, K.RSA_PUB, "PS256");
rsa("PS512", K.RSA, K.RSA_PUB, "PS512");
rsa("RS256_1024", K.RSA1024, K.RSA1024_PUB, "RS256");
rsa("RS384_1024", K.RSA1024, K.RSA1024_PUB, "RS384");
rsa("PS256_1024", K.RSA1024, K.RSA1024_PUB, "PS256");
rsa("RS256_2047", K.RSA2047, K.RSA2047_PUB, "RS256");
rsa("PS256_2047", K.RSA2047, K.RSA2047_PUB, "PS256");
// ECDSA: the order added or put in place of a half, and the negated s Node's verifier accepts.
const ORDERS = {
  ES256: [32, 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551n],
  ES384: [48, 0xffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973n],
};
for (const [algorithm, [size, order]] of Object.entries(ORDERS)) {
  const t = jwt.sign(claims, algorithm === "ES256" ? K.EC256 : K.EC384, { algorithm });
  const sig = Buffer.from(t.split(".")[2], "base64url");
  const r = sig.subarray(0, size), s = sig.subarray(size);
  const pair = (a, b) => Buffer.concat([a, b]);
  common(algorithm, t, sig);
  tampered[`${algorithm} negated s`] = withSig(t, pair(r, toBytes(order - toBig(s), size)));
  tampered[`${algorithm} r order`] = withSig(t, pair(toBytes(order, size), s));
  tampered[`${algorithm} s order`] = withSig(t, pair(r, toBytes(order, size)));
  tampered[`${algorithm} r zero`] = withSig(t, pair(Buffer.alloc(size), s));
  tampered[`${algorithm} s zero`] = withSig(t, pair(r, Buffer.alloc(size)));
  tampered[`${algorithm} all ones`] = withSig(t, Buffer.alloc(size * 2, 0xff));
}
E.tampered = tampered;
console.log(JSON.stringify(E, null, 1));
