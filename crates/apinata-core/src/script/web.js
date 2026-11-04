// Web globals for scripts: URL, URLSearchParams, TextEncoder, TextDecoder and structuredClone.
// Everything lives in one closure so the pieces share the UTF-8 helpers and state.
(() => {
  const define = (name, value) =>
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true });

  // UTF-8 encode; a lone surrogate becomes U+FFFD as in USVString conversion.
  const utf8Encode = (text) => {
    const out = [];
    for (const ch of String(text)) {
      let c = ch.codePointAt(0);
      if (c >= 0xd800 && c <= 0xdfff) c = 0xfffd;
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return out;
  };

  // UTF-8 decode with the WHATWG replacement rules; `fatal` returns null on the first bad
  // sequence instead of substituting U+FFFD. Code units collect in a bounded chunk flushed
  // into `parts`, so a large body is not built one concatenation per character.
  const utf8Decode = (bytes, fatal) => {
    const parts = [], chunk = [];
    const unit = (u) => {
      chunk.push(u);
      if (chunk.length >= 4096) { parts.push(String.fromCharCode.apply(null, chunk)); chunk.length = 0; }
    };
    const n = bytes.length;
    for (let i = 0; i < n; ) {
      const b = bytes[i];
      let need = 0, cp = 0, lo = 0x80, hi = 0xbf;
      if (b < 0x80) { unit(b); i++; continue; }
      else if (b >= 0xc2 && b <= 0xdf) { need = 1; cp = b & 0x1f; }
      else if (b >= 0xe0 && b <= 0xef) { need = 2; cp = b & 0x0f; if (b === 0xe0) lo = 0xa0; if (b === 0xed) hi = 0x9f; }
      else if (b >= 0xf0 && b <= 0xf4) { need = 3; cp = b & 0x07; if (b === 0xf0) lo = 0x90; if (b === 0xf4) hi = 0x8f; }
      else { if (fatal) return null; unit(0xfffd); i++; continue; }
      i++;
      let ok = true;
      for (let k = 0; k < need; k++) {
        const c = bytes[i];
        if (i >= n || c < lo || c > hi) { ok = false; break; }
        lo = 0x80; hi = 0xbf;
        cp = (cp << 6) | (c & 63);
        i++;
      }
      if (!ok) { if (fatal) return null; unit(0xfffd); continue; }
      if (cp < 0x10000) unit(cp);
      else { unit(0xd800 + ((cp - 0x10000) >> 10)); unit(0xdc00 + ((cp - 0x10000) & 0x3ff)); }
    }
    parts.push(String.fromCharCode.apply(null, chunk));
    return parts.join("");
  };

  // Buffer's utf8 toString decodes through the same replacement rules (libs.js).
  Object.defineProperty(__apinata, "utf8Decode", { value: (bytes) => utf8Decode(bytes, false) });

  // application/x-www-form-urlencoded: keep ASCII alphanumerics and `*-._`, space is `+`.
  const formEncode = (text) => {
    let out = "";
    for (const b of utf8Encode(text)) {
      if ((b >= 0x30 && b <= 0x39) || (b >= 0x41 && b <= 0x5a) || (b >= 0x61 && b <= 0x7a) ||
          b === 0x2a || b === 0x2d || b === 0x2e || b === 0x5f) out += String.fromCharCode(b);
      else if (b === 0x20) out += "+";
      else out += "%" + (b + 0x100).toString(16).slice(1).toUpperCase();
    }
    return out;
  };

  // Inverse of `formEncode`: `+` is a space, a malformed `%XX` stays literal, bytes decode lossily.
  const formDecode = (text) => {
    const bytes = [];
    // Walk code points, not UTF-16 units, so a raw astral character stays one scalar.
    const chars = Array.from(text);
    for (let i = 0; i < chars.length; i++) {
      const ch = chars[i];
      if (ch === "+") bytes.push(0x20);
      else if (ch === "%" && /^[0-9a-fA-F]{2}$/.test(chars.slice(i + 1, i + 3).join(""))) {
        bytes.push(parseInt(chars.slice(i + 1, i + 3).join(""), 16));
        i += 2;
      } else bytes.push(...utf8Encode(ch));
    }
    return utf8Decode(bytes, false);
  };

  const parseForm = (input) => {
    const list = [];
    for (const part of input.split("&")) {
      if (part === "") continue;
      const at = part.indexOf("=");
      const name = at < 0 ? part : part.slice(0, at);
      const value = at < 0 ? "" : part.slice(at + 1);
      list.push([formDecode(name), formDecode(value)]);
    }
    return list;
  };

  // Each object's pair list, plus an optional `update` callback a URL attaches to hear mutations.
  const states = new WeakMap();
  const attach = (params, update) => { states.get(params).update = update; };
  const stateOf = (self) => {
    const state = states.get(self);
    if (!state) throw new TypeError("Value of \"this\" must be of type URLSearchParams");
    return state;
  };
  const changed = (state) => { if (state.update) state.update(serialize(state.list)); };
  const serialize = (list) => list.map(([k, v]) => formEncode(k) + "=" + formEncode(v)).join("&");
  const text = (x) => String(x);
  const need = (args, count, name) => {
    if (args.length < count) {
      throw new TypeError(`Failed to execute '${name}' on 'URLSearchParams': ${count} arguments required, but only ${args.length} present.`);
    }
  };

  const iterate = (state, pick) => {
    let at = 0;
    const it = {
      next() {
        if (at >= state.list.length) return { value: undefined, done: true };
        return { value: pick(state.list[at++]), done: false };
      },
      [Symbol.iterator]() { return this; },
    };
    Object.defineProperty(it, Symbol.toStringTag, { value: "URLSearchParams Iterator", configurable: true });
    return it;
  };

  class URLSearchParams {
    constructor(init) {
      const state = { list: [], update: null };
      states.set(this, state);
      if (init === undefined || init === null) return;
      if (typeof init === "object" || typeof init === "function") {
        if (typeof init[Symbol.iterator] === "function") {
          for (const pair of init) {
            const items = [...pair];
            if (items.length !== 2) throw new TypeError("Each query pair must be an iterable [name, value] tuple");
            state.list.push([text(items[0]), text(items[1])]);
          }
        } else {
          for (const key of Object.keys(init)) state.list.push([key, text(init[key])]);
        }
        return;
      }
      const str = text(init);
      state.list = parseForm(str[0] === "?" ? str.slice(1) : str);
    }
    get size() { return stateOf(this).list.length; }
    append(...args) {
      need(args, 2, "append");
      const state = stateOf(this);
      state.list.push([text(args[0]), text(args[1])]);
      changed(state);
    }
    delete(...args) {
      need(args, 1, "delete");
      const state = stateOf(this);
      const name = text(args[0]);
      const value = args[1] === undefined ? undefined : text(args[1]);
      state.list = state.list.filter(([k, v]) => !(k === name && (value === undefined || v === value)));
      changed(state);
    }
    get(...args) {
      need(args, 1, "get");
      const hit = stateOf(this).list.find(([k]) => k === text(args[0]));
      return hit ? hit[1] : null;
    }
    getAll(...args) {
      need(args, 1, "getAll");
      const name = text(args[0]);
      return stateOf(this).list.filter(([k]) => k === name).map(([, v]) => v);
    }
    has(...args) {
      need(args, 1, "has");
      const name = text(args[0]);
      const value = args[1] === undefined ? undefined : text(args[1]);
      return stateOf(this).list.some(([k, v]) => k === name && (value === undefined || v === value));
    }
    set(...args) {
      need(args, 2, "set");
      const state = stateOf(this);
      const name = text(args[0]), value = text(args[1]);
      const first = state.list.findIndex(([k]) => k === name);
      if (first < 0) state.list.push([name, value]);
      else {
        state.list[first][1] = value;
        state.list = state.list.filter(([k], i) => k !== name || i === first);
      }
      changed(state);
    }
    sort() {
      const state = stateOf(this);
      // Array.prototype.sort is stable and `<` compares UTF-16 code units, as the standard asks.
      state.list.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
      changed(state);
    }
    forEach(callback, thisArg) {
      if (typeof callback !== "function") throw new TypeError("The \"callback\" argument must be of type function");
      const state = stateOf(this);
      for (let i = 0; i < state.list.length; i++) callback.call(thisArg, state.list[i][1], state.list[i][0], this);
    }
    keys() { return iterate(stateOf(this), ([k]) => k); }
    values() { return iterate(stateOf(this), ([, v]) => v); }
    entries() { return iterate(stateOf(this), ([k, v]) => [k, v]); }
    toString() { return serialize(stateOf(this).list); }
  }
  Object.defineProperty(URLSearchParams.prototype, Symbol.iterator, {
    value: URLSearchParams.prototype.entries, writable: true, configurable: true,
  });
  Object.defineProperty(URLSearchParams.prototype, Symbol.toStringTag, {
    value: "URLSearchParams", configurable: true,
  });
  // Methods are enumerable in the standard's interface objects; the class syntax hides them.
  for (const name of ["append", "delete", "get", "getAll", "has", "set", "sort", "forEach", "keys", "values", "entries", "toString", "size"]) {
    const d = Object.getOwnPropertyDescriptor(URLSearchParams.prototype, name);
    d.enumerable = true;
    Object.defineProperty(URLSearchParams.prototype, name, d);
  }

  define("URLSearchParams", URLSearchParams);

  const parseUrl = globalThis.__url_parse;
  const setUrl = globalThis.__url_set;
  delete globalThis.__url_parse;
  delete globalThis.__url_set;

  const urls = new WeakMap();
  const urlState = (self) => {
    const state = urls.get(self);
    if (!state) throw new TypeError("Value of \"this\" must be of type URL");
    return state;
  };

  class URL {
    constructor(input, base) {
      const parts = parseUrl(String(input), base === undefined ? null : String(base));
      const state = { parts: JSON.parse(parts), params: new URLSearchParams(), self: this };
      urls.set(this, state);
      // The list follows `search`; a change to the list writes the serialized form back. An
      // empty list clears the query, so no bare `?` is left behind.
      rebuild(state);
      attach(state.params, (serialized) => apply(state, "search", serialized));
    }
    static canParse(input, base) {
      try { parseUrl(String(input), base === undefined ? null : String(base)); return true; }
      catch { return false; }
    }
    get origin() { return urlState(this).parts.origin; }
    get href() { return urlState(this).parts.href; }
    set href(value) {
      const state = urlState(this);
      state.parts = JSON.parse(setUrl(state.parts.href, "href", String(value)));
      rebuild(state);
    }
    get searchParams() { return urlState(this).params; }
    toString() { return urlState(this).parts.href; }
    toJSON() { return urlState(this).parts.href; }
  }

  // Re-reads the pair list from the current query, as the standard does when `search` or `href` changes.
  function rebuild(state) {
    const search = state.parts.search;
    states.get(state.params).list = parseForm(search[0] === "?" ? search.slice(1) : search);
  }
  function apply(state, field, value) {
    state.parts = JSON.parse(setUrl(state.parts.href, field, value));
  }

  for (const field of ["protocol", "username", "password", "host", "hostname", "port", "pathname", "search", "hash"]) {
    Object.defineProperty(URL.prototype, field, {
      get() { return urlState(this).parts[field]; },
      set(value) {
        const state = urlState(this);
        apply(state, field, String(value));
        if (field === "search") rebuild(state);
      },
      enumerable: true,
      configurable: true,
    });
  }
  for (const name of ["origin", "href", "searchParams", "toString", "toJSON"]) {
    const d = Object.getOwnPropertyDescriptor(URL.prototype, name);
    d.enumerable = true;
    Object.defineProperty(URL.prototype, name, d);
  }
  Object.defineProperty(URL.prototype, Symbol.toStringTag, { value: "URL", configurable: true });
  define("URL", URL);
  // Bytes of an ArrayBuffer or any view over one (typed arrays, DataView, Node Buffer).
  const bytesOf = (input) => {
    if (input === undefined) return new Uint8Array(0);
    if (input instanceof ArrayBuffer) return new Uint8Array(input);
    if (ArrayBuffer.isView(input)) return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
    throw new TypeError("The \"input\" argument must be an ArrayBuffer or an ArrayBufferView");
  };

  class TextEncoder {
    get encoding() { return "utf-8"; }
    encode(text = "") { return new Uint8Array(utf8Encode(text)); }
    // Fill `dest` with whole characters only; `read` counts UTF-16 units consumed.
    encodeInto(text, dest) {
      let read = 0, written = 0;
      for (const ch of String(text)) {
        const bytes = utf8Encode(ch);
        if (written + bytes.length > dest.length) break;
        dest.set(bytes, written);
        written += bytes.length;
        read += ch.length;
      }
      return { read, written };
    }
  }
  Object.defineProperty(TextEncoder.prototype, Symbol.toStringTag, { value: "TextEncoder", configurable: true });

  const decoderState = new WeakMap();
  class TextDecoder {
    constructor(label = "utf-8", options = {}) {
      const name = String(label).trim().toLowerCase();
      if (name !== "utf-8" && name !== "utf8" && name !== "unicode-1-1-utf-8") {
        throw new RangeError("The \"" + label + "\" encoding is not supported");
      }
      decoderState.set(this, { fatal: !!(options && options.fatal), ignoreBOM: !!(options && options.ignoreBOM) });
    }
    get encoding() { return "utf-8"; }
    get fatal() { return decoderState.get(this).fatal; }
    get ignoreBOM() { return decoderState.get(this).ignoreBOM; }
    decode(input) {
      const { fatal, ignoreBOM } = decoderState.get(this);
      let bytes = bytesOf(input);
      if (!ignoreBOM && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) bytes = bytes.subarray(3);
      const text = utf8Decode(bytes, fatal);
      if (text === null) throw new TypeError("The encoded data was not valid for encoding utf-8");
      return text;
    }
  }
  Object.defineProperty(TextDecoder.prototype, Symbol.toStringTag, { value: "TextDecoder", configurable: true });
  define("TextEncoder", TextEncoder);
  define("TextDecoder", TextDecoder);

  const dataCloneError = (what) => {
    const e = new Error(what + " could not be cloned.");
    Object.defineProperty(e, "name", { value: "DataCloneError", configurable: true, writable: true });
    Object.defineProperty(e, "code", { value: 25, configurable: true, writable: true });
    return e;
  };

  // Deep copy following the structured clone rules; `memo` keeps cycles and shared references intact.
  const cloneValue = (v, memo) => {
    if (typeof v === "function" || typeof v === "symbol") throw dataCloneError(typeof v === "symbol" ? String(v) : "function");
    if (v === null || typeof v !== "object") return v;
    if (memo.has(v)) return memo.get(v);
    let out;
    if (v instanceof Date) { out = new Date(v.getTime()); memo.set(v, out); return out; }
    if (v instanceof RegExp) { out = new RegExp(v.source, v.flags); memo.set(v, out); return out; }
    if (v instanceof ArrayBuffer) { out = v.slice(0); memo.set(v, out); return out; }
    if (v instanceof Number || v instanceof String || v instanceof Boolean) { out = Object(v.valueOf()); memo.set(v, out); return out; }
    if (ArrayBuffer.isView(v)) {
      const buffer = cloneValue(v.buffer, memo);
      if (v instanceof DataView) out = new DataView(buffer, v.byteOffset, v.byteLength);
      else out = new globalThis[v[Symbol.toStringTag]](buffer, v.byteOffset, v.length);
      memo.set(v, out);
      return out;
    }
    if (v instanceof Map) {
      out = new Map(); memo.set(v, out);
      for (const [k, x] of v) out.set(cloneValue(k, memo), cloneValue(x, memo));
      return out;
    }
    if (v instanceof Set) {
      out = new Set(); memo.set(v, out);
      for (const x of v) out.add(cloneValue(x, memo));
      return out;
    }
    if (v instanceof Error) {
      const kinds = { EvalError, RangeError, ReferenceError, SyntaxError, TypeError, URIError };
      // Own-property lookup: a name like "constructor" must not resolve through Object.prototype.
      const Kind = Object.hasOwn(kinds, v.name) ? kinds[v.name] : Error;
      out = new Kind(v.message); memo.set(v, out);
      if (typeof v.stack === "string") Object.defineProperty(out, "stack", { value: v.stack, writable: true, configurable: true });
      if ("cause" in v) Object.defineProperty(out, "cause", { value: cloneValue(v.cause, memo), writable: true, configurable: true });
      return out;
    }
    out = Array.isArray(v) ? new Array(v.length) : {};
    memo.set(v, out);
    // Define, never assign: a `__proto__` key is plain data here, not a prototype swap.
    for (const k of Object.keys(v)) {
      Object.defineProperty(out, k, { value: cloneValue(v[k], memo), enumerable: true, writable: true, configurable: true });
    }
    return out;
  };
  define("structuredClone", function structuredClone(value) { return cloneValue(value, new Map()); });

  // querystring: Node's legacy API over encodeURIComponent rules.
  const qsEscape = (s) => encodeURIComponent(String(s));
  // Lenient percent-decoding: valid %XX pairs become bytes, everything else stays literal, and
  // the bytes are decoded as UTF-8 with replacement so malformed escapes never throw.
  const qsUnescape = (s) => {
    s = String(s);
    const bytes = [];
    for (let i = 0; i < s.length;) {
      if (s[i] === "%" && /^[0-9a-fA-F]{2}$/.test(s.slice(i + 1, i + 3))) {
        bytes.push(parseInt(s.slice(i + 1, i + 3), 16));
        i += 3;
      } else {
        const cp = s.codePointAt(i);
        bytes.push(...utf8Encode(String.fromCodePoint(cp)));
        i += cp > 0xffff ? 2 : 1;
      }
    }
    return utf8Decode(bytes, false);
  };
  const qsValue = (v) =>
    typeof v === "string" ? v
      : typeof v === "number" ? (Number.isFinite(v) ? String(v) : "")
      : typeof v === "bigint" || typeof v === "boolean" ? String(v)
      : "";
  const qsParse = (str, sep = "&", eq = "=") => {
    const out = Object.create(null);
    if (typeof str !== "string" || str.length === 0) return out;
    for (const part of str.split(sep || "&")) {
      if (part === "") continue;
      const at = part.indexOf(eq || "=");
      const rawKey = at >= 0 ? part.slice(0, at) : part;
      const rawVal = at >= 0 ? part.slice((eq || "=").length + at) : "";
      const key = qsUnescape(rawKey.replace(/\+/g, " "));
      const val = qsUnescape(rawVal.replace(/\+/g, " "));
      if (!(key in out)) out[key] = val;
      else if (Array.isArray(out[key])) out[key].push(val);
      else out[key] = [out[key], val];
    }
    return out;
  };
  const qsStringify = (obj, sep = "&", eq = "=") => {
    if (obj === null || typeof obj !== "object") return "";
    const parts = [];
    for (const k of Object.keys(obj)) {
      const v = obj[k];
      const key = qsEscape(qsValue(k)) + (eq || "=");
      if (Array.isArray(v)) for (const x of v) parts.push(key + qsEscape(qsValue(x)));
      else parts.push(key + qsEscape(qsValue(v)));
    }
    return parts.join(sep || "&");
  };
  const querystring = { parse: qsParse, stringify: qsStringify, escape: qsEscape, unescape: qsUnescape, decode: qsParse, encode: qsStringify };

  // `url` hands out the globals at call time, so a script that reassigns URL sees its own value.
  let urlModule;
  const urlExports = () => {
    if (!urlModule) {
      urlModule = {};
      Object.defineProperty(urlModule, "URL", { get: () => globalThis.URL, enumerable: true });
      Object.defineProperty(urlModule, "URLSearchParams", { get: () => globalThis.URLSearchParams, enumerable: true });
    }
    return urlModule;
  };
  const previous = globalThis.require;
  globalThis.require = (name) => {
    name = String(name);
    if (name === "url" || name === "node:url") return urlExports();
    if (name === "querystring" || name === "node:querystring") return querystring;
    return previous(name);
  };
})();
