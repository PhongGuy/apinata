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
  const proxyTarget = globalThis.__proxyTarget;
  delete globalThis.__proxyTarget;
  // Reflection captured before a script can patch it, so a proxy's target never reaches script code.
  const uncurry = Function.prototype.bind.bind(Function.prototype.call);
  const { getOwnPropertyDescriptor: gopd, getPrototypeOf: gpo } = Object;
  const ownKeys = Reflect.ownKeys, isArray = Array.isArray, isNaNum = Number.isNaN;
  const pie = uncurry(Object.prototype.propertyIsEnumerable);
  const getterOf = (proto, k) => uncurry(gopd(proto, k).get);
  const dateTime = uncurry(Date.prototype.getTime), dateISO = uncurry(Date.prototype.toISOString);
  const reSource = getterOf(RegExp.prototype, "source"), mapSize = getterOf(Map.prototype, "size"), setSize = getterOf(Set.prototype, "size");
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
  const typed = (algorithm, what = "algorithm") => {
    if (typeof algorithm !== "string") throw wrongType(what, "string", algorithm);
  };
  const checked = (algorithm, what) => {
    typed(algorithm, what);
    const name = algorithm.toLowerCase();
    if (ALGORITHMS.includes(name)) return name;
    throw new Error(`Digest method not supported: "${algorithm}" (supported: ${ALGORITHMS.join(", ")}; require('crypto-js') has more)`);
  };
  const BYTES = "string or an instance of Buffer, TypedArray, or DataView";
  const bytesOf = (data, encoding, wrong = (v) => wrongType("data", BYTES, v)) => {
    if (typeof data === "string") {
      if (String(encoding).toLowerCase() === "base64url") return globalThis.Buffer.from(data, "base64");
      return globalThis.Buffer.from(data, encoding ?? "utf8");
    }
    if (data instanceof ArrayBuffer) return new Uint8Array(data).slice();
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength).slice();
    throw wrong(data);
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
  // util.inspect's string form: single quotes unless the text holds one, control characters escaped.
  const ESCAPES = /[\x00-\x1f\x27\x5c\x7f-\x9f]|[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g;
  const NAMED = { 8: "\\b", 9: "\\t", 10: "\\n", 12: "\\f", 13: "\\r", 39: "\\'", 92: "\\\\" };
  const inspected = (s) => {
    let quote = "'";
    if (s.includes("'")) quote = !s.includes('"') ? '"' : !s.includes("`") && !s.includes("${") ? "`" : "'";
    return quote + s.replace(ESCAPES, (c) => {
      const code = c.charCodeAt(0);
      if (code === 39 && quote !== "'") return c;
      if (code in NAMED) return NAMED[code];
      return code >= 0xd800 && code <= 0xdfff ? `\\u${code.toString(16)}` : `\\x${code.toString(16).toUpperCase().padStart(2, "0")}`;
    }) + quote;
  };
  // A wrong-type error shows a string cut and quoted far more plainly than util.inspect does.
  const quoted = (s) => (s.includes("'") ? JSON.stringify(s) : `'${s}'`);
  // A numeric separator every three digits, as Node writes an integer beyond 2^32 in a range error.
  const grouped = (n) => {
    const text = String(n);
    if (!Number.isInteger(n) || Math.abs(n) <= 2 ** 32) return text;
    let i = text.length, out = "";
    for (const start = text.startsWith("-") ? 1 : 0; i >= start + 4; i -= 3) out = `_${text.slice(i - 3, i)}${out}`;
    return text.slice(0, i) + out;
  };
  // The builtin brand of a value, told apart by the engine's own brand checks and never by a script-set tag.
  const brandOf = (v) => {
    if (isArray(v)) return "Array";
    const probes = [["Date", dateTime], ["RegExp", reSource], ["Map", mapSize], ["Set", setSize]];
    for (const [name, probe] of probes) {
      try { probe(v); return name; } catch { /* not this brand */ }
    }
    return null;
  };
  // A RegExp's flags come from each brand-checked getter, since `flags` reads properties a null prototype lacks.
  const reFlags = [["d", "hasIndices"], ["g", "global"], ["i", "ignoreCase"], ["m", "multiline"], ["s", "dotAll"], ["u", "unicode"], ["v", "unicodeSets"], ["y", "sticky"]]
    .map(([c, k]) => [c, getterOf(RegExp.prototype, k)]);
  const regexpText = (v) => `/${reSource(v)}/${reFlags.map(([c, get]) => (get(v) ? c : "")).join("")}`;
  // util.inspect at depth -1, for an object that names no constructor: Node's getPrefix for a null prototype.
  // A proxy is read through its target, so no trap runs and the target reaches only captured functions.
  const bare = (v) => {
    const target = proxyTarget(v);
    if (target === null) return "<Revoked Proxy>";
    if (target !== undefined && target !== false) v = target;
    if (gpo(v) !== null) return isArray(v) ? "[Array]" : "[Object]";
    const brand = brandOf(v);
    const d = gopd(v, Symbol.toStringTag);
    const tag = d && typeof d.value === "string" && d.value !== "" ? d : null;
    // A shown tag renames a plain object; a hidden one follows the name in brackets.
    const kind = brand ?? (tag?.enumerable ? tag.value : "Object");
    const hidden = tag && !tag.enumerable && tag.value !== kind ? ` [${tag.value}]` : "";
    const keys = ownKeys(v).some((k) => pie(v, k));
    const size = brand === "Array" ? v.length : brand === "Map" ? mapSize(v) : brand === "Set" ? setSize(v) : 0;
    const head = (count) => `[${kind}${count}: null prototype]${hidden}`;
    if (brand === "Date" || brand === "RegExp") {
      if (keys) return head("");
      return `${head("")} ${brand === "Date" ? (isNaNum(dateTime(v)) ? "Invalid Date" : dateISO(v)) : regexpText(v)}`;
    }
    if (keys || size > 0) return head("");
    return `${head(brand === null ? "" : `(${size})`)} ${brand === "Array" ? "[]" : "{}"}`;
  };
  // Node reads `constructor` and `name` with plain property gets, so a getter or a proxy trap runs and
  // an error it throws passes up.
  const received = (v) => {
    if (v == null) return ` Received ${v}`;
    if (typeof v === "function") return ` Received function ${v.name}`;
    if (typeof v === "object") {
      const c = v.constructor;
      if (c) {
        // The engine words these two errors differently from Node, so they are thrown in Node's words.
        if (Object(c) !== c) throw new TypeError(`Cannot use 'in' operator to search for 'name' in ${String(c)}`);
        if ("name" in c) {
          const name = c.name;
          if (typeof name === "symbol") throw new TypeError("Cannot convert a Symbol value to a string");
          return ` Received an instance of ${name}`;
        }
      }
      // The depth -1 text reads only descriptors, but a proxy can still throw from its own traps.
      try { return ` Received ${bare(v)}`; } catch { return " Received [Object]"; }
    }
    // Node cuts a long string before it quotes it.
    const text = typeof v === "string" ? (v.length > 28 ? `${v.slice(0, 25)}...` : v) : v;
    const shown = typeof v === "string" ? quoted(text) : typeof v === "bigint" ? `${v}n` : Object.is(v, -0) ? "-0" : String(v);
    return ` Received type ${typeof v} (${shown})`;
  };
  const coded = (error, code) => Object.assign(error, { code });
  const wrongType = (what, type, v) =>
    coded(new TypeError(`The "${what}" argument must be of type ${type}.${received(v)}`), "ERR_INVALID_ARG_TYPE");
  const outputEncoding = (e) => {
    if (typeof e !== "string") throw wrongType("outputEncoding", "string", e);
    if (e === "" || e.toLowerCase() === "buffer" || ENCODINGS.includes(e.toLowerCase())) return e;
    throw coded(new TypeError(`The argument 'outputEncoding' is invalid. Received ${inspected(e)}`), "ERR_INVALID_ARG_VALUE");
  };
  const outputLength = (n) => {
    if (typeof n !== "number") throw wrongType("outputLength", "number", n);
    const range = (rule) => coded(new RangeError(`The value of "outputLength" is out of range. It must be ${rule}. Received ${grouped(n)}`), "ERR_OUT_OF_RANGE");
    if (!Number.isInteger(n)) throw range("an integer");
    if (n < 0 || n > 4294967295) throw range(">= 0 && <= 4294967295");
    return n;
  };
  // Node checks the algorithm's type, the data, then the options, and only then whether the algorithm exists.
  // No algorithm here is an XOF, so an outputLength must equal the digest length.
  const oneShot = (algorithm, data, options) => {
    typed(algorithm);
    const bytes = bytesOf(data, undefined, (v) => wrongType("input", BYTES, v));
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
    createHmac: (algorithm, key) => new Hmac(checked(algorithm, "hmac"), bytesOf(key, undefined, () => new TypeError(`The "key" argument must be of type ${BYTES}`))),
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
