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

// 7. Unused bits in a key's BIT STRING: OpenSSL clears them, so the key's last byte can change.
for (const unused of [1, 7, 8]) {
  pub(`rsa unused ${unused}`, spki(rsaAlg, pkcs1, unused), { real: RSA });
  pub(`ec unused ${unused}`, spki(ecAlg, pt, unused), { real: EC256_SEC1 });
  pub(`ed unused ${unused}`, spki(edAlg, edKey, unused), { ...ed, real: ED_PRIV });
}
pub("rsa unused empty", spki(rsaAlg, b(), 1));

// 8. Long-form lengths and padded integers open, verify, and export as canonical DER.
const long = (tag, ...parts) => { const c = Buffer.concat(parts); return Buffer.concat([b(tag, 0x82, c.length >> 8, c.length & 255), c]); };
pub("rsa long outer", long(0x30, rsaAlg, tlv(3, b(0), pkcs1)), { real: RSA, clean: rsaDer });
pub("rsa long alg", tlv(0x30, long(0x30, ...members(rsaAlg)), tlv(3, b(0), pkcs1)), { real: RSA, clean: rsaDer });
pub("rsa long bits", tlv(0x30, rsaAlg, long(3, b(0), pkcs1)), { real: RSA, clean: rsaDer });
pub("rsa padded e", spki(rsaAlg, rsaKey(n, Buffer.concat([b(0), e]))), { real: RSA, clean: rsaDer });
pub("rsa padded n", spki(rsaAlg, rsaKey(Buffer.concat([b(0), n]), e)), { real: RSA, clean: rsaDer });
pub("rsa padded pkcs1", rsaKey(Buffer.concat([b(0), n]), e), { ...pkcs1Form, real: RSA });
pub("ec long outer", long(0x30, ecAlg, tlv(3, b(0), pt)), { real: EC256_SEC1, clean: ecDer });
pub("ed long outer", long(0x30, edAlg, tlv(3, b(0), edKey)), { ...ed, real: ED_PRIV, clean: edDer });
for (const [name, key] of [["rsa", RSA], ["ec", EC256_SEC1], ["ed", ED_PRIV]]) {
  const d = pkcs8Der(key), x = long(0x30, ...members(d)), o = { key: x, format: "der", type: "pkcs8" };
  show(`pkcs8 ${name} long outer`, () => pkcs8Der(o).equals(d));
  show(`pkcs8 ${name} long outer pem`, () => pkcs8Der(pemOf(x, "PRIVATE KEY")).equals(d));
}

// 9. SPKI tags and the algorithm identifier's tail are checked; RSA takes any one parameter.
const retag = (d, tag) => { const c = Buffer.from(d); c[0] = tag; return c; };
const [rsaOid, rsaNull] = members(rsaAlg), [ecOid, ecCurve] = members(ecAlg);
pub("rsa outer set", retag(rsaDer, 0x31));
pub("rsa alg set", tlv(0x30, retag(rsaAlg, 0x31), tlv(3, b(0), pkcs1)));
pub("rsa bits octets", tlv(0x30, rsaAlg, tlv(4, b(0), pkcs1)));
pub("rsa oid tag", spki(tlv(0x30, retag(rsaOid, 0x04), rsaNull), pkcs1));
pub("rsa no params", spki(tlv(0x30, rsaOid), pkcs1), { real: RSA, clean: rsaDer });
pub("rsa integer params", spki(tlv(0x30, rsaOid, b(2, 1, 0)), pkcs1), { real: RSA, clean: rsaDer });
pub("rsa params tail", spki(tlv(0x30, rsaOid, rsaNull, b(5, 0)), pkcs1));
pub("ec params tail", spki(tlv(0x30, ecOid, ecCurve, b(5, 0)), pt));
pub("ec curve tag", spki(tlv(0x30, ecOid, retag(ecCurve, 0x04)), pt));
pub("ec no params", spki(tlv(0x30, ecOid), pt));

// 10. What each odd key exports as: OpenSSL rebuilds RSA keys and keeps an EC point's form.
const exported = (tag, d, type = "spki") => show(tag, () => {
  const x = crypto.createPublicKey({ key: d, format: "der", type }).export({ type: "spki", format: "der" });
  return x.equals(rsaDer) ? "rsaDer" : x.equals(ecDer) ? "ecDer" : x.equals(edDer) ? "edDer" : x.toString("hex");
});
exported("export rsa inner trailing", spki(rsaAlg, trailing(pkcs1)));
exported("export rsa unused 1", spki(rsaAlg, pkcs1, 1));
exported("export ec unused 1", spki(ecAlg, pt, 1));
exported("export ed unused 7", spki(edAlg, edKey, 7));
exported("export hybrid", spki(ecAlg, prefixed(pt, 6 + (pt[64] & 1))));
exported("export zero n", spki(rsaAlg, rsaKey(b(0), e)));
exported("export empty n", spki(rsaAlg, rsaKey(b(), e)));
exported("export negative e", spki(rsaAlg, rsaKey(n, b(0x81))));

// 11. A compressed EC point opens, exports compressed, and its JWK has the full point.
const compressed = (p) => Buffer.concat([b(2 + (p[p.length - 1] & 1)), p.subarray(1, 1 + (p.length - 1) / 2)]);
pub("compressed p256", spki(ecAlg, compressed(pt)), { real: EC256_SEC1 });
pub("compressed p384", spki(ec384Alg, compressed(pt384)), { digest: "sha384", real: EC384 });
pub("compressed p256 bad prefix", spki(ecAlg, Buffer.concat([b(5 - (pt[64] & 1)), pt.subarray(1, 33)])));
exported("export compressed p256", spki(ecAlg, compressed(pt)));

// 12. A JWK with a zero or empty RSA member opens as a public key that verifies nothing.
const rsaJwk = crypto.createPublicKey(RSA_PUB).export({ format: "jwk" }), rsaPrivateJwk = crypto.createPrivateKey(RSA).export({ format: "jwk" });
const jwkPub = (o) => crypto.createPublicKey({ key: o, format: "jwk" });
for (const [tag, o] of [["n empty", { ...rsaJwk, n: "" }], ["e empty", { ...rsaJwk, e: "" }], ["both empty", { kty: "RSA", n: "", e: "" }], ["n zero", { ...rsaJwk, n: "AA" }], ["e zero", { ...rsaJwk, e: "AA" }]]) {
  show(`jwk ${tag}`, () => jwkPub(o).export({ format: "jwk" }));
  show(`jwk ${tag} spki`, () => jwkPub(o).export({ type: "spki", format: "der" }).toString("hex"));
  show(`jwk ${tag} verify`, () => crypto.verify("sha256", m, jwkPub(o), sig));
}
for (const member of ["n", "e", "d", "p", "dp"]) show(`jwk private ${member} empty`, () => crypto.createPrivateKey({ key: { ...rsaPrivateJwk, [member]: "" }, format: "jwk" }).type);

// 13. Hostile RSASSA-PSS-params on the RSA key, read and checked against a real PSS signature.
const pssOid = tlv(6, b(0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0a));
const sha256Alg = tlv(0x30, tlv(6, b(0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01)));
const mgf1 = tlv(0x30, tlv(6, b(0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x08)), sha256Alg);
const [h0, m1, s2] = [tlv(0xa0, sha256Alg), tlv(0xa1, mgf1), tlv(0xa2, tlv(2, b(32)))];
const salted = (...bytes) => tlv(0x30, h0, m1, tlv(0xa2, tlv(2, b(...bytes))));
const pssSig = crypto.sign("sha256", m, { key: RSA, padding: 6, saltLength: 32 });
const details = (k) => `${k.asymmetricKeyType} ${JSON.stringify(k.asymmetricKeyDetails, (_, v) => (typeof v === "bigint" ? `${v}n` : v))}`;
const pssKey = (tag, params, salt = 32) => {
  const d = spki(tlv(0x30, pssOid, ...(params ? [params] : [])), pkcs1), o = { key: d, format: "der", type: "spki" };
  show(`${tag} der`, () => details(crypto.createPublicKey(o)));
  show(`${tag} pem`, () => crypto.createPublicKey(pemOf(d, "PUBLIC KEY")).type);
  show(`${tag} verify`, () => crypto.verify("sha256", m, { key: pemOf(d, "PUBLIC KEY"), saltLength: salt }, pssSig));
  show(`${tag} spki`, () => crypto.createPublicKey(o).export({ type: "spki", format: "der" }).subarray(0, 80).toString("hex"));
};
pssKey("pss plain", tlv(0x30, h0, m1, s2));
pssKey("pss none", null);
pssKey("pss trailer 1", tlv(0x30, h0, m1, s2, tlv(0xa3, tlv(2, b(1)))));
pssKey("pss trailer 2", tlv(0x30, h0, m1, s2, tlv(0xa3, tlv(2, b(2)))));
pssKey("pss trailer padded", tlv(0x30, h0, m1, s2, tlv(0xa3, tlv(2, b(0, 1)))));
pssKey("pss out of order", tlv(0x30, m1, h0, s2));
pssKey("pss salt first", tlv(0x30, s2, h0, m1));
pssKey("pss truncated", tlv(0x30, h0, m1, s2).subarray(0, 20));
pssKey("pss truncated inner", Buffer.concat([b(0x30, 0x10), h0.subarray(0, 16)]));
pssKey("pss salt padded", salted(0, 32));
pssKey("pss salt zero", salted(0), -2);
pssKey("pss salt 32767", salted(0x7f, 0xff), -2);
pssKey("pss salt 32768", salted(0, 0x80, 0), -2);
pssKey("pss salt 40000", salted(0, 0x9c, 0x40), -2);
pssKey("pss salt huge", salted(0x7f, 0xff, 0xff, 0xff), -2);
pssKey("pss salt negative", salted(0xff), -2);
pssKey("pss hash null params", tlv(0x30, tlv(0xa0, tlv(0x30, ...members(sha256Alg), b(5, 0))), m1, s2));
pssKey("pss sha1 explicit", tlv(0x30, tlv(0xa0, tlv(0x30, tlv(6, b(0x2b, 0x0e, 0x03, 0x02, 0x1a)))), tlv(0xa2, tlv(2, b(20)))), 20);
show("pss alg tail", () => crypto.createPublicKey({ key: spki(tlv(0x30, pssOid, tlv(0x30, h0, m1, s2), b(5, 0)), pkcs1), format: "der", type: "spki" }).type);
show("pss params trailing", () => crypto.createPublicKey({ key: spki(tlv(0x30, pssOid, Buffer.concat([tlv(0x30, h0, m1, s2), b(5, 0)])), pkcs1), format: "der", type: "spki" }).type);
const pssPrivate = (params) => { const [v, , k] = members(pkcs8Der(RSA)); return { key: tlv(0x30, v, tlv(0x30, pssOid, params), k), format: "der", type: "pkcs8" }; };
for (const [tag, params] of [["trailer 2", tlv(0x30, h0, m1, s2, tlv(0xa3, tlv(2, b(2))))], ["salt 32767", salted(0x7f, 0xff)], ["salt 223", salted(0, 223)], ["salt negative", salted(0xff)]]) {
  const o = pssPrivate(params);
  show(`pss private ${tag}`, () => crypto.createPrivateKey(o).asymmetricKeyDetails.saltLength);
  show(`pss private ${tag} sign`, () => crypto.sign("sha256", m, o).length);
  show(`pss private ${tag} sign 222`, () => crypto.sign("sha256", m, { ...o, saltLength: 222 }).length);
  show(`pss private ${tag} public`, () => crypto.createPublicKey(o).export({ type: "spki", format: "der" }).length);
  show(`pss private ${tag} export`, () => crypto.createPrivateKey(o).export({ type: "pkcs8", format: "der" }).length);
  show(`pss private ${tag} verify 222`, () => crypto.verify("sha256", m, { key: crypto.createPublicKey(o), saltLength: 222 }, pssSig));
}

// 14. A modulus of 8k + 1 bits encodes PSS messages one byte shorter than the key. The plain
// key signs at the digest length, as the shim's plain RSA keys only do; the PSS key at any salt.
const salts = { rsa_2049: [32], pss_2049: [0, 32, 222, 223] };
for (const name of ["rsa_2049", "pss_2049"]) {
  const key = K[name], pubKey = crypto.createPublicKey(key), nodeSig = Buffer.from(K[`${name}_sig`], "base64");
  show(`${name} details`, () => details(pubKey));
  for (const salt of salts[name]) {
    show(`${name} sign ${salt}`, () => crypto.verify("sha256", m, { key: pubKey, padding: 6, saltLength: salt }, crypto.sign("sha256", m, { key, padding: 6, saltLength: salt })));
    show(`${name} recovered ${salt}`, () => crypto.verify("sha256", m, { key: pubKey, padding: 6, saltLength: -2 }, crypto.sign("sha256", m, { key, padding: 6, saltLength: salt })));
  }
  for (const salt of [32, -2, 31, 33, 0]) show(`${name} node signature ${salt}`, () => crypto.verify("sha256", m, { key: pubKey, padding: 6, saltLength: salt }, nodeSig));
  show(`${name} node signature tampered`, () => crypto.verify("sha256", Buffer.from("n"), { key: pubKey, padding: 6, saltLength: -2 }, nodeSig));
}

// 15. A wrong passphrase whose decryption passes CBC padding by chance (PBKDF2 at one
// iteration): OpenSSL's decoder refuses the plaintext, not the cipher.
const iter1Der = derOf(K.ec_iter1, "ENCRYPTED PRIVATE KEY");
const message = (f) => { try { return `opened ${f().type}`; } catch (e) { return e.message.split("::").pop(); } };
for (const pw of ["right", "wrong", "w218", "w189647"]) {
  show(`padding ${pw} pem`, () => message(() => crypto.createPrivateKey({ key: K.ec_iter1, passphrase: pw })));
  show(`padding ${pw} der`, () => message(() => crypto.createPrivateKey({ key: iter1Der, format: "der", type: "pkcs8", passphrase: pw })));
  show(`padding ${pw} public`, () => message(() => crypto.createPublicKey({ key: K.ec_iter1, passphrase: pw })));
  show(`padding ${pw} sign`, () => message(() => ({ type: crypto.sign("sha256", m, { key: K.ec_iter1, passphrase: pw }).length > 0 })));
}

// 16. A compressed P-521 point, whose JWK needs the full point.
const ec521Der = spkiDer(EC521_PUB), [ec521Alg] = members(ec521Der), pt521 = bitsOf(ec521Der);
pub("compressed p521", spki(ec521Alg, compressed(pt521)), { digest: "sha512", real: EC521 });
exported("export compressed p521", spki(ec521Alg, compressed(pt521)));

// 17. Deep nesting where OpenSSL keeps nothing (RSA parameters, attributes) opens at any depth,
// and a PKCS#8 export drops attributes and a v2 public key. Built in one pass: O(depth).
const nested = (depth, core = b(5, 0)) => {
  const heads = [];
  for (let i = 0, length = core.length; i < depth; i++) { const head = Buffer.concat([b(0x30), size3(length)]); heads.push(head); length += head.length; }
  return Buffer.concat([...heads.reverse(), core]);
};
const size3 = (n) => Buffer.from(n < 128 ? [n] : n < 256 ? [0x81, n] : n < 65536 ? [0x82, n >> 8, n & 255] : [0x83, n >> 16, (n >> 8) & 255, n & 255]);
const big = (tag, ...parts) => { const c = Buffer.concat(parts); return Buffer.concat([b(tag), size3(c.length), c]); };
const rsaPkcs8 = pkcs8Der(RSA), [rsaVersion, rsaKeyAlg, rsaKeyBits] = members(rsaPkcs8), [rsaKeyOid] = members(rsaKeyAlg);
const deepKey = (tag, key) => {
  const o = { key, format: "der", type: "pkcs8" };
  show(`${tag} der`, () => pkcs8Der(o).equals(rsaPkcs8));
  show(`${tag} pem`, () => pkcs8Der(pemOf(key, "PRIVATE KEY")).equals(rsaPkcs8));
  show(`${tag} sign`, () => crypto.verify("sha256", m, RSA_PUB, crypto.sign("sha256", m, o)));
};
for (const depth of [1, 29, 30, 31, 100000]) {
  deepKey(`deep params ${depth}`, big(0x30, rsaVersion, big(0x30, rsaKeyOid, nested(depth)), rsaKeyBits));
  deepKey(`deep attribute ${depth}`, big(0x30, rsaVersion, rsaKeyAlg, rsaKeyBits, big(0xa0, big(0x30, tlv(6, b(0x2a, 3)), big(0x31, nested(depth))))));
}
const rsaInner = rsaKeyBits.subarray(4);
for (const depth of [1, 100000]) {
  const key = big(0x30, rsaVersion, rsaKeyAlg, big(4, big(0x30, ...members(rsaInner), nested(depth))));
  show(`deep inner ${depth}`, () => crypto.createPrivateKey({ key, format: "der", type: "pkcs8" }).type);
}
show("deep spki", () => crypto.createPublicKey({ key: big(0x30, big(0x30, rsaKeyOid, nested(100000)), tlv(3, b(0), pkcs1)), format: "der", type: "spki" }).export({ type: "spki", format: "der" }).equals(rsaDer));
const edPkcs8 = pkcs8Der(ED_PRIV), [, edKeyAlg, edKeyBits] = members(edPkcs8);
show("ed v2 export", () => pkcs8Der({ key: tlv(0x30, b(2, 1, 1), edKeyAlg, edKeyBits, tlv(0x81, b(0), edKey)), format: "der", type: "pkcs8" }).equals(edPkcs8));
show("ec attribute export", () => { const d = pkcs8Der(EC256_SEC1), [v, a, k] = members(d); return pkcs8Der({ key: tlv(0x30, v, a, k, tlv(0xa0, tlv(0x30, tlv(6, b(0x2a, 3)), tlv(0x31, b(5, 0))))), format: "der", type: "pkcs8" }).equals(d); });
