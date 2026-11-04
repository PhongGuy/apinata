// Which block of a PEM file is the key, and signatures that are almost right. Node's PEM reader
// is line-based and tries the public labels in a fixed order, so the cases hold those seams.
const T = K.tokens;
const PW = "correct horse";
const BROKEN = "-----BEGIN PRIVATE KEY-----\nbm9wZQ==\n-----END PRIVATE KEY-----\n";
const BROKEN_PUB = BROKEN.replaceAll("PRIVATE", "PUBLIC");
const BAD64 = "-----BEGIN PRIVATE KEY-----\n!!!!\n-----END PRIVATE KEY-----\n";
const BAD64_PUB = BAD64.replaceAll("PRIVATE", "PUBLIC");
const MISMATCH = "-----BEGIN PRIVATE KEY-----\nbm9wZQ==\n-----END PUBLIC KEY-----\n";
const relabel = (pem, label) => pem.replace(/(BEGIN|END) [A-Z ]*-----/g, `$1 ${label}-----`);
const bytes = (...parts) => Buffer.concat(parts.map((p) => Buffer.from(p)));
const FF = [0xff, 0xfe];
const opened = (f, key) => { try { return f(key).asymmetricKeyType; } catch (_) { return "unreadable"; } };
const SHIFT_LINES = K.RSA.replace(/\n(?=[A-Za-z0-9+/])/g, "\n  ");
const PEMS = {
  // Findings: an encrypted block reached past the block that is the key.
  ec_then_enc: K.EC256 + K.RSA_ENC, enc_then_ec: K.RSA_ENC + K.EC256, cert_then_enc: K.RSA_CRT + K.RSA_ENC, pub_then_enc: K.RSA_PUB + K.RSA_ENC, enc_then_enc: K.RSA_ENC + K.RSA_ENC,
  // Labels and bodies that disagree.
  pkcs8_under_rsa: relabel(K.RSA, "RSA PRIVATE KEY"), pkcs8_under_ec: relabel(K.EC384, "EC PRIVATE KEY"), pkcs8_under_enc: relabel(K.RSA, "ENCRYPTED PRIVATE KEY"),
  sec1_under_pkcs8: relabel(K.EC256, "PRIVATE KEY"), pkcs1_under_pkcs8: relabel(K.RSA_PKCS1, "PRIVATE KEY"), sec1_under_rsa: relabel(K.EC256, "RSA PRIVATE KEY"), pkcs1_under_ec: relabel(K.RSA_PKCS1, "EC PRIVATE KEY"), pkcs8_rsa_under_ec: relabel(K.RSA, "EC PRIVATE KEY"),
  // A block that cannot be read, ahead of or behind one that can.
  broken_then_rsa: BROKEN + K.RSA, rsa_then_broken: K.RSA + BROKEN, brokenpub_then_rsa: BROKEN_PUB + K.RSA, rsa_then_brokenpub: K.RSA + BROKEN_PUB, foo_then_rsa: "-----BEGIN FOO-----\nbm9wZQ==\n-----END FOO-----\n" + K.RSA,
  bad64_then_rsa: BAD64 + K.RSA, bad64pub_then_rsa: BAD64_PUB + K.RSA, bad64_then_pub: BAD64 + K.RSA_PUB, rsa_then_bad64: K.RSA + BAD64, mismatch_then_rsa: MISMATCH + K.RSA, rsa_then_mismatch: K.RSA + MISMATCH, truncated_then_rsa: `${K.RSA.slice(0, 300)}\n${K.RSA}`, broken_then_pub: BROKEN_PUB + K.RSA_PUB, brokenpriv_then_pub: BROKEN + K.RSA_PUB, brokencert_then_rsa: `${relabel(BROKEN, "CERTIFICATE")}${K.RSA}`,
  nested_begin: K.RSA.replace("-----END PRIVATE KEY-----", K.EC256),
  // Where a line may start and how long it may be.
  indented: K.RSA.replace(/^/gm, "  "), indented_ec: K.EC256.replace(/^/gm, "  "), indented_pub: K.RSA_PUB.replace(/^/gm, "  "), mid_line: `x ${K.RSA}`, note_line: `note\n${K.RSA}`, crlf: K.RSA.replace(/\n/g, "\r\n"), cr_only: K.RSA.replace(/\n/g, "\r"),
  trailing_junk: `${K.RSA}junk`, no_final_newline: K.RSA.trimEnd(), space_after_begin: K.RSA.replace("-----\nMII", "----- \nMII"), text_after_begin: K.RSA.replace("-----BEGIN PRIVATE KEY-----", "-----BEGIN PRIVATE KEY-----abc"), body_indented: SHIFT_LINES,
  one_line_body: K.RSA.replace(/\n(?=[A-Za-z0-9+/=]+\n[A-Za-z0-9+/=])/g, (m, at) => (at < 30 ? m : "")).replace(/([A-Za-z0-9+/=]{64})\n(?=[A-Za-z0-9+/=])/g, "$1"), no_newlines: K.RSA.replace(/\n(?!-)/g, ""), blank_body_line: K.RSA.replace(/(\n[A-Za-z0-9+/]{64})\n/, "$1\n\n"), space_before_end: K.RSA.replace("-----END", "  -----END"), lower_label: K.RSA.replaceAll("PRIVATE KEY", "private key"), extra_der: K.RSA.replace("-----END", "AAAA\n-----END"),
  // Bytes around the block that are not text.
  bytes_before: bytes(FF, "\n", K.RSA), bytes_after: bytes(K.RSA, FF), bytes_touching: bytes([0xff], K.RSA), byte_order_mark: bytes([0xef, 0xbb, 0xbf], K.RSA),
  ec_parameters: `-----BEGIN EC PARAMETERS-----\nBggqhkjOPQMBBw==\n-----END EC PARAMETERS-----\n${K.EC256}`,
  // The public labels are tried in a fixed order, whatever the file's order.
  cert_then_pub: K.RSA_CRT + K.EC256_PUB, pub_then_cert: K.EC256_PUB + K.RSA_CRT, private_then_pub: K.RSA + K.EC256_PUB, pub_then_private: K.EC256_PUB + K.RSA, cert_then_private: K.RSA_CRT + K.EC256, private_then_cert: K.EC256 + K.RSA_CRT,
  x509_label: relabel(K.RSA_CRT, "X509 CERTIFICATE"), trusted_label: relabel(K.RSA_CRT, "TRUSTED CERTIFICATE"),
};
for (const [name, key] of Object.entries(PEMS)) {
  S(`pem ${name} HS256`, key, "HS256");
  S(`pem ${name} RS256`, key, "RS256");
  V(`pem ${name} verify HS256`, T.HS256, key);
  V(`pem ${name} verify RS256`, T.RS256, key);
  V(`pem ${name} verify ES256`, T.ES256, key, { algorithms: ["ES256"] });
  console.log(`pem ${name} crypto: ${opened(crypto.createPrivateKey, key)} ${opened(crypto.createPublicKey, key)}`);
}
for (const [name, key] of Object.entries({ ec_then_enc: PEMS.ec_then_enc, enc_then_ec: PEMS.enc_then_ec, cert_then_enc: PEMS.cert_then_enc, enc_then_enc: PEMS.enc_then_enc })) {
  const pass = { key, passphrase: PW };
  S(`passphrase ${name} RS256`, pass, "RS256");
  S(`passphrase ${name} HS256`, pass, "HS256");
  console.log(`passphrase ${name} crypto: ${opened(crypto.createPrivateKey, pass)} ${opened(crypto.createPublicKey, pass)}`);
}
S("passphrase wrong enc_then_enc", { key: PEMS.enc_then_enc, passphrase: "wrong" }, "RS256");
S("passphrase wrong ec_then_enc", { key: PEMS.ec_then_enc, passphrase: "wrong" }, "ES256");
// Keys Node opens and no crate holds, each reached ahead of a key the shim could read.
const FOREIGN = {
  dsa: E.DSA_TRADITIONAL, three_prime: E.RSA_THREE_PRIME, ec_mismatch_sec1: E.EC_MISMATCH_SEC1, ec_mismatch_pkcs8: E.EC_MISMATCH_PKCS8, ec_zero: E.EC_ZERO,
  dsa_then_rsa: E.DSA_TRADITIONAL + K.RSA, dsa8_then_rsa: K.DSA + K.RSA, ed448_then_rsa: K.ED448 + K.RSA, three_then_rsa: E.RSA_THREE_PRIME + K.RSA, ed448pub_then_rsapub: K.ED448_PUB + K.RSA_PUB,
};
for (const [name, key] of Object.entries(FOREIGN)) {
  if (name !== "ed448pub_then_rsapub") {
    S(`foreign pem ${name} HS256`, key, "HS256");
    S(`foreign pem ${name} RS256`, key, "RS256");
    S(`foreign pem ${name} bytes HS256`, Buffer.from(key), "HS256");
  }
  V(`foreign pem ${name} verify HS256`, T.HS256, key);
  V(`foreign pem ${name} verify RS256`, T.RS256, key);
}
S("pem rsa_then_dsa RS256", K.RSA + E.DSA_TRADITIONAL, "RS256");
V("pem rsa_then_dsa verify RS256", T.RS256, K.RSA + E.DSA_TRADITIONAL);
// A signature Node refuses stays refused, on every size and family that left the crate.
const pubFor = (name) => (name.includes("1024") ? K.RSA1024_PUB : name.includes("2047") ? K.RSA2047_PUB : name.startsWith("ES256") ? K.EC256_PUB : name.startsWith("ES384") ? K.EC384_PUB : K.RSA_PUB);
for (const [name, token] of Object.entries(E.tampered)) {
  V(`tamper ${name}`, token, pubFor(name), { algorithms: [name.slice(0, 5)] });
}
// crypto.verify shares the RSA path, so the salt it recovers meets the same range.
const parts = (token) => { const [h, p, s] = token.split("."); return [Buffer.from(`${h}.${p}`), Buffer.from(s, "base64url")]; };
for (const name of ["PS256 plus n", "PS256_1024 plus n", "PS256_2047 plus n", "PS256 intact", "PS256_1024 intact"]) {
  const [data, signature] = parts(E.tampered[name]);
  for (const [salt, saltLength] of [["auto", -2], ["digest", -1]]) {
    line(`tamper crypto ${name} ${salt}`, () => crypto.verify("sha256", data, { key: pubFor(name), padding: crypto.constants.RSA_PKCS1_PSS_PADDING, saltLength }, signature));
  }
}
