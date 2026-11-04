// `uuid`, `nanoid` and `crypto` over the host's random, uuid and hash functions, plus `require` for
// them and for `atob`/`btoa`. The host owns generation; `parse`, `stringify`, `validate` and `version`
// follow uuid@10 in JavaScript.
(() => {
  const host = globalThis.__uuid;
  const random = globalThis.__random;
  const hash = globalThis.__hash;
  delete globalThis.__uuid;
  delete globalThis.__random;
  delete globalThis.__hash;
  const define = (name, value) =>
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true });

  const VALID = /^(?:[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$/i;
  const validate = (id) => typeof id === "string" && VALID.test(id);
  const parse = (id) => {
    if (!validate(id)) throw new TypeError("Invalid UUID");
    return Uint8Array.from(id.replace(/-/g, "").match(/../g), (h) => parseInt(h, 16));
  };
  const stringify = (bytes) => {
    const hex = Array.from(bytes, (b) => (b + 0x100).toString(16).slice(1)).join("");
    const id = [hex.slice(0, 8), hex.slice(8, 12), hex.slice(12, 16), hex.slice(16, 20), hex.slice(20)].join("-");
    if (!validate(id)) throw new TypeError("Stringified UUID is invalid");
    return id;
  };
  const version = (id) => {
    if (!validate(id)) throw new TypeError("Invalid UUID");
    return parseInt(id.slice(14, 15), 16);
  };
  // `parse` rejects any namespace that is not a canonical hyphenated UUID, as uuid@10 does.
  const named = (op) => (name, namespace) => {
    parse(namespace);
    return host(op, JSON.stringify([String(name), namespace]));
  };
  const uuid = {
    v1: () => host("v1", "[]"),
    v3: named("v3"),
    v4: () => host("v4", "[]"),
    v5: named("v5"),
    v6: () => host("v6", "[]"),
    v7: () => host("v7", "[]"),
    parse, stringify, validate, version,
  };

  // 64 symbols, so masking a random byte to six bits is unbiased.
  const ALPHABET = "useandom-26T198340PX75pxJACKVERYMINDBUSHWOLF_GQZbfghjklqvwyzrict";
  // Always 21 symbols: the size argument is ignored, as in the reference.
  const nanoid = () => {
    const bytes = JSON.parse(random(21));
    return bytes.map((b) => ALPHABET[b & 63]).join("");
  };
  const nanoidModule = { nanoid };

  const TYPED = [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array,
    Uint32Array, Float32Array, Float64Array, BigInt64Array, BigUint64Array];
  // Node's Hash and Hmac in one: chunks are copied as they arrive and hashed by the host in one
  // call at `digest()`. A `base64url` input decodes as base64, which already reads `-_`, and
  // digest output is mapped here, so hashing never depends on Buffer's own base64url.
  const ALGORITHMS = ["md5", "sha1", "sha224", "sha256", "sha384", "sha512"];
  const typed = (algorithm) => {
    if (typeof algorithm !== "string") throw new TypeError('The "algorithm" argument must be of type string');
  };
  const checked = (algorithm) => {
    typed(algorithm);
    const name = algorithm.toLowerCase();
    if (ALGORITHMS.includes(name)) return name;
    throw new Error(`Digest method not supported: "${algorithm}" (supported: ${ALGORITHMS.join(", ")}; require('crypto-js') has more)`);
  };
  const bytesOf = (data, encoding, what = "data") => {
    if (typeof data === "string") {
      if (String(encoding).toLowerCase() === "base64url") return globalThis.Buffer.from(data, "base64");
      return globalThis.Buffer.from(data, encoding ?? "utf8");
    }
    if (data instanceof ArrayBuffer) return new Uint8Array(data).slice();
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength).slice();
    throw new TypeError(`The "${what}" argument must be of type string or an instance of Buffer, TypedArray, or DataView`);
  };
  class Hash {
    #algorithm; #key; #chunks = []; #done = false;
    constructor(algorithm, key) { this.#algorithm = algorithm; this.#key = key; }
    #open() { if (this.#done) throw new Error("Digest already called"); }
    copy() {
      this.#open();
      const twin = new Hash(this.#algorithm, this.#key);
      twin.#chunks = this.#chunks.slice();
      return twin;
    }
    update(data, encoding) {
      this.#open();
      this.#chunks.push(bytesOf(data, encoding));
      return this;
    }
    digest(encoding) {
      this.#open();
      this.#done = true;
      const data = this.#chunks.flatMap((c) => Array.from(c));
      const out = globalThis.Buffer.from(this.#key === undefined
        ? hash(this.#algorithm, data)
        : hash(this.#algorithm, data, Array.from(this.#key)));
      const name = String(encoding).toLowerCase();
      if (!encoding || name === "buffer") return out;
      if (name === "base64url") {
        return out.toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
      }
      return out.toString(encoding);
    }
  }

  // Node's argument errors, which name what they received.
  const ENCODINGS = ["hex", "utf8", "utf-8", "ascii", "latin1", "binary", "base64", "base64url", "ucs2", "ucs-2", "utf16le", "utf-16le"];
  const quoted = (s) => (s.includes("'") ? JSON.stringify(s) : `'${s}'`);
  const received = (v) => {
    if (v == null) return ` Received ${v}`;
    if (typeof v === "function") return ` Received function ${v.name}`;
    if (typeof v === "object") return ` Received an instance of ${v.constructor?.name ?? "Object"}`;
    const shown = typeof v === "string" ? quoted(v) : typeof v === "bigint" ? `${v}n` : Object.is(v, -0) ? "-0" : String(v);
    return ` Received type ${typeof v} (${shown})`;
  };
  const coded = (error, code) => Object.assign(error, { code });
  const wrongType = (what, type, v) =>
    coded(new TypeError(`The "${what}" argument must be of type ${type}.${received(v)}`), "ERR_INVALID_ARG_TYPE");
  const outputEncoding = (e) => {
    if (typeof e !== "string") throw wrongType("outputEncoding", "string", e);
    if (e === "" || e.toLowerCase() === "buffer" || ENCODINGS.includes(e.toLowerCase())) return e;
    throw coded(new TypeError(`The argument 'outputEncoding' is invalid. Received ${quoted(e)}`), "ERR_INVALID_ARG_VALUE");
  };
  const outputLength = (n) => {
    if (typeof n !== "number") throw wrongType("outputLength", "number", n);
    const range = (rule) => coded(new RangeError(`The value of "outputLength" is out of range. It must be ${rule}. Received ${n}`), "ERR_OUT_OF_RANGE");
    if (!Number.isInteger(n)) throw range("an integer");
    if (n < 0 || n > 4294967295) throw range(">= 0 && <= 4294967295");
    return n;
  };
  // Node checks the algorithm's type, the data, then the options, and only then whether the algorithm exists.
  // No algorithm here is an XOF, so an outputLength must equal the digest length.
  const oneShot = (algorithm, data, options) => {
    typed(algorithm);
    const bytes = bytesOf(data);
    let encoding = "hex", length;
    if (typeof options === "string") encoding = outputEncoding(options);
    else if (options !== undefined) {
      if (options === null || typeof options !== "object" || Array.isArray(options)) throw wrongType("options", "object", options);
      // Each option is read once, as Node does, so a getter runs once.
      const { outputEncoding: e, outputLength: n } = options;
      if (e != null) encoding = outputEncoding(e);
      if (n !== undefined) length = outputLength(n);
    }
    const out = new Hash(checked(algorithm)).update(bytes).digest();
    if (length !== undefined && length !== out.length) {
      throw new Error(`Output length ${length} is invalid for ${algorithm}, which does not support XOF`);
    }
    if (encoding.toLowerCase() === "buffer") return out;
    return out.toString(encoding === "" ? "utf8" : encoding);
  };

  // Node gives Hmac no `copy`.
  class Hmac extends Hash {}
  Object.defineProperty(Hmac.prototype, "copy", { value: undefined });

  const crypto = {
    getRandomValues(array) {
      if (!TYPED.some((T) => array instanceof T)) throw new TypeError("Argument is not an integer or float typed array");
      const bytes = JSON.parse(random(array.byteLength));
      new Uint8Array(array.buffer, array.byteOffset, array.byteLength).set(bytes);
      return array;
    },
    createHash: (algorithm) => new Hash(checked(algorithm)),
    createHmac: (algorithm, key) => new Hmac(checked(algorithm), bytesOf(key, undefined, "key")),
    hash: oneShot,
    getHashes: () => ALGORITHMS.slice(),
    randomBytes(size, callback) {
      if (typeof size !== "number") throw new TypeError('The "size" argument must be of type number');
      if (!Number.isInteger(size) || size < 0 || size > 65536) {
        throw new RangeError('The "size" argument must be an integer from 0 to 65536');
      }
      const buf = globalThis.Buffer.from(JSON.parse(random(size)));
      if (typeof callback !== "function") return buf;
      Promise.resolve().then(() => callback(null, buf));
    },
    randomUUID: () => host("v4", "[]"),
  };

  define("uuid", uuid);
  define("nanoid", nanoidModule);
  define("crypto", crypto);

  const modules = { uuid, nanoid: nanoidModule, crypto, "node:crypto": crypto, atob: globalThis.atob, btoa: globalThis.btoa };
  const previous = globalThis.require;
  globalThis.require = (name) => {
    name = String(name);
    return Object.hasOwn(modules, name) ? modules[name] : previous(name);
  };
})();
