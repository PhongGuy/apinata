// RSA-PSS keys, run unchanged by Node (node gen.js pss.js > pss-node.txt) and by the shim
// (script_crypto_pss.rs). Each line is `tag => ok value` or `tag => ErrorName code | message`.
const show = (tag, f) => { try { console.log(tag, "=> ok", f()); } catch (e) { console.log(tag, "=>", e.name, e.code, "|", e.message); } };
const m = Buffer.from("msg"), hash = (b) => crypto.createHash("sha256").update(b).digest("hex").slice(0, 16);
const details = (k) => JSON.stringify(k.asymmetricKeyDetails, (_, v) => (typeof v === "bigint" ? `${v}n` : v));
const described = (k) => `${k.type} ${k.asymmetricKeyType} ${details(k)}`;
const der = (k) => k.export({ type: k.type === "private" ? "pkcs8" : "spki", format: "der" });
const NAMES = ["none", "s256", "s512", "mgf", "big", "dflt"];

// 1. Opening, details and export, in each form Node takes.
for (const n of [...NAMES, "s224"]) {
  const priv = P[n], pub = P[`${n}_pub`];
  show(`${n} private`, () => described(crypto.createPrivateKey(priv)));
  show(`${n} public`, () => described(crypto.createPublicKey(pub)));
  show(`${n} derived`, () => { const k = crypto.createPublicKey(priv); return `${described(k)} ${k.export({ type: "spki", format: "pem" }) === pub}`; });
  show(`${n} from object`, () => crypto.createPublicKey(crypto.createPrivateKey(priv)).export({ type: "spki", format: "pem" }) === pub);
  show(`${n} pkcs8 pem`, () => crypto.createPrivateKey(priv).export({ type: "pkcs8", format: "pem" }) === priv);
  show(`${n} pkcs8 der`, () => hash(der(crypto.createPrivateKey(priv))));
  show(`${n} spki der`, () => hash(der(crypto.createPublicKey(pub))));
  show(`${n} der in`, () => { const k = crypto.createPrivateKey({ key: der(crypto.createPrivateKey(priv)), format: "der", type: "pkcs8" }); return `${described(k)} ${k.export({ type: "pkcs8", format: "pem" }) === priv}`; });
  show(`${n} der in public`, () => { const k = crypto.createPublicKey({ key: der(crypto.createPublicKey(pub)), format: "der", type: "spki" }); return `${described(k)} ${k.export({ type: "spki", format: "pem" }) === pub}`; });
  show(`${n} der private to public`, () => described(crypto.createPublicKey({ key: der(crypto.createPrivateKey(priv)), format: "der", type: "pkcs8" })));
  show(`${n} jwk`, () => crypto.createPrivateKey(priv).export({ format: "jwk" }));
  show(`${n} jwk public`, () => crypto.createPublicKey(pub).export({ format: "jwk" }));
  show(`${n} pkcs1`, () => crypto.createPrivateKey(priv).export({ type: "pkcs1", format: "pem" }));
  show(`${n} pkcs1 public`, () => crypto.createPublicKey(pub).export({ type: "pkcs1", format: "der" }));
  show(`${n} same details`, () => { const k = crypto.createPublicKey(pub); return k.asymmetricKeyDetails === k.asymmetricKeyDetails; });
}

// 2. Encrypted PKCS#8: opened with the passphrase, refused without, and written again.
for (const [n, enc, plain] of [["s256", P.s256_enc, P.s256], ["none", K.pss_enc, K.pss]]) {
  show(`${n} encrypted`, () => { const k = crypto.createPrivateKey({ key: enc, passphrase: PW }); return `${described(k)} ${k.export({ type: "pkcs8", format: "pem" }) === plain}`; });
  show(`${n} encrypted public`, () => described(crypto.createPublicKey({ key: enc, passphrase: PW })));
  show(`${n} encrypted no passphrase`, () => crypto.createPrivateKey(enc));
  show(`${n} encrypted wrong passphrase`, () => crypto.createPrivateKey({ key: enc, passphrase: "wrong" }));
  show(`${n} re-encrypted`, () => { const again = crypto.createPrivateKey(plain).export({ type: "pkcs8", format: "pem", cipher: "aes-128-cbc", passphrase: "x" }); return crypto.createPrivateKey({ key: again, passphrase: "x" }).export({ type: "pkcs8", format: "pem" }) === plain; });
}
show("s256 encrypted signs", () => crypto.verify("sha256", m, P.s256_pub, crypto.sign("sha256", m, { key: P.s256_enc, passphrase: PW })));
show("s256 encrypted createSign", () => crypto.createVerify("sha256").update(m).verify(P.s256_pub, crypto.createSign("sha256").update(m).sign({ key: P.s256_enc, passphrase: PW, saltLength: 33 })));

// 3. Signing: the digest, padding and salt a key allows, and the salt each signature carries,
// read by exact verification under the key's plain RSA view, or under the key itself where that
// view cannot check it (the shim's plain RSA keys take no sha1 PSS, and none takes another MGF1).
const DIGESTS = [null, "sha1", "sha256", "sha512"];
const SALTS = [0, 5, 10, 20, 32, 33, 40, 64, 190, 206, 222, 234];
const SIGN_OPTIONS = [{}, { padding: 6 }, { padding: 1 }, { saltLength: -1 }, { saltLength: -2 }, { saltLength: -3 }, { saltLength: -4 }, { saltLength: -5 }, { saltLength: 0 }, { saltLength: 10 }, { saltLength: 33 }, { saltLength: 200 }, { padding: 6, saltLength: 32 }, { padding: 1, saltLength: 32 }];
const plainDigest = { s512: "sha512", dflt: "sha1" };
for (const n of NAMES) {
  for (const digest of DIGESTS) {
    for (const options of SIGN_OPTIONS) {
      show(`${n} sign ${digest} ${JSON.stringify(options)}`, () => {
        const s = crypto.sign(digest, m, { key: P[n], ...options }), d = digest ?? plainDigest[n] ?? "sha256";
        const view = n === "mgf" || d === "sha1" ? P[`${n}_pub`] : P[`${n}_rsa_pub`];
        const exact = (saltLength) => { try { return crypto.verify(d, m, { key: view, padding: 6, saltLength }, s); } catch (_) { return false; } };
        const salt = SALTS.find(exact);
        return `${s.length} salt ${salt} ${crypto.verify(digest, m, { key: P[`${n}_pub`], saltLength: -2 }, s)}`;
      });
    }
  }
}

// 4. Verifying Node's signatures: each salt against the options and digests a key allows.
const VERIFY_OPTIONS = [{}, { padding: 6 }, { padding: 1 }, { saltLength: -1 }, { saltLength: -2 }, { saltLength: -3 }, { saltLength: -4 }, { saltLength: -5 }, { saltLength: 0 }, { saltLength: 5 }, { saltLength: 20 }, { saltLength: 32 }, { saltLength: 33 }, { saltLength: 300 }, { padding: 6, saltLength: 32 }];
for (const n of NAMES) {
  for (const [salt, b64] of Object.entries(P.sigs[n])) {
    const s = Buffer.from(b64, "base64");
    for (const digest of DIGESTS) {
      show(`${n} verify ${salt} ${digest}`, () => VERIFY_OPTIONS.map((o) => { try { return crypto.verify(digest, m, { key: P[`${n}_pub`], ...o }, s); } catch (e) { return e.message.split("::").pop(); } }).join(","));
    }
  }
}
const sig = (n) => Buffer.from(P.sigs[n].default, "base64");
show("verify private pem", () => crypto.verify("sha256", m, P.s256, sig("s256")));
show("verify private object", () => crypto.verify("sha256", m, crypto.createPrivateKey(P.s256), sig("s256")));
show("verify public object", () => crypto.verify("sha256", m, crypto.createPublicKey(P.s256_pub), sig("s256")));
show("verify tampered", () => crypto.verify("sha256", Buffer.from("msh"), P.s256_pub, sig("s256")));
show("verify other key", () => crypto.verify("sha256", m, P.big_pub, sig("s256")));
show("verify short", () => crypto.verify("sha256", m, P.s256_pub, sig("s256").subarray(1)));
show("verify long", () => crypto.verify("sha256", m, P.s256_pub, Buffer.concat([sig("s256"), Buffer.alloc(1)])));
show("verify zeros", () => crypto.verify("sha256", m, P.none_pub, Buffer.alloc(256)));
show("verify ones", () => crypto.verify("sha256", m, P.none_pub, Buffer.alloc(256, 255)));
show("verify plain view", () => crypto.verify("sha256", m, { key: P.s256_rsa_pub, padding: 6, saltLength: 32 }, crypto.sign("sha256", m, P.s256)));

// 5. createSign and createVerify, and the plain RSA view signing for the key to verify.
show("createSign default", () => crypto.createVerify("sha256").update(m).verify(P.s256_pub, crypto.createSign("sha256").update("m").update("sg").sign(P.s256)));
show("createSign RSA-SHA256", () => crypto.createVerify("RSA-SHA256").update(m).verify(P.s256_pub, crypto.createSign("RSA-SHA256").update(m).sign({ key: P.s256, saltLength: 40 }, "base64"), "base64"));
show("createSign object", () => crypto.createVerify("sha512").update(m).verify(crypto.createPublicKey(P.s512_pub), crypto.createSign("sha512").update(m).sign(crypto.createPrivateKey(P.s512))));
show("createSign wrong digest", () => crypto.createSign("sha384").update(m).sign(P.s256));
show("createSign pkcs1", () => crypto.createSign("sha256").update(m).sign({ key: P.none, padding: 1 }));
show("createSign too large", () => crypto.createSign("sha512").update(m).sign({ key: P.none, saltLength: 191 }));
show("createSign max", () => crypto.createSign("sha512").update(m).sign({ key: P.none, saltLength: 190 }).length);
show("createVerify wrong digest", () => crypto.createVerify("sha512").update(m).verify(P.s256_pub, sig("s256")));
show("createVerify small salt", () => crypto.createVerify("sha256").update(m).verify({ key: P.big_pub, saltLength: 39 }, sig("big")));
show("createVerify pkcs1", () => crypto.createVerify("sha256").update(m).verify({ key: P.none_pub, padding: 1 }, sig("none")));
show("createVerify invalid salt", () => crypto.createVerify("sha256").update(m).verify({ key: P.s256_pub, saltLength: -5 }, sig("s256")));
show("createVerify mgf", () => crypto.createVerify("sha256").update(m).verify(P.mgf_pub, sig("mgf")));
show("createVerify mgf as plain", () => crypto.createVerify("sha256").update(m).verify({ key: P.mgf_rsa_pub, padding: 6, saltLength: -2 }, sig("mgf")));
show("dsaEncoding", () => crypto.verify("sha256", m, P.s256_pub, crypto.sign("sha256", m, { key: P.s256, dsaEncoding: "ieee-p1363" })));
