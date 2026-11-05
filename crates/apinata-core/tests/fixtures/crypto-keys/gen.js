// Records Node's answers to a case file, which the shim's are asserted against:
// node gen.js > malformed-node.txt, node gen.js pss.js > pss-node.txt. Run from this directory with Node 24.
const fs = require("fs");
const read = (f) => fs.readFileSync(f, "utf8");
Object.defineProperty(globalThis, "crypto", { value: require("crypto") });
Object.assign(globalThis, {
  RSA: read("../jwt/rsa.pem"), RSA_PUB: read("../jwt/rsa.pub.pem"), RSA_PKCS1: read("../jwt/rsa-pkcs1.pem"),
  EC256_SEC1: read("../jwt/ec256-sec1.pem"), EC256_PUB: read("../jwt/ec256.pub.pem"),
  EC384: read("../jwt/ec384.pem"), EC384_PUB: read("../jwt/ec384.pub.pem"),
  // The RFC 8032 section 7.1 test 1 key as PKCS#8 and SPKI.
  ED_PRIV: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----\n",
  ED_PUB: "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=\n-----END PUBLIC KEY-----\n",
  K: JSON.parse(read("vectors.json")), P: JSON.parse(read("pss-keys.json")), PW: "correct horse",
});
(0, eval)(read(process.argv[2] ?? "malformed.js"));
