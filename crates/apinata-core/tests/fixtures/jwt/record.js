// Records Node's answers to a case file, which the shim's are asserted against:
// node record.js plain > plain-node.txt, and the same for forms, edges, closed and strict, with
// Node 24 and jsonwebtoken@9.0.3 on NODE_PATH.
const fs = require("fs");
const path = require("path");
const read = (f) => fs.readFileSync(path.join(__dirname, f), "utf8");
Object.defineProperty(globalThis, "crypto", { value: require("crypto") });
Object.assign(globalThis, { jwt: require("jsonwebtoken"), K: JSON.parse(read("keys.json")), E: JSON.parse(read("edge.json")) });
(0, eval)(read("cases-prelude.js") + read(`${process.argv[2]}.js`));
