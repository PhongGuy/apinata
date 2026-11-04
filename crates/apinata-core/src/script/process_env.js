// Wraps `process.env` so a value counts as read, and so sensitive, only once an expression
// reads it. Reflection is captured here, before any expression can replace it.
(() => {
  const note = globalThis.__readEnv;
  delete globalThis.__readEnv;
  const process = globalThis.process;
  if (typeof process !== "object" || process === null) return;
  const env = process.env;
  if (typeof env !== "object" || env === null) return;
  const { get, getOwnPropertyDescriptor } = Reflect;
  process.env = new Proxy(env, {
    get(target, name, receiver) {
      if (typeof name === "string") note(name);
      return get(target, name, receiver);
    },
    getOwnPropertyDescriptor(target, name) {
      if (typeof name === "string") note(name);
      return getOwnPropertyDescriptor(target, name);
    },
  });
})();
