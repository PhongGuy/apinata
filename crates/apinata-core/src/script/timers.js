// Builds `bru.sleep`, `setTimeout` and `clearTimeout` over the async host wait, which it takes off
// the global object. `setTimeout` returns a promise and calls `fn` with no arguments.
// `clearTimeout(p)` makes that promise's timer skip `fn`; an awaited sleep still elapses.
// `setInterval` throws, since a repeating timer has no promise to return.
(() => {
  const host = globalThis.__sleep;
  delete globalThis.__sleep;
  const sleep = async (ms) => {
    await host(Number(ms));
    return "slept";
  };
  bru.sleep = sleep;
  const cleared = new WeakSet();
  globalThis.setTimeout = (fn, ms) => {
    const p = (async () => {
      await sleep(ms);
      if (!cleared.has(p)) fn.apply();
    })();
    return p;
  };
  globalThis.clearTimeout = (id) => {
    if (id !== null && (typeof id === "object" || typeof id === "function")) cleared.add(id);
  };
  globalThis.setInterval = () => {
    throw new Error("setInterval is not supported; use setTimeout or await bru.sleep(ms) in a loop");
  };
})();
