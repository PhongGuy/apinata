// Shared by the case files. A line carries a tag and either the result or the error's name and
// message. PS and ES signatures are random, so those lines carry the signing input and length.
const P = { sub: "x", iat: 1700000000 };
const line = (tag, f) => {
  let text;
  try { text = `ok ${f()}`; } catch (e) { text = `${e.name} ${e.message}`; }
  console.log(`${tag}: ${text}`);
};
const stable = (t) => {
  const [head, payload, signature] = t.split(".");
  if (!/^(ES|PS)/.test(JSON.parse(Buffer.from(head, "base64url").toString()).alg)) return t;
  return `${head}.${payload} signature ${Buffer.from(signature, "base64url").length}`;
};
const S = (tag, key, algorithm, extra) => line(tag, () => stable(jwt.sign(P, key, { algorithm, ...extra })));
const V = (tag, token, key, options) => line(tag, () => jwt.verify(token, key, options).sub);
// Signs, then checks the token with `pub`, so a random signature still proves the key was used.
const R = (tag, key, algorithm, pub, extra) => line(tag, () => {
  const t = jwt.sign(P, key, { algorithm, ...extra });
  return `${stable(t)} ${jwt.verify(t, pub, { algorithms: [algorithm] }).sub}`;
});
