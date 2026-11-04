// `moment`, `_` (lodash), `tv4`, `Buffer`, `Ajv` and `addFormats` are globals and their modules answer
// `require`, all evaluated on first use so a phase that never uses them never pays. `CryptoJS` is not a
// global (use `require('crypto-js')`); `Ajv` and `addFormats` come from one bundle, evaluated once for both.
(() => {
  const load = globalThis.__loadLib;
  delete globalThis.__loadLib;
  const cache = {};
  const get = (name) => (cache[name] ??= name === "buffer" ? nodeBuffer(load(name)) : load(name));
  const modules = {
    moment: () => get("moment"), lodash: () => get("lodash"), "crypto-js": () => get("crypto-js"), tv4: () => get("tv4"),
    buffer: () => get("buffer"), ajv: () => get("ajv").Ajv, "ajv-formats": () => get("ajv").addFormats,
  };
  const globals = { moment: modules.moment, _: modules.lodash, tv4: modules.tv4, Buffer: () => get("buffer").Buffer,
    Ajv: modules.ajv, addFormats: modules["ajv-formats"] };

  // The vendored Buffer predates Node's base64url. Decoding goes through base64, whose decoder already
  // reads `-_` and missing padding; encoding maps base64's `+/` to `-_` and strips the padding. Its
  // UTF-8 decoder replaces each bad byte, so utf8 decodes the clamped byte range itself with the WHATWG
  // replacement rules, as Node does.
  function nodeBuffer(mod) {
    const B = mod.Buffer, P = B.prototype;
    const url = (e) => e != null && String(e).toLowerCase() === "base64url";
    const utf8 = (e) => e === undefined || /^utf-?8$/i.test(String(e));
    const plain = (args) => args.map((a, i) => (i > 0 && url(a) ? "base64" : a));
    const { isEncoding, from, byteLength } = B;
    const { toString, write, fill } = P;
    // Node turns the encoding into a string first; the engine's own messages differ, so throw Node's.
    const asText = (e) => {
      if (typeof e === "symbol") throw new TypeError("Cannot convert a Symbol value to a string");
      const callable = (k) => typeof e[k] === "function";
      if (Object(e) === e && ![Symbol.toPrimitive, "toString", "valueOf"].some(callable)) {
        throw new TypeError("Cannot convert object to primitive value");
      }
      return String(e);
    };
    // util.format's %s: a plain object or array is inspected one level deep, the rest as String().
    const shown = (v) => {
      if (typeof v === "string") return v.includes("'") ? JSON.stringify(v) : `'${v}'`;
      if (typeof v === "function") return v.name ? `[Function: ${v.name}]` : "[Function (anonymous)]";
      if (Object(v) === v) return Array.isArray(v) ? "[Array]" : "[Object]";
      return nameOf(v);
    };
    const nameOf = (e) => {
      if (typeof e === "bigint") return `${e}n`;
      if (Object.is(e, -0)) return "-0";
      const plainObject = Object(e) === e && typeof e !== "function" && !Object.hasOwn(e, "toString")
        && (Array.isArray(e) ? e.toString === Array.prototype.toString : Object.getPrototypeOf(e) === Object.prototype);
      if (!plainObject) return String(e);
      const key = (k) => (typeof k === "symbol" ? String(k) : /^[A-Za-z_$][\w$]*$/.test(k) ? k : shown(k));
      const own = Reflect.ownKeys(e).filter((k) => Object.prototype.propertyIsEnumerable.call(e, k));
      const parts = Array.isArray(e) ? e.map(shown) : own.map((k) => `${key(k)}: ${shown(e[k])}`);
      if (parts.length === 0) return Array.isArray(e) ? "[]" : "{}";
      return Array.isArray(e) ? `[ ${parts.join(", ")} ]` : `{ ${parts.join(", ")} }`;
    };
    B.isEncoding = (e) => url(e) || isEncoding(e);
    B.from = function (...args) { return from.apply(this, plain(args)); };
    B.byteLength = (...args) => byteLength(...plain(args));
    P.write = function (...args) { return write.apply(this, plain(args)); };
    P.fill = function (...args) { return fill.apply(this, plain(args)); };
    P.toString = function (encoding, start, end) {
      // Only a non-string can fail to convert, and Node returns "" for an empty range before converting it.
      const empty = () => toString.call(this, "latin1", start, end) === "";
      if (encoding !== undefined && typeof encoding !== "string" && empty()) return "";
      const name = encoding === undefined ? "utf8" : asText(encoding);
      if (utf8(name)) {
        // Decode the bytes in place, clamped exactly as the vendored toString clamps its range.
        let s = start, e = end;
        if (s === undefined || s < 0) s = 0;
        if (s > this.length) return "";
        if (e === undefined || e > this.length) e = this.length;
        if (e <= 0) return "";
        e >>>= 0; s >>>= 0;
        return e <= s ? "" : __apinata.utf8Decode(this.subarray(s, e));
      }
      if (url(name)) {
        const text = toString.call(this, "base64", start, end);
        return text.replace(/[+/=]/g, (c) => (c === "+" ? "-" : c === "/" ? "_" : ""));
      }
      if (isEncoding(name)) return toString.call(this, name, start, end);
      // Node names any other encoding unknown, but only once the clamped range holds a byte.
      if (empty()) return "";
      throw Object.assign(new TypeError("Unknown encoding: " + nameOf(encoding)), { code: "ERR_UNKNOWN_ENCODING" });
    };
    P.toLocaleString = P.toString;
    return mod;
  }

  const settle = (name, value) =>
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true });
  const reading = new Set();
  for (const [name, read] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      get() {
        // lodash reads the old `_` while it loads; answer undefined then, not another load.
        if (reading.has(name)) return undefined;
        reading.add(name);
        try { const value = read(); settle(name, value); return value; } finally { reading.delete(name); }
      },
      set(value) { settle(name, value); },
    });
  }

  // The chai `jsonSchema` assertion reads ajv here, out of reach of a script that rebinds `require` or `Ajv`.
  Object.defineProperty(__apinata, "ajv", { value: () => get("ajv") });

  // Each library answers its own module names and hands the rest to the previous `require`.
  const previous = globalThis.require;
  globalThis.require = (name) => {
    name = String(name);
    return Object.hasOwn(modules, name) ? modules[name]() : previous(name);
  };
})();
