// Records Node's answers to a case file, which the shim's are asserted against:
// node gen.js > malformed-node.txt, node gen.js pss.js > pss-node.txt. Run from this directory with Node 24.
const fs = require("fs");
const read = (f) => fs.readFileSync(f, "utf8");
Object.defineProperty(globalThis, "crypto", { value: require("crypto") });
Object.assign(globalThis, {
  RSA: read("../jwt/rsa.pem"), RSA_PUB: read("../jwt/rsa.pub.pem"), RSA_PKCS1: read("../jwt/rsa-pkcs1.pem"),
  EC256_SEC1: read("../jwt/ec256-sec1.pem"), EC256_PUB: read("../jwt/ec256.pub.pem"),
  EC384: read("../jwt/ec384.pem"), EC384_PUB: read("../jwt/ec384.pub.pem"),
  // The P-521 pair the shim's test runner declares.
  EC521: "-----BEGIN PRIVATE KEY-----\nMIHuAgEAMBAGByqGSM49AgEGBSuBBAAjBIHWMIHTAgEBBEIAP7fpaoO7H0Ou/sIV\nkdOC+nV6HOVHhNfMChvCz5WiqaBx6uliiM43b5O+DmwvnvCo1ZG61j4mpNhpF1vD\n51k1Q5ehgYkDgYYABAGIghG8jT5TtlyKyBqicRi4OfJgaaQi4YOxn7t/jKsoFHVP\n2PdKc0DmIUOmTfGwDaX4JGv3UGE7L9MiP9Hl7lKimwAwoL+GqPZTf0WjFC2PUsST\nIMkp7cib6G9QPN2O3IbwOM08Imu0+iPoBH+1UcXN1PnAvYiJWX/PdrA/AHEGwaNH\nQQ==\n-----END PRIVATE KEY-----\n",
  EC521_PUB: "-----BEGIN PUBLIC KEY-----\nMIGbMBAGByqGSM49AgEGBSuBBAAjA4GGAAQBiIIRvI0+U7ZcisgaonEYuDnyYGmk\nIuGDsZ+7f4yrKBR1T9j3SnNA5iFDpk3xsA2l+CRr91BhOy/TIj/R5e5SopsAMKC/\nhqj2U39FoxQtj1LEkyDJKe3Im+hvUDzdjtyG8DjNPCJrtPoj6AR/tVHFzdT5wL2I\niVl/z3awPwBxBsGjR0E=\n-----END PUBLIC KEY-----\n",
  // The RFC 8032 section 7.1 test 1 key as PKCS#8 and SPKI.
  ED_PRIV: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----\n",
  ED_PUB: "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=\n-----END PUBLIC KEY-----\n",
  K: JSON.parse(read("vectors.json")), P: JSON.parse(read("pss-keys.json")), PW: "correct horse",
});
(0, eval)(read(process.argv[2] ?? "malformed.js"));
