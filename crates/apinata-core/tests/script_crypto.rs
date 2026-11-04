//! Node's `crypto` beyond hashing: known vectors and Node 24 outputs, pasted once from
//! `node` runs of the same code, plus the shim's errors and unsupported members.

use crate::script_support;

use apinata_core::Outcome;
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEYS: [(&str, &str); 23] = [
    // RSA keys of other sizes, made by Node 24, and its sha256 PSS signatures of "msg" (maximum salt).
    ("RSA512", include_str!("fixtures/rsa-sizes/rsa512.pem")),
    (
        "RSA512_PUB",
        include_str!("fixtures/rsa-sizes/rsa512.pub.pem"),
    ),
    ("RSA1024", include_str!("fixtures/rsa-sizes/rsa1024.pem")),
    (
        "RSA1024_PKCS1",
        include_str!("fixtures/rsa-sizes/rsa1024-pkcs1.pem"),
    ),
    (
        "RSA1024_PUB",
        include_str!("fixtures/rsa-sizes/rsa1024.pub.pem"),
    ),
    ("RSA8192", include_str!("fixtures/rsa-sizes/rsa8192.pem")),
    (
        "RSA8192_PUB",
        include_str!("fixtures/rsa-sizes/rsa8192.pub.pem"),
    ),
    (
        "PSS1024",
        include_str!("fixtures/rsa-sizes/rsa1024.pss-sha256.b64"),
    ),
    (
        "PSS8192",
        include_str!("fixtures/rsa-sizes/rsa8192.pss-sha256.b64"),
    ),
    ("RSA", include_str!("fixtures/jwt/rsa.pem")),
    ("RSA_PKCS1", include_str!("fixtures/jwt/rsa-pkcs1.pem")),
    ("RSA_PUB", include_str!("fixtures/jwt/rsa.pub.pem")),
    ("RSA_CRT", include_str!("fixtures/jwt/rsa.crt")),
    ("EC256_SEC1", include_str!("fixtures/jwt/ec256-sec1.pem")),
    ("EC256_PUB", include_str!("fixtures/jwt/ec256.pub.pem")),
    ("EC384", include_str!("fixtures/jwt/ec384.pem")),
    ("EC384_PUB", include_str!("fixtures/jwt/ec384.pub.pem")),
    // The RFC 8032 section 7.1 test 1 key as PKCS#8 and SPKI.
    (
        "ED_PRIV",
        "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----\n",
    ),
    (
        "ED_PUB",
        "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=\n-----END PUBLIC KEY-----\n",
    ),
    // rsa.pub.pem re-exported by Node as PKCS#1.
    (
        "RSA_PKCS1_PUB",
        "-----BEGIN RSA PUBLIC KEY-----\nMIIBCgKCAQEA0OnqCWWmbNHE15DgSlXdDx4Y6CGI5XKdv1zANsuAoMP6vPZMrCpO\nVC7c41bzsxxjxaai19htXiKN9FxRIxgB0HUoFJvFsyxE6XFoUJ2dZwRyvRGVKbAH\nYA1WNkglJzT81pmwZPNdcA2nsj1eg4cPMfaZWYZr+fjzHsv0u7wOqneR/Rb6Gg14\nD9ke5CdKHHxV8ohysBL7/8RaQ/7RmUrc4rMW8bUz4YcudRvYg4Agi6sn6B4n4mXn\nSYu/Nlc5hxhoxHG4Z+SzR0aXiibx6uKKXGeYF1L0w+sOLM5uwOsh7N4XpQ782jf5\nrVbTOmY4jpbne1l84uwLqMkH91jU/bIegQIDAQAB\n-----END RSA PUBLIC KEY-----\n",
    ),
    // A P-521 key pair and a secp256k1 public key made with openssl 3.6.
    (
        "EC521",
        "-----BEGIN PRIVATE KEY-----\nMIHuAgEAMBAGByqGSM49AgEGBSuBBAAjBIHWMIHTAgEBBEIAP7fpaoO7H0Ou/sIV\nkdOC+nV6HOVHhNfMChvCz5WiqaBx6uliiM43b5O+DmwvnvCo1ZG61j4mpNhpF1vD\n51k1Q5ehgYkDgYYABAGIghG8jT5TtlyKyBqicRi4OfJgaaQi4YOxn7t/jKsoFHVP\n2PdKc0DmIUOmTfGwDaX4JGv3UGE7L9MiP9Hl7lKimwAwoL+GqPZTf0WjFC2PUsST\nIMkp7cib6G9QPN2O3IbwOM08Imu0+iPoBH+1UcXN1PnAvYiJWX/PdrA/AHEGwaNH\nQQ==\n-----END PRIVATE KEY-----\n",
    ),
    (
        "EC521_PUB",
        "-----BEGIN PUBLIC KEY-----\nMIGbMBAGByqGSM49AgEGBSuBBAAjA4GGAAQBiIIRvI0+U7ZcisgaonEYuDnyYGmk\nIuGDsZ+7f4yrKBR1T9j3SnNA5iFDpk3xsA2l+CRr91BhOy/TIj/R5e5SopsAMKC/\nhqj2U39FoxQtj1LEkyDJKe3Im+hvUDzdjtyG8DjNPCJrtPoj6AR/tVHFzdT5wL2I\niVl/z3awPwBxBsGjR0E=\n-----END PUBLIC KEY-----\n",
    ),
    (
        "K1_PUB",
        "-----BEGIN PUBLIC KEY-----\nMFYwEAYHKoZIzj0CAQYFK4EEAAoDQgAEw9BllVI+nPWEURGHgzEjWGv/FcPQDzCk\nYJdY+HEpVigHuX2aISU4DOEM/yrA0JnMQl+QGut2fTAhCoU+twNtKQ==\n-----END PUBLIC KEY-----\n",
    ),
];

/// Runs `code` as a tests script with the keys defined and returns its console lines.
pub(crate) async fn run_crypto(code: &str) -> Vec<String> {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let mut full: String = KEYS
        .iter()
        .map(|(n, k)| format!("const {n} = {};\n", serde_json::to_string(k).expect("json")))
        .collect();
    full.push_str(code);
    let text = scripted("R", &format!("{}/r", s.uri()), &script("tests", &full), "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", tests(&result));
    console(&result)
}

/// The suffix every "Signature algorithm not supported" message ends with.
const SIG_SUPPORTED: &str = "(supported: RSA with sha1, sha256, sha384 or sha512; RSA-PSS with sha256, sha384 or sha512; ECDSA P-256, P-384 or P-521 with sha1, sha256, sha384 or sha512; Ed25519 with null; require('crypto-js') has more)";

/// The console line for an unsupported signature scheme named `what`.
fn unsupported_sig(what: &str) -> String {
    format!("log:Error Signature algorithm not supported: \"{what}\" {SIG_SUPPORTED}")
}

/// Node 24 outputs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timing_safe_equal_hash_copy_and_one_shot_hash() {
    let code = r#"const t = crypto.timingSafeEqual;
console.log(t(Buffer.from("abc"), Buffer.from("abc")), t(Buffer.from("abc"), Buffer.from("abd")), t(new Uint8Array([1, 2]), new DataView(new Uint8Array([1, 2]).buffer)), t(new Uint8Array([1, 2]).buffer, Buffer.from([1, 2])), t(Buffer.alloc(0), Buffer.alloc(0)), t(new Uint16Array([258]), Buffer.from([2, 1])));
const h = crypto.createHash("sha256").update("a");
const c = h.copy();
c.update("bc");
const d = c.copy();
console.log(h.digest("hex"), c.digest("hex"), d.update("d").digest("hex"), typeof crypto.createHmac("sha1", "k").copy);
console.log(crypto.hash("sha1", "abc"), crypto.hash("md5", Buffer.from("abc"), "base64"), Buffer.isBuffer(crypto.hash("sha256", "abc", "buffer")), crypto.hash("SHA512", new Uint8Array([97, 98, 99])).slice(0, 16));
console.log(["md5", "sha1", "sha224", "sha256", "sha384", "sha512"].every((a) => crypto.getHashes().includes(a)), ["aes-128-cbc", "aes-192-cbc", "aes-256-cbc", "aes-128-gcm", "aes-192-gcm", "aes-256-gcm"].every((a) => crypto.getCiphers().includes(a)));"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:true false true true true true",
            "log:ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad 88d4266fd4e6338d13b845fcf289579d209c897823b9217da3e161936f031589 undefined",
            "log:a9993e364706816aba3e25717850c26c9cd0d89d kAFQmDzST7DWlj99KOF/cg== true ddaf35a193617aba",
            "log:true true",
        ]
    );
}

/// RFC 6070 (PBKDF2-SHA1) and RFC 7914 sections 11 and 12 (scrypt, PBKDF2-SHA256); the rest is Node 24 output.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pbkdf2_and_scrypt_match_rfc_vectors() {
    let code = r#"const p = (...a) => crypto.pbkdf2Sync(...a).toString("hex");
console.log(p("password", "salt", 1, 20, "sha1"));
console.log(p("password", "salt", 2, 20, "sha1"));
console.log(p("password", "salt", 4096, 20, "sha1"));
console.log(p("passwordPASSWORDpassword", "saltSALTsaltSALTsaltSALTsaltSALTsalt", 4096, 25, "sha1"));
console.log(p(Buffer.from("pass\0word"), new Uint8Array(Buffer.from("sa\0lt")), 4096, 16, "SHA1"));
console.log(p("passwd", "salt", 1, 64, "sha256"));
console.log(p("Password", "NaCl", 80000, 64, "sha256"));
console.log(p("k", "s", 1, 48, "sha384"), p("k", "s", 1, 64, "sha512").slice(0, 32), Buffer.isBuffer(crypto.pbkdf2Sync("k", "s", 1, 1, "sha256")));
const s = (...a) => crypto.scryptSync(...a).toString("hex");
console.log(s("", "", 64, { N: 16, r: 1, p: 1 }));
console.log(s("password", "NaCl", 64, { N: 1024, r: 8, p: 16 }));
console.log(s("pleaseletmein", "SodiumChloride", 64, { cost: 16384, blockSize: 8, parallelization: 1 }));
console.log(s("k", "s", 16), s(Buffer.from("k"), Buffer.from("s"), 16, {}), crypto.scryptSync("k", "s", 0).length, s("k", "s", 8, { N: 16, r: 1, p: 1, maxmem: 2432 }));
crypto.pbkdf2("password", "salt", 1, 20, "sha1", (err, key) => {
  console.log("pbkdf2", err, key.toString("hex"));
  crypto.scrypt("password", "NaCl", 16, { N: 1024, r: 8, p: 16 }, (e, k) => {
    console.log("scrypt", e, k.toString("hex"));
    crypto.scrypt("k", "s", 16, (e2, k2) => console.log("scrypt", e2, k2.toString("hex")));
  });
});
console.log("sync first");"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:0c60c80f961f0e71f3a9b524af6012062fe037a6",
            "log:ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957",
            "log:4b007901b765489abead49d926f721d065a429c1",
            "log:3d2eec4fe41c849b80c8d83662c0e44a8b291a964cf2f07038",
            "log:56fa6aa75548099dcc37d7f03425e0c3",
            "log:55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783",
            "log:4ddcd8f60b98be21830cee5ef22701f9641a4418d04c0414aeff08876b34ab56a1d425a1225833549adb841b51c9b3176a272bdebba1d078478f62b397f33c8d",
            "log:bbb64b95101b82540734df0a2700b81724649b8805efb1b79d00f3a7343062441d90f0e341173ba518675d4b65f2f389 a16f740f23d612384f9ac29eced1b585 true",
            "log:77d6576238657b203b19ca42c18a0497f16b4844e3074ae8dfdffa3fede21442fcd0069ded0948f8326a753a0fc81f17e8d3e0fb2e0d3628cf35e20c38d18906",
            "log:fdbabe1c9d3472007856e7190d01e9fe7c6ad7cbc8237830e77376634b3731622eaf30d92e22a3886ff109279d9830dac727afb94a83ee6d8360cbdfa2cc0640",
            "log:7023bdcb3afd7348461c06cd81fd38ebfda8fbba904f8e3ea9b543f6545da1f2d5432955613f0fcf62d49705242a9af9e61e85dc0d651e40dfcf017b45575887",
            "log:ee9e793ad67ad23660821ba8526dffc4 ee9e793ad67ad23660821ba8526dffc4 0 db44e926f9edffe4",
            "log:sync first",
            "log:pbkdf2 null 0c60c80f961f0e71f3a9b524af6012062fe037a6",
            "log:scrypt null fdbabe1c9d3472007856e7190d01e9fe",
            "log:scrypt null ee9e793ad67ad23660821ba8526dffc4",
        ]
    );
}

/// NIST SP 800-38A F.2 (CBC) and the GCM spec test cases 4, 10 and 16; padded outputs are Node 24's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aes_cbc_and_gcm_match_nist_vectors() {
    let code = r#"const hex = (s) => Buffer.from(s, "hex");
const IV = hex("000102030405060708090a0b0c0d0e0f");
const K128 = hex("2b7e151628aed2a6abf7158809cf4f3c");
const K192 = hex("8e73b0f7da0e6452c810f32b809079e562f8ead2522c6b7b");
const K256 = hex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
const PT = hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710");
const run = (make, alg, key, iv, data, pad = true) => {
  const c = make(alg, key, iv);
  if (!pad) c.setAutoPadding(false);
  return Buffer.concat([c.update(data), c.final()]).toString("hex");
};
const enc = (...a) => run(crypto.createCipheriv, ...a);
const dec = (...a) => run(crypto.createDecipheriv, ...a);
console.log(enc("aes-128-cbc", K128, IV, PT, false));
console.log(enc("aes-192-cbc", K192, IV, PT, false));
console.log(enc("aes-256-cbc", K256, IV, PT, false));
console.log(dec("aes-256-cbc", K256, IV, hex(enc("aes-256-cbc", K256, IV, PT, false)), false) === PT.toString("hex"), dec("AES-128-CBC", K128, IV, hex(enc("aes-128-cbc", K128, IV, PT))) === PT.toString("hex"));
const c = crypto.createCipheriv("aes-128-cbc", K128, IV);
const b64 = c.update("hello world", "utf8", "base64") + c.final("base64");
const d = crypto.createDecipheriv("aes-128-cbc", K128, IV);
console.log(b64, d.update(b64, "base64", "utf8") + d.final("utf8"));
console.log(enc("aes-256-cbc", K256, IV, Buffer.alloc(16)), enc("aes-128-cbc", K128, IV, Buffer.alloc(0)), enc("aes-128-cbc", "0123456789abcdef", "fedcba9876543210", "x"));
const GK128 = hex("feffe9928665731c6d6a8f9467308308");
const GK192 = hex("feffe9928665731c6d6a8f9467308308feffe9928665731c");
const GK256 = hex("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308");
const GIV = hex("cafebabefacedbaddecaf888");
const GPT = hex("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39");
const AAD = hex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
const gcm = (alg, key) => {
  const g = crypto.createCipheriv(alg, key, GIV);
  g.setAAD(AAD);
  const ct = Buffer.concat([g.update(GPT.subarray(0, 7)), g.update(GPT.subarray(7)), g.final()]);
  return [ct.toString("hex"), g.getAuthTag().toString("hex")];
};
console.log(...gcm("aes-128-gcm", GK128));
console.log(...gcm("aes-192-gcm", GK192));
console.log(...gcm("aes-256-gcm", GK256));
const [ct, tag] = gcm("aes-256-gcm", GK256);
const g = crypto.createDecipheriv("aes-256-gcm", GK256, GIV);
g.setAAD(AAD);
g.setAuthTag(hex(tag));
console.log(Buffer.concat([g.update(hex(ct)), g.final()]).equals(GPT));
const e2 = crypto.createCipheriv("aes-128-gcm", Buffer.alloc(16), Buffer.alloc(12));
console.log(e2.update("", "utf8", "hex") + e2.final("hex"), e2.getAuthTag().toString("hex"));
const e3 = crypto.createCipheriv("aes-128-gcm", GK128, GIV);
const out = e3.update("hi", "utf8", "hex") + e3.final("hex");
const t3 = e3.getAuthTag();
const d3 = crypto.createDecipheriv("aes-128-gcm", GK128, GIV).setAuthTag(t3);
console.log(out, t3.toString("base64"), d3.update(out, "hex", "utf8") + d3.final("utf8"));"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b273bed6b8e3c1743b7116e69e222295163ff1caa1681fac09120eca307586e1a7",
            "log:4f021db243bc633d7178183a9fa071e8b4d9ada9ad7dedf4e5e738763f69145a571b242012fb7ae07fa9baac3df102e008b0e27988598881d920a9e64f5615cd",
            "log:f58c4c04d6e5f1ba779eabfb5f7bfbd69cfc4e967edb808d679f777bc6702c7d39f23369a9d9bacfa530e26304231461b2eb05e2c39be9fcda6c19078c6a9d1b",
            "log:true true",
            "log:Zu5N+uxWpxzTPNTFFR19iA== hello world",
            "log:b7bf3a5df43989dd97f0fa97ebce2f4a21c580b5614b51095c49a8594ec16889 c84af0b613435d5d9182801a9bd9320b ff1e52bcaf6c31fb3e11594e0d2db3c9",
            "log:42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091 5bc94fbc3221a5db94fae95ae7121a47",
            "log:3980ca0b3c00e841eb06fac4872a2757859e1ceaa6efd984628593b40ca1e19c7d773d00c144c525ac619d18c84a3f4718e2448b2fe324d9ccda2710 2519498e80f1478f37ba55bd6d27618c",
            "log:522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662 76fc6ece0f4e1768cddf8853bb2d551b",
            "log:true",
            "log: 58e2fccefa7e3061367f1d57a4e7455a",
            "log:f3db UPmmWC49obEXUSdxQtCNxg== hi",
        ]
    );
}

/// RSA signatures are Node 24's (PKCS#1 v1.5 is deterministic), ECDSA signatures were made by Node, Ed25519 is RFC 8032 test 1.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_and_verify_match_node() {
    let code = r#"const hex = (s) => Buffer.from(s, "hex");
const msg = Buffer.from("hello world");
const s1 = crypto.createSign("RSA-SHA256").update("hello ").update(Buffer.from("world")).sign(RSA, "base64");
console.log(s1);
console.log(crypto.sign("sha256", msg, RSA).toString("base64") === s1, crypto.sign("SHA256", msg, RSA_PKCS1).toString("base64") === s1, crypto.sign("sha256", msg, Buffer.from(RSA)).toString("base64") === s1, crypto.sign(null, msg, RSA).toString("base64") === s1);
console.log(crypto.sign("sha384", Buffer.from("x"), RSA).toString("hex").slice(0, 32), crypto.createSign("rsa-sha512").update("x").sign({ key: RSA }, "hex").slice(0, 32), Buffer.isBuffer(crypto.createSign("sha256").update("x").sign(RSA)));
const sig = Buffer.from(s1, "base64");
console.log(crypto.verify("sha256", msg, RSA_PUB, sig), crypto.createVerify("RSA-SHA256").update("hello world").verify(RSA_CRT, s1, "base64"), crypto.verify("sha256", msg, RSA, sig), crypto.verify("sha256", msg, RSA_PKCS1_PUB, sig), crypto.verify("sha256", Buffer.from("hello world!"), RSA_PUB, sig), crypto.verify("sha384", msg, RSA_PUB, sig), crypto.verify("sha256", msg, RSA_PUB, sig.subarray(1)));
const m = Buffer.from("msg");
console.log(crypto.verify("sha256", m, EC256_PUB, hex("304402204368cd15c6bfee331a2302ac455078aef2ae81341d6c0dfc6d08adb095b05ee702205135bfc592a5c7f8f86be38dfda45d8d525314fe068145524912efce0b88713a")), crypto.verify("sha384", m, EC384_PUB, hex("3065023100f66e41dbc8eb44a2e3fb7088b8884ea5e0fa98d8dc91a972d25e07aa358de396735a216d8e2605b1160c6de076a1629b0230346e576f03ee9c60e70b8dd5ec204cadcf0a7ed9c98407350b6bec63bb9479c337169cc0ff231ab6c403565c34145fa8")), crypto.verify("sha256", m, { key: EC256_PUB, dsaEncoding: "ieee-p1363" }, hex("494cfee413288ec039962e3e4b4b8dd496a083b36c62f3f76f7ce3ec29089174be8b5b9e6edf2dee277447f89b34db651dca447a25596d20f24f103ac063bdcb")), crypto.verify("sha256", Buffer.from("msh"), EC256_PUB, hex("304402204368cd15c6bfee331a2302ac455078aef2ae81341d6c0dfc6d08adb095b05ee702205135bfc592a5c7f8f86be38dfda45d8d525314fe068145524912efce0b88713a")));
const es = crypto.sign("sha256", m, EC256_SEC1);
console.log(es[0] === 0x30, crypto.verify("sha256", m, EC256_PUB, es), crypto.verify("sha256", m, EC256_SEC1, es), crypto.createVerify("SHA256").update("msg").verify(EC256_PUB, es.toString("hex"), "hex"));
const ep = crypto.sign("sha384", m, { key: EC384, dsaEncoding: "ieee-p1363" });
const es384 = crypto.createSign("sha384").update(m).sign(EC384);
console.log(ep.length, crypto.verify("sha384", m, { key: EC384_PUB, dsaEncoding: "ieee-p1363" }, ep), crypto.createVerify("SHA384").update("msg").verify({ key: EC384_PUB, dsaEncoding: "ieee-p1363" }, ep), crypto.verify("sha384", m, EC384_PUB, es384));
console.log(crypto.sign(null, Buffer.alloc(0), ED_PRIV).toString("hex"));
const edsig = hex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b");
console.log(crypto.verify(null, Buffer.alloc(0), ED_PUB, edsig), crypto.verify(undefined, new Uint8Array(0), ED_PRIV, edsig), crypto.verify(null, Buffer.from("x"), ED_PUB, edsig));
crypto.sign("sha256", msg, RSA, (e, signature) => {
  console.log("sign", e, signature.toString("base64") === s1);
  crypto.verify("sha256", msg, RSA_PUB, signature, (e2, ok) => console.log("verify", e2, ok));
});
console.log("sync first");"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:dDG8yIcTjUQOBqXL93WZv5VtC8iy+L6ovzjPbZeiq5ffcnAhnJ3dPBWEmdS1A3CJ8U5Ui7gF88FBCnf29p5pb7dH3Gir9V4IQjwh0EZCOdX9rfuDdv5rOuCWzx89AQiMWVzF3zxw8X57DSE0PKM4WAazw1AeDiNycIQoZ1dMUwHOnYU6ja+JexLRusTEUwSRS0WNrPcUt/gO3KwQQeSMKSsydukHb9n5Ame8tOmdTBdD0SAN0B1R2z+xqB7DcC8bW9egO32TAaDszSvkz9/0hF0VG8XcIzH+AAMj5q6/X6fjWjLVTriSvn45X/L26MMJncyIC2sAGH60H7k9uhz9Uw==",
            "log:true true true true",
            "log:0db3e9c1db50c1d79e0690665dcfc5dc 60c3737f652aa32b5beac46202f0f6cd true",
            "log:true true true true false false false",
            "log:true true true false",
            "log:true true true true",
            "log:96 true true true",
            "log:e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
            "log:true true false",
            "log:sync first",
            "log:sign null true",
            "log:verify null true",
        ]
    );
}

/// Each error keeps Node's type; messages follow Node 24 without OpenSSL codes, and every
/// unsupported algorithm, cipher, key form or option is named.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn errors_name_the_problem_with_node_types() {
    let code = r#"const show = (f) => { try { const r = f(); console.log("ok", Buffer.isBuffer(r) ? "Buffer" : String(r)); } catch (e) { console.log(e.name, e.message); } };
const K = Buffer.alloc(16), IV = Buffer.alloc(16), GIV = Buffer.alloc(12);
show(() => crypto.timingSafeEqual("a", Buffer.from("a")));
show(() => crypto.timingSafeEqual(Buffer.from("a"), [97]));
show(() => crypto.timingSafeEqual(Buffer.from("a"), Buffer.from("ab")));
show(() => crypto.pbkdf2Sync("p", "s", 1, 16));
show(() => crypto.pbkdf2Sync("p", "s", 1, 16, "md4"));
show(() => crypto.pbkdf2Sync("p", "s", 0, 16, "sha256"));
show(() => crypto.pbkdf2Sync("p", "s", 10000001, 16, "sha256"));
show(() => crypto.pbkdf2Sync("p", "s", 1.5, 16, "sha256"));
show(() => crypto.pbkdf2Sync("p", "s", "1", 16, "sha256"));
show(() => crypto.pbkdf2Sync("p", "s", 1, 65537, "sha256"));
show(() => crypto.pbkdf2Sync(1, "s", 1, 16, "sha256"));
show(() => crypto.pbkdf2("p", "s", 1, 16, "sha256"));
show(() => crypto.pbkdf2Sync("p", "s", 1, 0, "sha256").length);
show(() => crypto.scryptSync("p", "s", 16, { N: 3 }));
show(() => crypto.scryptSync("p", "s", 16, { N: 1 }));
show(() => crypto.scryptSync("p", "s", 16, { N: 2 ** 20 }));
show(() => crypto.scryptSync("p", "s", 16, { N: 16, r: 1, p: 1, maxmem: 2431 }));
show(() => crypto.scryptSync("p", "s", 16, { N: 16, cost: 16 }));
show(() => crypto.scryptSync("p", "s", 16, { maxmem: 2 ** 28 + 1 }));
show(() => crypto.scryptSync("p", "s", -1));
show(() => crypto.scrypt("p", "s", 16));
show(() => { const h = crypto.createHash("sha1"); h.digest(); return h.copy(); });
show(() => crypto.createCipheriv("aes-128-ecb", K, null));
show(() => crypto.createDecipheriv("chacha20-poly1305", Buffer.alloc(32), GIV));
show(() => crypto.createCipheriv("aes-128-cbc", Buffer.alloc(15), IV));
show(() => crypto.createCipheriv("aes-256-gcm", Buffer.alloc(16), GIV));
show(() => crypto.createCipheriv("aes-128-cbc", K, Buffer.alloc(8)));
show(() => crypto.createCipheriv("aes-128-gcm", K, Buffer.alloc(16)));
show(() => { const c = crypto.createCipheriv("aes-128-cbc", K, IV); c.setAutoPadding(false); c.update("abc"); return c.final(); });
show(() => { const d = crypto.createDecipheriv("aes-128-cbc", K, IV); d.update(Buffer.alloc(5)); return d.final(); });
show(() => { const d = crypto.createDecipheriv("aes-128-cbc", K, IV); d.update(Buffer.alloc(16)); return d.final(); });
show(() => { const c = crypto.createCipheriv("aes-128-cbc", K, IV); c.final(); return c.final(); });
show(() => { const c = crypto.createCipheriv("aes-128-cbc", K, IV); c.final(); return c.update("a"); });
show(() => crypto.createCipheriv("aes-128-gcm", K, GIV).getAuthTag());
show(() => { const c = crypto.createCipheriv("aes-128-cbc", K, IV); c.final(); return c.getAuthTag(); });
show(() => typeof crypto.createCipheriv("aes-128-gcm", K, GIV).setAuthTag);
show(() => { const c = crypto.createCipheriv("aes-128-gcm", K, GIV); c.update("a"); return c.setAAD(Buffer.alloc(1)); });
show(() => crypto.createDecipheriv("aes-128-gcm", K, GIV).setAuthTag(Buffer.alloc(3)));
show(() => { const d = crypto.createDecipheriv("aes-128-gcm", K, GIV); d.update(Buffer.alloc(3)); return d.final(); });
show(() => { const c = crypto.createCipheriv("aes-128-gcm", K, GIV); const ct = Buffer.concat([c.update("secret"), c.final()]); const tag = c.getAuthTag(); tag[0] ^= 1; const d = crypto.createDecipheriv("aes-128-gcm", K, GIV).setAuthTag(tag); d.update(ct); return d.final(); });
show(() => { const c = crypto.createCipheriv("aes-128-gcm", K, GIV); c.setAAD(Buffer.from("a")); const ct = Buffer.concat([c.update("secret"), c.final()]); const d = crypto.createDecipheriv("aes-128-gcm", K, GIV).setAuthTag(c.getAuthTag()); d.setAAD(Buffer.from("b")); d.update(ct); return d.final(); });
show(() => crypto.createSign("md4"));
show(() => crypto.sign("whirlpool", Buffer.from("m"), RSA));
show(() => crypto.sign("sha224", Buffer.from("m"), RSA).length);
show(() => crypto.verify("sha256", Buffer.from("m"), K1_PUB, Buffer.alloc(8)));
show(() => crypto.sign("sha256", Buffer.from("m"), ED_PRIV).length);
show(() => crypto.sign("sha256", Buffer.from("m"), { key: RSA, padding: 6, saltLength: 0 }).length);
show(() => crypto.sign("sha256", Buffer.from("m"), { key: RSA, passphrase: "x" }).length);
show(() => crypto.sign("sha256", Buffer.from("m"), "-----BEGIN ENCRYPTED PRIVATE KEY-----\nU0VDUkVUS0VZQllURVM=\n-----END ENCRYPTED PRIVATE KEY-----\n"));
show(() => crypto.sign("sha256", Buffer.from("m"), "-----BEGIN PRIVATE KEY-----\nU0VDUkVUS0VZQllURVM=\n-----END PRIVATE KEY-----\n"));
show(() => crypto.sign("sha256", Buffer.from("m"), "not a key"));
show(() => crypto.sign("sha256", Buffer.from("m"), RSA_PUB));
show(() => crypto.verify("sha256", Buffer.from("m"), "not a key", Buffer.alloc(4)));
show(() => crypto.sign("sha256", "m", RSA));
show(() => crypto.sign("sha256", Buffer.from("m"), { key: EC256_SEC1, dsaEncoding: "raw" }));
show(() => { const s = crypto.createSign("sha256"); s.update("a"); s.sign(RSA); return s.sign(RSA); });
show(() => crypto.sign("sha256", Buffer.from("m"), 42));"#;
    let sig = |x: &str| unsupported_sig(x);
    assert_eq!(
        run_crypto(code).await,
        [
            "log:TypeError The \"buf1\" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView.".to_string(),
            "log:TypeError The \"buf2\" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView.".to_string(),
            "log:RangeError Input buffers must have the same byte length".to_string(),
            "log:TypeError The \"digest\" argument must be of type string".to_string(),
            "log:Error Digest method not supported: \"md4\" (supported: sha1, sha256, sha384, sha512; require('crypto-js') has more)".to_string(),
            "log:RangeError The value of \"iterations\" is out of range. It must be >= 1 && <= 10000000. Received 0".to_string(),
            "log:RangeError The value of \"iterations\" is out of range. It must be >= 1 && <= 10000000. Received 10000001".to_string(),
            "log:RangeError The value of \"iterations\" is out of range. It must be an integer. Received 1.5".to_string(),
            "log:TypeError The \"iterations\" argument must be of type number".to_string(),
            "log:RangeError The value of \"keylen\" is out of range. It must be >= 0 && <= 65536. Received 65537".to_string(),
            "log:TypeError The \"password\" argument must be of type string or an instance of ArrayBuffer, Buffer, TypedArray, or DataView".to_string(),
            "log:TypeError The \"callback\" argument must be of type function".to_string(),
            "log:ok 0".to_string(),
            "log:RangeError Invalid scrypt params".to_string(),
            "log:RangeError Invalid scrypt params".to_string(),
            "log:RangeError Invalid scrypt params: memory limit exceeded".to_string(),
            "log:RangeError Invalid scrypt params: memory limit exceeded".to_string(),
            "log:TypeError Option \"N\" cannot be used in combination with option \"cost\"".to_string(),
            "log:RangeError The value of \"maxmem\" is out of range. It must be >= 0 && <= 268435456. Received 268435457".to_string(),
            "log:RangeError The value of \"keylen\" is out of range. It must be >= 0 && <= 65536. Received -1".to_string(),
            "log:TypeError The \"callback\" argument must be of type function".to_string(),
            "log:Error Digest already called".to_string(),
            "log:Error Cipher not supported: \"aes-128-ecb\" (supported: aes-128-cbc, aes-192-cbc, aes-256-cbc, aes-128-ctr, aes-192-ctr, aes-256-ctr, aes-128-gcm, aes-192-gcm, aes-256-gcm; require('crypto-js') has more)".to_string(),
            "log:Error Cipher not supported: \"chacha20-poly1305\" (supported: aes-128-cbc, aes-192-cbc, aes-256-cbc, aes-128-ctr, aes-192-ctr, aes-256-ctr, aes-128-gcm, aes-192-gcm, aes-256-gcm; require('crypto-js') has more)".to_string(),
            "log:RangeError Invalid key length".to_string(),
            "log:RangeError Invalid key length".to_string(),
            "log:TypeError Invalid initialization vector".to_string(),
            "log:TypeError Invalid initialization vector".to_string(),
            "log:Error wrong final block length".to_string(),
            "log:Error wrong final block length".to_string(),
            "log:Error bad decrypt".to_string(),
            "log:Error Invalid state".to_string(),
            "log:Error Trying to add data in unsupported state".to_string(),
            "log:Error Invalid state for operation getAuthTag".to_string(),
            "log:Error Invalid state for operation getAuthTag".to_string(),
            "log:ok undefined".to_string(),
            "log:Error Invalid state for operation setAAD".to_string(),
            "log:TypeError Invalid authentication tag length: 3".to_string(),
            "log:Error Unsupported state or unable to authenticate data".to_string(),
            "log:Error Unsupported state or unable to authenticate data".to_string(),
            "log:Error Unsupported state or unable to authenticate data".to_string(),
            sig("md4"),
            sig("whirlpool"),
            sig("sha224"),
            sig("an EC key on an unsupported curve"),
            sig("sha256 with an Ed25519 key"),
            sig("RSA-PSS with saltLength 0"),
            "log:ok 256".to_string(),
            "log:Error Unsupported key: the encrypted private key could not be read".to_string(),
            "log:Error Unsupported key: expected a PEM private key (PKCS#8, PKCS#1 or SEC1)".to_string(),
            "log:Error Unsupported key: expected a PEM private key (PKCS#8, PKCS#1 or SEC1)".to_string(),
            "log:Error Unsupported key: expected a PEM private key (PKCS#8, PKCS#1 or SEC1)".to_string(),
            "log:Error Unsupported key: expected a PEM public key (SPKI, PKCS#1 or X.509 certificate) or private key".to_string(),
            "log:ok Buffer".to_string(),
            "log:TypeError The property 'options.dsaEncoding' is invalid. Received 'raw'".to_string(),
            "log:Error Not initialised".to_string(),
            "log:TypeError The \"key\" argument must be of type string or an instance of ArrayBuffer, Buffer, TypedArray, or DataView".to_string(),
        ]
    );
}

/// Node members outside the subset throw a named error; `subtle` stays undefined for feature checks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_members_throw_named_errors() {
    let code = r#"const stubs = ["createCipher", "createDecipher", "createECDH", "createDiffieHellman", "createDiffieHellmanGroup", "getDiffieHellman", "diffieHellman", "generateKeyPair", "generateKeyPairSync", "generateKey", "generateKeySync", "publicEncrypt", "privateDecrypt", "privateEncrypt", "publicDecrypt", "hkdf", "hkdfSync", "generatePrime", "generatePrimeSync", "checkPrime", "checkPrimeSync", "getCurves"];
for (const n of stubs) { try { require("node:crypto")[n](); console.log(n, "no error"); } catch (e) { console.log(e.name, e.message); } }
console.log(typeof crypto.subtle, typeof crypto.webcrypto, "subtle" in crypto, typeof require("crypto").pbkdf2Sync);"#;
    let mut expected: Vec<String> = [
        "createCipher",
        "createDecipher",
        "createECDH",
        "createDiffieHellman",
        "createDiffieHellmanGroup",
        "getDiffieHellman",
        "diffieHellman",
        "generateKeyPair",
        "generateKeyPairSync",
        "generateKey",
        "generateKeySync",
        "publicEncrypt",
        "privateDecrypt",
        "privateEncrypt",
        "publicDecrypt",
        "hkdf",
        "hkdfSync",
        "generatePrime",
        "generatePrimeSync",
        "checkPrime",
        "checkPrimeSync",
        "getCurves",
    ]
    .iter()
    .map(|n| format!("log:Error crypto.{n} is not supported (require('crypto-js') has more)"))
    .collect();
    expected.push("log:undefined undefined false function".into());
    assert_eq!(run_crypto(code).await, expected);
}

/// Work caps on PBKDF2 and scrypt, Node's appending `setAAD`, the `authTagLength` option, and
/// Node's sha256 default for a null algorithm on a P-384 key (the signature below is Node's).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_caps_aad_tag_length_and_p384_default() {
    let code = r#"const show = (f) => { try { const r = f(); console.log("ok", Buffer.isBuffer(r) ? r.length : String(r)); } catch (e) { console.log(e.name, e.message); } };
const K = Buffer.alloc(16), GIV = Buffer.alloc(12);
show(() => crypto.pbkdf2Sync("p", "s", 10000000, 64, "sha256"));
show(() => crypto.pbkdf2Sync("p", "s", 1, 65536, "sha1"));
show(() => crypto.scryptSync("p", "s", 16, { N: 2 ** 14, r: 8, p: 16000 }));
const e = crypto.createCipheriv("aes-128-gcm", K, GIV);
e.setAAD(Buffer.from("a"));
e.setAAD(Buffer.from("b"));
e.final();
console.log(e.getAuthTag().toString("hex"));
show(() => crypto.createCipheriv("aes-128-gcm", K, GIV, { authTagLength: 8 }));
show(() => crypto.createDecipheriv("aes-256-gcm", Buffer.alloc(32), GIV, { authTagLength: 12 }));
show(() => typeof crypto.createCipheriv("aes-128-gcm", K, GIV, { authTagLength: 16 }).update);
show(() => crypto.verify(null, Buffer.from("msg"), EC384_PUB, crypto.sign(null, Buffer.from("msg"), EC384)));
const sig = Buffer.from("3064023023efc633b7409b534b5af12a6d4247224e205b6e36a9b9368b1d889e6e469680205eed535bfe8dc1e2d40daadd7503920230472f505ba4b603ccc95377eda40f2aaa6ee90f3b26c7cc4df3182f451d8cadfb74738fd88c52afa090d08c7d982bb76e", "hex");
console.log(crypto.verify(null, Buffer.from("msg"), EC384_PUB, sig), crypto.verify("sha256", Buffer.from("msg"), EC384_PUB, sig), crypto.verify(null, Buffer.from("msh"), EC384_PUB, sig));"#;
    let tag = |n: u32| {
        format!(
            "log:Error Cipher option not supported: \"authTagLength: {n}\" (supported: 16; require('crypto-js') has more)"
        )
    };
    assert_eq!(
        run_crypto(code).await,
        [
            "log:RangeError The pbkdf2 work (iterations x ceil(keylen / digest length)) must be <= 16777216. Received 20000000".to_string(),
            "log:ok 65536".into(),
            "log:RangeError Invalid scrypt params: N * r * p must be <= 33554432".into(),
            "log:1f75121277d1d74647a3d2948034b720".into(),
            tag(8),
            tag(12),
            "log:ok function".into(),
            "log:ok true".into(),
            "log:true true false".into(),
        ]
    );
}

/// RSA PKCS#1 v1.5 with sha1 is deterministic, so its signature is Node 24's; the ECDSA signatures in `NODE` were made by Node 24, and ours are checked by verifying them back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rsa_sha1_and_ecdsa_digests_match_node() {
    let code = r#"const hex = (s) => Buffer.from(s, "hex");
const m = Buffer.from("msg"), bad = Buffer.from("msh");
const r1 = crypto.sign("sha1", m, RSA).toString("base64");
console.log(r1);
console.log(crypto.createSign("RSA-SHA1").update("msg").sign(RSA_PKCS1, "base64") === r1, crypto.sign("SHA1", m, Buffer.from(RSA)).toString("base64") === r1, crypto.verify("sha1", m, RSA_PUB, hex(Buffer.from(r1, "base64").toString("hex"))), crypto.createVerify("rsa-sha1").update("msg").verify(RSA_CRT, r1, "base64"), crypto.verify("sha1", bad, RSA_PUB, Buffer.from(r1, "base64")), crypto.verify("sha256", m, RSA_PUB, Buffer.from(r1, "base64")));
const PUB = { EC256_SEC1: EC256_PUB, EC384: EC384_PUB, EC521: EC521_PUB };
const PRIV = { EC256_SEC1, EC384, EC521 };
const NODE = [
  ["sha1", "EC256_SEC1", "30450221008f3bef16f95fcbde8d92456b7dc1eb61742f36efcba692aaf25bcc97514fe34b0220162bfbee5f51b6f032198b11c882addde655298a89ce3d3672972c2f2e923fbf"],
  ["sha384", "EC256_SEC1", "3045022100d17da8460476b563e9f6b98851baac01f8c40ab43e946d295381be217cb00e1c022007c7fa030db2b96d9c90fe77c5a8e4229c0a047c0ff686b5da6c0c5aa1707ce6"],
  ["sha512", "EC256_SEC1", "3046022100cef37987241219d609018508026d7428968c72fd201b807cc7b6eefcc5e54f710221008559e77e294036865bfea86601fa89242169f05a2a1e67ba226b2b0bb9e4e81a"],
  ["sha1", "EC384", "3066023100e409fb736e5dccece6766e359dbabbf89463a99ed3a0f075148d2f8f160a5ab8dbb5394cda76b6b41271320e159a3df7023100b933a9e06cba9760c12ec207690029546e22df7b7916475215f16f1658957e534447d0b8e7717ba2f4d9050eb215af4e"],
  ["sha256", "EC384", "30650230715b961e99ce3e4d3d64fa5a7cb86a497defa1d09d233156cba88b8189a980cc404a4a38cb01f4d48c9717342748d4b7023100c3de83db3fdc74c7a5a55fe4770b957aa40005726657193ffd16c6d068dd97b74b75725a89c69a5e7b7f781aa9a94516"],
  ["sha512", "EC384", "30650230253b28c7e3d60c40469f0ffdfce62089c95a54e74852cde2c6f621f6b54bd82c0b2a4c7a405cea76ccc2c80282f4883a023100837bfca5fa5503454abffa90df8cb6c602000362eb7a953db5c616ad6def0df349f740dc6127ff947cc3eaf623d6f3e6"],
  ["sha1", "EC521", "308188024200d3aa5f5ff4ed7042bda7f4a328f17a3c8b535a5cbe24889ef4ec676bd5ddd0746593ce08f07f19bf19eae68a72b882de2761422af78dec8a72934055961f6623ad024200e91376761c3c4c6aaaedbe56a3d0cbacdd042d4692fe85a139fa7274009f533907bb310701605c347544b834119b65811fca6c3275f480ecec7d3d3313749b75eb"],
  ["sha256", "EC521", "30818702416cb73e73f15ee736422a39eeb1ef47f7da6994c462b5812676bf715a953157cad483086575cb0107d9642c4cdc7bb0e6b960cc3bcc7381491a130d9b27ae59560f024201c1d8cf036cdb7ed8080d56a2c80f6bf9d14d6cb03172664ad5666863d60548d1374ef1a2b9e42be7f58a3dbaa867dcefc2e6f7de2657ee71dc3599a804ca4d031f"],
  ["sha384", "EC521", "308188024200a11c1a2303f1dbc84baa2fe7a0db1ed493859f87ff901aba3dc79ec3cbc89f9c25e840eecf858ae2ee95a26a2efd0a8a0bbcaf2ba48b8c2e34747211fa3219c04c0242016ad3b20c1663b0e76ec6aaba37d83e087773569a540069769329c441e7e3e211be124bc867e184bc1aa6d36605c20d2dcad5325896f0f13e1c0b0f718004f09130"],
  ["sha512", "EC521", "3081880242015d1c2660d4a590634c31c1df6a52742a96a43483977a05ab0bb94754c7dd7e67b9b8b095a7d9e259ec542ef80204bdaf6f5086a859aeb2bcbe59869c0f923e1e27024201245fcb40b4f97e5c2d5c793e74e8da359248a5034699081c418c19d4dc5b845b7e9a9772ebc011e5de6d73e7cf85c7850c7b294b24f66d939bd2d0149d404c35f4"],
];
for (const [d, k, sig] of NODE) {
  const der = crypto.sign(d, m, PRIV[k]), raw = crypto.sign(d, m, { key: PRIV[k], dsaEncoding: "ieee-p1363" });
  const p1363 = { key: PUB[k], dsaEncoding: "ieee-p1363" };
  console.log(d, k, crypto.verify(d, m, PUB[k], hex(sig)), crypto.verify(d, bad, PUB[k], hex(sig)), der[0] === 0x30, raw.length, crypto.verify(d, m, PUB[k], der), crypto.verify(d, m, PRIV[k], der), crypto.verify(d, m, p1363, raw), crypto.createVerify(d).update(m).verify(p1363, raw), crypto.verify(d, bad, PUB[k], der));
}
console.log(crypto.verify("sha256", m, { key: EC384_PUB, dsaEncoding: "ieee-p1363" }, hex("7f549960e23417e72e2e6dfd8508612a9d5c2da8584be0eced0966347e65068b3bdf79b8eff6b0a60ce9572d3405cc20371356d41bb72aafa9f345bb4816cb21f5176dd5646af12809c3961ed1e5da6217a0f6d1090592d0def06f4a97237c7c")), crypto.verify("sha1", m, { key: EC521_PUB, dsaEncoding: "ieee-p1363" }, hex("002a93ff19fea2631092cc4959d7e4c2cf3b08ff853578e88da43d234be6856313783a96d1b9176b31c83feabc0b0a5b426b21aec431d308f91ab5196e669bc827930006bb13e62235494877d1d00f7b585d1d1896acb1417ea32aacaf94a8aa13f312dc2f1f2c9906147e1dbed876b98f037969736a49acbf2fcba9c6d9dbb15ada132a")));
const es = crypto.sign(null, m, EC384);
console.log(crypto.verify(null, m, EC384_PUB, es), crypto.verify("sha256", m, EC384_PUB, es), crypto.verify("sha384", m, EC384_PUB, es), crypto.verify(null, m, EC521_PUB, crypto.sign(null, m, EC521)));"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:Evzd2IrHkMD3PK6orvsXLix01Eqs/voLflb2F16kqYmfKE9Inx9j/fbcQYSRmo1kxvPGf8lkMYSSArI8p1Xa8Us6ypqKRKNahwImWsKuVPC76vaAwweEorCcWezWw8aRgvBJJmb+N4o79cw9JNQrwS6nR02t9aZQ0HcyjScXZIzZZa0InJGLf6hzqR/aHng3OCrKyFXNg/RKkH53Wgj8I3ke+iZ4bJ7DTuYCDF9qldcX10e6MrwW5sjK8//O0ANDiqpXxIxlTLTQ3eDhppMJ+e6+35XUMHwsLL3GI7obIFz2oyxlgdpwZBtmyZRCpXxvcU6Ij6q8myCvVIJ4CDj9xQ==",
            "log:true true true true false false",
            "log:sha1 EC256_SEC1 true false true 64 true true true true false",
            "log:sha384 EC256_SEC1 true false true 64 true true true true false",
            "log:sha512 EC256_SEC1 true false true 64 true true true true false",
            "log:sha1 EC384 true false true 96 true true true true false",
            "log:sha256 EC384 true false true 96 true true true true false",
            "log:sha512 EC384 true false true 96 true true true true false",
            "log:sha1 EC521 true false true 132 true true true true false",
            "log:sha256 EC521 true false true 132 true true true true false",
            "log:sha384 EC521 true false true 132 true true true true false",
            "log:sha512 EC521 true false true 132 true true true true false",
            "log:true true",
            "log:true true false true",
        ]
    );
}

/// RSA-PSS: signatures in `NODE` were made by Node 24 with the salt lengths shown (-2 is Node's maximum); ours use a digest-length salt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rsa_pss_signs_and_verifies_like_node() {
    let code = r#"const m = Buffer.from("msg"), bad = Buffer.from("msh");
const C = crypto.constants;
console.log(C.RSA_PKCS1_PADDING, C.RSA_PKCS1_PSS_PADDING, C.RSA_PSS_SALTLEN_DIGEST, C.RSA_PSS_SALTLEN_MAX_SIGN, C.RSA_PSS_SALTLEN_AUTO);
const pss = { padding: C.RSA_PKCS1_PSS_PADDING };
const HLEN = { sha256: 32, sha384: 48, sha512: 64 };
for (const d of ["sha256", "sha384", "sha512"]) {
  const s = crypto.sign(d, m, { key: RSA, ...pss, saltLength: C.RSA_PSS_SALTLEN_DIGEST });
  const s2 = crypto.createSign("RSA-" + d.toUpperCase()).update("m").update("sg").sign({ key: RSA_PKCS1, ...pss, saltLength: HLEN[d] }, "base64");
  console.log(d, s.length, crypto.verify(d, m, { key: RSA_PUB, ...pss }, s), crypto.verify(d, m, { key: RSA_PUB, ...pss, saltLength: HLEN[d] }, s), crypto.verify(d, m, { key: RSA_PUB, ...pss, saltLength: C.RSA_PSS_SALTLEN_AUTO }, s), crypto.createVerify(d).update("msg").verify({ key: RSA_CRT, ...pss }, s2, "base64"), crypto.verify(d, m, { key: RSA, ...pss }, s), crypto.verify(d, m, RSA_PUB, s), crypto.verify(d, bad, { key: RSA_PUB, ...pss }, s), crypto.verify(d, m, { key: RSA_PUB, ...pss, saltLength: HLEN[d] + 1 }, s));
}
const NODE = [
  ["sha256", -2, "nfLi5PXdg5gUi/+VyJqlr+IO9AlRlBTEDwzuq78SH9ETVYkK2hnLtkEHmQg0hvlOh8Sj0QkJRBjP4AWgY7GXsjTRI/iQkKVY+uqh1YNVOBClNgPCSBHxQmwl/HHDO9mMYIwEGnV1xMqviW88l+IVzaYEZjaYUpNaBPGvow3IIav+iWX/0yqYuCIHF1u2EpbVEnxS3mSZAcWIHgubH+Lv1gUVK1jZwDTBPLcucHNuHdqSqmBlYpiAQfG4azdbqgJdnLIBzJaDW/CeueLDI7Irt8x+wpxIOioQnQB/PBjaYytSyN2RjlGmyotQ+a/xlvZ2diQXUZDhjPd5BUoi8JWoPA==", 222],
  ["sha256", 0, "vJys3A4MxEuCIemzCZYRg0NXoPrJUNeNifrA7TwAOD71WGEts+265VZ80LtctzU2hi50gJuNomBBG7GsoRw7EMFk2cUG5+pcCP2ZDJdVyqJgikxnNHSx2x5rnIPjKwSLUc0un4k9T7iL4VrH4HFvCJBvUk7Bt5U8yqde2vCHBBWL7Nif8TztRLB+gdT0eC42/whcLFHH/9WMU2CZgX+JV7g9vDM+a2Wk0VI+DJ0MQeZd1NqW7c6VMJ1YYEGa/UomFRFtr3tviBjQRfY/nGItp3duvZ2cPYQHd+kX2+VBH7yYsI+HLOpEsSTiJHG+mBEqlMKuj0gAQTCg+PgfjTLA8w==", 0],
  ["sha256", 20, "jVH7nWGeKwFJhmSg4bH89KHhCJtzgzBQIfnxHhNuHXlMAVZO/Sgj7VTFz/0BqZG2QtTckdMi9HKBtIs36qpoT94JNgDQQFFMWhkwwMYR5M5gi/DVLklqkUKR5PYJFedLPIr9GEC38gHKeugDomlmvyPOXg82IEC4fQ5LmSqil66YYWGLBRNx/osmYVmNBvoAshFSO94uTAX5x+1k1ee5TxM0M0JcZU8QI7XLXrb7S4F/yF3io2HZyM2Sg077E4tI7tCK6PljRu3yhVAZk4BNs4DySk8Xo7gV9Htje9bUTBvqtFqQvd1UY2gaa9W/cMWvcEWtj0R9x4G7DnlzjlShng==", 20],
  ["sha512", -2, "iFAjqn36/tYJv91jnF2ciunG1ck6DIvxI64kMKv/huv27zZEhY1I2yxcmjFd4d3hNnFumCnnEyD9zD7rAEbPGal0qmiiEFhWULoPeWGvLDjti+voBnLumUq/m27redHFZnLY8xamf0OY/61Pf27fhR1eEzhi9TubjIh0ltPP2u16FYSnw9LS2LFAkmc6CW6jUT6kse9IwLy5ORQ4SPeT4v2+uEDf0ou7G6KxcR0YDEpk9NS11/9WKTXr/t83paSUCRJb3CHHDAYMrBZEx0ScKyPMqA/y5Zy6lr/dngI2ZPqKpiBtd2ADmwCIRS6DAGt8281kHthPfKOqmt78H100bA==", 190],
  ["sha384", 48, "Ur1olShov7vZuWmDhLk6oFhs6bIUQN3gf5/0JDhkMwS3URH7RTpIn4vinnm5wrIgKG9KI83zZiRTAnUNFeHoOC+A11Q32M2qw1L2RKZxuMxRce2rt9OognKDxljL0uSmkzB7RJPP9dUR+sOGdxDBAh1yq3iCb+IWnUuHd8rbX/9ePbrB/5amFkgxbzuHDZReybDVhHa2M5L0MA1m5jMjn9SbnxgLq9al3zI/PWH8g34+zjX/CBZXphqZOR0R+iQ/yuYi+um98EiDn6N66oYHxUGvKDoHfbJvATLUDWrr59wKlVKj7DMJQSVWYWLMkRgHNF8i08T4SQsKAy7o77qzJg==", 48],
];
for (const [d, signedWith, b64, salt] of NODE) {
  const s = Buffer.from(b64, "base64");
  console.log(d, signedWith, crypto.verify(d, m, { key: RSA_PUB, ...pss }, s), crypto.verify(d, m, { key: RSA_PUB, ...pss, saltLength: salt }, s), crypto.verify(d, m, { key: RSA_PUB, ...pss, saltLength: salt + 1 }, s), crypto.verify(d, bad, { key: RSA_PUB, ...pss }, s), crypto.verify(d, m, RSA_PUB, s));
}
console.log(crypto.verify("sha256", m, { key: EC256_PUB, ...pss }, crypto.sign("sha256", m, { key: EC256_SEC1, ...pss })), crypto.verify(null, Buffer.alloc(0), { key: ED_PUB, ...pss }, crypto.sign(null, Buffer.alloc(0), { key: ED_PRIV, ...pss })));"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:1 6 -1 -2 -2",
            "log:sha256 256 true true true true true false false false",
            "log:sha384 256 true true true true true false false false",
            "log:sha512 256 true true true true true false false false",
            "log:sha256 -2 true true false false false",
            "log:sha256 0 true true false false false",
            "log:sha256 20 true true false false false",
            "log:sha512 -2 true true false false false",
            "log:sha384 48 true true false false false",
            "log:true true",
        ]
    );
}

/// Our default PSS salt is the digest length (Node's is the maximum), so a strict
/// digest-length verifier accepts it; sha1 PSS and other salts are named as unsupported.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pss_salt_deviation_and_unsupported_forms() {
    let code = r#"const show = (f) => { try { console.log("ok", String(f())); } catch (e) { console.log(e.name, e.message); } };
const m = Buffer.from("msg"), pss = { padding: 6 };
show(() => crypto.verify("sha256", m, { key: RSA_PUB, ...pss, saltLength: 32 }, crypto.sign("sha256", m, { key: RSA, ...pss })));
show(() => crypto.verify("sha512", m, { key: RSA_PUB, ...pss, saltLength: crypto.constants.RSA_PSS_SALTLEN_DIGEST }, crypto.sign("sha512", m, { key: RSA, ...pss, saltLength: 64 })));
show(() => crypto.sign("sha1", m, { key: RSA, ...pss }));
show(() => crypto.verify("sha1", m, { key: RSA_PUB, ...pss }, Buffer.alloc(256)));
show(() => crypto.sign("sha256", m, { key: RSA, ...pss, saltLength: crypto.constants.RSA_PSS_SALTLEN_MAX_SIGN }));
show(() => crypto.sign("sha256", m, { key: RSA, padding: 3 }));
show(() => crypto.verify("sha256", m, { key: RSA_PUB, ...pss }, Buffer.alloc(256)));
show(() => crypto.verify("sha256", m, { key: RSA_PUB, ...pss }, Buffer.alloc(255)));"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:ok true".to_string(),
            "log:ok true".to_string(),
            unsupported_sig("sha1 with RSA-PSS padding"),
            unsupported_sig("sha1 with RSA-PSS padding"),
            unsupported_sig("RSA-PSS with saltLength -2"),
            unsupported_sig("padding 3"),
            "log:ok false".to_string(),
            "log:ok false".to_string(),
        ]
    );
}

/// Per-update output lengths and strings are Node 24's; the ciphertexts are NIST SP 800-38A F.2.1 (CBC) and F.5.1, F.5.3, F.5.5 (CTR).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cbc_and_ctr_stream_update_output_like_node() {
    let code = r#"const hex = (s) => Buffer.from(s, "hex");
const K128 = hex("2b7e151628aed2a6abf7158809cf4f3c");
const K192 = hex("8e73b0f7da0e6452c810f32b809079e562f8ead2522c6b7b");
const K256 = hex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
const IV = hex("000102030405060708090a0b0c0d0e0f");
const CTR = hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
const PT = hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710");
// Feeds `data` in pieces of `sizes` and logs each update's output length, then the whole output.
const chunked = (c, data, sizes) => {
  const outs = [];
  let at = 0;
  for (const n of sizes) { outs.push(c.update(data.subarray(at, at + n))); at += n; }
  outs.push(c.final());
  return outs.map((b) => b.length).join(",") + " " + Buffer.concat(outs).toString("hex");
};
const enc = (alg, key, iv) => crypto.createCipheriv(alg, key, iv);
const dec = (alg, key, iv) => crypto.createDecipheriv(alg, key, iv);
console.log(chunked(enc("aes-128-cbc", K128, IV).setAutoPadding(false), PT, [5, 20, 39]));
console.log(chunked(enc("aes-128-cbc", K128, IV), PT, [16, 16, 32]));
const full = (() => { const c = enc("aes-128-cbc", K128, IV); return Buffer.concat([c.update(PT), c.final()]); })();
console.log(full.length, chunked(dec("aes-128-cbc", K128, IV), full, [16, 16, 48]));
console.log(chunked(dec("aes-128-cbc", K128, IV).setAutoPadding(false), full.subarray(0, 64), [17, 47]));
console.log(chunked(enc("aes-128-ctr", K128, CTR), PT, [1, 15, 17, 31]));
console.log(chunked(enc("aes-192-ctr", K192, CTR), PT, [64]));
console.log(chunked(enc("AES-256-CTR", K256, CTR), PT, [3, 3, 58]));
const ct = hex(chunked(enc("aes-256-ctr", K256, CTR), PT, [64]).split(" ")[1]);
console.log(chunked(dec("aes-256-ctr", K256, CTR), ct, [7, 0, 57]).split(" ")[1] === PT.toString("hex"));
console.log(chunked(enc("aes-128-ctr", K128, hex("ffffffffffffffffffffffffffffffff")), Buffer.alloc(40), [7, 25, 8]));
console.log(chunked(enc("aes-128-ctr", K128, CTR), Buffer.alloc(0), []), chunked(enc("aes-128-ctr", K128, CTR).setAutoPadding(false), Buffer.from("x"), [1]));
const c = enc("aes-128-cbc", K128, IV);
const s = [c.update(PT.subarray(0, 16), undefined, "base64"), c.update(PT.subarray(16, 33), undefined, "base64"), c.update(PT.subarray(33), undefined, "base64"), c.final("base64")];
console.log(s.map((x) => x.length).join(","), s.join("") === full.toString("base64"));
const u = enc("aes-128-ctr", K128, CTR);
const words = Buffer.from("héllo wörld €");
const d = dec("aes-128-ctr", K128, CTR);
const ctw = Buffer.concat([u.update(words), u.final()]);
const t = [d.update(ctw.subarray(0, 2), undefined, "utf8"), d.update(ctw.subarray(2, 15), undefined, "utf8"), d.update(ctw.subarray(15), undefined, "utf8"), d.final("utf8")];
console.log(JSON.stringify(t));
const h = enc("aes-128-ctr", K128, CTR);
console.log(h.update("abc", "utf8", "hex") + h.update("def", "utf8", "hex") + h.final("hex"));
console.log(["aes-128-ctr", "aes-192-ctr", "aes-256-ctr"].every((a) => crypto.getCiphers().includes(a)));"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:0,16,48,0 7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b273bed6b8e3c1743b7116e69e222295163ff1caa1681fac09120eca307586e1a7",
            "log:16,16,32,16 7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b273bed6b8e3c1743b7116e69e222295163ff1caa1681fac09120eca307586e1a78cb82807230e1321d3fae00d18cc2012",
            "log:80 0,16,48,0 6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710",
            "log:16,48,0 6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710",
            "log:1,15,17,31,0 874d6191b620e3261bef6864990db6ce9806f66b7970fdff8617187bb9fffdff5ae4df3edbd5d35e5b4f09020db03eab1e031dda2fbe03d1792170a0f3009cee",
            "log:64,0 1abc932417521ca24f2b0459fe7e6e0b090339ec0aa6faefd5ccc2c6f4ce8e941e36b26bd1ebc670d1bd1d665620abf74f78a7f6d29809585a97daec58c6b050",
            "log:3,3,58,0 601ec313775789a5b7a7f504bbf3d228f443e3ca4d62b59aca84e990cacaf5c52b0930daa23de94ce87017ba2d84988ddfc9c58db67aada613c2dd08457941a6",
            "log:true",
            "log:7,25,8,0 8af2860142f786f409307c1a3f7eaaac7df76b0c1ab899b33e42f047b91b546f57127d4034b1bebf",
            "log:0  1,0 94",
            "log:20,20,44,24 true",
            "log:[\"h\",\"éllo wörld \",\"€\",\"\"]",
            "log:8deebc17fd06",
            "log:true",
        ]
    );
}

/// GCM update output arrives per update as in Node 24; the vector is NIST GCM test case 4 (McGrew and Viega).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gcm_streams_update_output_like_node() {
    let code = r#"const hex = (s) => Buffer.from(s, "hex");
const GK = hex("feffe9928665731c6d6a8f9467308308");
const GIV = hex("cafebabefacedbaddecaf888");
const GPT = hex("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39");
const AAD = hex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
const run = (c, data, sizes) => {
  const outs = [];
  let at = 0;
  for (const n of sizes) { outs.push(c.update(data.subarray(at, at + n))); at += n; }
  return outs;
};
const e = crypto.createCipheriv("aes-128-gcm", GK, GIV);
e.setAAD(AAD.subarray(0, 4)).setAAD(AAD.subarray(4));
const parts = run(e, GPT, [1, 6, 16, 37]);
const last = e.final();
const ct = Buffer.concat([...parts, last]), tag = e.getAuthTag();
console.log(parts.map((p) => p.length).join(","), last.length, ct.toString("hex"), tag.toString("hex"));
const d = crypto.createDecipheriv("aes-128-gcm", GK, GIV).setAuthTag(tag);
d.setAAD(AAD);
const plain = run(d, ct, [30, 30]);
console.log(plain.map((p) => p.length).join(","), Buffer.concat([...plain, d.final()]).equals(GPT));
const bad = Buffer.from(tag); bad[15] ^= 1;
const d2 = crypto.createDecipheriv("aes-128-gcm", GK, GIV).setAuthTag(bad);
d2.setAAD(AAD);
const early = d2.update(ct);
let err; try { d2.final(); } catch (x) { err = x.message; }
console.log(early.equals(GPT), err);
const e2 = crypto.createCipheriv("aes-256-gcm", Buffer.alloc(32, 7), Buffer.alloc(12, 9));
const big = Buffer.alloc(100000, 3);
const outs = run(e2, big, [33333, 33333, 33334]);
e2.final();
const d3 = crypto.createDecipheriv("aes-256-gcm", Buffer.alloc(32, 7), Buffer.alloc(12, 9)).setAuthTag(e2.getAuthTag());
console.log(outs.map((o) => o.length).join(","), crypto.hash("sha256", Buffer.concat(outs)), Buffer.concat([d3.update(Buffer.concat(outs)), d3.final()]).equals(big), e2.getAuthTag().toString("hex"));
const e3 = crypto.createCipheriv("aes-192-gcm", Buffer.alloc(24), Buffer.alloc(12));
console.log(e3.update("ab", "utf8", "hex") + e3.update("c", "utf8", "hex") + e3.final("hex"), e3.getAuthTag().toString("hex"));"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:1,6,16,37 0 42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091 5bc94fbc3221a5db94fae95ae7121a47",
            "log:30,30 true",
            "log:true Unsupported state or unable to authenticate data",
            "log:33333,33333,33334 16f462fed7ea159d4a5193b86a15f34a36bdf7c49962250e2cac7fb55dbd8180 true bfc61bd79ada796581818aa697c97bff",
            "log:f98547 f5725980d8fe1f17069cd243a6c7299e",
        ]
    );
}

/// KeyObject properties and exports; hashes of the exported PEM are Node 24's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_objects_create_and_export_like_node() {
    let code = r#"const der = (pem) => Buffer.from(pem.slice(pem.lastIndexOf("-----BEGIN")).replace(/-----[^-]+-----/g, "").replace(/\s+/g, ""), "base64");
const sk = crypto.createSecretKey(Buffer.from("secret-key"));
console.log(sk instanceof crypto.KeyObject, sk.type, sk.symmetricKeySize, sk.asymmetricKeyType, sk.export().toString(), Buffer.isBuffer(sk.export({ format: "buffer" })), crypto.createSecretKey("6b6579", "hex").export().toString());
const keys = [["RSA", RSA], ["RSA_PKCS1", RSA_PKCS1], ["EC256_SEC1", EC256_SEC1], ["EC384", EC384], ["EC521", EC521], ["ED_PRIV", ED_PRIV]];
for (const [n, pem] of keys) {
  const k = crypto.createPrivateKey(pem);
  const p8 = k.export({ type: "pkcs8", format: "pem" });
  const pub = crypto.createPublicKey(pem);
  console.log(n, k instanceof crypto.KeyObject, k.type, k.asymmetricKeyType, k.symmetricKeySize, crypto.hash("sha256", p8), k.export({ type: "pkcs8", format: "der" }).equals(der(p8)), pub.type, pub.asymmetricKeyType, crypto.hash("sha256", pub.export({ type: "spki", format: "pem" })), crypto.createPublicKey(k).export({ type: "spki", format: "der" }).equals(pub.export({ type: "spki", format: "der" })));
}
const pubs = [["RSA_PUB", RSA_PUB], ["RSA_PKCS1_PUB", RSA_PKCS1_PUB], ["RSA_CRT", RSA_CRT], ["EC256_PUB", EC256_PUB], ["EC384_PUB", EC384_PUB], ["EC521_PUB", EC521_PUB], ["ED_PUB", ED_PUB]];
for (const [n, pem] of pubs) {
  const k = crypto.createPublicKey(pem);
  console.log(n, k.type, k.asymmetricKeyType, crypto.hash("sha256", k.export({ type: "spki", format: "pem" })), k.export({ format: "pem", type: "spki" }) === crypto.createPublicKey(Buffer.from(pem)).export({ format: "pem", type: "spki" }));
}
console.log(crypto.createPrivateKey({ key: der(RSA), format: "der", type: "pkcs8" }).export({ type: "pkcs8", format: "pem" }) === crypto.createPrivateKey(RSA).export({ type: "pkcs8", format: "pem" }), crypto.createPrivateKey({ key: der(RSA_PKCS1), format: "der", type: "pkcs1" }).asymmetricKeyType, crypto.createPrivateKey({ key: der(EC256_SEC1), format: "der", type: "sec1" }).asymmetricKeyType, crypto.createPublicKey({ key: der(RSA_PUB), format: "der", type: "spki" }).asymmetricKeyType, crypto.createPublicKey({ key: der(RSA_PKCS1_PUB), format: "der", type: "pkcs1" }).asymmetricKeyType, crypto.createPrivateKey({ key: RSA, format: "pem" }).type);
console.log(crypto.createPrivateKey(RSA).export({ type: "pkcs8", format: "pem" }) === RSA, crypto.createPublicKey(RSA_PUB).export({ type: "spki", format: "pem" }) === RSA_PUB);
console.log(JSON.stringify(crypto.createPrivateKey(EC256_SEC1).export({ type: "pkcs8", format: "pem" })));
console.log(JSON.stringify(crypto.createPublicKey(ED_PRIV).export({ type: "spki", format: "pem" })));"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:true secret 10 undefined secret-key true key",
            "log:RSA true private rsa undefined c2af78ab30f8b711b3d76879fed2f5affaa6dfc6a68b660535a6ed9dc2f5a5da true public rsa f4201f1620c107615dac85039b9ce1db785abfb567259da0d6a72c5b4d4a1b46 true",
            "log:RSA_PKCS1 true private rsa undefined c2af78ab30f8b711b3d76879fed2f5affaa6dfc6a68b660535a6ed9dc2f5a5da true public rsa f4201f1620c107615dac85039b9ce1db785abfb567259da0d6a72c5b4d4a1b46 true",
            "log:EC256_SEC1 true private ec undefined 2f4a6b9c5cfdc79c6d958ef1f40f33c621f03f3f90716be1fbff191b5d0b8e80 true public ec a2d7cb141881a89bf4860040ca04a6d071a63dca1ab1a7bd5f597ec119e61347 true",
            "log:EC384 true private ec undefined 6f5bd4dab19519ffe5994d158515921799ef3c679471c35c8a0284698996361b true public ec 64d6875370b2a7b0f6de8e2eb9e2b037bfbc4dc760c80740dc32141247f710ef true",
            "log:EC521 true private ec undefined 0faaf0796ba1f094e2bbb9764c98c52bdfb6b6ec8457dee4a6af79d348a8e239 true public ec b3288eca7af637eb6c633e5e26a780ae02182d290a3372f2c3dc9907b6f9fd04 true",
            "log:ED_PRIV true private ed25519 undefined c4932a9b6b97423b249a53e58d706f820185467464699038ed7ca5b29815ba03 true public ed25519 7f2d9ed0b71b8e5a6c5cf30e647d6e20b5bca6dac8071f11abe3fef8014db610 true",
            "log:RSA_PUB public rsa f4201f1620c107615dac85039b9ce1db785abfb567259da0d6a72c5b4d4a1b46 true",
            "log:RSA_PKCS1_PUB public rsa f4201f1620c107615dac85039b9ce1db785abfb567259da0d6a72c5b4d4a1b46 true",
            "log:RSA_CRT public rsa f4201f1620c107615dac85039b9ce1db785abfb567259da0d6a72c5b4d4a1b46 true",
            "log:EC256_PUB public ec a2d7cb141881a89bf4860040ca04a6d071a63dca1ab1a7bd5f597ec119e61347 true",
            "log:EC384_PUB public ec 64d6875370b2a7b0f6de8e2eb9e2b037bfbc4dc760c80740dc32141247f710ef true",
            "log:EC521_PUB public ec b3288eca7af637eb6c633e5e26a780ae02182d290a3372f2c3dc9907b6f9fd04 true",
            "log:ED_PUB public ed25519 7f2d9ed0b71b8e5a6c5cf30e647d6e20b5bca6dac8071f11abe3fef8014db610 true",
            "log:true rsa ec rsa rsa private",
            "log:true true",
            "log:\"-----BEGIN PRIVATE KEY-----\\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgge+yZCaHB605XyK/\\nx0bsfDQ7YPKyrSZ1LYgZT1B/YjKhRANCAAScvd1MGmeXgCft2r8v+q0QF3eFW+9/\\n/qDBWlPgZbicO0f9sr65b9rzi+8Xv5RkkYnl1hL3Pp/nz66N/itNXf8i\\n-----END PRIVATE KEY-----\\n\"",
            "log:\"-----BEGIN PUBLIC KEY-----\\nMCowBQYDK2VwAyEA11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=\\n-----END PUBLIC KEY-----\\n\"",
        ]
    );
}

/// KeyObject input and export errors name the unsupported form.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_object_errors_name_the_problem() {
    let code = r#"const show = (f) => { try { console.log("ok", String(f())); } catch (e) { console.log(e.name, e.message); } };
show(() => crypto.createPrivateKey(RSA_PUB));
show(() => crypto.createPrivateKey("not a key"));
show(() => crypto.createPublicKey("not a key"));
show(() => crypto.createPrivateKey(42));
show(() => crypto.createSecretKey(42));
show(() => crypto.createPrivateKey(RSA).export({ type: "pkcs1", format: "pem" }));
show(() => crypto.createPublicKey(RSA_PUB).export({ type: "pkcs8", format: "pem" }));
show(() => crypto.createPublicKey(RSA_PUB).export({ type: "spki", format: "jwk" }));
show(() => crypto.createPrivateKey(RSA).export());
show(() => crypto.createSecretKey(Buffer.alloc(4)).export({ format: "jwk" }));
show(() => crypto.createPrivateKey({ key: Buffer.alloc(4), format: "der", type: "pkcs8" }));
show(() => crypto.createPrivateKey({ key: RSA, format: "jwk" }));
show(() => crypto.createPrivateKey({ key: Buffer.alloc(4), format: "der" }));
show(() => crypto.createPrivateKey("-----BEGIN ENCRYPTED PRIVATE KEY-----\nU0VDUkVUS0VZQllURVM=\n-----END ENCRYPTED PRIVATE KEY-----\n"));
const sk = crypto.createSecretKey(Buffer.alloc(4)), m = Buffer.from("m"), sig = Buffer.alloc(4);
show(() => crypto.verify("sha256", m, sk, sig));
show(() => crypto.createVerify("sha256").update(m).verify(sk, sig));
show(() => crypto.verify("sha256", m, { key: sk }, sig));
show(() => crypto.createPrivateKey(crypto.createPrivateKey(RSA)));
show(() => crypto.createPrivateKey(crypto.createPublicKey(RSA_PUB)));
show(() => crypto.createPrivateKey(sk));
show(() => crypto.createPrivateKey({ key: Buffer.alloc(4), format: "der", type: "toString" }));
show(() => crypto.createPublicKey({ key: Buffer.alloc(4), format: "der", type: "__proto__" }));
show(() => sk.export("x"));"#;
    const PRIVATE: &str =
        "log:Error Unsupported key: expected a PEM private key (PKCS#8, PKCS#1 or SEC1)";
    const KEY_TYPE: &str = "log:TypeError The \"key\" argument must be of type string or an instance of ArrayBuffer, Buffer, TypedArray, or DataView";
    const SECRET_VERIFY: &str =
        "log:TypeError Invalid key object type secret, expected private or public.";
    let export = |t: &str, f: &str| {
        format!(
            "log:Error KeyObject export not supported: type \"{t}\" with format \"{f}\" (supported: pkcs8 for private keys and spki for public keys, as pem or der, or jwk)"
        )
    };
    assert_eq!(
        run_crypto(code).await,
        [
            PRIVATE.to_string(),
            PRIVATE.to_string(),
            "log:Error Unsupported key: expected a PEM public key (SPKI, PKCS#1 or X.509 certificate) or private key".to_string(),
            KEY_TYPE.to_string(),
            KEY_TYPE.to_string(),
            export("pkcs1", "pem"),
            "log:TypeError The property 'options.type' is invalid. Received 'pkcs8'".to_string(),
            "log:ok [object Object]".to_string(),
            "log:TypeError The \"options\" argument must be of type object.".to_string(),
            "log:ok [object Object]".to_string(),
            PRIVATE.to_string(),
            "log:TypeError The \"key.key\" property must be of type object. Received type string".to_string(),
            "log:TypeError The property 'options.type' is invalid. Received undefined".to_string(),
            "log:Error Unsupported key: the encrypted private key could not be read".to_string(),
            SECRET_VERIFY.to_string(),
            SECRET_VERIFY.to_string(),
            SECRET_VERIFY.to_string(),
            KEY_TYPE.to_string(),
            KEY_TYPE.to_string(),
            KEY_TYPE.to_string(),
            "log:TypeError The property 'options.type' is invalid. Received 'toString'".to_string(),
            "log:TypeError The property 'options.type' is invalid. Received '__proto__'".to_string(),
            "log:TypeError The \"options\" argument must be of type object.".to_string(),
        ]
    );
}

/// KeyObjects in sign, verify, createHmac and the ciphers; outputs and messages are Node 24's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_objects_work_in_sign_hmac_and_ciphers() {
    let code = r#"const m = Buffer.from("msg");
const priv = crypto.createPrivateKey(RSA), pub = crypto.createPublicKey(RSA_PUB);
const s1 = crypto.sign("sha256", m, priv).toString("base64");
console.log(s1 === crypto.sign("sha256", m, RSA).toString("base64"), crypto.createSign("sha256").update(m).sign(priv, "base64") === s1, crypto.verify("sha256", m, pub, Buffer.from(s1, "base64")), crypto.createVerify("sha256").update(m).verify(crypto.createPublicKey(priv), s1, "base64"), crypto.verify("sha256", m, priv, Buffer.from(s1, "base64")));
const ec = crypto.createPrivateKey(EC384), ecPub = crypto.createPublicKey(EC384_PUB);
const raw = crypto.sign("sha384", m, { key: ec, dsaEncoding: "ieee-p1363" });
console.log(raw.length, crypto.verify("sha384", m, { key: ecPub, dsaEncoding: "ieee-p1363" }, raw));
const ps = crypto.sign("sha256", m, { key: priv, padding: crypto.constants.RSA_PKCS1_PSS_PADDING, saltLength: 32 });
console.log(crypto.verify("sha256", m, { key: pub, padding: crypto.constants.RSA_PKCS1_PSS_PADDING }, ps));
console.log(crypto.sign(null, Buffer.alloc(0), crypto.createPrivateKey(ED_PRIV)).toString("hex").slice(0, 32));
const sk = crypto.createSecretKey(Buffer.from("key"));
console.log(crypto.createHmac("sha256", sk).update("msg").digest("hex"), crypto.createHmac("sha256", "key").update("msg").digest("hex") === crypto.createHmac("sha256", sk).update("msg").digest("hex"));
const ak = crypto.createSecretKey(Buffer.alloc(16, 1)), iv = Buffer.alloc(16, 2);
const c = crypto.createCipheriv("aes-128-cbc", ak, iv);
const ct = Buffer.concat([c.update("hello"), c.final()]);
const d = crypto.createDecipheriv("aes-128-cbc", ak, iv);
console.log(ct.toString("hex"), Buffer.concat([d.update(ct), d.final()]).toString());
const show = (f) => { try { console.log("ok", String(f())); } catch (e) { console.log(e.name, e.message); } };
show(() => crypto.sign("sha256", m, sk));
show(() => crypto.createCipheriv("aes-128-cbc", priv, iv));
show(() => crypto.createHmac("sha256", priv));
show(() => crypto.sign("sha256", m, pub));"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:true true true true true",
            "log:96 true",
            "log:true",
            "log:e5564300c360ac729086e2cc806e828a",
            "log:2d93cbc1be167bcb1637a4a23cbff01a7878f0c50ee833954ea5221bb1b8c628 true",
            "log:f230676c38a7d716ebed81718e4fc8aa hello",
            "log:TypeError Invalid key object type secret, expected private.",
            "log:TypeError Invalid key object type private, expected secret.",
            "log:TypeError Invalid key object type private, expected secret.",
            "log:TypeError Invalid key object type public, expected private.",
        ]
    );
}

/// Pinned from Node 24.16: 1024- and 8192-bit RSA sign and verify in both paddings; PKCS#1 v1.5
/// signatures are deterministic, so their hashes are Node's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rsa_1024_and_8192_bit_keys_sign_and_verify_like_node() {
    let code = r#"const m = Buffer.from("msg"), bad = Buffer.from("msh"), C = crypto.constants;
const pss = { padding: C.RSA_PKCS1_PSS_PADDING }, auto = { ...pss, saltLength: C.RSA_PSS_SALTLEN_AUTO };
const h = (b) => crypto.createHash("sha256").update(b).digest("hex").slice(0, 16);
const flip = (b) => { const t = Buffer.from(b); t[t.length >> 1] ^= 1; return t; };
for (const [bits, K, P, nodePss] of [[1024, RSA1024, RSA1024_PUB, PSS1024], [8192, RSA8192, RSA8192_PUB, PSS8192]]) {
  const v = (d, s, o = {}, data = m) => crypto.verify(d, data, { key: P, ...o }, s);
  const s = crypto.sign("sha256", m, K), s384 = crypto.sign("sha384", m, K);
  console.log(bits, s.length, h(s), h(s384), v("sha256", s), v("sha384", s384), v("sha256", s, {}, bad), v("sha256", flip(s)));
  const n = Buffer.from(nodePss.trim(), "base64"), p = crypto.sign("sha256", m, { key: K, ...pss });
  console.log(bits, p.length, v("sha256", n, pss), v("sha256", n, auto), v("sha256", flip(n), pss), v("sha256", n, pss, bad), v("sha256", p, pss), v("sha256", flip(p), pss), v("sha256", n));
}
console.log(h(crypto.sign("sha1", m, RSA1024)), crypto.verify("sha1", m, RSA1024_PUB, crypto.sign("sha1", m, RSA1024_PKCS1)), h(crypto.sign("sha512", m, RSA1024_PKCS1)));
const q = crypto.sign("sha512", m, { key: RSA1024, ...pss });
console.log(q.length, crypto.verify("sha512", m, { key: RSA1024_PUB, ...auto }, q), crypto.verify("sha512", m, { key: RSA1024_PUB, ...pss, saltLength: 62 }, q), crypto.verify("sha512", bad, { key: RSA1024_PUB, ...auto }, q));
const k = crypto.createPrivateKey(RSA1024_PKCS1), pub = crypto.createPublicKey(RSA1024);
console.log(k.asymmetricKeyType, k.export({ type: "pkcs8", format: "pem" }) === RSA1024, pub.export({ type: "spki", format: "pem" }) === RSA1024_PUB, crypto.createPublicKey(RSA8192).export({ type: "spki", format: "pem" }) === RSA8192_PUB, h(crypto.sign("sha256", m, k)) === h(crypto.sign("sha256", m, RSA1024)), crypto.verify("sha256", m, pub, crypto.sign("sha256", m, k)));"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:1024 128 8fb23c70a3861cd5 ed7bf6038e570ecf true true false false",
            "log:1024 128 true true false false true false false",
            "log:8192 1024 645bfec705b55227 c834460c3eaf14e3 true true false false",
            "log:8192 1024 true true false false true false false",
            "log:2d35d3d1c339e00a true c81c41ec93ebcea8",
            "log:128 true true false",
            "log:rsa true true true true true",
        ]
    );
}

/// Deviation: Node 24 signs and verifies with a 512-bit key; Apinata names the size and refuses
/// it. A PSS salt that does not fit the key fails with Node's reason in our words.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rsa_keys_outside_1024_to_8192_bits_are_named() {
    let code = r#"const RSA512_SIG = "TcyqhD5kl87QCZ2eQI03vFTAfd45728v1IEwWTTkuNifEGVe1Qdy8e7IprlmzuCE9bo+VtIFzpUczeV3sJmjnA==";
const show = (f) => { try { console.log("ok", String(f())); } catch (e) { console.log(e.name, e.message); } };
const m = Buffer.from("msg"), pss = { padding: 6 };
const s512 = Buffer.from(RSA512_SIG, "base64");
show(() => crypto.sign("sha256", m, RSA512).length);
show(() => crypto.sign("sha256", m, { key: RSA512, ...pss }).length);
show(() => crypto.verify("sha256", m, RSA512_PUB, s512));
show(() => crypto.verify("sha256", m, { key: RSA512_PUB, ...pss }, s512));
show(() => crypto.verify("sha256", m, RSA512, s512));
show(() => crypto.createPrivateKey(RSA512).asymmetricKeyType);
show(() => crypto.sign("sha512", m, { key: RSA1024, ...pss, saltLength: 64 }).length);
show(() => crypto.sign("sha512", m, { key: RSA1024, ...pss, saltLength: crypto.constants.RSA_PSS_SALTLEN_DIGEST }).length);"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:Error Unsupported key: a 512-bit RSA key (supported: 1024 to 8192 bits)",
            "log:Error Unsupported key: a 512-bit RSA key (supported: 1024 to 8192 bits)",
            "log:Error Unsupported key: a 512-bit RSA key (supported: 1024 to 8192 bits)",
            "log:Error Unsupported key: a 512-bit RSA key (supported: 1024 to 8192 bits)",
            "log:Error Unsupported key: a 512-bit RSA key (supported: 1024 to 8192 bits)",
            "log:ok rsa",
            "log:Error Unsupported key: RSA-PSS with sha512 and saltLength 64 does not fit a 1024-bit RSA key (data too large for key size)",
            "log:Error Unsupported key: RSA-PSS with sha512 and saltLength 64 does not fit a 1024-bit RSA key (data too large for key size)",
        ]
    );
}

/// Pinned from Node 24.16: a streamed utf8 decipher replaces each broken sequence once, wherever
/// the chunks split.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decipher_utf8_replaces_each_broken_sequence_once() {
    let code = r#"const K = Buffer.alloc(16, 7), IV = Buffer.alloc(16, 9);
const PT = Buffer.from([0x61, 0xf0, 0x9f, 0x98, 0x80, 0xe2, 0x82, 0x61, 0x80, 0x80, 0xed, 0xa0, 0x80, 0x62, 0xf0, 0x9f]);
for (const alg of ["aes-128-ctr", "aes-128-cbc"]) {
  const c = crypto.createCipheriv(alg, K, IV), ct = Buffer.concat([c.update(PT), c.final()]);
  for (const cut of [[3, 9], [1, 15], [6, 14]]) {
    const d = crypto.createDecipheriv(alg, K, IV);
    const parts = [d.update(ct.subarray(0, cut[0]), undefined, "utf8"), d.update(ct.subarray(cut[0], cut[1]), undefined, "utf8"), d.update(ct.subarray(cut[1]), undefined, "utf8"), d.final("utf8")];
    console.log(alg, cut.join(","), JSON.stringify(parts.join("")), parts.join("").length);
  }
}"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:aes-128-ctr 3,9 \"a\u{1f600}\u{fffd}a\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}b\u{fffd}\" 12",
            "log:aes-128-ctr 1,15 \"a\u{1f600}\u{fffd}a\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}b\u{fffd}\" 12",
            "log:aes-128-ctr 6,14 \"a\u{1f600}\u{fffd}a\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}b\u{fffd}\" 12",
            "log:aes-128-cbc 3,9 \"a\u{1f600}\u{fffd}a\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}b\u{fffd}\" 12",
            "log:aes-128-cbc 1,15 \"a\u{1f600}\u{fffd}a\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}b\u{fffd}\" 12",
            "log:aes-128-cbc 6,14 \"a\u{1f600}\u{fffd}a\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}b\u{fffd}\" 12",
        ]
    );
}

/// Pinned from Node 24.16: a falsy output encoding keeps digest, cipher and sign output a Buffer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn falsy_output_encodings_return_a_buffer() {
    let code = r#"const key = Buffer.alloc(16, 1), iv = Buffer.alloc(16, 2);
for (const e of [null, "", 0]) {
  const c = crypto.createCipheriv("aes-128-cbc", key, iv);
  console.log(JSON.stringify(e), Buffer.isBuffer(crypto.createHash("sha256").update("x").digest(e)), Buffer.isBuffer(crypto.createHmac("sha256", "k").update("x").digest(e)), Buffer.isBuffer(c.update("x", "utf8", e)), Buffer.isBuffer(c.final(e)), Buffer.isBuffer(crypto.createSign("sha256").update("x").sign(RSA, e)));
}"#;
    assert_eq!(
        run_crypto(code).await,
        [
            "log:null true true true true true",
            "log:\"\" true true true true true",
            "log:0 true true true true true",
        ]
    );
}

/// Pinned from Node 24.16: `crypto.hash` takes an output encoding string or an options object.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hash_takes_an_encoding_or_options() {
    let code = r#"const show = (v) => Buffer.isBuffer(v) ? "Buffer " + v.toString("hex").slice(0, 8) : JSON.stringify(v).replace(/[^ -~]/g, (c) => "\\u" + c.charCodeAt(0).toString(16).padStart(4, "0"));
const tryIt = (f) => { try { return show(f()); } catch (e) { return e.name + " " + e.code + " " + JSON.stringify(e.message); } };
const h = (o) => tryIt(() => crypto.hash("sha1", "a", o));
// Apinata has no XOF algorithm (md5 to sha512), so only the digest length passes as outputLength.
for (const o of [null, "", 0, false, NaN, undefined, "buffer", "BUFFER", "Base64", "bogus", "it's", "raw", {}, { outputEncoding: null }, { outputEncoding: "base64" }, { outputEncoding: "" }, { outputEncoding: "buffer" }, { outputEncoding: 1 }, { outputEncoding: Symbol("s") }, { outputEncoding: "bogus", outputLength: -1 }, { outputLength: 20 }, { outputLength: 0 }, { outputLength: 21 }, { outputLength: -1 }, { outputLength: 1.5 }, { outputLength: NaN }, { outputLength: null }, { outputLength: "x" }, [], function foo() {}, () => {}, Symbol("s"), 1n, new String("base64")]) console.log(h(o));
console.log(tryIt(() => crypto.hash("sha1", "a")), tryIt(() => crypto.hash("nope", "a", null)), Buffer.isBuffer(crypto.createHash("sha1").update("a").digest("")));
console.log(tryIt(() => crypto.hash("sha1", 1, null)));
let reads = 0;
const counted = { get outputEncoding() { reads++; return "base64"; }, get outputLength() { reads += 10; return 20; } };
console.log(h(counted), reads);"#;
    assert_eq!(
        run_crypto(code).await,
        [
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"options\" argument must be of type object. Received null""#,
            r#"log:"\ufffd\ufffd\ufffd7\ufffd\ufffd\ufffd\ufffd\ufffd]\u001d\u0739\ufffd\ufffd\ufffd7vg\ufffd""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"options\" argument must be of type object. Received type number (0)""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"options\" argument must be of type object. Received type boolean (false)""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"options\" argument must be of type object. Received type number (NaN)""#,
            r#"log:"86f7e437faa5a7fce15d1ddcb9eaeaea377667b8""#,
            r#"log:Buffer 86f7e437"#,
            r#"log:Buffer 86f7e437"#,
            r#"log:"hvfkN/qlp/zhXR3cuerq6jd2Z7g=""#,
            r#"log:TypeError ERR_INVALID_ARG_VALUE "The argument 'outputEncoding' is invalid. Received 'bogus'""#,
            r#"log:TypeError ERR_INVALID_ARG_VALUE "The argument 'outputEncoding' is invalid. Received \"it's\"""#,
            r#"log:TypeError ERR_INVALID_ARG_VALUE "The argument 'outputEncoding' is invalid. Received 'raw'""#,
            r#"log:"86f7e437faa5a7fce15d1ddcb9eaeaea377667b8""#,
            r#"log:"86f7e437faa5a7fce15d1ddcb9eaeaea377667b8""#,
            r#"log:"hvfkN/qlp/zhXR3cuerq6jd2Z7g=""#,
            r#"log:"\ufffd\ufffd\ufffd7\ufffd\ufffd\ufffd\ufffd\ufffd]\u001d\u0739\ufffd\ufffd\ufffd7vg\ufffd""#,
            r#"log:Buffer 86f7e437"#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"outputEncoding\" argument must be of type string. Received type number (1)""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"outputEncoding\" argument must be of type string. Received type symbol (Symbol(s))""#,
            r#"log:TypeError ERR_INVALID_ARG_VALUE "The argument 'outputEncoding' is invalid. Received 'bogus'""#,
            r#"log:"86f7e437faa5a7fce15d1ddcb9eaeaea377667b8""#,
            r#"log:Error undefined "Output length 0 is invalid for sha1, which does not support XOF""#,
            r#"log:Error undefined "Output length 21 is invalid for sha1, which does not support XOF""#,
            r#"log:RangeError ERR_OUT_OF_RANGE "The value of \"outputLength\" is out of range. It must be >= 0 && <= 4294967295. Received -1""#,
            r#"log:RangeError ERR_OUT_OF_RANGE "The value of \"outputLength\" is out of range. It must be an integer. Received 1.5""#,
            r#"log:RangeError ERR_OUT_OF_RANGE "The value of \"outputLength\" is out of range. It must be an integer. Received NaN""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"outputLength\" argument must be of type number. Received null""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"outputLength\" argument must be of type number. Received type string ('x')""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"options\" argument must be of type object. Received an instance of Array""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"options\" argument must be of type object. Received function foo""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"options\" argument must be of type object. Received function ""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"options\" argument must be of type object. Received type symbol (Symbol(s))""#,
            r#"log:TypeError ERR_INVALID_ARG_TYPE "The \"options\" argument must be of type object. Received type bigint (1n)""#,
            r#"log:"86f7e437faa5a7fce15d1ddcb9eaeaea377667b8""#,
            r#"log:"86f7e437faa5a7fce15d1ddcb9eaeaea377667b8" TypeError ERR_INVALID_ARG_TYPE "The \"options\" argument must be of type object. Received null" true"#,
            r#"log:TypeError undefined "The \"data\" argument must be of type string or an instance of Buffer, TypedArray, or DataView""#,
            r#"log:"hvfkN/qlp/zhXR3cuerq6jd2Z7g=" 11"#,
        ]
    );
}
