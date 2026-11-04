// Builds `bru.runRequest` over the async host function it takes off the global object. The
// promise never rejects: a failed run resolves to an object naming why.
(() => {
  const host = globalThis.__runRequest;
  delete globalThis.__runRequest;
  bru.runRequest = async (path) => JSON.parse(await host(String(path ?? "")));
})();
