// The reader takes base64 lines ending LF or CRLF between exact markers, and text before a block.
// Anything else near a marker refuses the whole text, so no line here may become a guess.
const T = K.tokens;
const BOM = [0xef, 0xbb, 0xbf];
const bytes = (...parts) => Buffer.concat(parts.map((p) => Buffer.from(p)));
const JUNK = "x".repeat(254);
const relabel = (pem, label) => pem.replace(/(BEGIN|END) [A-Z ]*-----/g, `$1 ${label}-----`);
const firstBody = (p, f) => { const l = p.split("\n"); l[1] = f(l[1]); return l.join("\n"); };
const inBody = (ch) => (p) => firstBody(p, (x) => `${x.slice(0, 10)}${ch}${x.slice(10)}`);
const blanks = (n) => (p) => p.replace("-----\n", `-----\n${"\n".repeat(n)}`);
const onMarker = (byte) => (p) => { const at = p.indexOf("\n"); return bytes(p.slice(0, at), [byte], p.slice(at)); };
const opened = (f, key) => { try { return f(key).asymmetricKeyType; } catch (_) { return "unreadable"; } };
const forge = (key) => {
  const head = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url");
  const body = Buffer.from(JSON.stringify({ sub: "forged" })).toString("base64url");
  return `${head}.${body}.${crypto.createHmac("sha256", key).update(`${head}.${body}`).digest("base64url")}`;
};
const ask = (tag, key, signTag = tag) => {
  for (const alg of ["HS256", "RS256", "ES256"]) S(`${signTag} ${alg}`, key, alg);
  V(`${tag} forged`, forge(key), key);
  V(`${tag} rs`, T.RS256, key);
  V(`${tag} es`, T.ES256, key, { algorithms: ["ES256"] });
  console.log(`${tag} crypto: ${opened(crypto.createPrivateKey, key)} ${opened(crypto.createPublicKey, key)}`);
};
const LOOSE = {
  // Bytes OpenSSL's base64 table refuses inside a body line.
  body_vt_public: inBody("\v")(K.RSA_PUB), body_ff_public: inBody("\f")(K.RSA_PUB), body_vt_private: inBody("\v")(K.RSA), body_space_public: inBody(" ")(K.RSA_PUB), body_tab_public: inBody("\t")(K.RSA_PUB),
  // Runs of blank lines, which OpenSSL reads by stricter rules than one.
  blank_one_public: blanks(1)(K.RSA_PUB), blank_two_public: blanks(2)(K.RSA_PUB), blank_two_private: blanks(2)(K.RSA), blank_two_cert: blanks(2)(K.RSA_CRT), blank_three_public: blanks(3)(K.RSA_PUB), trailing_space_body: firstBody(K.RSA, (x) => `${x}  `), space_on_marker: K.RSA.replace("-----\n", "----- \n"),
  // A BEGIN the reader cannot read, ahead of one it can.
  high_marker_then_rsa: bytes(onMarker(0xff)(K.EC256), K.RSA), high_public_then_public: bytes(onMarker(0xff)(K.EC256_PUB), K.RSA_PUB), junk_ec_then_rsa: JUNK + K.EC256 + K.RSA, junk_rsa_then_ec: JUNK + K.RSA + K.EC256, junk_public_then_public: JUNK + K.EC256_PUB + K.RSA_PUB, bom_later_block: bytes(K.RSA_CRT, BOM, K.RSA, K.EC256), bom_on_second: bytes(K.EC256, BOM, K.RSA),
  // Labels that are not UTF-8, ended and not.
  label_bytes_then_rsa: bytes("-----BEGIN ", [0xff], " KEY-----\nbm9wZQ==\n-----END ", [0xff], " KEY-----\n", K.RSA), label_bytes_open_then_pkcs1: bytes("-----BEGIN ", [0xff], " KEY-----\n", K.RSA_PKCS1),
  mid_text_marker_then_rsa: `note -----BEGIN X-----\n${K.RSA}`,
};
for (const [name, key] of Object.entries(LOOSE)) ask(`strict ${name}`, key);
// Certificate labels are one pass in file order, so the EC certificate wins either way round.
const X509_EC = relabel(E.EC_CERT, "X509 CERTIFICATE");
ask("strict x509_ec_then_rsa_cert", X509_EC + K.RSA_CRT);
ask("strict ec_then_x509_rsa_cert", E.EC_CERT + relabel(K.RSA_CRT, "X509 CERTIFICATE"));
// What real files look like. None of these may ever be listed as refused, except that a key with
// no private half on sign is the marked text that is never a secret, so those lines stand apart.
const crlf = (p) => p.replace(/\n/g, "\r\n");
const FORMS = { lf: (p) => p, crlf, no_final_newline: (p) => p.trimEnd(), crlf_no_final_newline: (p) => crlf(p).trimEnd() };
for (const [name, key] of Object.entries({ rsa: K.RSA, rsa_pkcs1: K.RSA_PKCS1, ec: K.EC256, ed: K.ED, public: K.RSA_PUB, ec_public: K.EC256_PUB, cert: K.RSA_CRT })) {
  const publicOnly = ["public", "ec_public", "cert"].includes(name);
  for (const [form, f] of Object.entries(FORMS)) ask(`strict ordinary ${name} ${form}`, f(key), `strict ${publicOnly ? "public_on_sign" : "ordinary"} ${name} ${form}`);
}
ask("strict ordinary cert_then_key", K.RSA_CRT + K.RSA);
ask("strict ordinary key_then_cert", K.RSA + K.RSA_CRT);
ask("strict ordinary prose", `# production key\n\nissued 2026\n${K.RSA}`);
ask("strict ordinary prose crlf", crlf(`# production key\n${K.RSA}`));
ask("strict ordinary secret", "a plain shared secret");
