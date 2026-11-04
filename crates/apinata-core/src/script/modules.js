// The POSIX `path` module, answering `require("path")` and a lazy `path` global, with the
// collection root as the working directory for `resolve` and `relative`, and `require` of
// collection files. Written for Apinata.
(() => {
  const { root, load, resolve: resolveModule } = globalThis.__modules;
  delete globalThis.__modules;

  const str = (p) => {
    if (typeof p !== "string") throw new TypeError('The "path" argument must be of type string');
    return p;
  };
  // Collapses `.`, empty and `..` segments; an absolute path drops `..` at its root, a relative
  // one keeps the leading `..` it cannot cancel.
  const segments = (p, absolute) => {
    const out = [];
    for (const seg of p.split("/")) {
      if (seg === "" || seg === ".") continue;
      if (seg !== "..") out.push(seg);
      else if (out.length && out[out.length - 1] !== "..") out.pop();
      else if (!absolute) out.push("..");
    }
    return out;
  };

  const isAbsolute = (p) => str(p).startsWith("/");
  const normalize = (p) => {
    if (str(p) === "") return ".";
    const absolute = isAbsolute(p);
    let body = segments(p, absolute).join("/");
    if (body === "" && !absolute) body = ".";
    if (body !== "" && p.endsWith("/")) body += "/";
    return (absolute ? "/" : "") + body;
  };
  const join = (...parts) => {
    const kept = parts.map(str).filter((p) => p !== "");
    return kept.length ? normalize(kept.join("/")) : ".";
  };
  // Prepends arguments right to left until one is absolute, then falls back to the root.
  const resolve = (...parts) => {
    let joined = "";
    for (let i = parts.length - 1; i >= 0 && !joined.startsWith("/"); i--) {
      if (str(parts[i]) !== "") joined = joined ? parts[i] + "/" + joined : parts[i];
    }
    if (!joined.startsWith("/")) joined = joined ? root + "/" + joined : root;
    const absolute = joined.startsWith("/");
    return (absolute ? "/" : "") + segments(joined, absolute).join("/");
  };
  const relative = (from, to) => {
    const a = segments(resolve(from), true);
    const b = segments(resolve(to), true);
    let common = 0;
    while (common < a.length && common < b.length && a[common] === b[common]) common++;
    return [...a.slice(common).map(() => ".."), ...b.slice(common)].join("/");
  };
  const trimmed = (p) => {
    let end = str(p).length;
    while (end > 1 && p[end - 1] === "/") end--;
    return p.slice(0, end);
  };
  const dirname = (p) => {
    const s = trimmed(p);
    const i = s.lastIndexOf("/");
    return i === -1 ? "." : i === 0 ? "/" : s.slice(0, i);
  };
  const basename = (p, ext) => {
    const s = trimmed(p);
    const base = s === "/" ? "" : s.slice(s.lastIndexOf("/") + 1);
    return typeof ext === "string" && ext !== base && base.endsWith(ext)
      ? base.slice(0, base.length - ext.length)
      : base;
  };
  const extname = (p) => {
    const base = basename(p);
    const i = base.lastIndexOf(".");
    return i <= 0 || base === ".." ? "" : base.slice(i);
  };

  const path = { resolve, join, dirname, basename, extname, relative, normalize, isAbsolute, sep: "/" };
  const settle = (value) =>
    Object.defineProperty(globalThis, "path", { value, writable: true, configurable: true });
  Object.defineProperty(globalThis, "path", {
    configurable: true,
    get() { settle(path); return path; },
    set(value) { settle(value); },
  });

  // A local module runs as a CommonJS body. The script's own names are cached raw; a module's
  // local names resolve against its directory first, so one file reached two ways loads once.
  const isLocal = (name) => name.startsWith(".") || name.startsWith(root);
  const cache = new Map();
  const loadModule = (key) => {
    if (cache.has(key)) return cache.get(key);
    const { source, dir } = JSON.parse(load(key));
    const module = { exports: {} };
    const localRequire = (sub) => {
      sub = String(sub);
      return isLocal(sub) ? loadModule(resolveModule(sub, dir)) : requireAny(sub);
    };
    new Function("module", "exports", "require", source)(module, module.exports, localRequire);
    cache.set(key, module.exports);
    return module.exports;
  };

  const previous = globalThis.require;
  const requireAny = (name) => {
    name = String(name);
    if (name === "path") return path;
    return isLocal(name) ? loadModule(name) : previous(name);
  };
  globalThis.require = requireAny;
})();
