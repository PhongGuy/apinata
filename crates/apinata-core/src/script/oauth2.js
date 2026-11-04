// Builds `bru.resetOauth2Credential` over the async host function it takes off the global
// object. It returns at once; the phase waits for the persisted clear.
(() => {
  const host = globalThis.__resetOauth2;
  delete globalThis.__resetOauth2;
  bru.resetOauth2Credential = (id) => {
    if (typeof id !== "string" || id === "") {
      throw new Error("bru.resetOauth2Credential: credentialsId must be a non-empty string");
    }
    host(id);
  };
})();
