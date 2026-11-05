// Node's timingSafeEqual, PBKDF2, scrypt, AES ciphers, signatures and KeyObjects, added to the
// `crypto` object host_libs.js built. Arguments are checked here with Node's messages; the host
// works on byte arrays, so ciphers keep their pending bytes, chained IV and counter here between
// calls, and KeyObjects keep their bytes here. Node's `Received` text is kept for values that
// cannot be secret; a passphrase or a JWK member is described by its type alone.
(() => {
  const NAMES = ["__timingSafeEqual", "__pbkdf2", "__scrypt", "__cipher", "__sign", "__verify", "__keyObject", "__jwkKey", "__jwkParts", "__keyDetails", "__encryptKey"];
  const [equal, pbkdf2, scrypt, cipher, sign, verify, keyObject, jwkKey, jwkParts, keyDetails, encryptKey] = NAMES.map((n) => globalThis[n]);
  for (const n of NAMES) delete globalThis[n];
  const crypto = globalThis.crypto;
  const Buf = () => globalThis.Buffer;

  const BYTES = "an instance of ArrayBuffer, Buffer, TypedArray, or DataView";
  const isBytes = (v) => v instanceof ArrayBuffer || ArrayBuffer.isView(v);
  const list = (v) => Array.from(v instanceof ArrayBuffer ? new Uint8Array(v) : new Uint8Array(v.buffer, v.byteOffset, v.byteLength));
  // A string in `encoding` (utf8 by default) or bytes, as the number array the host takes.
  const input = (value, what, encoding) => {
    if (typeof value === "string") return list(Buf().from(value, encoding ?? "utf8"));
    if (isBytes(value)) return list(value);
    throw new TypeError(`The "${what}" argument must be of type string or ${BYTES}`);
  };
  const output = (bytes, encoding) => {
    const buf = Buf().from(bytes);
    return !encoding || String(encoding).toLowerCase() === "buffer" ? buf : buf.toString(encoding);
  };
  const integer = (value, what, min, max) => {
    if (typeof value !== "number") throw new TypeError(`The "${what}" argument must be of type number`);
    if (!Number.isInteger(value)) throw new RangeError(`The value of "${what}" is out of range. It must be an integer. Received ${value}`);
    if (value < min || value > max) throw new RangeError(`The value of "${what}" is out of range. It must be >= ${min} && <= ${max}. Received ${value}`);
    return value;
  };
  const callbackOf = (callback) => {
    if (typeof callback !== "function") throw new TypeError('The "callback" argument must be of type function');
    return callback;
  };
  // Callback forms check their arguments at once, do the work, and report on a microtask.
  const later = (callback, work) => {
    let result, error = null;
    try { result = work(); } catch (e) { error = e; }
    Promise.resolve().then(() => (error ? callback(error) : callback(null, result)));
  };

  crypto.timingSafeEqual = (buf1, buf2) => {
    [buf1, buf2].forEach((b, i) => {
      if (!isBytes(b)) throw new TypeError(`The "buf${i + 1}" argument must be ${BYTES}.`);
    });
    const [a, b] = [list(buf1), list(buf2)];
    if (a.length !== b.length) throw new RangeError("Input buffers must have the same byte length");
    return equal(a, b);
  };

  // Iterations and lengths are capped, since a host call cannot be interrupted by the timeout.
  const PBKDF2 = { sha1: 20, sha256: 32, sha384: 48, sha512: 64 };
  const PBKDF2_WORK = 2 ** 24;
  const pbkdf2Work = (password, salt, iterations, keylen, digest) => {
    const p = input(password, "password"), s = input(salt, "salt");
    integer(iterations, "iterations", 1, 10000000);
    integer(keylen, "keylen", 0, 65536);
    if (typeof digest !== "string") throw new TypeError('The "digest" argument must be of type string');
    const name = digest.toLowerCase();
    if (!Object.hasOwn(PBKDF2, name)) {
      throw new Error(`Digest method not supported: "${digest}" (supported: ${Object.keys(PBKDF2).join(", ")}; require('crypto-js') has more)`);
    }
    const work = iterations * Math.ceil(keylen / PBKDF2[name]);
    if (work > PBKDF2_WORK) {
      throw new RangeError(`The pbkdf2 work (iterations x ceil(keylen / digest length)) must be <= ${PBKDF2_WORK}. Received ${work}`);
    }
    return () => Buf().from(pbkdf2(name, p, s, iterations, keylen));
  };
  crypto.pbkdf2Sync = (password, salt, iterations, keylen, digest) => pbkdf2Work(password, salt, iterations, keylen, digest)();
  crypto.pbkdf2 = (password, salt, iterations, keylen, digest, callback) => {
    callbackOf(callback);
    later(callback, pbkdf2Work(password, salt, iterations, keylen, digest));
  };

  const scryptWork = (password, salt, keylen, options) => {
    const p = input(password, "password"), s = input(salt, "salt");
    integer(keylen, "keylen", 0, 65536);
    const o = options ?? {};
    const pick = (short, long, fallback) => {
      if (o[short] !== undefined && o[long] !== undefined) {
        throw new TypeError(`Option "${short}" cannot be used in combination with option "${long}"`);
      }
      return o[short] ?? o[long] ?? fallback;
    };
    const N = pick("N", "cost", 16384), r = pick("r", "blockSize", 8), par = pick("p", "parallelization", 1);
    const maxmem = integer(o.maxmem ?? 33554432, "maxmem", 0, 268435456);
    const valid = [N, r, par].every(Number.isInteger) && N > 1 && Number.isInteger(Math.log2(N)) && r >= 1 && par >= 1 && r * par < 2 ** 30;
    if (!valid) throw new RangeError("Invalid scrypt params");
    if (N * r * par > 2 ** 25) throw new RangeError("Invalid scrypt params: N * r * p must be <= 33554432");
    // OpenSSL's memory estimate, which Node compares with maxmem.
    if (128 * r * (N + 2 + par) > maxmem) throw new RangeError("Invalid scrypt params: memory limit exceeded");
    return () => Buf().from(scrypt(p, s, keylen, N, r, par));
  };
  crypto.scryptSync = (password, salt, keylen, options) => scryptWork(password, salt, keylen, options)();
  crypto.scrypt = (password, salt, keylen, options, callback) => {
    if (typeof options === "function") [options, callback] = [undefined, options];
    callbackOf(callback);
    later(callback, scryptWork(password, salt, keylen, options));
  };

  // A KeyObject's type, asymmetric kind and bytes (secret, PKCS#8 or SPKI) live here, out of
  // the script's reach; only this file's constructor token can make one.
  const KEY_DATA = new WeakMap();
  const MAKE = Symbol("KeyObject");
  class KeyObject {
    constructor(token, type, kind, bytes) {
      if (token !== MAKE) throw new TypeError("Illegal constructor");
      KEY_DATA.set(this, { type, kind, bytes });
    }
    get type() { return KEY_DATA.get(this).type; }
    get asymmetricKeyType() { return KEY_DATA.get(this).kind; }
    // Node builds the object once for RSA and EC keys and anew on each read for Ed25519. An
    // RSA-PSS key adds the parameters it is held to, when it has them.
    get asymmetricKeyDetails() {
      const data = KEY_DATA.get(this);
      if (data.type === "secret") return undefined;
      if (data.kind === "ed25519") return {};
      if (data.details === undefined) {
        const [bits, exponent, curve, hashAlgorithm, mgf1HashAlgorithm, saltLength] = keyDetails(data.kind, data.bytes, data.type === "private");
        data.details = data.kind === "ec" ? { namedCurve: curve } : { modulusLength: bits, publicExponent: BigInt("0x" + (Buf().from(exponent).toString("hex") || "0")) };
        if (hashAlgorithm !== "") Object.assign(data.details, { hashAlgorithm, mgf1HashAlgorithm, saltLength });
      }
      return data.details;
    }
    get symmetricKeySize() { const k = KEY_DATA.get(this); return k.type === "secret" ? k.bytes.length : undefined; }
    export(options) {
      const { type, kind, bytes } = KEY_DATA.get(this);
      if (type === "secret") {
        if (options !== undefined && (options === null || typeof options !== "object")) {
          throw new TypeError('The "options" argument must be of type object.');
        }
        const format = options?.format;
        if (format === "jwk") return { kty: "oct", k: base64url(bytes) };
        if (format !== undefined && format !== "buffer") oneOf(format, "options.format", ["buffer", "jwk"], "undefined, ");
        return Buf().from(bytes);
      }
      if (options === null || typeof options !== "object") throw new TypeError('The "options" argument must be of type object.');
      const { type: form, format } = options;
      if (format === "jwk") {
        if (type === "private" && options.passphrase !== undefined) throw new Error("The selected key encoding jwk does not support encryption.");
        if (kind === "rsa-pss") throw Object.assign(new Error("Unsupported JWK Key Type."), { code: "ERR_CRYPTO_JWK_UNSUPPORTED_KEY_TYPE" });
        return exportJwk(type, kind, bytes);
      }
      if (format !== "pem" && format !== "der") throw invalidOption("options.format", format);
      if (!EXPORT_TYPES[type].includes(form)) throw invalidOption("options.type", form);
      if (kind === "rsa-pss" && form === "pkcs1") {
        throw Object.assign(new Error("The selected key encoding pkcs1 can only be used for RSA keys."), { code: "ERR_CRYPTO_INCOMPATIBLE_KEY_OPTIONS" });
      }
      if (form !== (type === "private" ? "pkcs8" : "spki")) {
        throw new Error(`KeyObject export not supported: type "${form}" with format "${format}" (supported: pkcs8 for private keys and spki for public keys, as pem or der, or jwk)`);
      }
      const encryption = type === "private" ? encryptionOf(options) : null;
      if (encryption !== null) {
        const der = Buf().from(encryptKey(bytes, ...encryption));
        return format === "der" ? der : pemOf(der, "ENCRYPTED PRIVATE KEY");
      }
      return format === "der" ? Buf().from(bytes) : pemOf(bytes, type === "private" ? "PRIVATE KEY" : "PUBLIC KEY");
    }
  }
  // The encodings Node takes for each key type; the first alone is built here.
  const EXPORT_TYPES = { private: ["pkcs8", "pkcs1", "sec1"], public: ["spki", "pkcs1"] };
  const pemOf = (bytes, label) => {
    const lines = Buf().from(bytes).toString("base64").match(/.{1,64}/g) ?? [];
    return `-----BEGIN ${label}-----\n${lines.join("\n")}\n-----END ${label}-----\n`;
  };
  const keyData = (key) => (key instanceof KeyObject ? KEY_DATA.get(key) : undefined);
  // DER arrives as PEM under the label its `type` names, so the host reads one form.
  const DER_LABELS = {
    private: { pkcs8: "PRIVATE KEY", pkcs1: "RSA PRIVATE KEY", sec1: "EC PRIVATE KEY" },
    public: { spki: "PUBLIC KEY", pkcs1: "RSA PUBLIC KEY", pkcs8: "PRIVATE KEY", sec1: "EC PRIVATE KEY" },
  };
  const base64url = (bytes) => Buf().from(bytes).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  const RSA_PRIVATE = ["d", "p", "q", "dp", "dq", "qi"];
  // Members come back from the host in a fixed order; these objects keep Node's member order.
  const exportJwk = (type, kind, bytes) => {
    const [crv, parts] = jwkParts(bytes, type === "private");
    const [first, second, third, ...rest] = parts.map(base64url);
    const isPrivate = type === "private";
    if (kind === "ec") return isPrivate ? { kty: "EC", x: first, y: second, crv, d: third } : { kty: "EC", x: first, y: second, crv };
    if (kind !== "rsa") return isPrivate ? { crv, d: first, x: second, kty: "OKP" } : { crv, x: first, kty: "OKP" };
    const jwk = { kty: "RSA", n: first, e: second };
    if (isPrivate) [third, ...rest].forEach((value, i) => { jwk[RSA_PRIVATE[i]] = value; });
    return jwk;
  };
  const described = (value) => {
    if (value === undefined || value === null) return String(value);
    if (typeof value === "object") return `an instance of ${value.constructor?.name ?? "Object"}`;
    return `type ${typeof value}`;
  };
  const shown = (value) => {
    if (typeof value === "string") return `'${value}'`;
    return typeof value === "object" ? described(value) : String(value);
  };
  const invalidValue = (message) => Object.assign(new TypeError(message), { code: "ERR_INVALID_ARG_VALUE" });
  const invalidOption = (what, value) => invalidValue(`The property '${what}' is invalid. Received ${shown(value)}`);
  const oneOf = (value, what, allowed, prefix = "") => {
    if (allowed.includes(value)) return;
    throw invalidValue(`The property '${what}' must be one of: ${prefix}${allowed.map((a) => `'${a}'`).join(", ")}. Received ${shown(value)}`);
  };
  const strings = (jwk, names) => {
    for (const name of names) {
      if (typeof jwk[name] !== "string") throw new TypeError(`The "key.${name}" property must be of type string. Received ${described(jwk[name])}`);
    }
  };
  const JWK_CURVES = ["P-256", "P-384", "P-521", "Ed25519"];
  // Node's checks in Node's order, then the members one key form needs: a public key reads
  // nothing private, and Ed25519 keeps only `d` or only `x`, as Node does.
  const importJwk = (jwk, type) => {
    if (jwk === null || typeof jwk !== "object" || Array.isArray(jwk)) {
      throw new TypeError(`The "key.key" property must be of type object. Received ${described(jwk)}`);
    }
    const isPublic = type === "public";
    oneOf(jwk.kty, "key.kty", ["RSA", "EC", "OKP", "AKP"]);
    if (jwk.kty === "AKP") throw new Error('Unsupported key: JWK key type "AKP" (supported: RSA, EC and OKP with Ed25519)');
    let name = "RSA", members = isPublic ? ["n", "e"] : ["n", "e", ...RSA_PRIVATE];
    if (jwk.kty !== "RSA") {
      strings(jwk, ["crv"]);
      const ec = jwk.kty === "EC";
      oneOf(jwk.crv, "key.crv", ec ? ["P-256", "secp256k1", "P-384", "P-521"] : ["Ed25519", "Ed448", "X25519", "X448"]);
      name = jwk.crv;
      members = ec ? (isPublic ? ["x", "y"] : ["x", "y", "d"]) : isPublic ? ["x"] : ["x", "d"];
    }
    strings(jwk, members);
    if (jwk.kty !== "RSA" && !JWK_CURVES.includes(name)) throw new Error(`Unsupported key: JWK curve "${name}" (supported: P-256, P-384, P-521 and Ed25519)`);
    if (jwk.kty === "OKP") members = [isPublic ? "x" : "d"];
    const [kind, der] = jwkKey(name, members.map((m) => list(Buf().from(jwk[m], "base64"))), isPublic);
    return new KeyObject(MAKE, type, kind, der);
  };
  const passphraseOf = (value, encoding) => {
    if (typeof value === "string") return list(Buf().from(value, encoding ?? "utf8"));
    if (isBytes(value)) return list(value);
    throw new TypeError("The property 'options.passphrase' is invalid.");
  };
  // Public keys ignore both options. On a private key either one asks for encryption, and half a
  // pair is Node's error. Returns null or `[cipher, passphrase bytes]` for the host.
  const encryptionOf = ({ cipher: name, passphrase, encoding }) => {
    if ((name === undefined || name === null) && passphrase === undefined) return null;
    if (typeof name !== "string") throw invalidOption("options.cipher", name);
    const bytes = passphraseOf(passphrase, keyEncoding(encoding));
    const lower = name.toLowerCase(), alias = /^aes(128|192|256)$/.exec(lower);
    const canonical = alias === null ? lower : `aes-${alias[1]}-cbc`;
    if (EXPORT_CIPHERS.includes(canonical)) return [canonical, bytes];
    if (!NODE_CIPHERS.has(lower)) throw new Error("Unknown cipher");
    throw new Error(`KeyObject export not supported: cipher "${name}" (supported: aes-128-cbc, aes-192-cbc and aes-256-cbc)`);
  };
  const EXPORT_CIPHERS = ["aes-128-cbc", "aes-192-cbc", "aes-256-cbc"];
  // Node 24's getCiphers(), lowercased; other names are unknown there.
  const NODE_CIPHERS = new Set("aes-128-cbc aes-128-cbc-hmac-sha1 aes-128-cbc-hmac-sha256 aes-128-ccm aes-128-cfb aes-128-cfb1 aes-128-cfb8 aes-128-ctr aes-128-ecb aes-128-gcm aes-128-ocb aes-128-ofb aes-128-xts aes-192-cbc aes-192-ccm aes-192-cfb aes-192-cfb1 aes-192-cfb8 aes-192-ctr aes-192-ecb aes-192-gcm aes-192-ocb aes-192-ofb aes-256-cbc aes-256-cbc-hmac-sha1 aes-256-cbc-hmac-sha256 aes-256-ccm aes-256-cfb aes-256-cfb1 aes-256-cfb8 aes-256-ctr aes-256-ecb aes-256-gcm aes-256-ocb aes-256-ofb aes-256-xts aes128 aes128-wrap aes128-wrap-pad aes192 aes192-wrap aes192-wrap-pad aes256 aes256-wrap aes256-wrap-pad aria-128-cbc aria-128-ccm aria-128-cfb aria-128-cfb1 aria-128-cfb8 aria-128-ctr aria-128-ecb aria-128-gcm aria-128-ofb aria-192-cbc aria-192-ccm aria-192-cfb aria-192-cfb1 aria-192-cfb8 aria-192-ctr aria-192-ecb aria-192-gcm aria-192-ofb aria-256-cbc aria-256-ccm aria-256-cfb aria-256-cfb1 aria-256-cfb8 aria-256-ctr aria-256-ecb aria-256-gcm aria-256-ofb aria128 aria192 aria256 camellia-128-cbc camellia-128-cfb camellia-128-cfb1 camellia-128-cfb8 camellia-128-ctr camellia-128-ecb camellia-128-ofb camellia-192-cbc camellia-192-cfb camellia-192-cfb1 camellia-192-cfb8 camellia-192-ctr camellia-192-ecb camellia-192-ofb camellia-256-cbc camellia-256-cfb camellia-256-cfb1 camellia-256-cfb8 camellia-256-ctr camellia-256-ecb camellia-256-ofb camellia128 camellia192 camellia256 chacha20 chacha20-poly1305 des-ede des-ede-cbc des-ede-cfb des-ede-ecb des-ede-ofb des-ede3 des-ede3-cbc des-ede3-cfb des-ede3-cfb1 des-ede3-cfb8 des-ede3-ecb des-ede3-ofb des3 des3-wrap id-aes128-ccm id-aes128-gcm id-aes128-wrap id-aes128-wrap-pad id-aes192-ccm id-aes192-gcm id-aes192-wrap id-aes192-wrap-pad id-aes256-ccm id-aes256-gcm id-aes256-wrap id-aes256-wrap-pad id-smime-alg-cms3deswrap sm4 sm4-cbc sm4-cfb sm4-ctr sm4-ecb sm4-ofb".split(" "));
  // Node's `encoding` for a string key or passphrase, where "buffer" means utf8.
  const keyEncoding = (encoding) => (encoding === "buffer" ? "utf8" : encoding);
  // PKCS#1 DER holds either half; a private key opens with the integer version 0, which is no modulus.
  const holdsPrivate = (der) => {
    const at = 2 + (der[1] > 0x80 ? der[1] & 0x7f : 0);
    return der[at] === 2 && der[at + 1] === 1 && der[at + 2] === 0;
  };
  // createPublicKey also takes a private KeyObject or private key PEM and derives its public half.
  const importKey = (key, type) => {
    let options = {};
    if (key !== null && typeof key === "object" && !isBytes(key) && !(key instanceof KeyObject)) [options, key] = [key, key.key];
    const held = keyData(key);
    if (held !== undefined) {
      // As in Node, createPrivateKey takes no KeyObject and createPublicKey only a private one.
      if (type === "private") throw new TypeError(`The "key" argument must be of type string or ${BYTES}`);
      if (held.type !== "private") throw new TypeError(`Invalid key object type ${held.type}, expected private.`);
      key = pemOf(held.bytes, "PRIVATE KEY");
    }
    const format = held === undefined && options.format !== undefined ? options.format : "pem";
    if (format === "jwk") return importJwk(key, type);
    if (format !== "pem" && format !== "der") throw invalidOption("options.format", format);
    const encoding = held === undefined ? keyEncoding(options.encoding) : undefined;
    let bytes = input(key, "key", encoding);
    if (format === "der") {
      const label = Object.hasOwn(DER_LABELS[type], options.type) ? DER_LABELS[type][options.type] : undefined;
      if (label === undefined) {
        throw invalidOption("options.type", options.type);
      }
      const pkcs1Private = type === "public" && options.type === "pkcs1" && holdsPrivate(bytes);
      bytes = list(Buf().from(pemOf(bytes, pkcs1Private ? DER_LABELS.private.pkcs1 : label)));
    }
    const passphrase = held === undefined && options.passphrase !== undefined ? passphraseOf(options.passphrase, encoding) : null;
    const [kind, der] = keyObject(bytes, type === "public", passphrase);
    return new KeyObject(MAKE, type, kind, der);
  };
  crypto.KeyObject = KeyObject;
  crypto.createSecretKey = (key, encoding) => new KeyObject(MAKE, "secret", undefined, input(key, "key", encoding));
  crypto.createPrivateKey = (key) => importKey(key, "private");
  crypto.createPublicKey = (key) => importKey(key, "public");
  // HMAC and cipher keys may be secret KeyObjects, handed on as their bytes.
  const secretOf = (key) => {
    const held = keyData(key);
    if (held === undefined) return key;
    if (held.type !== "secret") throw new TypeError(`Invalid key object type ${held.type}, expected secret.`);
    return Buf().from(held.bytes);
  };
  const hmac = crypto.createHmac;
  crypto.createHmac = (algorithm, key) => hmac(algorithm, secretOf(key));

  // [key length, IV length]; CTR takes a 16-byte counter block, GCM a 12-byte IV and a 16-byte tag.
  const CIPHERS = {
    "aes-128-cbc": [16, 16], "aes-192-cbc": [24, 16], "aes-256-cbc": [32, 16],
    "aes-128-ctr": [16, 16], "aes-192-ctr": [24, 16], "aes-256-ctr": [32, 16],
    "aes-128-gcm": [16, 12], "aes-192-gcm": [24, 12], "aes-256-gcm": [32, 12],
  };
  // The 16-byte block `base` read as a 128-bit big-endian counter and advanced by `blocks`, wrapping.
  const counterAt = (base, blocks) => {
    const out = base.slice();
    for (let i = 15, carry = blocks; i >= 0 && carry > 0; i--) {
      const sum = out[i] + carry;
      out[i] = sum % 256;
      carry = Math.floor(sum / 256);
    }
    return out;
  };
  const ENCODING_NAMES = { "utf-8": "utf8", ucs2: "utf16le", "ucs-2": "utf16le", "utf-16le": "utf16le", binary: "latin1" };
  // How many trailing bytes Node's StringDecoder keeps until more arrive: an incomplete UTF-8
  // sequence, the bytes past a whole base64 group, or the odd byte of a UTF-16 unit.
  const heldBack = (bytes, encoding) => {
    if (encoding === "base64" || encoding === "base64url") return bytes.length % 3;
    if (encoding === "utf16le") return bytes.length % 2;
    if (encoding !== "utf8") return 0;
    for (let back = 1; back <= Math.min(3, bytes.length); back++) {
      const b = bytes[bytes.length - back];
      if ((b & 0xc0) === 0x80) continue;
      const need = b >= 0xf8 ? 1 : b >= 0xf0 ? 4 : b >= 0xe0 ? 3 : b >= 0xc0 ? 2 : 1;
      return need > back ? back : 0;
    }
    return 0;
  };
  class Cipher {
    #name; #mode; #gcm; #encrypt; #key; #iv; #chunks = []; #aad = []; #tag; #pad = true; #updated = false; #done = false;
    #pending = []; #offset = 0; #counter; #streamed = []; #decoder; #held = [];
    constructor(algorithm, key, iv, options, encrypt) {
      const length = options?.authTagLength;
      if (length !== undefined && length !== 16) {
        throw new Error(`Cipher option not supported: "authTagLength: ${length}" (supported: 16; require('crypto-js') has more)`);
      }
      const name = String(algorithm).toLowerCase();
      if (!Object.hasOwn(CIPHERS, name)) {
        throw new Error(`Cipher not supported: "${algorithm}" (supported: ${Object.keys(CIPHERS).join(", ")}; require('crypto-js') has more)`);
      }
      const [keyLength, ivLength] = CIPHERS[name];
      this.#key = input(secretOf(key), "key");
      this.#iv = iv === null ? [] : input(iv, "iv");
      if (this.#key.length !== keyLength) throw new RangeError("Invalid key length");
      if (this.#iv.length !== ivLength) throw new TypeError("Invalid initialization vector");
      [this.#name, this.#mode, this.#gcm, this.#encrypt] = [name, name.slice(-3), ivLength === 12, encrypt];
      // GCM's payload keystream is CTR from the block after the tag's: the IV then a 32-bit 2.
      this.#counter = this.#gcm ? this.#iv.concat([0, 0, 0, 2]) : this.#iv;
    }
    update(data, inputEncoding, outputEncoding) {
      if (this.#done) throw new Error("Trying to add data in unsupported state");
      this.#updated = true;
      const bytes = input(data, "data", inputEncoding);
      if (this.#mode === "cbc") return this.#emit(this.#cbcBlocks(bytes), outputEncoding, false);
      if (this.#mode === "ctr") return this.#emit(this.#ctr(bytes), outputEncoding, false);
      // GCM streams through CTR and keeps everything for the one-shot tag at `final`.
      const out = this.#ctr(bytes);
      this.#chunks.push(bytes);
      if (this.#encrypt) this.#streamed.push(out);
      return this.#emit(out, outputEncoding, false);
    }
    final(outputEncoding) {
      if (this.#done) throw new Error("Invalid state");
      this.#done = true;
      let out = [];
      if (this.#mode === "cbc") {
        const op = (this.#encrypt ? "encrypt" : "decrypt") + (this.#pad ? "" : "-nopad");
        out = cipher(`${this.#name} ${op}`, this.#key, this.#iv, this.#pending, [], []);
      } else if (this.#gcm) {
        const sealed = cipher(`${this.#name} ${this.#encrypt ? "encrypt" : "decrypt"}`, this.#key, this.#iv, this.#chunks.flat(), this.#aad, this.#tag ?? []);
        if (this.#encrypt) {
          const streamed = this.#streamed.flat();
          if (sealed.length !== streamed.length + 16 || streamed.some((b, i) => b !== sealed[i])) {
            throw new Error("Unsupported state or unable to authenticate data");
          }
          this.#tag = Buf().from(sealed.slice(-16));
        }
      }
      return this.#emit(out, outputEncoding, true);
    }
    // CBC runs every whole block it has, except that padded decryption keeps the last one for
    // `final` to check; the next IV is the last ciphertext block.
    #cbcBlocks(bytes) {
      const all = this.#pending.concat(bytes);
      let take = all.length - (all.length % 16);
      if (!this.#encrypt && this.#pad && take === all.length && take > 0) take -= 16;
      this.#pending = all.slice(take);
      if (take === 0) return [];
      const blocks = all.slice(0, take);
      const op = this.#encrypt ? "encrypt-nopad" : "decrypt-nopad";
      const out = cipher(`${this.#name} ${op}`, this.#key, this.#iv, blocks, [], []);
      this.#iv = (this.#encrypt ? out : blocks).slice(-16);
      return out;
    }
    // CTR starts the keystream at the block holding `offset` and drops the bytes already used.
    #ctr(bytes) {
      const skip = this.#offset % 16;
      const counter = counterAt(this.#counter, Math.floor(this.#offset / 16));
      this.#offset += bytes.length;
      if (bytes.length === 0) return [];
      const out = cipher(`${this.#name.slice(0, -3)}ctr encrypt`, this.#key, counter, new Array(skip).fill(0).concat(bytes), [], []);
      return out.slice(skip);
    }
    // Bytes out as a Buffer, or a string through one decoder per cipher, as Node's StringDecoder.
    #emit(bytes, encoding, end) {
      if (!encoding || String(encoding).toLowerCase() === "buffer") return Buf().from(bytes);
      const lower = String(encoding).toLowerCase();
      const name = ENCODING_NAMES[lower] ?? lower;
      if (this.#decoder !== undefined && this.#decoder !== name) throw new Error("Cannot change encoding");
      this.#decoder = name;
      const all = this.#held.concat(bytes);
      const keep = end ? 0 : heldBack(all, name);
      this.#held = all.slice(all.length - keep);
      return Buf().from(all.slice(0, all.length - keep)).toString(name);
    }
    setAutoPadding(autoPadding = true) {
      if (this.#done) throw new Error("Invalid state for operation setAutoPadding");
      this.#pad = Boolean(autoPadding);
      return this;
    }
    setAAD(buffer) {
      if (!this.#gcm || this.#updated || this.#done) throw new Error("Invalid state for operation setAAD");
      if (!isBytes(buffer)) throw new TypeError(`The "buffer" argument must be ${BYTES}.`);
      this.#aad = this.#aad.concat(list(buffer));
      return this;
    }
    getAuthTag() {
      if (!this.#gcm || !this.#done || this.#tag === undefined) throw new Error("Invalid state for operation getAuthTag");
      return this.#tag;
    }
    setAuthTag(tag) {
      if (!this.#gcm || this.#done) throw new Error("Invalid state for operation setAuthTag");
      if (!isBytes(tag)) throw new TypeError(`The "buffer" argument must be ${BYTES}.`);
      const bytes = list(tag);
      if (bytes.length !== 16) throw new TypeError(`Invalid authentication tag length: ${bytes.length}`);
      this.#tag = bytes;
      return this;
    }
  }
  // As in Node, only a Cipheriv reads its tag and only a Decipheriv takes one.
  class Cipheriv extends Cipher { constructor(algorithm, key, iv, options) { super(algorithm, key, iv, options, true); } }
  class Decipheriv extends Cipher { constructor(algorithm, key, iv, options) { super(algorithm, key, iv, options, false); } }
  Object.defineProperty(Cipheriv.prototype, "setAuthTag", { value: undefined });
  Object.defineProperty(Decipheriv.prototype, "getAuthTag", { value: undefined });
  crypto.createCipheriv = (algorithm, key, iv, options) => new Cipheriv(algorithm, key, iv, options);
  crypto.createDecipheriv = (algorithm, key, iv, options) => new Decipheriv(algorithm, key, iv, options);
  crypto.getCiphers = () => Object.keys(CIPHERS);

  // The key type picks RSA, ECDSA or Ed25519 and `padding` PKCS#1 v1.5 or PSS; the name only picks the digest.
  const SIGNING = ["sha1", "sha256", "sha384", "sha512"];
  const unsupported = (what) => new Error(`Signature algorithm not supported: "${what}" (supported: RSA with sha1, sha256, sha384 or sha512; RSA-PSS with sha256, sha384 or sha512; ECDSA P-256, P-384 or P-521 with sha1, sha256, sha384 or sha512; Ed25519 with null; require('crypto-js') has more)`);
  // "" stands for null or undefined: the key's default digest.
  const digestOf = (algorithm) => {
    if (algorithm === null || algorithm === undefined) return "";
    if (typeof algorithm !== "string") throw new TypeError('The "algorithm" argument must be of type string');
    const name = algorithm.toLowerCase().replace(/^rsa-/, "");
    if (SIGNING.includes(name)) return name;
    throw unsupported(algorithm);
  };
  crypto.constants = Object.freeze({
    RSA_PKCS1_PADDING: 1, RSA_PKCS1_PSS_PADDING: 6, RSA_PSS_SALTLEN_DIGEST: -1, RSA_PSS_SALTLEN_MAX_SIGN: -2, RSA_PSS_SALTLEN_AUTO: -2,
  });
  // A PEM string, Buffer or KeyObject, or `{ key, format, type, passphrase, dsaEncoding, padding,
  // saltLength }`, as the host's [bytes, dsaEncoding, [padding, saltLength]], each null when
  // unset, since the key type settles their defaults. A key needing a format, encoding or
  // passphrase is imported first, as Node does.
  const keyOf = (key, signing) => {
    let options = {};
    if (key !== null && typeof key === "object" && !isBytes(key) && !(key instanceof KeyObject)) [options, key] = [key, key.key];
    const plainPem = (options.format === undefined || options.format === "pem") && options.passphrase === undefined && options.encoding === undefined;
    if (keyData(key) === undefined && !plainPem) key = importKey(options, signing ? "private" : "public");
    const held = keyData(key);
    if (held !== undefined) {
      if (held.type === "secret" || (signing && held.type !== "private")) {
        throw new TypeError(`Invalid key object type ${held.type}, expected ${signing ? "private" : "private or public"}.`);
      }
      key = pemOf(held.bytes, held.type === "private" ? "PRIVATE KEY" : "PUBLIC KEY");
    }
    if (typeof key !== "string" && !isBytes(key)) {
      throw new TypeError(`The "key" argument must be of type string or ${BYTES}`);
    }
    const dsa = options.dsaEncoding ?? "der";
    if (dsa !== "der" && dsa !== "ieee-p1363") throw new TypeError(`The property 'options.dsaEncoding' is invalid. Received '${dsa}'`);
    const padding = options.padding ?? null;
    if (padding !== null && padding !== 1 && padding !== 6) throw unsupported(`padding ${padding}`);
    const salt = options.saltLength ?? null;
    if (salt !== null && !Number.isInteger(salt)) throw new TypeError(`The "options.saltLength" property must be of type number`);
    return [input(key, "key"), dsa, [padding, salt]];
  };
  crypto.sign = (algorithm, data, key, callback) => {
    const digest = digestOf(algorithm), bytes = input(data, "data"), [k, dsa, options] = keyOf(key, true);
    const work = () => Buf().from(sign(digest, bytes, k, dsa, options));
    if (callback === undefined) return work();
    later(callbackOf(callback), work);
  };
  crypto.verify = (algorithm, data, key, signature, callback) => {
    const digest = digestOf(algorithm), bytes = input(data, "data"), [k, dsa, options] = keyOf(key, false);
    const sig = input(signature, "signature");
    const work = () => verify(digest, bytes, k, sig, dsa, options);
    if (callback === undefined) return work();
    later(callbackOf(callback), work);
  };
  // OpenSSL's refusals of an RSA-PSS key's digest, padding or salt, which Node's createVerify
  // reports as a failed verification where crypto.verify throws them.
  const PSS_REFUSALS = ["digest not allowed", "illegal or unsupported padding mode", "invalid salt length", "pss saltlen too small"];
  // createSign and createVerify gather chunks until one sign or verify call.
  const gather = (algorithm) => {
    if (typeof algorithm !== "string") throw new TypeError('The "algorithm" argument must be of type string');
    const digest = digestOf(algorithm), chunks = [];
    let done = false;
    const open = () => { if (done) throw new Error("Not initialised"); };
    return {
      digest,
      add(data, encoding) { open(); chunks.push(input(data, "data", encoding)); },
      take() { open(); done = true; return chunks.flat(); },
    };
  };
  crypto.createSign = (algorithm) => {
    const g = gather(algorithm);
    return {
      update(data, encoding) { g.add(data, encoding); return this; },
      sign(key, outputEncoding) {
        const [k, dsa, options] = keyOf(key, true);
        return output(sign(g.digest, g.take(), k, dsa, options), outputEncoding);
      },
    };
  };
  crypto.createVerify = (algorithm) => {
    const g = gather(algorithm);
    return {
      update(data, encoding) { g.add(data, encoding); return this; },
      verify(key, signature, signatureEncoding) {
        const [k, dsa, options] = keyOf(key, false);
        const sig = input(signature, "signature", signatureEncoding);
        try {
          return verify(g.digest, g.take(), k, sig, dsa, options);
        } catch (error) {
          if (PSS_REFUSALS.includes(error.message)) return false;
          throw error;
        }
      },
    };
  };

  // Named so a script learns what is missing; `subtle` and `webcrypto` stay undefined for feature checks.
  const STUBS = ["createCipher", "createDecipher", "createECDH", "createDiffieHellman", "createDiffieHellmanGroup",
    "getDiffieHellman", "diffieHellman", "generateKeyPair", "generateKeyPairSync", "generateKey", "generateKeySync",
    "publicEncrypt", "privateDecrypt", "privateEncrypt", "publicDecrypt", "hkdf", "hkdfSync", "generatePrime",
    "generatePrimeSync", "checkPrime", "checkPrimeSync", "getCurves"];
  for (const name of STUBS) {
    crypto[name] = () => { throw new Error(`crypto.${name} is not supported (require('crypto-js') has more)`); };
  }
})();
