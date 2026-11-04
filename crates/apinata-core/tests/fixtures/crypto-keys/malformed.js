// Malformed and hostile keys, run unchanged by Node (gen.js) and by the shim (script_crypto_keys.rs).
// Each line is `tag => ok value` or `tag => ErrorName code | message`.
const show = (tag, f) => { try { const r = f(); console.log(tag, "=> ok", r instanceof crypto.KeyObject ? r.type : Buffer.isBuffer(r) ? r.toString("hex") : typeof r === "object" ? JSON.stringify(r) : r); } catch (e) { console.log(tag, "=>", e.name, e.code, "|", e.message); } };
const b = (...x) => Buffer.from(x), m = b(109), sig = Buffer.alloc(8);
const size = (n) => Buffer.from(n < 128 ? [n] : n < 256 ? [0x81, n] : [0x82, n >> 8, n & 255]);
const tlv = (tag, ...parts) => { const c = Buffer.concat(parts); return Buffer.concat([b(tag), size(c.length), c]); };
const pemOf = (d, label) => `-----BEGIN ${label}-----\n${d.toString("base64")}\n-----END ${label}-----\n`;
const derOf = (pem, label) => Buffer.from(pem.split(`-----BEGIN ${label}-----`)[1].split("-----END")[0].replace(/\s/g, ""), "base64");
const spkiDer = (pub) => crypto.createPublicKey(pub).export({ type: "spki", format: "der" });
const pkcs8Der = (key) => crypto.createPrivateKey(key).export({ type: "pkcs8", format: "der" });
// The members of a DER SEQUENCE, each with its tag and length.
const members = (d) => {
  const at = d[1] < 128 ? 2 : 2 + (d[1] & 127), out = [];
  for (let i = at; i < d.length;) {
    const head = d[i + 1] < 128 ? 2 : 2 + (d[i + 1] & 127);
    const len = d[i + 1] < 128 ? d[i + 1] : d.subarray(i + 2, i + head).reduce((a, v) => a * 256 + v, 0);
    out.push(d.subarray(i, i + head + len));
    i += head + len;
  }
  return out;
};
const bitsOf = (d) => { const [, bits] = members(d); return bits.subarray(bits[1] < 128 ? 3 : 3 + (bits[1] & 127)); };
const spki = (alg, bits, unused = 0, trail = b()) => tlv(0x30, alg, tlv(3, b(unused), bits), trail);

// Every way a public key is read. `der` is skipped where Node's two parsers disagree on one key.
const pub = (tag, d, { label = "PUBLIC KEY", type = "spki", digest = "sha256", skip = [], clean, real } = {}) => {
  const o = { key: d, format: "der", type }, pem = pemOf(d, label);
  const ops = {
    der: () => crypto.createPublicKey(o),
    pem: () => crypto.createPublicKey(pem),
    "verify-pem": () => crypto.verify(digest, m, pem, sig),
    "verify-opts": () => crypto.verify(digest, m, { key: pem }, sig),
    "verify-der": () => crypto.verify(digest, m, o, sig),
    jwk: () => crypto.createPublicKey(pem).export({ format: "jwk" }),
  };
  if (digest) ops["create-verify"] = () => crypto.createVerify(digest).update(m).verify(pem, sig);
  if (real) ops.real = () => crypto.verify(digest, m, pem, crypto.sign(digest, m, real));
  if (clean) ops.clean = () => crypto.createPublicKey(o).export({ type: "spki", format: "der" }).equals(clean);
  for (const [name, run] of Object.entries(ops)) if (!skip.includes(name)) show(`${tag} ${name}`, run);
};

// 1. Bytes after a private key's DER value are ignored, and a re-export drops them.
const privates = [["rsa", RSA, RSA_PUB, "sha256"], ["ec", EC256_SEC1, EC256_PUB, "sha256"], ["ed", ED_PRIV, ED_PUB, null]];
for (const [name, pem, pubPem, digest] of privates) {
  const d = pkcs8Der(pem);
  for (const [junk, tail] of [["null", b(5, 0)], ["ff", b(0xff)], ["zeros", Buffer.alloc(40)], ["seq", tlv(0x30, b(2, 1, 0))]]) {
    const x = Buffer.concat([d, tail]), o = { key: x, format: "der", type: "pkcs8" }, text = pemOf(x, "PRIVATE KEY");
    show(`trail ${name} ${junk} der`, () => { const k = crypto.createPrivateKey(o); return `${k.asymmetricKeyType} ${k.export({ type: "pkcs8", format: "der" }).equals(d)}`; });
    show(`trail ${name} ${junk} pem`, () => pkcs8Der(text).equals(d));
    show(`trail ${name} ${junk} public`, () => crypto.createPublicKey(o).export({ type: "spki", format: "der" }).equals(spkiDer(pubPem)));
    show(`trail ${name} ${junk} sign der`, () => crypto.verify(digest, m, pubPem, crypto.sign(digest, m, o)));
    show(`trail ${name} ${junk} sign pem`, () => crypto.verify(digest, m, pubPem, crypto.sign(digest, m, text)));
  }
}
const trailing = (d) => Buffer.concat([d, b(5, 0)]);
const pkcs1Private = derOf(RSA_PKCS1, "RSA PRIVATE KEY"), sec1Private = derOf(EC256_SEC1, "EC PRIVATE KEY");
show("trail pkcs1 der", () => crypto.createPrivateKey({ key: trailing(pkcs1Private), format: "der", type: "pkcs1" }));
show("trail pkcs1 pem", () => crypto.createPrivateKey(pemOf(trailing(pkcs1Private), "RSA PRIVATE KEY")));
show("trail sec1 der", () => crypto.createPrivateKey({ key: trailing(sec1Private), format: "der", type: "sec1" }));
show("trail sec1 pem", () => crypto.createPrivateKey(pemOf(trailing(sec1Private), "EC PRIVATE KEY")));
const encrypted = Buffer.from(K.rsa_der, "base64");
show("trail encrypted der", () => crypto.createPrivateKey({ key: trailing(encrypted), format: "der", type: "pkcs8", passphrase: PW }));
show("trail encrypted pem", () => crypto.createPrivateKey({ key: pemOf(trailing(encrypted), "ENCRYPTED PRIVATE KEY"), passphrase: PW }));

// 2 and 3. What createPublicKey refuses, a verify given PEM refuses too, and the reverse.
const rsaDer = spkiDer(RSA_PUB), ecDer = spkiDer(EC256_PUB), edDer = spkiDer(ED_PUB), ec384Der = spkiDer(EC384_PUB);
const [rsaAlg] = members(rsaDer), [ecAlg] = members(ecDer), [edAlg] = members(edDer), [ec384Alg] = members(ec384Der);
const [edOid] = members(edAlg), pkcs1 = bitsOf(rsaDer), [n, e] = members(pkcs1).map((i) => i.subarray(i[1] < 128 ? 2 : 2 + (i[1] & 127)));
const pt = bitsOf(ecDer), pt384 = bitsOf(ec384Der), edKey = bitsOf(edDer);
const ed = { digest: null };
const flip = (p) => { const c = Buffer.from(p); c[c.length - 1] ^= 1; return c; };
pub("good rsa", rsaDer, { real: RSA });
pub("good ec", ecDer, { real: EC256_SEC1 });
pub("good ed", edDer, { ...ed, real: ED_PRIV });
pub("ed null params", spki(tlv(0x30, edOid, b(5, 0)), edKey), ed);
pub("ed integer params", spki(tlv(0x30, edOid, b(2, 1, 0)), edKey), ed);
pub("ed truncated params", spki(tlv(0x30, edOid, b(5)), edKey), ed);
pub("ed unparsable params", spki(tlv(0x30, edOid, b(0xff, 0xff)), edKey), ed);
pub("ed overlong params", spki(tlv(0x30, edOid, b(5, 9, 0)), edKey), ed);
pub("ed trailing member", spki(edAlg, edKey, 0, b(5, 0)), ed);
pub("ed top trailing", trailing(edDer), { ...ed, clean: edDer, real: ED_PRIV });
for (const [tag, bits] of [["ed 31", edKey.subarray(0, 31)], ["ed 33", Buffer.alloc(33, 7)], ["ed 0", b()]]) pub(tag, spki(edAlg, bits), ed);
const rsaKey = (...ints) => tlv(0x30, ...ints.map((i) => tlv(2, i)));
const pkcs1Form = { label: "RSA PUBLIC KEY", type: "pkcs1" };
pub("rsa third integer spki", spki(rsaAlg, rsaKey(n, e, b(1))));
pub("rsa third integer pkcs1", rsaKey(n, e, b(1)), pkcs1Form);
pub("rsa one integer", spki(rsaAlg, rsaKey(n)));
pub("rsa trailing member", spki(rsaAlg, pkcs1, 0, b(5, 0)));
pub("rsa top trailing", trailing(rsaDer), { clean: rsaDer, real: RSA });
pub("rsa inner trailing", spki(rsaAlg, trailing(pkcs1)));
pub("rsa inner trailing pkcs1", trailing(pkcs1), pkcs1Form);
pub("rsa junk", spki(rsaAlg, b(4)));
pub("ec trailing member", spki(ecAlg, pt, 0, b(5, 0)));
pub("ec top trailing", trailing(ecDer), { clean: ecDer, real: EC256_SEC1 });
pub("ec off curve", spki(ecAlg, flip(pt)));
pub("ec bits 04", spki(ecAlg, b(4)));
pub("ec empty", spki(ecAlg, b()));
pub("ec short", spki(ecAlg, pt.subarray(0, 64)));
pub("ec zero xy", spki(ecAlg, Buffer.concat([b(4), Buffer.alloc(64)])));
pub("p384 off curve", spki(ec384Alg, flip(pt384)));

// 4. An RSA key with a zero member opens and verifies nothing, and exports the member as "".
pub("zero n", spki(rsaAlg, rsaKey(b(0), e)));
pub("zero e", spki(rsaAlg, rsaKey(n, b(0))));
pub("zero both", spki(rsaAlg, rsaKey(b(0), b(0))));
pub("empty n", spki(rsaAlg, rsaKey(b(), e)));
pub("zero e pkcs1", rsaKey(n, b(0)), pkcs1Form);
// Node's DER reader alone refuses this one; the host cannot tell DER from PEM here.
pub("zero n pkcs1", rsaKey(b(0), e), { ...pkcs1Form, skip: ["der", "verify-der"] });

// 5. Export options: a bad format, then a bad type, are TypeErrors.
const held = [["private", crypto.createPrivateKey(RSA), "pkcs8", "spki"], ["public", crypto.createPublicKey(RSA_PUB), "spki", "pkcs8"], ["secret", crypto.createSecretKey(b(1, 2))]];
for (const [name, k, type, wrong] of held) {
  for (const format of [null, 0, "", false, true, 1, "PEM", "x", "buffer"]) {
    show(`export ${name} format ${JSON.stringify(format)}`, () => k.export({ format }).length);
    show(`export ${name} typed format ${JSON.stringify(format)}`, () => k.export({ type, format }).length);
  }
  show(`export ${name} no format`, () => k.export({ type }).length);
  show(`export ${name} empty`, () => k.export({}).length);
  show(`export ${name} undefined format`, () => k.export({ format: undefined }).length);
  for (const t of [null, 0, "", "x", 1, undefined, wrong]) {
    show(`export ${name} type ${JSON.stringify(t)}`, () => k.export({ type: t, format: "pem" }).length);
    show(`export ${name} der type ${JSON.stringify(t)}`, () => k.export({ type: t, format: "der" }).length);
  }
}

// 6. A hybrid point carries y's parity in its prefix: it opens when that is right, as an uncompressed point does.
const prefixed = (p, prefix) => { const c = Buffer.from(p); c[0] = prefix; return c; };
for (const [name, alg, p, digest, real] of [["p256", ecAlg, pt, "sha256", EC256_SEC1], ["p384", ec384Alg, pt384, "sha384", EC384]]) {
  const odd = p[p.length - 1] & 1;
  pub(`hybrid ${name} right`, spki(alg, prefixed(p, 6 + odd)), { digest, real });
  pub(`hybrid ${name} wrong`, spki(alg, prefixed(p, 7 - odd)), { digest });
  pub(`prefix 5 ${name}`, spki(alg, prefixed(p, 5)), { digest });
  pub(`prefix 0 ${name}`, spki(alg, prefixed(p, 0)), { digest });
}
