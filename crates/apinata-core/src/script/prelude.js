// Runs once per phase before any installer or script. It takes the host's capture functions
// off the global object, so scripts reach them only through `console` and `test`.
(() => {
  const host = globalThis.__host;
  delete globalThis.__host;

  // String() throws for objects without a prototype; fall back to their tag.
  const str = (v) => {
    try { return String(v); } catch (e) { return Object.prototype.toString.call(v); }
  };
  const json = (v) => {
    try { const s = JSON.stringify(v); return s === undefined ? str(v) : s; } catch (e) { return str(v); }
  };
  // Console text follows a formatting table: one rule per kind, JSON for the rest.
  const format = (v) => {
    if (typeof v === "string") return v;
    if (v === undefined) return "undefined";
    if (typeof v === "function") return "function " + v.name + "() {\n    [native code]\n}";
    if (typeof v === "number" || typeof v === "bigint") return String(v);
    if (v instanceof Date || v instanceof RegExp || v instanceof Error) return str(v);
    if (v instanceof Set) return "Set " + json([...v]);
    if (v instanceof Map) return "Map " + json([...v]);
    return json(v);
  };
  // Every console method ends in `emit`, the one host.log call, so redaction sees all output.
  // A group indents each line of the text, not only its first.
  let depth = 0;
  const emit = (level, text) => {
    const pad = "  ".repeat(depth);
    host.log(level, pad ? text.split("\n").map((l) => pad + l).join("\n") : text);
  };
  const line = (args) => args.map(format).join(" ");
  const console = {};
  for (const level of ["log", "debug", "info", "warn", "error"]) {
    console[level] = (...args) => emit(level, line(args));
  }
  console.dir = (v) => emit("log", format(v));
  console.group = (...args) => {
    if (args.length) emit("log", line(args));
    depth += 1;
  };
  console.groupEnd = () => { depth = Math.max(0, depth - 1); };
  console.assert = (cond, ...args) => {
    if (cond) return;
    emit("error", args.length ? "Assertion failed: " + line(args) : "Assertion failed");
  };
  const counts = new Map();
  console.count = (label = "default") => {
    label = str(label);
    const n = (counts.get(label) || 0) + 1;
    counts.set(label, n);
    emit("log", label + ": " + n);
  };
  const timers = new Map();
  console.time = (label = "default") => { timers.set(str(label), Date.now()); };
  console.timeEnd = (label = "default") => {
    label = str(label);
    if (!timers.has(label)) return emit("warn", "Timer '" + label + "' does not exist");
    emit("log", label + ": " + (Date.now() - timers.get(label)) + "ms");
    timers.delete(label);
  };
  // Rows are the entries of an array or object; an object row spreads into one column per key
  // and anything else lands in "Values". Padded cells keep the columns aligned.
  const TABLE_ROWS = 100;
  console.table = (data) => {
    if (data === null || typeof data !== "object") return emit("log", format(data));
    const entries = Array.isArray(data) ? Array.from(data, (v, i) => [String(i), v]) : Object.entries(data);
    // Collections and other value-like objects show as one cell; newlines are escaped so a
    // cell never breaks the alignment.
    const isRow = (v) => v !== null && typeof v === "object" &&
      !(v instanceof Map || v instanceof Set || v instanceof Date || v instanceof RegExp || v instanceof Error);
    const cell = (v) => format(v).replace(/\n/g, "\\n");
    const shown = entries.slice(0, TABLE_ROWS);
    const cols = [];
    let values = false;
    for (const [, v] of shown) {
      if (isRow(v)) {
        for (const k of Object.keys(v)) if (!cols.includes(k)) cols.push(k);
      } else values = true;
    }
    const head = ["(index)", ...cols, ...(values ? ["Values"] : [])];
    const rows = shown.map(([key, v]) => {
      const obj = isRow(v);
      return [key, ...cols.map((c) => (obj && c in v ? cell(v[c]) : "")), ...(values ? [obj ? "" : cell(v)] : [])];
    });
    const widths = head.map((h, i) => Math.max(h.length, ...rows.map((r) => r[i].length)));
    const fmt = (r) => r.map((c, i) => c.padEnd(widths[i])).join(" | ").trimEnd();
    emit("log", fmt(head));
    for (const r of rows) emit("log", fmt(r));
    if (entries.length > shown.length) emit("log", "... " + (entries.length - shown.length) + " more rows");
  };
  globalThis.console = console;

  // Unsupported members throw an ordinary Error with a hidden marker, so the host and `test`
  // can report them as skipped instead of failed.
  const unsupportedError = (message) => {
    const e = new Error(message);
    Object.defineProperty(e, "__apinataUnsupported", { value: true });
    return e;
  };
  Object.defineProperty(globalThis, "__apinata", {
    enumerable: false,
    value: {
      segment: 0,
      unsupported(name) { throw unsupportedError(name + " is not supported by Apinata"); },
      unsupportedBecause(message) { throw unsupportedError(message); },
      testResults: () => JSON.parse(host.results()),
    },
  });

  // A test passes when its function returns or its promise resolves; nothing is rethrown,
  // so one failing test never ends the phase.
  globalThis.test = (name, fn) => {
    name = str(name);
    const fail = (e) => {
      if (e && e.__apinataUnsupported) host.test(name, "skip", str(e.message));
      else host.test(name, "fail", str((e && e.message) || "An unexpected error occurred."));
    };
    try {
      const r = fn();
      if (r && typeof r.then === "function") return r.then(() => host.test(name, "pass", ""), fail);
      host.test(name, "pass", "");
    } catch (e) {
      fail(e);
    }
  };
})();
