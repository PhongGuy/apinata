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
    // util.format's %s: an object with a builtin toString is inspected one level deep, the rest as String().
    // The inspect subset covers plain objects, class instances, arrays, Date, RegExp, Map and Set.
    const quote = (v) => (v.includes("'") ? JSON.stringify(v) : `'${v}'`);
    const tagOf = (v) => Object.prototype.toString.call(v).slice(8, -1);
    const ctorName = (v) => {
      for (let o = v; o !== null; o = Object.getPrototypeOf(o)) {
        const d = Object.getOwnPropertyDescriptor(o, "constructor");
        if (d && typeof d.value === "function" && d.value.name !== "") return d.value.name;
      }
      return null;
    };
    // Node treats a toString found on a builtin's prototype as builtin, so a class instance still gets inspected.
    const builtinToString = (e) => {
      const prim = e[Symbol.toPrimitive];
      if (typeof prim === "function" && prim !== Date.prototype[Symbol.toPrimitive]) return false;
      if (typeof e.toString !== "function") return true;
      if (Object.hasOwn(e, "toString")) return false;
      let o = e;
      do o = Object.getPrototypeOf(o); while (!Object.hasOwn(o, "toString"));
      const d = Object.getOwnPropertyDescriptor(o, "constructor");
      return d !== undefined && typeof d.value === "function"
        && Object.getOwnPropertyNames(globalThis).includes(d.value.name) && /^[A-Z][a-zA-Z0-9]+$/.test(d.value.name);
    };
    const isIndex = (k) => typeof k === "string" && /^(0|[1-9]\d*)$/.test(k) && Number(k) < 2 ** 32 - 1;
    // Node packs more than six array entries into aligned columns; this is its groupArrayElements at breakLength 80.
    const group = (output, value) => {
      let total = 0, widest = 0, count = output.length;
      if (100 < output.length) count--;
      const lengths = [];
      for (let i = 0; i < count; i++) {
        lengths.push(output[i].length);
        total += output[i].length + 2;
        widest = Math.max(widest, output[i].length);
      }
      const cell = widest + 2;
      if (cell * 3 >= 80 || !(total / cell > 5 || widest <= 6)) return output;
      const bias = Math.max(cell - 3 - Math.sqrt(cell - total / output.length), 1);
      const columns = Math.min(Math.round(Math.sqrt(2.5 * bias * count) / bias), Math.floor(80 / cell), 12, 15);
      if (columns <= 1) return output;
      const widths = [];
      for (let i = 0; i < columns; i++) {
        let w = 0;
        for (let j = i; j < output.length; j += columns) w = Math.max(w, lengths[j] ?? 0);
        widths.push(w + 2);
      }
      const numeric = Array.from({ length: output.length }, (_, i) => i).every((i) => typeof value[i] === "number" || typeof value[i] === "bigint");
      const pad = (t, n) => (numeric ? t.padStart(n) : t.padEnd(n));
      const rows = [];
      for (let i = 0; i < count; i += columns) {
        const max = Math.min(i + columns, count);
        let row = "", j = i;
        for (; j < max - 1; j++) row += pad(`${output[j]}, `, widths[j - i]);
        row += numeric ? output[j].padStart(widths[j - i] - 2) : output[j];
        rows.push(row);
      }
      if (100 < output.length) rows.push(output[count]);
      return rows;
    };
    // One line when it fits in 80 columns, otherwise one entry per line.
    const join = (open, close, output, array, value) => {
      const entries = output.length;
      if (array && entries > 6) output = group(output, value);
      if (entries === output.length) {
        let total = output.length + output.length + open.length + 10;
        if (total + output.length <= 80 && output.every((o) => (total += o.length) <= 80)) return `${open} ${output.join(", ")} ${close}`;
      }
      return `${open}\n  ${output.join(",\n  ")}\n${close}`;
    };
    const inspect = (v, top) => {
      const tag = tagOf(v);
      if (tag === "Date") return Number.isNaN(Date.prototype.getTime.call(v)) ? "Invalid Date" : Date.prototype.toISOString.call(v);
      if (tag === "RegExp") return RegExp.prototype.toString.call(v);
      const array = Array.isArray(v), map = tag === "Map", set = tag === "Set";
      const name = ctorName(v);
      if (!array && !map && !set && tag !== "Object") return `[${name ?? tag}]`;
      const keys = Reflect.ownKeys(v).filter((k) => !(array && isIndex(k)) && Object.prototype.propertyIsEnumerable.call(v, k));
      const size = array ? v.length : map || set ? v.size : 0;
      const named = name === null ? "[Object: null prototype]" : name;
      const open = array ? (name === "Array" ? "[" : `${named}(${size}) [`)
        : map || set ? `${named}(${size}) ${named === (map ? "Map" : "Set") ? "" : `[${map ? "Map" : "Set"}] `}{`
        : name === "Object" ? "{" : `${named} {`;
      const close = array ? "]" : "}";
      if (size === 0 && keys.length === 0) return open + close;
      if (!top) return `[${named}]`;
      const key = (k) => (typeof k === "symbol" ? String(k) : /^[A-Za-z_$][\w$]*$/.test(k) ? k : quote(k));
      const output = [];
      if (array) {
        let i = 0;
        while (i < size && output.length < 100) {
          if (Object.hasOwn(v, i)) output.push(shown(v[i++]));
          else {
            let j = i;
            while (j < size && !Object.hasOwn(v, j)) j++;
            output.push(`<${j - i} empty item${j - i > 1 ? "s" : ""}>`);
            i = j;
          }
        }
        if (i < size) output.push(`... ${size - i} more item${size - i > 1 ? "s" : ""}`);
      } else if (map) for (const [k, x] of v) output.push(`${shown(k)} => ${shown(x)}`);
      else if (set) for (const x of v) output.push(shown(x));
      for (const k of keys) {
        const d = Object.getOwnPropertyDescriptor(v, k);
        const text = d.get ? (d.set ? "[Getter/Setter]" : "[Getter]") : d.set ? "[Setter]" : shown(d.value);
        output.push(`${key(k)}: ${text}`);
      }
      return join(open, close, output, array, v);
    };
    const shown = (v) => {
      if (typeof v === "string") return quote(v);
      if (typeof v === "function") return v.name ? `[Function: ${v.name}]` : "[Function (anonymous)]";
      return Object(v) === v ? inspect(v, false) : nameOf(v);
    };
    const nameOf = (e) => {
      if (typeof e === "bigint") return `${e}n`;
      if (Object.is(e, -0)) return "-0";
      const inspectable = Object(e) === e && typeof e !== "function" && builtinToString(e)
        && (Array.isArray(e) || ["Object", "Date", "RegExp", "Map", "Set"].includes(tagOf(e)));
      return inspectable ? inspect(e, true) : String(e);
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
