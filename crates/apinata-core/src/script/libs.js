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
      // Joining a sparse array past the engine's string limit fails in Node at once; do not walk the holes.
      if (Array.isArray(e) && e.length >= 536870890 && e.toString === Array.prototype.toString && e.join === Array.prototype.join) {
        throw new RangeError("Invalid string length");
      }
      return String(e);
    };
    // util.format's %s: an object with a builtin toString is inspected one level deep, the rest as String().
    // The inspect subset covers plain objects, class instances, arrays, Date, RegExp, Map and Set.
    // util.inspect's string form: single quotes unless the text holds one, control characters escaped.
    const ESCAPES = /[\x00-\x1f\x27\x5c\x7f-\x9f]|[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g;
    const NAMED = { 8: "\\b", 9: "\\t", 10: "\\n", 12: "\\f", 13: "\\r", 39: "\\'", 92: "\\\\" };
    const quote = (s) => {
      let mark = "'";
      if (s.includes("'")) mark = !s.includes('"') ? '"' : !s.includes("`") && !s.includes("${") ? "`" : "'";
      return mark + s.replace(ESCAPES, (c) => {
        const code = c.charCodeAt(0);
        if (code === 39 && mark !== "'") return c;
        if (code in NAMED) return NAMED[code];
        return code >= 0xd800 && code <= 0xdfff ? `\\u${code.toString(16)}` : `\\x${code.toString(16).toUpperCase().padStart(2, "0")}`;
      }) + mark;
    };
    // The formatter runs on an error path, so it reads properties through descriptors and never runs a getter.
    const find = (o, k) => {
      for (; o !== null; o = Object.getPrototypeOf(o)) {
        const d = Object.getOwnPropertyDescriptor(o, k);
        if (d) return { d, holder: o };
      }
      return null;
    };
    const fnName = (f) => {
      const d = Object.getOwnPropertyDescriptor(f, "name");
      return d && typeof d.value === "string" ? d.value : "";
    };
    const GLOBALS = () => Object.getOwnPropertyNames(globalThis).filter((n) => /^[A-Z][a-zA-Z0-9]+$/.test(n));
    const builtinCtor = (holder) => {
      const c = Object.getOwnPropertyDescriptor(holder, "constructor");
      return c !== undefined && typeof c.value === "function" && GLOBALS().includes(fnName(c.value));
    };
    // Builtins are told apart by brand, never by a Symbol.toStringTag a script can set.
    const brandOf = (v) => {
      if (Array.isArray(v)) return "Array";
      const probes = {
        Date: () => Date.prototype.getTime.call(v),
        RegExp: () => Object.getOwnPropertyDescriptor(RegExp.prototype, "source").get.call(v),
        Map: () => Map.prototype.has.call(v),
        Set: () => Set.prototype.has.call(v),
      };
      for (const [name, probe] of Object.entries(probes)) {
        try { probe(); return name; } catch { /* not this brand */ }
      }
      return null;
    };
    // "Object" for anything a script tagged itself; a builtin's own tag (Error, Promise, ...) otherwise.
    const tagOf = (v) => {
      const t = find(v, Symbol.toStringTag);
      if (t && (!("value" in t.d) || !builtinCtor(t.holder))) return "Object";
      return Object.prototype.toString.call(v).slice(8, -1);
    };
    const ctorName = (v) => {
      for (let o = v; o !== null; o = Object.getPrototypeOf(o)) {
        const d = Object.getOwnPropertyDescriptor(o, "constructor");
        if (d && typeof d.value === "function" && fnName(d.value) !== "") return fnName(d.value);
      }
      return null;
    };
    // Node treats a toString found on a builtin's prototype as builtin, so a class instance still gets inspected.
    const builtinToString = (e) => {
      const prim = find(e, Symbol.toPrimitive);
      if (prim && (!("value" in prim.d) || (typeof prim.d.value === "function" && prim.d.value !== Date.prototype[Symbol.toPrimitive]))) return false;
      const t = find(e, "toString");
      if (!t) return true;
      if (!("value" in t.d)) return false;
      if (typeof t.d.value !== "function") return true;
      return t.holder !== e && builtinCtor(t.holder);
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
      const typeAt = (i) => {
        const d = Object.getOwnPropertyDescriptor(value, i);
        return d && "value" in d ? typeof d.value : "accessor";
      };
      const numeric = Array.from({ length: output.length }, (_, i) => i).every((i) => typeAt(i) === "number" || typeAt(i) === "bigint");
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
    const MAX_ITEMS = 100;
    const more = (n) => `... ${n} more item${n > 1 ? "s" : ""}`;
    const sizeOf = {
      Map: Object.getOwnPropertyDescriptor(Map.prototype, "size").get,
      Set: Object.getOwnPropertyDescriptor(Set.prototype, "size").get,
    };
    const prop = (v, k) => {
      const d = Object.getOwnPropertyDescriptor(v, k);
      if (!d) return "undefined";
      if (!("value" in d)) return d.get ? (d.set ? "[Getter/Setter]" : "[Getter]") : "[Setter]";
      return shown(d.value);
    };
    // Node's formatArray, which jumps from hole to hole through the present keys and stops after 100 entries.
    const arrayItems = (v, size) => {
      const output = [], max = Math.min(MAX_ITEMS, size);
      let i = 0;
      for (; i < max; i++) {
        if (!Object.hasOwn(v, i)) break;
        output.push(prop(v, i));
      }
      if (i === max) {
        if (size > max) output.push(more(size - max));
        return output;
      }
      const keys = Object.keys(v);
      let index = i;
      for (; i < keys.length && output.length < max; i++) {
        const key = keys[i];
        if (!/^(0|[1-9][0-9]*)$/.test(key) || Number(key) > 2 ** 32 - 2) break;
        if (`${index}` !== key) {
          const gap = Number(key) - index;
          output.push(`<${gap} empty item${gap > 1 ? "s" : ""}>`);
          index = Number(key);
          if (output.length === max) break;
        }
        output.push(prop(v, key));
        index++;
      }
      const rest = size - index;
      if (output.length !== max) {
        if (rest > 0) output.push(`<${rest} empty item${rest > 1 ? "s" : ""}>`);
      } else if (rest > 0) output.push(more(rest));
      return output;
    };
    const inspect = (v, top) => {
      const brand = brandOf(v);
      if (brand === "Date") return Number.isNaN(Date.prototype.getTime.call(v)) ? "Invalid Date" : Date.prototype.toISOString.call(v);
      if (brand === "RegExp") {
        const part = (k) => Object.getOwnPropertyDescriptor(RegExp.prototype, k).get.call(v);
        return `/${part("source")}/${part("flags")}`;
      }
      const array = brand === "Array", map = brand === "Map", set = brand === "Set";
      const name = ctorName(v);
      if (!brand && tagOf(v) !== "Object") return `[${name ?? tagOf(v)}]`;
      const size = array ? v.length : map || set ? sizeOf[brand].call(v) : 0;
      // A nested container with items prints as its name alone, so its keys need not be listed.
      const keys = !top && size > 0 ? [] : Reflect.ownKeys(v).filter((k) => !(array && isIndex(k)) && Object.prototype.propertyIsEnumerable.call(v, k));
      const named = name === null ? "[Object: null prototype]" : name;
      const open = array ? (name === "Array" ? "[" : `${named}(${size}) [`)
        : map || set ? `${named}(${size}) ${named === brand ? "" : `[${brand}] `}{`
        : name === "Object" ? "{" : `${named} {`;
      const close = array ? "]" : "}";
      if (size === 0 && keys.length === 0) return open + close;
      if (!top) return `[${named}]`;
      const key = (k) => (typeof k === "symbol" ? String(k) : /^[A-Za-z_$][\w$]*$/.test(k) ? k : quote(k));
      let output = [];
      if (array) output = arrayItems(v, size);
      else if (map || set) {
        const it = (map ? Map : Set).prototype[map ? "entries" : "values"].call(v);
        const max = Math.min(MAX_ITEMS, size);
        for (let n = 0; n < max; n++) {
          const { value } = it.next();
          output.push(map ? `${shown(value[0])} => ${shown(value[1])}` : shown(value));
        }
        if (size > max) output.push(more(size - max));
      }
      for (const k of keys) output.push(`${key(k)}: ${prop(v, k)}`);
      return join(open, close, output, array, v);
    };
    const shown = (v) => {
      if (typeof v === "string") return quote(v);
      if (typeof v === "function") return fnName(v) ? `[Function: ${fnName(v)}]` : "[Function (anonymous)]";
      return Object(v) === v ? inspect(v, false) : nameOf(v);
    };
    const nameOf = (e) => {
      if (typeof e === "bigint") return `${e}n`;
      if (Object.is(e, -0)) return "-0";
      if (Object(e) !== e || typeof e === "function") return String(e);
      // A proxy trap or a hostile prototype can still throw; the error text must not.
      try {
        const brand = brandOf(e);
        const inspectable = builtinToString(e) && (brand !== null || tagOf(e) === "Object");
        return inspectable ? inspect(e, true) : String(e);
      } catch {
        return "[Object]";
      }
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
