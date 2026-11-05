//! Encrypted private keys and JWK in `require("crypto")`. Expected lines are Node 24 outputs,
//! pasted once from `node` runs of the same scripts, with OpenSSL codes dropped; lines Node does
//! not produce are the named refusals. `fixtures/crypto-keys/vectors.json` holds keys made for
//! these tests by Node 24 and openssl 3.6, and hostile keys built by hand.

use crate::script_crypto::run_crypto;

const VECTORS: &str = include_str!("fixtures/crypto-keys/vectors.json");

const MALFORMED: &str = include_str!("fixtures/crypto-keys/malformed.js");
const NODE_ANSWERS: &str = include_str!("fixtures/crypto-keys/malformed-node.txt");

/// `run_crypto` with the vectors as `K` and their passphrase as `PW`.
async fn run_keys(code: &str) -> Vec<String> {
    run_crypto(&format!(
        "const K = {VECTORS};\nconst PW = \"correct horse\";\n{code}"
    ))
    .await
}

/// Keys encrypted by Node and openssl decrypt to the PKCS#8 they came from, and sign the same bytes as the plain key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_rsa_keys_sign_and_verify_like_node() {
    let code = r#"const m = Buffer.from("hello world");
const plain = crypto.sign("sha256", m, RSA).toString("base64");
console.log(plain);
for (const n of ["rsa_aes256", "rsa_aes128", "rsa_sha512", "rsa_scrypt"]) {
  const o = { key: K[n], passphrase: PW };
  const k = crypto.createPrivateKey(o);
  console.log(n, k.type, k.asymmetricKeyType, k.export({ type: "pkcs8", format: "pem" }) === RSA, crypto.createPublicKey(o).export({ type: "spki", format: "pem" }) === RSA_PUB, crypto.sign("sha256", m, o).toString("base64") === plain, crypto.createSign("sha256").update(m).sign(o, "base64") === plain, crypto.verify("sha256", m, o, Buffer.from(plain, "base64")), crypto.createVerify("sha256").update(m).verify(o, plain, "base64"));
}
const d = Buffer.from(K.rsa_der, "base64");
const der = { key: d, format: "der", type: "pkcs8", passphrase: PW };
console.log(crypto.createPrivateKey(der).export({ type: "pkcs8", format: "pem" }) === RSA, crypto.createPublicKey(der).export({ type: "spki", format: "pem" }) === RSA_PUB, crypto.sign("sha256", m, der).toString("base64") === plain, crypto.verify("sha256", m, der, Buffer.from(plain, "base64")));
const same = (key, passphrase) => crypto.createPrivateKey({ key, passphrase }).export({ type: "pkcs8", format: "pem" }) === RSA;
const pw = Buffer.from(PW), bytes = new Uint8Array([0xff, 0, 0x80]);
console.log(same(K.rsa_aes256, pw), same(K.rsa_aes256, new Uint8Array(pw)), same(K.rsa_aes256, new Uint8Array(pw).buffer), same(K.rsa_aes256, new DataView(new Uint8Array(pw).buffer)), same(Buffer.from(K.rsa_aes256), PW), same(K.rsa_bytes, bytes), same(K.rsa_bytes, Buffer.from(bytes)), same(K.rsa_empty, ""), same(K.rsa_1024, "x".repeat(1024)));
console.log(same(RSA, "ignored"), same(RSA, Buffer.from("ignored")), same(RSA, undefined), crypto.createPublicKey({ key: RSA_PUB, passphrase: "ignored" }).export({ type: "spki", format: "pem" }) === RSA_PUB, crypto.createPrivateKey({ key: crypto.createPrivateKey(RSA).export({ type: "pkcs8", format: "der" }), format: "der", type: "pkcs8", passphrase: "ignored" }).type);
const pss = crypto.sign("sha256", m, { key: K.rsa_aes128, passphrase: PW, padding: crypto.constants.RSA_PKCS1_PSS_PADDING });
console.log(pss.length, crypto.verify("sha256", m, { key: RSA_PUB, padding: crypto.constants.RSA_PKCS1_PSS_PADDING }, pss));"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:dDG8yIcTjUQOBqXL93WZv5VtC8iy+L6ovzjPbZeiq5ffcnAhnJ3dPBWEmdS1A3CJ8U5Ui7gF88FBCnf29p5pb7dH3Gir9V4IQjwh0EZCOdX9rfuDdv5rOuCWzx89AQiMWVzF3zxw8X57DSE0PKM4WAazw1AeDiNycIQoZ1dMUwHOnYU6ja+JexLRusTEUwSRS0WNrPcUt/gO3KwQQeSMKSsydukHb9n5Ame8tOmdTBdD0SAN0B1R2z+xqB7DcC8bW9egO32TAaDszSvkz9/0hF0VG8XcIzH+AAMj5q6/X6fjWjLVTriSvn45X/L26MMJncyIC2sAGH60H7k9uhz9Uw==",
            "log:rsa_aes256 private rsa true true true true true true",
            "log:rsa_aes128 private rsa true true true true true true",
            "log:rsa_sha512 private rsa true true true true true true",
            "log:rsa_scrypt private rsa true true true true true true",
            "log:true true true true",
            "log:true true true true true true true true true",
            "log:true true true true private",
            "log:256 true",
        ]
    );
}

/// The `K.sig_*` signatures were made by Node with the encrypted keys; ours are checked by verifying them back, and Ed25519 is deterministic.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_ec_and_ed25519_keys_sign_and_verify_like_node() {
    let code = r#"const m = Buffer.from("msg"), hex = (s) => Buffer.from(s, "hex");
const cases = [["ec256_aes192", "sha256", K.sig_ec256, EC256_PUB, "der"], ["ec384_aes256", "sha384", K.sig_ec384, EC384_PUB, "ieee-p1363"], ["ec521_aes128", "sha512", K.sig_ec521, EC521_PUB, "der"]];
for (const [n, digest, sig, pub, dsaEncoding] of cases) {
  const o = { key: K[n], passphrase: PW, dsaEncoding };
  const k = crypto.createPrivateKey(o);
  const v = { key: pub, dsaEncoding };
  console.log(n, k.asymmetricKeyType, crypto.createPublicKey(o).export({ type: "spki", format: "pem" }) === pub, crypto.verify(digest, m, v, hex(sig)), crypto.verify(digest, m, o, hex(sig)), crypto.verify(digest, m, v, crypto.sign(digest, m, o)), crypto.verify(digest, Buffer.from("msh"), v, crypto.createSign(digest).update(m).sign(o)));
}
const ed = { key: K.ed_aes256, passphrase: PW };
console.log(crypto.createPrivateKey(ed).asymmetricKeyType, crypto.createPrivateKey(ed).export({ type: "pkcs8", format: "pem" }) === ED_PRIV, crypto.createPublicKey(ed).export({ type: "spki", format: "pem" }) === ED_PUB, crypto.sign(null, m, ed).toString("hex"), crypto.verify(null, m, ed, crypto.sign(null, m, ED_PRIV)));"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:ec256_aes192 ec true true true true false",
            "log:ec384_aes256 ec true true true true false",
            "log:ec521_aes128 ec true true true true false",
            "log:ed25519 true true 8f4d9bff7d6f7066f4a0a57ca196ca33e97908c8903db41eeaa8830d0b7662a43c484c815646779f048e10e2844cf6b2e7e7c73e6b55af7d26bd0843d664880a true",
        ]
    );
}

/// Node's errors for a missing, wrong, badly typed or over-long passphrase, OpenSSL codes dropped; passphrase values are not echoed, unlike Node.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_key_failures_keep_node_errors() {
    let code = r#"const show = (tag, f) => { try { const r = f(); console.log(tag, "ok", Buffer.isBuffer(r) ? r.length : r instanceof crypto.KeyObject ? r.type : JSON.stringify(r)); } catch (e) { console.log(tag, e.name, e.message); } };
const m = Buffer.from("m"), E = K.rsa_aes256, sig = Buffer.alloc(256), d = Buffer.from(K.rsa_der, "base64");
show("none", () => crypto.createPrivateKey(E));
show("none opts", () => crypto.createPrivateKey({ key: E }));
show("none undefined", () => crypto.createPrivateKey({ key: E, passphrase: undefined }));
show("none empty key", () => crypto.createPrivateKey(K.rsa_empty));
show("none public", () => crypto.createPublicKey(E));
show("none sign", () => crypto.sign("sha256", m, E));
show("none sign opts", () => crypto.sign("sha256", m, { key: E }));
show("none verify", () => crypto.verify("sha256", m, E, sig));
show("none createSign", () => crypto.createSign("sha256").update(m).sign(E));
show("none createVerify", () => crypto.createVerify("sha256").update(m).verify(E, sig));
show("none der", () => crypto.createPrivateKey({ key: d, format: "der", type: "pkcs8" }));
show("none der sign", () => crypto.sign("sha256", m, { key: d, format: "der", type: "pkcs8" }));
show("wrong", () => crypto.createPrivateKey({ key: E, passphrase: "wrong" }));
show("wrong empty", () => crypto.createPrivateKey({ key: E, passphrase: "" }));
show("wrong public", () => crypto.createPublicKey({ key: E, passphrase: "wrong" }));
show("wrong sign", () => crypto.sign("sha256", m, { key: E, passphrase: "wrong" }));
show("wrong verify", () => crypto.verify("sha256", m, { key: E, passphrase: "wrong" }, sig));
show("wrong der", () => crypto.createPrivateKey({ key: d, format: "der", type: "pkcs8", passphrase: "wrong" }));
show("flipped", () => crypto.createPrivateKey({ key: K.bad_flipped, passphrase: PW }));
show("type number", () => crypto.createPrivateKey({ key: E, passphrase: 123 }));
show("type null", () => crypto.createPrivateKey({ key: E, passphrase: null }));
show("type object", () => crypto.createPrivateKey({ key: E, passphrase: {} }));
show("type plain key", () => crypto.createPrivateKey({ key: RSA, passphrase: 1 }));
show("type public", () => crypto.createPublicKey({ key: RSA_PUB, passphrase: 1 }));
show("type sign", () => crypto.sign("sha256", m, { key: E, passphrase: 1 }));
show("type keyobject", () => crypto.createPublicKey({ key: crypto.createPrivateKey(RSA), passphrase: 1 }));
show("long 1025", () => crypto.createPrivateKey({ key: E, passphrase: "x".repeat(1025) }));
show("long buffer", () => crypto.createPrivateKey({ key: E, passphrase: Buffer.alloc(2000, 1) }));
show("long plain key", () => crypto.createPrivateKey({ key: RSA, passphrase: "x".repeat(2000) }));
show("der no type", () => crypto.createPrivateKey({ key: d, format: "der", passphrase: PW }));
show("der wrong type", () => crypto.createPublicKey({ key: d, format: "der", type: "spki", passphrase: PW }).type && "parsed");
show("format", () => crypto.createPrivateKey({ key: RSA, format: "pkcs" }));
show("format sign", () => crypto.sign("sha256", m, { key: RSA, format: "pkcs" }));"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:none Error interrupted or cancelled",
            "log:none opts Error interrupted or cancelled",
            "log:none undefined Error interrupted or cancelled",
            "log:none empty key Error interrupted or cancelled",
            "log:none public Error interrupted or cancelled",
            "log:none sign Error interrupted or cancelled",
            "log:none sign opts Error interrupted or cancelled",
            "log:none verify Error interrupted or cancelled",
            "log:none createSign Error interrupted or cancelled",
            "log:none createVerify Error interrupted or cancelled",
            "log:none der TypeError Passphrase required for encrypted key",
            "log:none der sign TypeError Passphrase required for encrypted key",
            "log:wrong Error bad decrypt",
            "log:wrong empty Error bad decrypt",
            "log:wrong public Error bad decrypt",
            "log:wrong sign Error bad decrypt",
            "log:wrong verify Error bad decrypt",
            "log:wrong der Error bad decrypt",
            "log:flipped Error bad decrypt",
            "log:type number TypeError The property 'options.passphrase' is invalid.",
            "log:type null TypeError The property 'options.passphrase' is invalid.",
            "log:type object TypeError The property 'options.passphrase' is invalid.",
            "log:type plain key TypeError The property 'options.passphrase' is invalid.",
            "log:type public TypeError The property 'options.passphrase' is invalid.",
            "log:type sign TypeError The property 'options.passphrase' is invalid.",
            "log:type keyobject ok public",
            "log:long 1025 Error interrupted or cancelled",
            "log:long buffer Error interrupted or cancelled",
            "log:long plain key ok private",
            "log:der no type TypeError The property 'options.type' is invalid. Received undefined",
            "log:der wrong type Error Unsupported key: expected a PEM public key (SPKI, PKCS#1 or X.509 certificate) or private key",
            "log:format TypeError The property 'options.format' is invalid. Received 'pkcs'",
            "log:format sign TypeError The property 'options.format' is invalid. Received 'pkcs'",
        ]
    );
}

/// PBES2 keys from older openssl defaults, PBKDF2-HMAC-SHA1 and DES-EDE3-CBC, open to the PKCS#8 they came from, and a wrong or missing passphrase fails as in Node.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn older_pbes2_keys_open_like_node() {
    let code = r#"const show = (tag, f) => { try { const r = f(); console.log(tag, "ok", Buffer.isBuffer(r) ? r.length : r instanceof crypto.KeyObject ? r.type : JSON.stringify(r)); } catch (e) { console.log(tag, e.name, e.message); } };
const m = Buffer.from("hello world"), plain = crypto.sign("sha256", m, RSA).toString("base64");
for (const n of ["rsa_sha1", "rsa_des3"]) {
  const o = { key: K[n], passphrase: PW };
  console.log(n, crypto.createPrivateKey(o).export({ type: "pkcs8", format: "pem" }) === RSA, crypto.createPublicKey(o).export({ type: "spki", format: "pem" }) === RSA_PUB, crypto.sign("sha256", m, o).toString("base64") === plain);
  show(n + " wrong", () => crypto.createPrivateKey({ key: K[n], passphrase: "wrong" }));
  show(n + " none", () => crypto.createPrivateKey(K[n]));
}"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:rsa_sha1 true true true",
            "log:rsa_sha1 wrong Error bad decrypt",
            "log:rsa_sha1 none Error interrupted or cancelled",
            "log:rsa_des3 true true true",
            "log:rsa_des3 wrong Error bad decrypt",
            "log:rsa_des3 none Error interrupted or cancelled",
        ]
    );
}

/// Schemes Node reads and the crates do not, and keys Node also rejects, are named refusals rather than a wrong passphrase.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_encrypted_keys_are_named() {
    let code = r#"const show = (tag, f) => { try { const r = f(); console.log(tag, "ok", Buffer.isBuffer(r) ? r.length : r instanceof crypto.KeyObject ? r.type : JSON.stringify(r)); } catch (e) { console.log(tag, e.name, e.message); } };
const open = (n) => show(n, () => crypto.createPrivateKey({ key: K[n], passphrase: PW }));
for (const n of ["rsa_camellia", "rsa_pbes1", "rsa_legacy", "bad_truncated", "bad_trunc_data"]) open(n);
show("camellia none", () => crypto.createPrivateKey(K.rsa_camellia));
show("legacy none", () => crypto.createPrivateKey(K.rsa_legacy));
show("legacy public", () => crypto.createPublicKey({ key: K.rsa_legacy, passphrase: PW }));
show("legacy sign", () => crypto.sign("sha256", Buffer.from("m"), K.rsa_legacy));
show("pbes1 none", () => crypto.createPrivateKey(K.rsa_pbes1));"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:rsa_camellia Error Unsupported key: an encrypted key using Camellia (supported: PBES2 with PBKDF2 or scrypt and AES-CBC, AES-GCM or DES-EDE3-CBC)",
            "log:rsa_pbes1 Error Unsupported key: an encrypted key using a PBES1 scheme (supported: PBES2 with PBKDF2 or scrypt and AES-CBC, AES-GCM or DES-EDE3-CBC)",
            "log:rsa_legacy Error Unsupported key: a legacy encrypted PEM (Proc-Type: 4,ENCRYPTED) (convert it to encrypted PKCS#8)",
            "log:bad_truncated Error Unsupported key: the encrypted private key could not be read",
            "log:bad_trunc_data Error Unsupported key: the encrypted private key could not be read",
            "log:camellia none Error Unsupported key: an encrypted key using Camellia (supported: PBES2 with PBKDF2 or scrypt and AES-CBC, AES-GCM or DES-EDE3-CBC)",
            "log:legacy none Error Unsupported key: a legacy encrypted PEM (Proc-Type: 4,ENCRYPTED) (convert it to encrypted PKCS#8)",
            "log:legacy public Error Unsupported key: a legacy encrypted PEM (Proc-Type: 4,ENCRYPTED) (convert it to encrypted PKCS#8)",
            "log:legacy sign Error Unsupported key: a legacy encrypted PEM (Proc-Type: 4,ENCRYPTED) (convert it to encrypted PKCS#8)",
            "log:pbes1 none Error Unsupported key: an encrypted key using a PBES1 scheme (supported: PBES2 with PBKDF2 or scrypt and AES-CBC, AES-GCM or DES-EDE3-CBC)",
        ]
    );
}

/// `fixtures/crypto-keys/exports.json`: `node_*` are Node 24 `export({ cipher, passphrase })`
/// outputs; `ours_*` are this shim's, which Node 24 and `openssl pkcs8 -passin` (openssl 3.6)
/// decrypted to the original keys when they were recorded.
const EXPORTS: &str = include_str!("fixtures/crypto-keys/exports.json");

/// The DER shape Node writes: PBES2, PBKDF2 with a 16-byte salt, 2048 iterations and
/// HMAC-SHA-256, then the AES-CBC OID with a 16-byte IV. `X` is the cipher OID's last byte.
const PBES2_SHAPE: &str = r#"const shape = (der, x) => new RegExp(`^30(81..|82....)305f06092a864886f70d01050d3052303106092a864886f70d01050c30240410[0-9a-f]{32}02020800300c06082a864886f70d02090500301d060960864801650304${x}0410[0-9a-f]{32}04`).test(Buffer.from(der).toString("hex"));
const OID = { "aes-128-cbc": "0102", "aes-192-cbc": "0116", "aes-256-cbc": "012a" };"#;

/// Each AES-CBC name and alias Node takes exports PEM and DER that open again with the
/// passphrase, in Node's PBES2 shape, with a fresh salt and IV each time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_pkcs8_export_round_trips() {
    let code = format!(
        r#"{PBES2_SHAPE}
const keys = {{ RSA, EC256_SEC1, EC384, ED_PRIV }};
const plain = (key) => crypto.createPrivateKey(key).export({{ type: "pkcs8", format: "pem" }});
const names = [["aes-128-cbc", "aes-128-cbc"], ["aes-192-cbc", "aes-192-cbc"], ["aes-256-cbc", "aes-256-cbc"], ["AES-256-CBC", "aes-256-cbc"], ["aes128", "aes-128-cbc"], ["Aes192", "aes-192-cbc"], ["aes256", "aes-256-cbc"]];
for (const [cipher, canonical] of names) {{
  const row = [cipher];
  for (const pem of Object.values(keys)) {{
    const k = crypto.createPrivateKey(pem);
    const o = {{ type: "pkcs8", format: "pem", cipher, passphrase: PW }};
    const enc = k.export(o), der = k.export({{ ...o, format: "der" }});
    const opens = plain({{ key: enc, passphrase: PW }}) === plain(pem) && plain({{ key: der, format: "der", type: "pkcs8", passphrase: PW }}) === plain(pem);
    const header = enc.startsWith("-----BEGIN ENCRYPTED PRIVATE KEY-----\n") && enc.endsWith("\n-----END ENCRYPTED PRIVATE KEY-----\n");
    const pemDer = Buffer.from(enc.split("\n").slice(1, -2).join(""), "base64");
    const fresh = enc !== k.export(o) && !der.equals(k.export({{ ...o, format: "der" }}));
    row.push(Buffer.isBuffer(der), opens, header, shape(der, OID[canonical]), shape(pemDer, OID[canonical]), fresh);
  }}
  console.log(row.join(" "));
}}
const k = crypto.createPrivateKey(EC256_SEC1), o = {{ type: "pkcs8", format: "pem", cipher: "aes-256-cbc" }};
console.log(plain({{ key: k.export({{ ...o, passphrase: Buffer.from(PW) }}), passphrase: PW }}) === plain(EC256_SEC1), plain({{ key: k.export({{ ...o, passphrase: new Uint8Array([0xff, 0]) }}), passphrase: Buffer.from([0xff, 0]) }}) === plain(EC256_SEC1), plain({{ key: k.export({{ ...o, passphrase: "" }}), passphrase: "" }}) === plain(EC256_SEC1), plain({{ key: k.export({{ ...o, passphrase: "70", encoding: "hex" }}), passphrase: Buffer.from([0x70]) }}) === plain(EC256_SEC1), plain({{ key: k.export({{ ...o, passphrase: "x".repeat(1024) }}), passphrase: "x".repeat(1024) }}) === plain(EC256_SEC1));"#
    );
    let row = "true true true true true true";
    let line = |name: &str| format!("log:{name} {row} {row} {row} {row}");
    assert_eq!(
        run_keys(&code).await,
        [
            line("aes-128-cbc"),
            line("aes-192-cbc"),
            line("aes-256-cbc"),
            line("AES-256-CBC"),
            line("aes128"),
            line("Aes192"),
            line("aes256"),
            "log:true true true true true".to_string(),
        ]
    );
}

/// Node's exports open here, ours were opened by Node and openssl when recorded and open here,
/// and both have the same shape.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recorded_encrypted_exports_open_in_both_directions() {
    let code = format!(
        r#"{PBES2_SHAPE}
const X = {EXPORTS};
const PW = "correct horse";
const plain = (key) => crypto.createPrivateKey(key).export({{ type: "pkcs8", format: "pem" }});
const cases = [["rsa_aes128", RSA, "aes-128-cbc"], ["ec256_aes192", EC256_SEC1, "aes-192-cbc"], ["ed_aes256", ED_PRIV, "aes-256-cbc"]];
for (const who of ["node", "ours"]) {{
  for (const [n, original, cipher] of cases) {{
    const pem = X[`${{who}}_${{n}}`];
    const der = Buffer.from(pem.split("\n").slice(1, -2).join(""), "base64");
    console.log(who, n, plain({{ key: pem, passphrase: PW }}) === plain(original), shape(der, OID[cipher]));
  }}
  const der = Buffer.from(X[`${{who}}_ec384_der_aes256`], "base64");
  console.log(who, "ec384_der", plain({{ key: der, format: "der", type: "pkcs8", passphrase: PW }}) === plain(EC384), shape(der, OID["aes-256-cbc"]));
}}"#
    );
    assert_eq!(
        run_crypto(&code).await,
        [
            "log:node rsa_aes128 true true",
            "log:node ec256_aes192 true true",
            "log:node ed_aes256 true true",
            "log:node ec384_der true true",
            "log:ours rsa_aes128 true true",
            "log:ours ec256_aes192 true true",
            "log:ours ed_aes256 true true",
            "log:ours ec384_der true true",
        ]
    );
}

/// Node's errors for encrypted export options, OpenSSL codes dropped and passphrases not echoed;
/// ciphers Node writes and the shim does not are named refusals. A passphrase over 1024 bytes,
/// which Node writes but cannot read back, is refused with the error reading it would give.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_export_failures_keep_node_errors() {
    let code = r#"const show = (tag, f) => { try { const r = f(); console.log(tag, "ok", Buffer.isBuffer(r) ? r.length : typeof r === "string" ? r.split("\n")[0] : JSON.stringify(r)); } catch (e) { console.log(tag, e.name, e.message); } };
const k = crypto.createPrivateKey(EC256_SEC1), x = (o) => () => k.export({ type: "pkcs8", format: "pem", ...o });
show("no passphrase", x({ cipher: "aes-256-cbc" }));
show("no cipher", x({ passphrase: "p" }));
show("cipher null", x({ cipher: null, passphrase: "p" }));
show("cipher number", x({ cipher: 1, passphrase: "p" }));
show("passphrase number", x({ cipher: "aes-256-cbc", passphrase: 1 }));
show("unknown", x({ cipher: "nope", passphrase: "p" }));
show("unknown aes", x({ cipher: "aes-999-cbc", passphrase: "p" }));
show("unknown design", x({ cipher: "design", passphrase: "p" }));
show("unknown aes mode", x({ cipher: "aes-128-nope", passphrase: "p" }));
show("passphrase 1025", x({ cipher: "aes-256-cbc", passphrase: "x".repeat(1025) }));
show("passphrase 1025 der", () => k.export({ type: "pkcs8", format: "der", cipher: "aes-256-cbc", passphrase: Buffer.alloc(1025, 1) }));
for (const cipher of ["aes-256-gcm", "aes-128-ctr", "aes-256-ecb", "des-ede3-cbc", "DES-EDE3-CBC", "des-ede", "camellia-256-cbc", "chacha20", "id-aes256-wrap"]) show(cipher, x({ cipher, passphrase: "p" }));
show("der unknown", () => k.export({ type: "pkcs8", format: "der", cipher: "nope", passphrase: "p" }));
show("sec1", () => k.export({ type: "sec1", format: "pem", cipher: "aes-256-cbc", passphrase: "p" }));
show("jwk", () => k.export({ format: "jwk", cipher: "aes-256-cbc", passphrase: "p" }));
show("public", () => crypto.createPublicKey(k).export({ type: "spki", format: "pem", cipher: "aes-256-cbc", passphrase: "p" }));
const enc = k.export({ type: "pkcs8", format: "pem", cipher: "aes-128-cbc", passphrase: "p" });
show("open none", () => crypto.createPrivateKey(enc));
show("open wrong", () => crypto.createPrivateKey({ key: enc, passphrase: "q" }));
const der = k.export({ type: "pkcs8", format: "der", cipher: "aes-128-cbc", passphrase: "p" });
show("open der none", () => crypto.createPrivateKey({ key: der, format: "der", type: "pkcs8" }));
show("open der wrong", () => crypto.createPrivateKey({ key: der, format: "der", type: "pkcs8", passphrase: "q" }));"#;
    let refused = |cipher: &str| {
        format!(
            "log:{cipher} Error KeyObject export not supported: cipher \"{cipher}\" (supported: aes-128-cbc, aes-192-cbc and aes-256-cbc)"
        )
    };
    let mut expected: Vec<String> = [
        "log:no passphrase TypeError The property 'options.passphrase' is invalid.",
        "log:no cipher TypeError The property 'options.cipher' is invalid. Received undefined",
        "log:cipher null TypeError The property 'options.cipher' is invalid. Received null",
        "log:cipher number TypeError The property 'options.cipher' is invalid. Received 1",
        "log:passphrase number TypeError The property 'options.passphrase' is invalid.",
        "log:unknown Error Unknown cipher",
        "log:unknown aes Error Unknown cipher",
        "log:unknown design Error Unknown cipher",
        "log:unknown aes mode Error Unknown cipher",
        "log:passphrase 1025 Error interrupted or cancelled",
        "log:passphrase 1025 der Error interrupted or cancelled",
    ]
    .map(String::from)
    .to_vec();
    expected.extend(
        [
            "aes-256-gcm",
            "aes-128-ctr",
            "aes-256-ecb",
            "des-ede3-cbc",
            "DES-EDE3-CBC",
            "des-ede",
            "camellia-256-cbc",
            "chacha20",
            "id-aes256-wrap",
        ]
        .map(refused),
    );
    expected.extend(
        [
            "log:der unknown Error Unknown cipher",
            "log:sec1 Error KeyObject export not supported: type \"sec1\" with format \"pem\" (supported: pkcs8 for private keys and spki for public keys, as pem or der, or jwk)",
            "log:jwk Error The selected key encoding jwk does not support encryption.",
            "log:public ok -----BEGIN PUBLIC KEY-----",
            "log:open none Error interrupted or cancelled",
            "log:open wrong Error bad decrypt",
            "log:open der none TypeError Passphrase required for encrypted key",
            "log:open der wrong Error bad decrypt",
        ]
        .map(String::from),
    );
    assert_eq!(run_crypto(code).await, expected);
}

/// A key file's KDF parameters are untrusted: each is held to the script work caps before any derivation, so none of these takes time or memory.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hostile_kdf_parameters_are_refused_before_deriving() {
    let code = r#"const show = (tag, f) => { try { const r = f(); console.log(tag, "ok", Buffer.isBuffer(r) ? r.length : r instanceof crypto.KeyObject ? r.type : JSON.stringify(r)); } catch (e) { console.log(tag, e.name, e.message); } };
for (const n of ["iter", "iter0", "iter_max", "keylen", "salt", "n_huge", "n_mem", "n_zero", "n_three", "n_one", "r_zero", "p_zero", "rp"]) {
  show(n, () => crypto.createPrivateKey({ key: K["hostile_" + n], passphrase: PW }));
  show(n + " none", () => crypto.sign("sha256", Buffer.from("m"), K["hostile_" + n]));
}"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:iter Error Unsupported key: the encrypted key's PBKDF2 work is over the limit (iterations 1 to 10000000, iterations x ceil(key length / digest length) at most 16777216)",
            "log:iter none Error Unsupported key: the encrypted key's PBKDF2 work is over the limit (iterations 1 to 10000000, iterations x ceil(key length / digest length) at most 16777216)",
            "log:iter0 Error Unsupported key: the encrypted key's PBKDF2 work is over the limit (iterations 1 to 10000000, iterations x ceil(key length / digest length) at most 16777216)",
            "log:iter0 none Error Unsupported key: the encrypted key's PBKDF2 work is over the limit (iterations 1 to 10000000, iterations x ceil(key length / digest length) at most 16777216)",
            "log:iter_max Error Unsupported key: the encrypted key's PBKDF2 work is over the limit (iterations 1 to 10000000, iterations x ceil(key length / digest length) at most 16777216)",
            "log:iter_max none Error Unsupported key: the encrypted key's PBKDF2 work is over the limit (iterations 1 to 10000000, iterations x ceil(key length / digest length) at most 16777216)",
            "log:keylen Error Unsupported key: an encrypted key with an unreadable or unsupported PBES2 scheme (supported: PBES2 with PBKDF2 or scrypt and AES-CBC, AES-GCM or DES-EDE3-CBC)",
            "log:keylen none Error interrupted or cancelled",
            "log:salt Error Unsupported key: an encrypted key with an unreadable or unsupported PBES2 scheme (supported: PBES2 with PBKDF2 or scrypt and AES-CBC, AES-GCM or DES-EDE3-CBC)",
            "log:salt none Error Unsupported key: an encrypted key with an unreadable or unsupported PBES2 scheme (supported: PBES2 with PBKDF2 or scrypt and AES-CBC, AES-GCM or DES-EDE3-CBC)",
            "log:n_huge Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:n_huge none Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:n_mem Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:n_mem none Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:n_zero Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:n_zero none Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:n_three Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:n_three none Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:n_one Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:n_one none Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:r_zero Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:r_zero none Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:p_zero Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:p_zero none Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:rp Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
            "log:rp none Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)",
        ]
    );
}

/// Member order and values are Node's: these lines are `JSON.stringify` of Node's exports.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwk_export_matches_node() {
    let code = r#"const j = (k) => JSON.stringify(k.export({ format: "jwk" }));
for (const [priv, pub] of [[RSA, RSA_PUB], [RSA1024, RSA1024_PUB], [EC256_SEC1, EC256_PUB], [EC384, EC384_PUB], [EC521, EC521_PUB], [ED_PRIV, ED_PUB]]) {
  console.log(j(crypto.createPrivateKey(priv)));
  console.log(j(crypto.createPublicKey(pub)), j(crypto.createPublicKey(pub)) === j(crypto.createPublicKey(priv)));
}
console.log(crypto.hash("sha256", j(crypto.createPrivateKey(RSA8192))), crypto.hash("sha256", j(crypto.createPublicKey(RSA8192_PUB))));
console.log(j(crypto.createSecretKey(Buffer.from([251, 255, 0]))), j(crypto.createSecretKey(Buffer.from("key"))), j(crypto.createSecretKey(Buffer.alloc(0))));
console.log(JSON.stringify(crypto.createPublicKey(RSA_PUB).export({ type: "spki", format: "jwk" })) === j(crypto.createPublicKey(RSA_PUB)), JSON.stringify(crypto.createPrivateKey(RSA).export({ type: "pkcs8", format: "jwk" })) === j(crypto.createPrivateKey(RSA)));"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:{\"kty\":\"RSA\",\"n\":\"0OnqCWWmbNHE15DgSlXdDx4Y6CGI5XKdv1zANsuAoMP6vPZMrCpOVC7c41bzsxxjxaai19htXiKN9FxRIxgB0HUoFJvFsyxE6XFoUJ2dZwRyvRGVKbAHYA1WNkglJzT81pmwZPNdcA2nsj1eg4cPMfaZWYZr-fjzHsv0u7wOqneR_Rb6Gg14D9ke5CdKHHxV8ohysBL7_8RaQ_7RmUrc4rMW8bUz4YcudRvYg4Agi6sn6B4n4mXnSYu_Nlc5hxhoxHG4Z-SzR0aXiibx6uKKXGeYF1L0w-sOLM5uwOsh7N4XpQ782jf5rVbTOmY4jpbne1l84uwLqMkH91jU_bIegQ\",\"e\":\"AQAB\",\"d\":\"T-jpaGbFXeJvFLWJ1WU6hUnR5RjOsFVDGat0qoD01omgIx9i_IfCJlk2jI758IzvbO7sYvuZ-ePlwI5_SpkoOIBXsGpVGM_aHCEQ8aUYTAtbiA5tq-B1MbxoOaMm8vRFsiBfZH3iQ7Y1auwmdtD2cvoMdj4Al0EuXZ1OI3EveuxIqc3Cjc6lVlEQLHCg73tAG-bHyby8WobZE6FMCJ28gkF6QpoL0PazXTe0Tmc2If389nGPttaCOZGTK6HBpRU_8yYW2-v1uEcEN6_a-hrLiTeAW5WA2l2GKgdw7sHATBpoDoFbNbUoC9mktbk3ui9PVormoGLT81TK3wsSMwYtcQ\",\"p\":\"7DyVnrSKKtLiUX8CP1m746kmhW4k0NGJlkP5fIoAWc0nRZQsNrUWO4wTqRjZSDwkiB96WwTC60sbuX2IXEa6a6GLYh7BPn8tJ4LK71RPCxTha-uwmOhwTz7wWcd31oO05enRqxXykbrC7mt9kKeboPnBc4PvS_bp7BNhbWuwyuM\",\"q\":\"4mQpfMZFjANXthiLeh0biywa70D8R_IbrwTwdkAWYJuXiUQqMeD3tL0166B0ZB_FtJy-C0dfKhIWg89jVkH4tr_m7eVwPD6Nura5E9jli7z5eQ6xtqpvwIbe_b_y1Z_e7em2DRv4TmgfodKQxmwopJpoO0E8BTKUK2otETw3-ks\",\"dp\":\"3mFlw0FWs1tTOr5h4jrZmUT5AzAdLgmiCDcy2x6Zl-98oY4LVPCA3ktNkqfWmp4rEYz4REgmKRLKJYHbZGocQQeQSyyCYw-a9Q-1czgdNGRSk6IJ2tcsKh0oaotP7zqKhsfl3LTLZI23xHKhgBOXeCDED7bKA-uKZvYTh8qRYVs\",\"dq\":\"h_OWZnO6uAYZ2iSLPd3U-JKX6i2gUqQ1s7jHnnuF9KX7Lldc728l0cf-ZW2J98aMt5WZGrTVlTIfizAze01KypQbXal26Pa3rfSk2CJcVro5JlWIqyVWzssvhjqK3XaQLVc4GE8_ofYtaMRGZKH3FkITbeIS5RwmlpvATKVZCFk\",\"qi\":\"WYf0lxzhBkx0e9-aOwDMhPnnUNAs_94YMwVyw8C6ICskvFlXwG6YHudktrKRGzJ63nq_lQeFJNxd0HV4d4UMBuE0VXfYTOPgjAKQtWzJpchjKXZoJbuoLjSTCqUf7Jh-Bcx9YA8S5NO8OA3gX1HLKawr0h43kxNPFhbliaJmYBc\"}",
            "log:{\"kty\":\"RSA\",\"n\":\"0OnqCWWmbNHE15DgSlXdDx4Y6CGI5XKdv1zANsuAoMP6vPZMrCpOVC7c41bzsxxjxaai19htXiKN9FxRIxgB0HUoFJvFsyxE6XFoUJ2dZwRyvRGVKbAHYA1WNkglJzT81pmwZPNdcA2nsj1eg4cPMfaZWYZr-fjzHsv0u7wOqneR_Rb6Gg14D9ke5CdKHHxV8ohysBL7_8RaQ_7RmUrc4rMW8bUz4YcudRvYg4Agi6sn6B4n4mXnSYu_Nlc5hxhoxHG4Z-SzR0aXiibx6uKKXGeYF1L0w-sOLM5uwOsh7N4XpQ782jf5rVbTOmY4jpbne1l84uwLqMkH91jU_bIegQ\",\"e\":\"AQAB\"} true",
            "log:{\"kty\":\"RSA\",\"n\":\"taFvpVL98MsGJ-aV_BLqfFo2LJgLXZVtE6DshruTVsd7JKam9s360bcVhOZCv5jHdfeXHMnYOS-fvjJOU0vf3PcBX6U-I5JTc4DnsWXTChb7OlITdl0xonDvBcwr_6nkHte6YRxGSQLcXuAZbWVAuBUWBAlBk1BLLelpG8o7DiE\",\"e\":\"AQAB\",\"d\":\"mVqSz33BFxv0OKBCJrE59oCcmQ5PGlbd3-LRkfiyMGUBHjLvahcEHSl-K9XWqbqvPnUry4zI2Y_SSl5jL4UynwxD8IGc0Mpxr0zSweSfIe4ICEQ2tfPRlHjsTNJY3OysJosF8TEVTi3VI1_10OY07_qkEny5fQjv_C4QDr6W9CE\",\"p\":\"3sbXQJ3aMBmrg13UHDR-pO9TrbFygkbjMT9LZPq1zxKv_F32VApUo7LYLCQtBVwe972__2Cbzw2eTYX5vB7grQ\",\"q\":\"0Le4kNSSq4siRyBn4WFx_EHabVnyHWkVtv1hMWEOKpwyCphF7FF2NAKFBeopmmP7X4VzpB2Qr0TY2svuqVjtxQ\",\"dp\":\"XfnHR8zPA8dwYKQuHA_Q9Ma4p-bZBTfeEyI-LcTM-SNK9cy1YtJQzhVdt5EzJLWtoph668HXOEroYo-Hv-HOqQ\",\"dq\":\"gVGz-5iaH8ufO26qroyP8Xpj-ekHCI35tF2eANhfzKrmnoXmnRuEH4mIHjgN-ukD8RVgp0jiQpZnvnkosKi2-Q\",\"qi\":\"sdLYIHTjM_GZYHBo7ZW-qYE1fTkxpJN2wLhaII5crhZsb6Elcm1PhS3Ze3mZ8LPTuV8p_LekEF_No3RSHGYDdA\"}",
            "log:{\"kty\":\"RSA\",\"n\":\"taFvpVL98MsGJ-aV_BLqfFo2LJgLXZVtE6DshruTVsd7JKam9s360bcVhOZCv5jHdfeXHMnYOS-fvjJOU0vf3PcBX6U-I5JTc4DnsWXTChb7OlITdl0xonDvBcwr_6nkHte6YRxGSQLcXuAZbWVAuBUWBAlBk1BLLelpG8o7DiE\",\"e\":\"AQAB\"} true",
            "log:{\"kty\":\"EC\",\"x\":\"nL3dTBpnl4An7dq_L_qtEBd3hVvvf_6gwVpT4GW4nDs\",\"y\":\"R_2yvrlv2vOL7xe_lGSRieXWEvc-n-fPro3-K01d_yI\",\"crv\":\"P-256\",\"d\":\"ge-yZCaHB605XyK_x0bsfDQ7YPKyrSZ1LYgZT1B_YjI\"}",
            "log:{\"kty\":\"EC\",\"x\":\"nL3dTBpnl4An7dq_L_qtEBd3hVvvf_6gwVpT4GW4nDs\",\"y\":\"R_2yvrlv2vOL7xe_lGSRieXWEvc-n-fPro3-K01d_yI\",\"crv\":\"P-256\"} true",
            "log:{\"kty\":\"EC\",\"x\":\"2WmuSgua24iJw8o-3ulRb_7vE4B1de09-TzoWVlkGvAMwFhvJlVOH0JiL-6JVB_9\",\"y\":\"NH78-ewzhT1HZ-isgNAisdt67m4D6ws1HZRRIt2P-B6wpUR7yc9uhjtye5UfFIz9\",\"crv\":\"P-384\",\"d\":\"Krrz2rc32ELNvzKkNy10d94TNhboWpyGScGfV00UdmaWTUTW1zesP4_C0lKUftyc\"}",
            "log:{\"kty\":\"EC\",\"x\":\"2WmuSgua24iJw8o-3ulRb_7vE4B1de09-TzoWVlkGvAMwFhvJlVOH0JiL-6JVB_9\",\"y\":\"NH78-ewzhT1HZ-isgNAisdt67m4D6ws1HZRRIt2P-B6wpUR7yc9uhjtye5UfFIz9\",\"crv\":\"P-384\"} true",
            "log:{\"kty\":\"EC\",\"x\":\"AYiCEbyNPlO2XIrIGqJxGLg58mBppCLhg7Gfu3-MqygUdU_Y90pzQOYhQ6ZN8bANpfgka_dQYTsv0yI_0eXuUqKb\",\"y\":\"ADCgv4ao9lN_RaMULY9SxJMgySntyJvob1A83Y7chvA4zTwia7T6I-gEf7VRxc3U-cC9iIlZf892sD8AcQbBo0dB\",\"crv\":\"P-521\",\"d\":\"AD-36WqDux9Drv7CFZHTgvp1ehzlR4TXzAobws-VoqmgcerpYojON2-Tvg5sL57wqNWRutY-JqTYaRdbw-dZNUOX\"}",
            "log:{\"kty\":\"EC\",\"x\":\"AYiCEbyNPlO2XIrIGqJxGLg58mBppCLhg7Gfu3-MqygUdU_Y90pzQOYhQ6ZN8bANpfgka_dQYTsv0yI_0eXuUqKb\",\"y\":\"ADCgv4ao9lN_RaMULY9SxJMgySntyJvob1A83Y7chvA4zTwia7T6I-gEf7VRxc3U-cC9iIlZf892sD8AcQbBo0dB\",\"crv\":\"P-521\"} true",
            "log:{\"crv\":\"Ed25519\",\"d\":\"nWGxne_9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A\",\"x\":\"11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo\",\"kty\":\"OKP\"}",
            "log:{\"crv\":\"Ed25519\",\"x\":\"11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo\",\"kty\":\"OKP\"} true",
            "log:0841a2fbdc0865592f5c110d048d3db9272e5cd9eb26d419011745e9f3bcd01d 6e848eab2a62a58b34e7da8a9e634c05b22d7b78ebb318764f330abe2d99c16f",
            "log:{\"kty\":\"oct\",\"k\":\"-_8A\"} {\"kty\":\"oct\",\"k\":\"a2V5\"} {\"kty\":\"oct\",\"k\":\"\"}",
            "log:true true",
        ]
    );
}

/// A JWK imports to the DER Node makes of it, and a public key can come from a private JWK.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwk_import_round_trips_to_node_der() {
    let code = r#"const h = crypto.hash.bind(null, "sha256");
const J = (pem) => crypto.createPrivateKey(pem).export({ format: "jwk" });
for (const [name, priv, pub] of [["rsa", RSA, RSA_PUB], ["rsa1024", RSA1024, RSA1024_PUB], ["rsa8192", RSA8192, RSA8192_PUB], ["p256", EC256_SEC1, EC256_PUB], ["p384", EC384, EC384_PUB], ["p521", EC521, EC521_PUB], ["ed", ED_PRIV, ED_PUB]]) {
  const key = J(priv);
  const k = crypto.createPrivateKey({ key, format: "jwk" });
  const p = crypto.createPublicKey({ key, format: "jwk" });
  const p2 = crypto.createPublicKey({ key: crypto.createPublicKey(pub).export({ format: "jwk" }), format: "jwk" });
  console.log(name, k.type, k.asymmetricKeyType, h(k.export({ type: "pkcs8", format: "pem" })), JSON.stringify(k.export({ format: "jwk" })) === JSON.stringify(key), p.type, p.asymmetricKeyType, p.export({ type: "spki", format: "pem" }) === crypto.createPublicKey(pub).export({ type: "spki", format: "pem" }), p2.export({ type: "spki", format: "pem" }) === p.export({ type: "spki", format: "pem" }));
}
const ec = J(EC256_SEC1), ed = J(ED_PRIV), rsa = J(RSA);
const pem = (k) => k.export({ type: "spki", format: "pem" });
console.log(JSON.stringify(crypto.createPrivateKey({ key: { ...ec, kid: "x", use: "sig", alg: "ES256", key_ops: [] }, format: "jwk" }).export({ format: "jwk" })) === JSON.stringify(ec));
console.log(pem(crypto.createPublicKey({ key: { kty: "EC", crv: "P-256", x: Buffer.from(ec.x, "base64url").toString("base64"), y: ec.y + "=" }, format: "jwk" })) === EC256_PUB, crypto.createPrivateKey({ key: { ...ec, d: ec.d.slice(0, 10) + "*" + ec.d.slice(10) }, format: "jwk" }).type, crypto.createPrivateKey({ key: { ...ec, d: "AAAA" + ec.d }, format: "jwk" }).type);
console.log(crypto.createPrivateKey({ key: { ...ed, x: "A".repeat(43) }, format: "jwk" }).export({ format: "jwk" }).x === ed.x, pem(crypto.createPublicKey({ key: { ...ed, d: "A".repeat(43) }, format: "jwk" })) === ED_PUB, crypto.createPrivateKey({ key: { ...ed, x: "!!!!" }, format: "jwk" }).type, crypto.createPrivateKey({ key: { ...ed, d: ed.d.slice(0, 5) + "*" + ed.d.slice(5) }, format: "jwk" }).type);
console.log(pem(crypto.createPublicKey({ key: { ...ec, d: K.jwk_other_ec.d }, format: "jwk" })) === EC256_PUB, pem(crypto.createPublicKey({ key: { kty: "RSA", n: rsa.n, e: rsa.e }, format: "jwk" })) === RSA_PUB);"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:rsa private rsa c2af78ab30f8b711b3d76879fed2f5affaa6dfc6a68b660535a6ed9dc2f5a5da true public rsa true true",
            "log:rsa1024 private rsa 2696cbd80f4acfae0045b15b9094fb87fa96f584c2d305a6c5af12c848c3cb3d true public rsa true true",
            "log:rsa8192 private rsa 7184fa9201af49eaaeed37122ef63eece4cddd31b27b77c3d76ffa8fdaac3f26 true public rsa true true",
            "log:p256 private ec 2f4a6b9c5cfdc79c6d958ef1f40f33c621f03f3f90716be1fbff191b5d0b8e80 true public ec true true",
            "log:p384 private ec 6f5bd4dab19519ffe5994d158515921799ef3c679471c35c8a0284698996361b true public ec true true",
            "log:p521 private ec 0faaf0796ba1f094e2bbb9764c98c52bdfb6b6ec8457dee4a6af79d348a8e239 true public ec true true",
            "log:ed private ed25519 c4932a9b6b97423b249a53e58d706f820185467464699038ed7ca5b29815ba03 true public ed25519 true true",
            "log:true",
            "log:true private private",
            "log:true true private private",
            "log:true true",
        ]
    );
}

/// `format: "jwk"` in the key options of sign, verify, createSign and createVerify; RSA PKCS#1 v1.5 and Ed25519 signatures are deterministic.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwk_keys_work_in_sign_and_verify_options() {
    let code = r#"const m = Buffer.from("msg");
const J = (pem) => crypto.createPrivateKey(pem).export({ format: "jwk" }), P = (pem) => crypto.createPublicKey(pem).export({ format: "jwk" });
const rsa = { key: J(RSA), format: "jwk" }, rsaPub = { key: P(RSA_PUB), format: "jwk" };
const s = crypto.sign("sha256", m, rsa);
console.log(s.toString("base64") === crypto.sign("sha256", m, RSA).toString("base64"), crypto.createSign("sha256").update(m).sign(rsa, "hex").slice(0, 32), crypto.verify("sha256", m, rsaPub, s), crypto.verify("sha256", m, rsa, s), crypto.createVerify("sha256").update(m).verify(rsaPub, s), crypto.verify("sha256", Buffer.from("msh"), rsaPub, s));
const pss = crypto.sign("sha256", m, { ...rsa, padding: 6 });
console.log(crypto.verify("sha256", m, { ...rsaPub, padding: 6 }, pss));
const ec = { key: J(EC384), format: "jwk", dsaEncoding: "ieee-p1363" }, ecPub = { key: P(EC384_PUB), format: "jwk", dsaEncoding: "ieee-p1363" };
const e = crypto.sign("sha384", m, ec);
console.log(e.length, crypto.verify("sha384", m, ecPub, e), crypto.verify("sha384", m, { key: EC384_PUB, dsaEncoding: "ieee-p1363" }, e), crypto.createVerify("sha384").update(m).verify(ecPub, crypto.createSign("sha384").update(m).sign(ec)));
const ed = { key: J(ED_PRIV), format: "jwk" }, edPub = { key: P(ED_PUB), format: "jwk" };
const es = crypto.sign(null, m, ed);
console.log(es.toString("hex"), crypto.verify(null, m, edPub, es), crypto.verify(null, m, ed, es));
const der = crypto.createPrivateKey(RSA).export({ type: "pkcs8", format: "der" });
console.log(crypto.sign("sha256", m, { key: der, format: "der", type: "pkcs8" }).equals(s), crypto.verify("sha256", m, { key: crypto.createPublicKey(RSA_PUB).export({ type: "spki", format: "der" }), format: "der", type: "spki" }, s));"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:true 09caebb8a1a76404ad41098be3312c2c true true true false",
            "log:true",
            "log:96 true true true",
            "log:8f4d9bff7d6f7066f4a0a57ca196ca33e97908c8903db41eeaa8830d0b7662a43c484c815646779f048e10e2844cf6b2e7e7c73e6b55af7d26bd0843d664880a true true",
            "log:true true",
        ]
    );
}

/// Node's types and messages, with values of non-string members left out of `Received`, since they could be key material.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwk_errors_follow_node() {
    let code = r#"const show = (tag, f) => { try { const r = f(); console.log(tag, "ok", Buffer.isBuffer(r) ? r.length : r instanceof crypto.KeyObject ? r.type : JSON.stringify(r)); } catch (e) { console.log(tag, e.name, e.message); } };
const J = (pem) => crypto.createPrivateKey(pem).export({ format: "jwk" });
const rsa = J(RSA), ec = J(EC256_SEC1), ed = J(ED_PRIV), m = Buffer.from("m");
const priv = (tag, key) => show(tag, () => crypto.createPrivateKey({ key, format: "jwk" }));
const pub = (tag, key) => show(tag, () => crypto.createPublicKey({ key, format: "jwk" }));
const without = (o, k) => { const c = { ...o }; delete c[k]; return c; };
priv("string", "x"); priv("number", 1); priv("null", null); priv("undefined", undefined); priv("array", []); pub("pub string", "x");
priv("buffer", Buffer.alloc(2)); priv("empty", {}); priv("oct", { kty: "oct", k: "AA" }); priv("kty number", { kty: 1 }); priv("lower", { kty: "rsa" }); pub("pub oct", { kty: "oct", k: "AA" }); priv("akp", { kty: "AKP", alg: "ML-DSA-44" });
for (const k of ["n", "e", "d", "p", "q", "dp", "dq", "qi"]) priv("rsa -" + k, without(rsa, k));
for (const k of ["n", "e"]) pub("rsa pub -" + k, without(rsa, k));
for (const k of ["crv", "x", "y", "d"]) priv("ec -" + k, without(ec, k));
for (const k of ["crv", "x", "y"]) pub("ec pub -" + k, without(ec, k));
for (const k of ["crv", "x", "d"]) priv("ed -" + k, without(ed, k));
for (const k of ["crv", "x"]) pub("ed pub -" + k, without(ed, k));
priv("n number", { ...rsa, n: 1 }); priv("crv number", { ...ec, crv: 1 }); priv("x null", { ...ec, x: null }); priv("d object", { ...ec, d: {} }); priv("d number", { ...ec, d: 123 });
for (const c of ["P-224", "P-256K", "prime256v1", "Ed25519", ""]) priv("ec crv " + c, { ...ec, crv: c });
for (const c of ["P-256", ""]) priv("okp crv " + c, { ...ed, crv: c });
priv("off curve", { ...ec, x: K.jwk_other_ec.x }); pub("pub off curve", { ...ec, x: K.jwk_other_ec.x });
priv("x bad", { ...ec, x: "!!!!" }); pub("pub x bad", { ...ec, x: "!!!!" }); priv("x empty", { ...ec, x: "" }); pub("pub x short", { ...ec, x: ec.x.slice(0, 20) });
priv("ed bad length", { ...ed, d: "AAAA" }); pub("ed pub bad length", { ...ed, x: "AAAA" }); priv("ed bad text", { ...ed, d: "!!!!" });
show("sign bad", () => crypto.sign("sha256", m, { key: "x", format: "jwk" }));
show("sign public", () => crypto.sign("sha256", m, { key: without(rsa, "d"), format: "jwk" }));
show("verify bad", () => crypto.verify("sha256", m, { key: {}, format: "jwk" }, Buffer.alloc(4)));
show("createSign bad", () => crypto.createSign("sha256").update(m).sign({ key: 1, format: "jwk" }));
show("createVerify bad", () => crypto.createVerify("sha256").update(m).verify({ key: 1, format: "jwk" }, Buffer.alloc(4)));
show("passphrase ignored", () => crypto.createPrivateKey({ key: ec, format: "jwk", passphrase: 1 }));"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:string TypeError The \"key.key\" property must be of type object. Received type string",
            "log:number TypeError The \"key.key\" property must be of type object. Received type number",
            "log:null TypeError The \"key.key\" property must be of type object. Received null",
            "log:undefined TypeError The \"key.key\" property must be of type object. Received undefined",
            "log:array TypeError The \"key.key\" property must be of type object. Received an instance of Array",
            "log:pub string TypeError The \"key.key\" property must be of type object. Received type string",
            "log:buffer TypeError The property 'key.kty' must be one of: 'RSA', 'EC', 'OKP', 'AKP'. Received undefined",
            "log:empty TypeError The property 'key.kty' must be one of: 'RSA', 'EC', 'OKP', 'AKP'. Received undefined",
            "log:oct TypeError The property 'key.kty' must be one of: 'RSA', 'EC', 'OKP', 'AKP'. Received 'oct'",
            "log:kty number TypeError The property 'key.kty' must be one of: 'RSA', 'EC', 'OKP', 'AKP'. Received 1",
            "log:lower TypeError The property 'key.kty' must be one of: 'RSA', 'EC', 'OKP', 'AKP'. Received 'rsa'",
            "log:pub oct TypeError The property 'key.kty' must be one of: 'RSA', 'EC', 'OKP', 'AKP'. Received 'oct'",
            "log:akp Error Unsupported key: JWK key type \"AKP\" (supported: RSA, EC and OKP with Ed25519)",
            "log:rsa -n TypeError The \"key.n\" property must be of type string. Received undefined",
            "log:rsa -e TypeError The \"key.e\" property must be of type string. Received undefined",
            "log:rsa -d TypeError The \"key.d\" property must be of type string. Received undefined",
            "log:rsa -p TypeError The \"key.p\" property must be of type string. Received undefined",
            "log:rsa -q TypeError The \"key.q\" property must be of type string. Received undefined",
            "log:rsa -dp TypeError The \"key.dp\" property must be of type string. Received undefined",
            "log:rsa -dq TypeError The \"key.dq\" property must be of type string. Received undefined",
            "log:rsa -qi TypeError The \"key.qi\" property must be of type string. Received undefined",
            "log:rsa pub -n TypeError The \"key.n\" property must be of type string. Received undefined",
            "log:rsa pub -e TypeError The \"key.e\" property must be of type string. Received undefined",
            "log:ec -crv TypeError The \"key.crv\" property must be of type string. Received undefined",
            "log:ec -x TypeError The \"key.x\" property must be of type string. Received undefined",
            "log:ec -y TypeError The \"key.y\" property must be of type string. Received undefined",
            "log:ec -d TypeError The \"key.d\" property must be of type string. Received undefined",
            "log:ec pub -crv TypeError The \"key.crv\" property must be of type string. Received undefined",
            "log:ec pub -x TypeError The \"key.x\" property must be of type string. Received undefined",
            "log:ec pub -y TypeError The \"key.y\" property must be of type string. Received undefined",
            "log:ed -crv TypeError The \"key.crv\" property must be of type string. Received undefined",
            "log:ed -x TypeError The \"key.x\" property must be of type string. Received undefined",
            "log:ed -d TypeError The \"key.d\" property must be of type string. Received undefined",
            "log:ed pub -crv TypeError The \"key.crv\" property must be of type string. Received undefined",
            "log:ed pub -x TypeError The \"key.x\" property must be of type string. Received undefined",
            "log:n number TypeError The \"key.n\" property must be of type string. Received type number",
            "log:crv number TypeError The \"key.crv\" property must be of type string. Received type number",
            "log:x null TypeError The \"key.x\" property must be of type string. Received null",
            "log:d object TypeError The \"key.d\" property must be of type string. Received an instance of Object",
            "log:d number TypeError The \"key.d\" property must be of type string. Received type number",
            "log:ec crv P-224 TypeError The property 'key.crv' must be one of: 'P-256', 'secp256k1', 'P-384', 'P-521'. Received 'P-224'",
            "log:ec crv P-256K TypeError The property 'key.crv' must be one of: 'P-256', 'secp256k1', 'P-384', 'P-521'. Received 'P-256K'",
            "log:ec crv prime256v1 TypeError The property 'key.crv' must be one of: 'P-256', 'secp256k1', 'P-384', 'P-521'. Received 'prime256v1'",
            "log:ec crv Ed25519 TypeError The property 'key.crv' must be one of: 'P-256', 'secp256k1', 'P-384', 'P-521'. Received 'Ed25519'",
            "log:ec crv  TypeError The property 'key.crv' must be one of: 'P-256', 'secp256k1', 'P-384', 'P-521'. Received ''",
            "log:okp crv P-256 TypeError The property 'key.crv' must be one of: 'Ed25519', 'Ed448', 'X25519', 'X448'. Received 'P-256'",
            "log:okp crv  TypeError The property 'key.crv' must be one of: 'Ed25519', 'Ed448', 'X25519', 'X448'. Received ''",
            "log:off curve TypeError Invalid JWK EC key",
            "log:pub off curve TypeError Invalid JWK EC key",
            "log:x bad TypeError Invalid JWK EC key",
            "log:pub x bad TypeError Invalid JWK EC key",
            "log:x empty TypeError Invalid JWK EC key",
            "log:pub x short TypeError Invalid JWK EC key",
            "log:ed bad length TypeError Invalid JWK data",
            "log:ed pub bad length TypeError Invalid JWK data",
            "log:ed bad text TypeError Invalid JWK data",
            "log:sign bad TypeError The \"key.key\" property must be of type object. Received type string",
            "log:sign public TypeError The \"key.d\" property must be of type string. Received undefined",
            "log:verify bad TypeError The property 'key.kty' must be one of: 'RSA', 'EC', 'OKP', 'AKP'. Received undefined",
            "log:createSign bad TypeError The \"key.key\" property must be of type object. Received type number",
            "log:createVerify bad TypeError The \"key.key\" property must be of type object. Received type number",
            "log:passphrase ignored ok private",
        ]
    );
}

/// Keys Node holds and the shim has no type for, and inconsistent keys Node accepts and our crates refuse, are named refusals, never a key that signs wrongly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwk_kinds_and_inconsistent_keys_are_named() {
    let code = r#"const show = (tag, f) => { try { const r = f(); console.log(tag, "ok", Buffer.isBuffer(r) ? r.length : r instanceof crypto.KeyObject ? r.type : JSON.stringify(r)); } catch (e) { console.log(tag, e.name, e.message); } };
const J = (pem) => crypto.createPrivateKey(pem).export({ format: "jwk" });
const rsa = J(RSA), ec = J(EC256_SEC1), o = K.jwk_other_ec;
const priv = (tag, key) => show(tag, () => crypto.createPrivateKey({ key, format: "jwk" }));
const pub = (tag, key) => show(tag, () => crypto.createPublicKey({ key, format: "jwk" }));
priv("ed448", K.jwk_ed448); pub("ed448 pub", K.jwk_ed448); priv("x25519", K.jwk_x25519); pub("x25519 pub", K.jwk_x25519); priv("x448", K.jwk_x448); priv("k1", K.jwk_k1); pub("k1 pub", K.jwk_k1);
priv("ec d", { ...ec, d: o.d }); priv("ec d empty", { ...ec, d: "" }); priv("ec d zero", { ...ec, d: "A".repeat(43) }); priv("ec d long", { ...ec, d: "AQ" + ec.d });
priv("rsa d", { ...rsa, d: "AQAB" }); priv("rsa p", { ...rsa, p: rsa.q }); priv("rsa dp", { ...rsa, dp: "AQAB" }); priv("rsa qi", { ...rsa, qi: "AQAB" }); priv("rsa n", { ...rsa, n: "!!!!" });
pub("rsa pub n", { ...rsa, n: "!!!!" }); pub("rsa pub e", { ...rsa, e: "" }); pub("rsa pub tiny", { kty: "RSA", n: "AQAB", e: "AQAB" });
show("rsa-pss", () => crypto.createPrivateKey(K.pss).export({ format: "jwk" }));
show("rsa-pss public", () => crypto.createPublicKey(K.pss_pub).export({ format: "jwk" }));
show("rsa-pss encrypted", () => crypto.createPrivateKey({ key: K.pss_enc, passphrase: PW }));
show("x25519 pem", () => crypto.createPrivateKey(K.x25519));
show("compressed", () => crypto.createPublicKey(K.ec256_compressed_pub).export({ format: "jwk" }));"#;
    assert_eq!(
        run_keys(code).await,
        [
            "log:ed448 Error Unsupported key: JWK curve \"Ed448\" (supported: P-256, P-384, P-521 and Ed25519)",
            "log:ed448 pub Error Unsupported key: JWK curve \"Ed448\" (supported: P-256, P-384, P-521 and Ed25519)",
            "log:x25519 Error Unsupported key: JWK curve \"X25519\" (supported: P-256, P-384, P-521 and Ed25519)",
            "log:x25519 pub Error Unsupported key: JWK curve \"X25519\" (supported: P-256, P-384, P-521 and Ed25519)",
            "log:x448 Error Unsupported key: JWK curve \"X448\" (supported: P-256, P-384, P-521 and Ed25519)",
            "log:k1 Error Unsupported key: JWK curve \"secp256k1\" (supported: P-256, P-384, P-521 and Ed25519)",
            "log:k1 pub Error Unsupported key: JWK curve \"secp256k1\" (supported: P-256, P-384, P-521 and Ed25519)",
            "log:ec d Error Unsupported key: the JWK's private and public values do not match",
            "log:ec d empty Error Unsupported key: the JWK's private and public values do not match",
            "log:ec d zero Error Unsupported key: the JWK's private and public values do not match",
            "log:ec d long Error Unsupported key: the JWK's private and public values do not match",
            "log:rsa d Error Unsupported key: the JWK's RSA values do not form a usable key",
            "log:rsa p Error Unsupported key: the JWK's RSA values do not form a usable key",
            "log:rsa dp Error Unsupported key: the JWK's RSA values do not form a usable key",
            "log:rsa qi Error Unsupported key: the JWK's RSA values do not form a usable key",
            "log:rsa n Error Unsupported key: the JWK's RSA values do not form a usable key",
            "log:rsa pub n Error Unsupported key: the JWK's RSA values do not form a usable key",
            "log:rsa pub e Error Unsupported key: the JWK's RSA values do not form a usable key",
            "log:rsa pub tiny Error Unsupported key: the JWK's RSA values do not form a usable key",
            "log:rsa-pss Error Unsupported JWK Key Type.",
            "log:rsa-pss public Error Unsupported JWK Key Type.",
            "log:rsa-pss encrypted ok private",
            "log:x25519 pem Error Unsupported key: expected a PEM private key (PKCS#8, PKCS#1 or SEC1)",
            "log:compressed Error Unsupported key: JWK export of a compressed EC public key",
        ]
    );
}

/// Truncated and damaged key files fail as errors, never as a panic in a host function.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn damaged_keys_never_panic() {
    let code = r#"const der = Buffer.from(K.rsa_der, "base64");
const label = "ENCRYPTED PRIVATE KEY";
const pem = (b) => `-----BEGIN ${label}-----\n${b.toString("base64")}\n-----END ${label}-----\n`;
const attempt = (key) => { try { crypto.createPrivateKey(key); return 1; } catch (e) { return typeof e.message === "string" ? 0 : 2; } };
let opened = 0, odd = 0;
const count = (key) => { const r = attempt(key); opened += r === 1 ? 1 : 0; odd += r === 2 ? 1 : 0; };
for (let n = 0; n < der.length; n += 13) {
  count({ key: pem(der.subarray(0, n)), passphrase: PW });
  count({ key: der.subarray(0, n), format: "der", type: "pkcs8", passphrase: PW });
}
for (let i = 0; i < 90; i++) for (const bit of [0x01, 0x80]) {
  const d = Buffer.from(der);
  d[i] ^= bit;
  count({ key: d, format: "der", type: "pkcs8", passphrase: PW });
}
const b64 = "AQAB_-!*=";
for (let n = 0; n < 200; n++) {
  const v = Array.from({ length: n % 90 }, (_, i) => b64[(i * n + n) % b64.length]).join("");
  for (const key of [{ kty: "EC", crv: "P-256", x: v, y: v, d: v }, { kty: "EC", crv: "P-521", x: v, y: v }, { kty: "OKP", crv: "Ed25519", x: v, d: v }, { kty: "RSA", n: v, e: v, d: v, p: v, q: v, dp: v, dq: v, qi: v }]) {
    count({ key, format: "jwk" });
    try { crypto.createPublicKey({ key, format: "jwk" }); } catch (e) { odd += typeof e.message === "string" ? 0 : 1; }
  }
}
console.log(odd);"#;
    assert_eq!(run_keys(code).await, ["log:0"]);
}

/// A sentinel passphrase and sentinel key bytes through every failure path: errors name the
/// problem and never carry either, even where Node would echo the value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn errors_never_carry_passphrases_or_key_material() {
    let code = r#"const PASS = "SENTINELPASS", NUM = 73577357, BYTES = "U0VOVElORUxLRVlCWVRFUw";
const J = (pem) => crypto.createPrivateKey(pem).export({ format: "jwk" });
const ec = J(EC256_SEC1), ed = J(ED_PRIV), rsa = J(RSA), m = Buffer.from("m"), sig = Buffer.alloc(4);
const wrapped = (label) => `-----BEGIN ${label}-----\n${BYTES}==\n-----END ${label}-----\n`;
const fail = (f) => { try { f(); console.log("no error"); } catch (e) { console.log(e.name, e.message, String(e.stack)); } };
const sneaky = { toString() { return PASS; } };
for (const passphrase of [PASS, Buffer.from(PASS), NUM, sneaky, [PASS], PASS.repeat(200)]) {
  for (const key of [K.rsa_aes256, K.rsa_sha1, K.rsa_pbes1, K.rsa_legacy, K.hostile_iter, K.bad_truncated, wrapped("ENCRYPTED PRIVATE KEY")]) {
    fail(() => crypto.createPrivateKey({ key, passphrase }));
    fail(() => crypto.createPublicKey({ key, passphrase }));
    fail(() => crypto.sign("sha256", m, { key, passphrase }));
    fail(() => crypto.verify("sha256", m, { key, passphrase }, sig));
  }
}
for (const label of ["PRIVATE KEY", "RSA PRIVATE KEY", "EC PRIVATE KEY", "PUBLIC KEY"]) {
  fail(() => crypto.createPrivateKey({ key: wrapped(label), passphrase: PASS }));
  fail(() => crypto.sign("sha256", m, wrapped(label)));
}
for (const key of [PASS, wrapped("PRIVATE KEY"), NUM, [PASS], Buffer.from(PASS).toString("hex")]) {
  fail(() => crypto.createPrivateKey({ key, format: "jwk" }));
  fail(() => crypto.createPublicKey({ key, format: "jwk" }));
}
for (const d of [NUM, sneaky, [PASS], true, BYTES, ""]) {
  fail(() => crypto.createPrivateKey({ key: { ...ec, d }, format: "jwk" }));
  fail(() => crypto.createPrivateKey({ key: { ...ed, d }, format: "jwk" }));
  fail(() => crypto.createPrivateKey({ key: { ...rsa, d }, format: "jwk" }));
  fail(() => crypto.sign("sha256", m, { key: { ...rsa, p: d }, format: "jwk" }));
  fail(() => crypto.createPublicKey({ key: { ...ec, x: d }, format: "jwk" }));
}"#;
    let lines = run_keys(code).await;
    assert!(lines.len() > 150, "{}", lines.len());
    for line in lines {
        for sentinel in ["SENTINEL", "U0VOVEl", "73577357", "53454e54"] {
            assert!(!line.contains(sentinel), "{line}");
        }
        assert!(!line.contains("no error"), "{line}");
    }
}

/// The `show` helper the tests below share, and DER builders for hand-made keys.
const HELPERS: &str = r#"const show = (tag, f) => { try { const r = f(); console.log(tag, "ok", Buffer.isBuffer(r) ? r.length : r instanceof crypto.KeyObject ? r.type : typeof r === "object" ? JSON.stringify(r) : r); } catch (e) { console.log(tag, e.name, e.message); } };
const size = (n) => Buffer.from(n < 128 ? [n] : n < 256 ? [0x81, n] : n < 65536 ? [0x82, n >> 8, n & 255] : [0x83, n >> 16, (n >> 8) & 255, n & 255]);
const tlv = (tag, ...parts) => { const b = Buffer.concat(parts); return Buffer.concat([Buffer.from([tag]), size(b.length), b]); };
const spki = (pub, bits, unused = 0, trail = Buffer.alloc(0)) => {
  const d = crypto.createPublicKey(pub).export({ type: "spki", format: "der" });
  const at = d[1] < 128 ? 2 : 2 + (d[1] & 127);
  return tlv(0x30, d.subarray(at, at + 2 + d[at + 1]), tlv(3, Buffer.from([unused]), bits), trail);
};
const pemOf = (b, label) => `-----BEGIN ${label}-----\n${b.toString("base64")}\n-----END ${label}-----\n`;
"#;

async fn run_with_helpers(code: &str) -> Vec<String> {
    run_keys(&format!("{HELPERS}{code}")).await
}

/// A passphrase never leaves a private key in the clear as JWK; `cipher` alone is ignored there, as in Node.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_jwk_export_refuses_a_passphrase() {
    let code = r#"const opts = [{ passphrase: "x" }, { passphrase: "" }, { passphrase: null }, { passphrase: 1 }, { type: "pkcs8", passphrase: "x" }, { cipher: "aes-256-cbc", passphrase: "x" }, { cipher: null, passphrase: "x" }];
for (const pem of [RSA, EC256_SEC1, ED_PRIV]) {
  const k = crypto.createPrivateKey(pem), plain = JSON.stringify(k.export({ format: "jwk" }));
  for (const o of opts) show(k.asymmetricKeyType, () => k.export({ format: "jwk", ...o }));
  console.log([{ cipher: "aes-256-cbc" }, { cipher: null }, { cipher: "" }, { cipher: 1 }, { passphrase: undefined }].map((o) => JSON.stringify(k.export({ format: "jwk", ...o })) === plain).join(" "));
}"#;
    let refused = "Error The selected key encoding jwk does not support encryption.";
    let mut expected = Vec::new();
    for kind in ["rsa", "ec", "ed25519"] {
        expected.extend((0..7).map(|_| format!("log:{kind} {refused}")));
        expected.push("log:true true true true true".to_string());
    }
    assert_eq!(run_with_helpers(code).await, expected);
}

/// Public keys have nothing to encrypt, so both options are ignored for them. A private key
/// exports in the clear only with neither set; anything else keeps Node's error or the refusal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn export_cipher_options_follow_node() {
    let code = r#"const priv = crypto.createPrivateKey(RSA), pub = crypto.createPublicKey(RSA_PUB);
for (const format of ["pem", "der"]) {
  const plain = (k, type) => k.export({ type, format }).toString("hex");
  for (const o of [{ passphrase: "x" }, { passphrase: null }, { cipher: null }, { cipher: "aes-256-cbc" }, { cipher: 1 }, { cipher: "aes-256-cbc", passphrase: "x" }]) {
    console.log(pub.export({ type: "spki", format, ...o }).toString("hex") === plain(pub, "spki"));
  }
  console.log(priv.export({ type: "pkcs8", format, cipher: null }).toString("hex") === plain(priv, "pkcs8"), priv.export({ type: "pkcs8", format, cipher: undefined, passphrase: undefined }).toString("hex") === plain(priv, "pkcs8"));
  const bad = (tag, o) => show(tag, () => priv.export({ type: "pkcs8", format, ...o }).length > 0);
  bad("pass only", { passphrase: "x" });
  bad("pass empty", { passphrase: "" });
  bad("pass null", { passphrase: null });
  bad("pass, cipher null", { cipher: null, passphrase: "x" });
  bad("cipher only", { cipher: "aes-256-cbc" });
  bad("cipher empty", { cipher: "" });
  bad("cipher number", { cipher: 1 });
  bad("cipher number pass", { cipher: 1, passphrase: "x" });
  bad("cipher, pass number", { cipher: "aes-256-cbc", passphrase: 1 });
}"#;
    let mut expected = Vec::new();
    for _ in 0..2 {
        expected.extend((0..6).map(|_| "log:true".to_string()));
        expected.extend(
            [
                "log:true true",
                "log:pass only TypeError The property 'options.cipher' is invalid. Received undefined",
                "log:pass empty TypeError The property 'options.cipher' is invalid. Received undefined",
                "log:pass null TypeError The property 'options.cipher' is invalid. Received undefined",
                "log:pass, cipher null TypeError The property 'options.cipher' is invalid. Received null",
                "log:cipher only TypeError The property 'options.passphrase' is invalid.",
                "log:cipher empty TypeError The property 'options.passphrase' is invalid.",
                "log:cipher number TypeError The property 'options.cipher' is invalid. Received 1",
                "log:cipher number pass TypeError The property 'options.cipher' is invalid. Received 1",
                "log:cipher, pass number TypeError The property 'options.passphrase' is invalid.",
            ]
            .map(String::from),
        );
    }
    assert_eq!(run_with_helpers(code).await, expected);
}

/// RSA integers over the size cap stop before the crates validate them, whichever form brings them: that work grows with the integers and cannot be interrupted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_rsa_integers_are_refused_before_any_work() {
    let code = r#"const J = crypto.createPrivateKey(RSA).export({ format: "jwk" }), m = Buffer.from("m");
const one = (bytes) => { const b = Buffer.alloc(bytes, 255); b[0] = 1; return b; };
const int = (b) => tlv(2, Buffer.from([0]), b);
const rsaOid = Buffer.from("06092a864886f70d0101010500", "hex");
const pkcs1 = (b) => tlv(0x30, tlv(2, Buffer.from([0])), ...Array.from({ length: 8 }, () => int(b)));
const pkcs8 = (b) => tlv(0x30, tlv(2, Buffer.from([0])), tlv(0x30, rsaOid), tlv(4, pkcs1(b)));
for (const [tag, b] of [["8193", one(1025)], ["100k", one(12500)]]) {
  const v = b.toString("base64url");
  for (const member of ["n", "d", "p", "q", "dp", "dq", "qi"]) show(`${tag} jwk ${member}`, () => crypto.createPrivateKey({ key: { ...J, [member]: v }, format: "jwk" }));
  show(`${tag} jwk sign`, () => crypto.sign("sha256", m, { key: { ...J, p: v, q: v }, format: "jwk" }));
  show(`${tag} pkcs1 der`, () => crypto.createPrivateKey({ key: pkcs1(b), format: "der", type: "pkcs1" }));
  show(`${tag} pkcs1 pem`, () => crypto.createPrivateKey(pemOf(pkcs1(b), "RSA PRIVATE KEY")));
  show(`${tag} pkcs1 public`, () => crypto.createPublicKey(pemOf(pkcs1(b), "RSA PRIVATE KEY")));
  show(`${tag} pkcs1 sign`, () => crypto.sign("sha256", m, pemOf(pkcs1(b), "RSA PRIVATE KEY")));
  show(`${tag} pkcs8 der`, () => crypto.createPrivateKey({ key: pkcs8(b), format: "der", type: "pkcs8" }));
  show(`${tag} pkcs8 pem`, () => crypto.createPrivateKey(pemOf(pkcs8(b), "PRIVATE KEY")));
}
show("8192 still opens", () => crypto.createPrivateKey({ key: crypto.createPrivateKey(RSA8192).export({ format: "jwk" }), format: "jwk" }));"#;
    let lines = run_with_helpers(code).await;
    let mut expected = Vec::new();
    for (tag, bits) in [("8193", 8193), ("100k", 99_993)] {
        let refused =
            format!("Error Unsupported key: a {bits}-bit RSA key (supported: 1024 to 8192 bits)");
        let names = [
            "jwk n",
            "jwk d",
            "jwk p",
            "jwk q",
            "jwk dp",
            "jwk dq",
            "jwk qi",
            "jwk sign",
            "pkcs1 der",
            "pkcs1 pem",
            "pkcs1 public",
            "pkcs1 sign",
            "pkcs8 der",
            "pkcs8 pem",
        ];
        expected.extend(names.map(|name| format!("log:{tag} {name} {refused}")));
    }
    expected.push("log:8192 still opens ok private".to_string());
    assert_eq!(lines, expected);
}

/// Node's "Failed to read asymmetric key" at import for what is not a key: wrong lengths, off-curve points, or an extra member. Node words the PEM lines by OpenSSL's decoder; here they read the same as DER.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unreadable_public_keys_are_refused_at_import() {
    let code = r#"const m = Buffer.from("m"), b = (...x) => Buffer.from(x);
const P = (pem) => crypto.createPublicKey(pem).export({ type: "spki", format: "der" });
const pt256 = P(EC256_PUB).subarray(-65), pt384 = P(EC384_PUB).subarray(-97), pt521 = P(EC521_PUB).subarray(-133);
const flip = (p) => { const c = Buffer.from(p); c[c.length - 1] ^= 1; return c; };
const cases = [
  ["bits 04", spki(EC256_PUB, b(4))], ["empty", spki(EC256_PUB, b())], ["short", spki(EC256_PUB, pt256.subarray(0, 64))],
  ["long", spki(EC256_PUB, Buffer.concat([pt256, b(0)]))], ["off curve", spki(EC256_PUB, flip(pt256))],
  ["zero xy", spki(EC256_PUB, Buffer.concat([b(4), Buffer.alloc(64)]))], ["p384 off curve", spki(EC384_PUB, flip(pt384))],
  ["p521 off curve", spki(EC521_PUB, flip(pt521))], ["compressed x over p", spki(EC256_PUB, Buffer.concat([b(2), Buffer.alloc(32, 255)]))],
  ["trailing member", spki(EC256_PUB, pt256, 0, b(5, 0))],
  ["ed 31", spki(ED_PUB, Buffer.alloc(31, 7))], ["ed 33", spki(ED_PUB, Buffer.alloc(33, 7))], ["ed 0", spki(ED_PUB, b())],
  ["rsa junk", spki(RSA_PUB, b(4))],
];
for (const [tag, d] of cases) {
  show(tag, () => crypto.createPublicKey({ key: d, format: "der", type: "spki" }));
  show(tag + " pem", () => crypto.createPublicKey(pemOf(d, "PUBLIC KEY")));
  show(tag + " verify", () => crypto.verify("sha256", m, { key: d, format: "der", type: "spki" }, Buffer.alloc(8)));
}
show("good", () => crypto.createPublicKey({ key: spki(EC256_PUB, pt256), format: "der", type: "spki" }).export({ format: "jwk" }).crv);
show("compressed", () => crypto.createPublicKey(K.ec256_compressed_pub).type);
show("ed any bytes", () => crypto.createPublicKey({ key: spki(ED_PUB, Buffer.alloc(32, 255)), format: "der", type: "spki" }).export({ format: "jwk" }).x);"#;
    let mut expected = Vec::new();
    for tag in [
        "bits 04",
        "empty",
        "short",
        "long",
        "off curve",
        "zero xy",
        "p384 off curve",
        "p521 off curve",
        "compressed x over p",
        "trailing member",
        "ed 31",
        "ed 33",
        "ed 0",
        "rsa junk",
    ] {
        for suffix in ["", " pem", " verify"] {
            expected.push(format!(
                "log:{tag}{suffix} Error Failed to read asymmetric key"
            ));
        }
    }
    expected.extend(
        [
            "log:good ok P-256",
            "log:compressed ok public",
            "log:ed any bytes ok __________________________________________8",
        ]
        .map(String::from),
    );
    assert_eq!(run_with_helpers(code).await, expected);
}

/// Node opens the point at infinity and aborts the process exporting it, so here it stays out at import.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_point_at_infinity_is_refused_at_import() {
    let code = r#"show("identity", () => crypto.createPublicKey({ key: spki(EC256_PUB, Buffer.from([0])), format: "der", type: "spki" }));"#;
    assert_eq!(
        run_with_helpers(code).await,
        ["log:identity Error Failed to read asymmetric key"]
    );
}

/// Only an absent `format` means PEM: null and other falsy values are Node's error, and a KeyObject reads no format.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn null_and_falsy_formats_are_invalid() {
    let code = r#"const m = Buffer.from("m"), sig = Buffer.alloc(4), held = crypto.createPrivateKey(RSA);
for (const format of [null, "", 0, false, "PEM"]) {
  show("private", () => crypto.createPrivateKey({ key: RSA, format }));
  show("public", () => crypto.createPublicKey({ key: RSA_PUB, format }));
  show("sign", () => crypto.sign("sha256", m, { key: RSA, format }));
  show("verify", () => crypto.verify("sha256", m, { key: RSA_PUB, format }, sig));
  show("createSign", () => crypto.createSign("sha256").update(m).sign({ key: RSA, format }));
  show("createVerify", () => crypto.createVerify("sha256").update(m).verify({ key: RSA_PUB, format }, sig));
}
show("undefined", () => crypto.createPrivateKey({ key: RSA, format: undefined }));
show("held", () => crypto.createPublicKey({ key: held, format: null }));
show("held sign", () => crypto.sign("sha256", m, { key: held, format: null }));"#;
    let mut expected = Vec::new();
    for received in ["null", "''", "0", "false", "'PEM'"] {
        for tag in [
            "private",
            "public",
            "sign",
            "verify",
            "createSign",
            "createVerify",
        ] {
            expected.push(format!(
                "log:{tag} TypeError The property 'options.format' is invalid. Received {received}"
            ));
        }
    }
    expected.extend(
        [
            "log:undefined ok private",
            "log:held ok public",
            "log:held sign ok 256",
        ]
        .map(String::from),
    );
    assert_eq!(run_with_helpers(code).await, expected);
}

/// DER typed pkcs8 that is neither plain nor encrypted PKCS#8 is Node's "wrong tag" with OpenSSL's code dropped, and bytes after the plaintext of an encrypted key are ignored, as in Node.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_key_edges_follow_node() {
    let code = r#"const m = Buffer.from("m"), d = crypto.createPublicKey(RSA_PUB).export({ type: "spki", format: "der" }), e = crypto.createPublicKey(EC256_PUB).export({ type: "spki", format: "der" });
const o = { key: d, format: "der", type: "pkcs8" };
show("spki private", () => crypto.createPrivateKey(o));
show("spki private pw", () => crypto.createPrivateKey({ ...o, passphrase: "x" }));
show("spki public", () => crypto.createPublicKey(o));
show("spki sign", () => crypto.sign("sha256", m, o));
show("ec spki private", () => crypto.createPrivateKey({ ...o, key: e }));
const same = (k) => k.export({ type: "pkcs8", format: "pem" }) === RSA;
console.log(same(crypto.createPrivateKey({ key: K.rsa_trailing, passphrase: PW })), crypto.createPublicKey({ key: K.rsa_trailing, passphrase: PW }).export({ type: "spki", format: "pem" }) === RSA_PUB);
show("trailing wrong", () => crypto.createPrivateKey({ key: K.rsa_trailing, passphrase: "wrong" }));
show("sha1 none", () => crypto.createPrivateKey(K.rsa_sha1));"#;
    assert_eq!(
        run_with_helpers(code).await,
        [
            "log:spki private Error wrong tag",
            "log:spki private pw Error wrong tag",
            "log:spki public Error wrong tag",
            "log:spki sign Error wrong tag",
            "log:ec spki private Error wrong tag",
            "log:true true",
            "log:trailing wrong Error bad decrypt",
            "log:sha1 none Error interrupted or cancelled",
        ]
    );
}

/// PKCS#1 DER holds either half: `createPublicKey` and verify read a private key too, as they do for the other private forms.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_key_paths_read_pkcs1_private_der() {
    let code = r#"const m = Buffer.from("m"), der = (pem) => Buffer.from(pem.replace(/-----[^-]+-----|\s/g, ""), "base64");
const o = { key: der(RSA_PKCS1), format: "der", type: "pkcs1" };
const k = crypto.createPublicKey(o);
console.log(k.type, k.asymmetricKeyType, k.export({ type: "spki", format: "pem" }) === crypto.createPublicKey(RSA_PKCS1).export({ type: "spki", format: "pem" }), crypto.verify("sha256", m, o, crypto.sign("sha256", m, RSA_PKCS1)));
console.log(crypto.createPublicKey({ key: k.export({ type: "spki", format: "der" }), format: "der", type: "spki" }).type, crypto.createPublicKey({ key: der(RSA_PKCS1_PUB), format: "der", type: "pkcs1" }).type);"#;
    assert_eq!(
        run_with_helpers(code).await,
        ["log:public rsa true true", "log:public public"]
    );
}

/// `encoding` decodes a string key and a string passphrase, in every place a key is taken, and has no effect on bytes, JWK or a KeyObject.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_encoding_applies_to_key_and_passphrase_strings() {
    let code = r#"const m = Buffer.from("m"), H = (s) => Buffer.from(s).toString("hex"), B = (s) => Buffer.from(s).toString("base64"), U = (s) => Buffer.from(s).toString("base64url");
const type = (k) => crypto.createPrivateKey(k).type, sig = crypto.sign("sha256", m, RSA);
console.log(type({ key: H(RSA), encoding: "hex" }), type({ key: B(RSA), encoding: "base64" }), type({ key: U(RSA), encoding: "base64url" }), type({ key: H(RSA), encoding: "HEX" }), type({ key: RSA, encoding: "utf8" }), type({ key: RSA, encoding: "latin1" }));
console.log(type({ key: RSA, encoding: "buffer" }), type({ key: RSA, encoding: "" }), type({ key: RSA, encoding: null }), type({ key: RSA, encoding: 1 }), type({ key: RSA, encoding: {} }), type({ key: Buffer.from(RSA), encoding: "hex" }));
console.log(type({ key: H(K.rsa_aes256), encoding: "hex", passphrase: H(PW) }), type({ key: Buffer.from(K.rsa_aes256), encoding: "hex", passphrase: H(PW) }), type({ key: Buffer.from(K.rsa_aes256), encoding: "hex", passphrase: Buffer.from(PW) }), type({ key: K.rsa_bytes, encoding: "latin1", passphrase: "\xff\x00\x80" }), type({ key: K.rsa_aes256, encoding: "buffer", passphrase: PW }));
const der = crypto.createPrivateKey(RSA).export({ type: "pkcs8", format: "der" }), spkiDer = crypto.createPublicKey(RSA_PUB).export({ type: "spki", format: "der" });
console.log(type({ key: der.toString("hex"), format: "der", type: "pkcs8", encoding: "hex" }), crypto.createPublicKey({ key: spkiDer.toString("base64"), format: "der", type: "spki", encoding: "base64" }).type, crypto.createPublicKey({ key: H(RSA_PUB), encoding: "hex" }).type);
console.log(crypto.sign("sha256", m, { key: H(RSA), encoding: "hex" }).equals(sig), crypto.sign("sha256", m, { key: H(K.rsa_aes256), encoding: "hex", passphrase: H(PW) }).equals(sig), crypto.verify("sha256", m, { key: H(RSA_PUB), encoding: "hex" }, sig), crypto.createSign("sha256").update(m).sign({ key: H(RSA), encoding: "hex" }).equals(sig), crypto.createVerify("sha256").update(m).verify({ key: H(RSA_PUB), encoding: "hex" }, sig));
const held = crypto.createPrivateKey(RSA);
console.log(crypto.createPublicKey({ key: held, encoding: "nope" }).type, crypto.sign("sha256", m, { key: held, encoding: "hex" }).equals(sig), crypto.createPrivateKey({ key: held.export({ format: "jwk" }), format: "jwk", encoding: "nope" }).type);
show("unknown", () => crypto.createPrivateKey({ key: RSA, encoding: "nope" }));
show("unknown sign", () => crypto.sign("sha256", m, { key: RSA, encoding: "nope" }));
show("unknown passphrase", () => crypto.createPrivateKey({ key: Buffer.from(RSA), encoding: "nope", passphrase: "x" }));
show("unknown bytes", () => crypto.createPrivateKey({ key: Buffer.from(RSA), encoding: "nope" }));
show("not decoded", () => crypto.createPrivateKey({ key: K.rsa_aes256, passphrase: H(PW) }));"#;
    assert_eq!(
        run_with_helpers(code).await,
        [
            "log:private private private private private private",
            "log:private private private private private private",
            "log:private private private private private",
            "log:private public public",
            "log:true true true true true",
            "log:public true private",
            "log:unknown TypeError Unknown encoding: nope",
            "log:unknown sign TypeError Unknown encoding: nope",
            "log:unknown passphrase TypeError Unknown encoding: nope",
            "log:unknown bytes ok private",
            "log:not decoded Error bad decrypt",
        ]
    );
}

/// A corpus line at the fidelity of this file: an error's name, code and message, except that
/// OpenSSL's own errors read as the DER path's message and have no code.
fn comparable(line: &str) -> String {
    let Some((head, message)) = line.split_once(" | ") else {
        return line.to_string();
    };
    let (name, code) = head.rsplit_once(' ').expect("an error line has a code");
    if message.starts_with("error:") {
        return format!("{name} undefined | Failed to read asymmetric key");
    }
    format!("{name} {code} | {message}")
}

/// Every line of `malformed.js` as Node 24 answered it; `gen.js` records `malformed-node.txt`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_keys_get_nodes_answers() {
    let ours: Vec<String> = run_keys(MALFORMED)
        .await
        .iter()
        .map(|line| comparable(line.strip_prefix("log:").unwrap_or(line)))
        .collect();
    let node: Vec<String> = NODE_ANSWERS.lines().map(comparable).collect();
    assert_eq!(ours.len(), node.len());
    let wrong: Vec<String> = ours
        .iter()
        .zip(&node)
        .filter(|(ours, node)| ours != node)
        .map(|(ours, node)| format!("ours {ours}\nnode {node}"))
        .collect();
    assert!(
        wrong.is_empty(),
        "{} of {} differ:\n{}",
        wrong.len(),
        node.len(),
        wrong.join("\n")
    );
}

/// `{ key, passphrase }` opens as `createPrivateKey` opens it, in each form that takes; RSA signatures are deterministic, so the tokens equal the plain key's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwt_signs_with_encrypted_keys_like_node() {
    let code = r#"const P = { sub: "x", iat: 1700000000 }, E = K.rsa_aes256, H = (s) => Buffer.from(s).toString("hex");
const round = (tag, key, pub, algorithm, plain) => show(tag, () => { const t = jwt.sign(P, key, { algorithm }); return `${Buffer.from(t.split(".")[0], "base64url")} ${jwt.verify(t, pub, { algorithms: [algorithm] }).sub} ${plain === undefined || t === jwt.sign(P, plain, { algorithm })}`; });
for (const a of ["RS256", "RS384", "RS512"]) round(a, { key: E, passphrase: PW }, RSA_PUB, a, RSA);
for (const a of ["PS256", "PS384", "PS512"]) round(a, { key: K.rsa_aes128, passphrase: PW }, RSA_PUB, a);
round("scrypt", { key: K.rsa_scrypt, passphrase: PW }, RSA_PUB, "RS256", RSA);
round("sha512 prf", { key: K.rsa_sha512, passphrase: PW }, RSA_PUB, "RS256", RSA);
round("es256", { key: K.ec256_aes192, passphrase: PW }, EC256_PUB, "ES256");
round("es384", { key: K.ec384_aes256, passphrase: PW }, EC384_PUB, "ES384");
round("key bytes", { key: Buffer.from(E), passphrase: PW }, RSA_PUB, "RS256", RSA);
round("passphrase bytes", { key: E, passphrase: Buffer.from(PW) }, RSA_PUB, "RS256", RSA);
round("empty passphrase", { key: K.rsa_empty, passphrase: "" }, RSA_PUB, "RS256", RSA);
round("hex", { key: H(E), encoding: "hex", passphrase: H(PW) }, RSA_PUB, "RS256", RSA);
round("der", { key: Buffer.from(K.rsa_der, "base64"), format: "der", type: "pkcs8", passphrase: PW }, RSA_PUB, "RS256", RSA);
round("plain key", { key: RSA, passphrase: "ignored" }, RSA_PUB, "RS256", RSA);
jwt.sign(P, { key: E, passphrase: PW }, { algorithm: "RS256" }, (e, t) => console.log("callback", e, jwt.verify(t, RSA_PUB).sub));"#;
    assert_eq!(
        run_with_helpers(code).await,
        [
            r#"log:RS256 ok {"alg":"RS256","typ":"JWT"} x true"#,
            r#"log:RS384 ok {"alg":"RS384","typ":"JWT"} x true"#,
            r#"log:RS512 ok {"alg":"RS512","typ":"JWT"} x true"#,
            r#"log:PS256 ok {"alg":"PS256","typ":"JWT"} x true"#,
            r#"log:PS384 ok {"alg":"PS384","typ":"JWT"} x true"#,
            r#"log:PS512 ok {"alg":"PS512","typ":"JWT"} x true"#,
            r#"log:scrypt ok {"alg":"RS256","typ":"JWT"} x true"#,
            r#"log:sha512 prf ok {"alg":"RS256","typ":"JWT"} x true"#,
            r#"log:es256 ok {"alg":"ES256","typ":"JWT"} x true"#,
            r#"log:es384 ok {"alg":"ES384","typ":"JWT"} x true"#,
            r#"log:key bytes ok {"alg":"RS256","typ":"JWT"} x true"#,
            r#"log:passphrase bytes ok {"alg":"RS256","typ":"JWT"} x true"#,
            r#"log:empty passphrase ok {"alg":"RS256","typ":"JWT"} x true"#,
            r#"log:hex ok {"alg":"RS256","typ":"JWT"} x true"#,
            r#"log:der ok {"alg":"RS256","typ":"JWT"} x true"#,
            r#"log:plain key ok {"alg":"RS256","typ":"JWT"} x true"#,
            "log:callback null x",
        ]
    );
}

/// jsonwebtoken 9.0.3 turns any failure to open a key object into one message, a wrong passphrase included.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwt_passphrase_failures_follow_node() {
    let code = r#"const P = { sub: "x" }, E = K.rsa_aes256;
const bad = (tag, key, algorithm = "RS256") => show(tag, () => jwt.sign(P, key, { algorithm }));
bad("wrong", { key: E, passphrase: "wrong" });
bad("wrong empty", { key: E, passphrase: "" });
bad("wrong bytes", { key: E, passphrase: Buffer.from("wrong") });
bad("wrong hs", { key: E, passphrase: "wrong" }, "HS256");
bad("wrong es", { key: K.ec256_aes192, passphrase: "wrong" }, "ES256");
bad("number", { key: E, passphrase: 1 });
bad("null", { key: E, passphrase: null });
bad("plain number", { key: RSA, passphrase: 1 });
bad("undefined", { key: E, passphrase: undefined });
bad("none", { key: E });
bad("bare", E);
bad("secret", { key: "secret", passphrase: "x" }, "HS256");
bad("no key", { passphrase: "x" });
bad("public", { key: RSA_PUB, passphrase: "x" });
bad("flipped", { key: K.bad_flipped, passphrase: PW });
bad("truncated", { key: K.bad_truncated, passphrase: PW });
bad("plain hs", { key: RSA, passphrase: "x" }, "HS256");
bad("ec as rs", { key: K.ec256_aes192, passphrase: PW });
const ed = { key: K.ed_aes256, passphrase: PW };
for (const a of ["HS256", "HS512", "RS256", "PS256", "ES256"]) bad("ed " + a, ed, a);
bad("plain ed hs", { key: ED_PRIV, passphrase: "x" }, "HS256");
jwt.sign(P, { key: E, passphrase: "wrong" }, { algorithm: "RS256" }, (e, t) => console.log("callback", e.name, e.message, t));"#;
    assert_eq!(
        run_with_helpers(code).await,
        [
            "log:wrong Error secretOrPrivateKey is not valid key material",
            "log:wrong empty Error secretOrPrivateKey is not valid key material",
            "log:wrong bytes Error secretOrPrivateKey is not valid key material",
            "log:wrong hs Error secretOrPrivateKey is not valid key material",
            "log:wrong es Error secretOrPrivateKey is not valid key material",
            "log:number Error secretOrPrivateKey is not valid key material",
            "log:null Error secretOrPrivateKey is not valid key material",
            "log:plain number Error secretOrPrivateKey is not valid key material",
            "log:undefined Error secretOrPrivateKey is not valid key material",
            "log:none Error secretOrPrivateKey is not valid key material",
            "log:bare Error Unsupported key: text with a PEM marker is never a secret, and this is no key that opens here (supported: RSA, ECDSA P-256, P-384 or P-521, and Ed25519)",
            "log:secret Error secretOrPrivateKey is not valid key material",
            "log:no key Error secretOrPrivateKey is not valid key material",
            "log:public Error secretOrPrivateKey is not valid key material",
            "log:flipped Error secretOrPrivateKey is not valid key material",
            "log:truncated Error secretOrPrivateKey is not valid key material",
            "log:plain hs Error secretOrPrivateKey must be a symmetric key when using HS256",
            r#"log:ec as rs Error "alg" parameter for "ec" key type must be one of: ES256, ES384, ES512."#,
            "log:ed HS256 Error secretOrPrivateKey must be a symmetric key when using HS256",
            "log:ed HS512 Error secretOrPrivateKey must be a symmetric key when using HS512",
            r#"log:ed RS256 Error Unknown key type "ed25519"."#,
            r#"log:ed PS256 Error Unknown key type "ed25519"."#,
            r#"log:ed ES256 Error Unknown key type "ed25519"."#,
            "log:plain ed hs Error secretOrPrivateKey must be a symmetric key when using HS256",
            "log:callback Error secretOrPrivateKey is not valid key material undefined",
        ]
    );
}

/// The jwt path meets crypto's refusals: KDF parameters stop at the work caps before any derivation, and schemes the crates cannot read are named rather than called a bad key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwt_keeps_cryptos_refusals_and_work_caps() {
    let code = r#"for (const n of ["hostile_iter", "hostile_iter0", "hostile_iter_max", "hostile_n_huge", "hostile_n_mem", "hostile_n_zero", "hostile_n_three", "hostile_n_one", "hostile_r_zero", "hostile_p_zero", "hostile_rp", "hostile_salt", "rsa_camellia", "rsa_pbes1", "rsa_legacy", "pss_enc"]) {
  show(n, () => jwt.sign({}, { key: K[n], passphrase: PW }, { algorithm: "RS256" }));
}"#;
    const PBKDF2: &str = "Error Unsupported key: the encrypted key's PBKDF2 work is over the limit (iterations 1 to 10000000, iterations x ceil(key length / digest length) at most 16777216)";
    const SCRYPT: &str = "Error Unsupported key: the encrypted key's scrypt work is over the limit (N a power of two above 1, r and p from 1, N x r x p at most 33554432, 256 MiB of memory)";
    let mut expected: Vec<String> = ["hostile_iter", "hostile_iter0", "hostile_iter_max"]
        .map(|n| format!("log:{n} {PBKDF2}"))
        .to_vec();
    expected.extend(
        [
            "n_huge", "n_mem", "n_zero", "n_three", "n_one", "r_zero", "p_zero", "rp",
        ]
        .map(|n| format!("log:hostile_{n} {SCRYPT}")),
    );
    expected.extend(
        [
            "log:hostile_salt Error Unsupported key: an encrypted key with an unreadable or unsupported PBES2 scheme (supported: PBES2 with PBKDF2 or scrypt and AES-CBC, AES-GCM or DES-EDE3-CBC)",
            "log:rsa_camellia Error Unsupported key: an encrypted key using Camellia (supported: PBES2 with PBKDF2 or scrypt and AES-CBC, AES-GCM or DES-EDE3-CBC)",
            "log:rsa_pbes1 Error Unsupported key: an encrypted key using a PBES1 scheme (supported: PBES2 with PBKDF2 or scrypt and AES-CBC, AES-GCM or DES-EDE3-CBC)",
            "log:rsa_legacy Error Unsupported key: a legacy encrypted PEM (Proc-Type: 4,ENCRYPTED) (convert it to encrypted PKCS#8)",
            "log:pss_enc Error \"alg\" parameter for \"rsa-pss\" key type must be one of: PS256, PS384, PS512.",
        ]
        .map(String::from),
    );
    assert_eq!(run_with_helpers(code).await, expected);
}

/// Keys Node opens and the shim cannot hold are named on the jwt path, whatever the refusal; only a key that will not open reads as bad material.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwt_names_keys_node_opens_and_the_shim_refuses() {
    let code = r#"const J = (pem) => crypto.createPrivateKey(pem).export({ format: "jwk" }), rsa = J(RSA), ec = J(EC256_SEC1);
const named = (tag, key, algorithm = "RS256") => show(tag, () => jwt.sign({}, key, { algorithm }));
named("8200 bits", { key: K.rsa_8200_enc, passphrase: PW });
named("8200 bits wrong", { key: K.rsa_8200_enc, passphrase: "wrong" });
named("jwk curve", { key: K.jwk_ed448, format: "jwk", passphrase: "x" });
named("jwk mismatch", { key: { ...ec, d: K.jwk_other_ec.d }, format: "jwk", passphrase: "x" }, "ES256");
named("jwk rsa", { key: { ...rsa, d: "AQAB" }, format: "jwk", passphrase: "x" });
named("jwk invalid", { key: { kty: "EC", crv: "P-256", x: "AA", y: "AA", d: "AA" }, format: "jwk", passphrase: "x" }, "ES256");"#;
    assert_eq!(
        run_with_helpers(code).await,
        [
            "log:8200 bits Error Unsupported key: a 8200-bit RSA key (supported: 1024 to 8192 bits)",
            "log:8200 bits wrong Error secretOrPrivateKey is not valid key material",
            "log:jwk curve Error Unsupported key: JWK curve \"Ed448\" (supported: P-256, P-384, P-521 and Ed25519)",
            "log:jwk mismatch Error Unsupported key: the JWK's private and public values do not match",
            "log:jwk rsa Error Unsupported key: the JWK's RSA values do not form a usable key",
            "log:jwk invalid Error secretOrPrivateKey is not valid key material",
        ]
    );
}

/// Damaged keys fail as errors on the jwt path too, and no error carries a passphrase or key bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwt_failures_never_panic_or_leak() {
    let code = r#"const der = Buffer.from(K.rsa_der, "base64"), o = { algorithm: "RS256" };
const pem = (b) => pemOf(b, "ENCRYPTED PRIVATE KEY");
let failed = 0;
const attempt = (key) => { try { jwt.sign({}, key, o); } catch (e) { failed += 1; console.log(e.name, e.message, String(e.stack)); } };
for (let n = 0; n < der.length; n += 53) attempt({ key: pem(der.subarray(0, n)), passphrase: "SENTINELPASS" });
for (let i = 0; i < 60; i += 3) { const d = Buffer.from(der); d[i] ^= 0x80; attempt({ key: pem(d), passphrase: PW }); }
for (const passphrase of ["SENTINELPASS", Buffer.from("SENTINELPASS"), 73577357, { toString() { return "SENTINELPASS"; } }, ["SENTINELPASS"]]) {
  for (const key of [K.rsa_aes256, K.rsa_sha1, K.hostile_iter, pem(Buffer.from("SENTINELKEYBYTES"))]) attempt({ key, passphrase });
}
console.log("failed", failed > 40);"#;
    let lines = run_with_helpers(code).await;
    assert_eq!(lines.last().map(String::as_str), Some("log:failed true"));
    for line in lines {
        for sentinel in ["SENTINEL", "U0VOVEl", "73577357", "53454e54"] {
            assert!(!line.contains(sentinel), "{line}");
        }
    }
}

/// `asymmetricKeyDetails` of private and public KeyObjects matches Node 24, including the BigInt `publicExponent`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn asymmetric_key_details_match_node() {
    let keys = [
        ("RSA", include_str!("fixtures/jwt/rsa.pem")),
        ("RSA_PUB", include_str!("fixtures/jwt/rsa.pub.pem")),
        ("RSA512", include_str!("fixtures/rsa-sizes/rsa512.pem")),
        (
            "RSA1024_PUB",
            include_str!("fixtures/rsa-sizes/rsa1024.pub.pem"),
        ),
        ("RSA8192", include_str!("fixtures/rsa-sizes/rsa8192.pem")),
        ("EC256", include_str!("fixtures/jwt/ec256-sec1.pem")),
        ("EC256_PUB", include_str!("fixtures/jwt/ec256.pub.pem")),
        ("EC384", include_str!("fixtures/jwt/ec384.pem")),
        ("EC384_PUB", include_str!("fixtures/jwt/ec384.pub.pem")),
        ("EC521", include_str!("fixtures/jwt/ec521.pem")),
        ("EC521_PUB", include_str!("fixtures/jwt/ec521.pub.pem")),
        // The 2048-bit key as PKCS#8 with an empty `[0]` attributes field after the key.
        (
            "RSA_ATTRS",
            include_str!("fixtures/crypto-keys/rsa-attrs.pkcs8.pem"),
        ),
    ];
    let mut code = String::from("const P = {};\n");
    for (name, pem) in keys {
        code.push_str(&format!("P.{name} = {pem:?};\n"));
    }
    code.push_str(
        r#"const ED = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----\n";
const show = (k) => JSON.stringify(k.asymmetricKeyDetails, (_, v) => (typeof v === "bigint" ? v + "n" : v));
for (const n of Object.keys(P)) {
  const k = n.endsWith("PUB") ? crypto.createPublicKey(P[n]) : crypto.createPrivateKey(P[n]);
  console.log(n, k.type, k.asymmetricKeyType, show(k));
}
const ed = crypto.createPrivateKey(ED);
console.log("ED", show(ed), show(crypto.createPublicKey(ed)));
const rsa = crypto.createPrivateKey(P.RSA).asymmetricKeyDetails;
const same = (k) => k.asymmetricKeyDetails === k.asymmetricKeyDetails;
console.log(same(crypto.createPrivateKey(P.RSA)), same(crypto.createPublicKey(P.EC384_PUB)), same(ed));
console.log(typeof rsa.publicExponent, show(crypto.createPublicKey(crypto.createPrivateKey(P.RSA))), show(crypto.createSecretKey(Buffer.from("a"))));"#,
    );
    assert_eq!(
        run_crypto(&code).await,
        [
            r#"log:RSA private rsa {"modulusLength":2048,"publicExponent":"65537n"}"#,
            r#"log:RSA_PUB public rsa {"modulusLength":2048,"publicExponent":"65537n"}"#,
            r#"log:RSA512 private rsa {"modulusLength":512,"publicExponent":"65537n"}"#,
            r#"log:RSA1024_PUB public rsa {"modulusLength":1024,"publicExponent":"65537n"}"#,
            r#"log:RSA8192 private rsa {"modulusLength":8192,"publicExponent":"65537n"}"#,
            r#"log:EC256 private ec {"namedCurve":"prime256v1"}"#,
            r#"log:EC256_PUB public ec {"namedCurve":"prime256v1"}"#,
            r#"log:EC384 private ec {"namedCurve":"secp384r1"}"#,
            r#"log:EC384_PUB public ec {"namedCurve":"secp384r1"}"#,
            r#"log:EC521 private ec {"namedCurve":"secp521r1"}"#,
            r#"log:EC521_PUB public ec {"namedCurve":"secp521r1"}"#,
            r#"log:RSA_ATTRS private rsa {"modulusLength":2048,"publicExponent":"65537n"}"#,
            "log:ED {} {}",
            "log:true true false",
            r#"log:bigint {"modulusLength":2048,"publicExponent":"65537n"} undefined"#,
        ]
    );
}
