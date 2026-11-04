// `expect`, `assert` and `require("chai")` share one chai, evaluated on first use so a phase
// that never asserts never pays for it. Each global getter replaces itself with its value.
(() => {
  const load = globalThis.__loadChai;
  delete globalThis.__loadChai;
  let chai;
  const getChai = () => (chai ??= load());

  const settle = (name, value) =>
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true });
  for (const name of ["expect", "assert"]) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      get() { const value = getChai()[name]; settle(name, value); return value; },
      set(value) { settle(name, value); },
    });
  }

  // Library modules and collection files are answered by the layers installed above this one.
  globalThis.require = (name) => {
    name = String(name);
    if (name === "chai") return getChai();
    throw new Error("Cannot find module " + name);
  };
})();
