// Globals that later plans implement. Each is a Proxy over a function, so `typeof` still
// reports it for feature detection, while calling, constructing or reading it throws as
// unsupported, naming the global.
(() => {
  const unsupported = __apinata.unsupported;
  const stub = (name) => {
    const fail = () => unsupported(name);
    return new Proxy(function () {}, { apply: fail, construct: fail, get: fail });
  };
  const names = ["axios", "moment", "uuid", "nanoid", "jwt", "path", "crypto", "tv4", "Ajv", "addFormats", "setTimeout"];
  // QuickJS-ng ships `atob`; `btoa` is stubbed only where the engine lacks it.
  if (typeof globalThis.btoa === "undefined") names.push("btoa");
  for (const name of names) {
    Object.defineProperty(globalThis, name, { value: stub(name), writable: true, configurable: true });
  }
})();
