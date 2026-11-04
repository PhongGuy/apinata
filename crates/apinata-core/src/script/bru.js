// Builds `bru` over the host's JSON dispatch function, which it takes off the global object.
// Name checks and messages follow the reference; values cross as JSON, `undefined` by omission.
(() => {
  const host = globalThis.__bru;
  delete globalThis.__bru;
  const unsupported = __apinata.unsupported;

  // Host answers: {v} a value, {} undefined, {p} a whole store as [name, value?] pairs, {e} an error.
  const call = (op, ...args) => {
    const answer = JSON.parse(host(op, JSON.stringify(args)));
    if ("e" in answer) throw new Error(answer.e);
    if ("p" in answer) {
      const out = {};
      for (const [name, ...value] of answer.p) out[name] = value.length ? value[0] : undefined;
      return out;
    }
    return answer.v;
  };
  const key = (name) => String(name);

  const variableNameRegex = /^[\w-.]*$/;
  const assertValidVariableName = (name) => {
    if (variableNameRegex.test(name) === false) {
      throw new Error(
        `Variable name: "${name}" contains invalid characters!`
        + ' Names must only contain alpha-numeric characters, "-", "_", "."'
      );
    }
  };
  // A setter sends the value only when it is defined, so the host can tell `undefined` apart.
  const setter = (scope, emptyMessage) => (name, value) => {
    if (!name) throw new Error(emptyMessage);
    assertValidVariableName(name);
    if (value === undefined) call("set", scope, key(name));
    else call("set", scope, key(name), value);
  };
  const noName = "Creating a variable without specifying a name is not allowed.";
  const noEnvName = "Creating a env variable without specifying a name is not allowed.";

  const interpolate = (value) => {
    if (!value) return value;
    if (typeof value === "object") return call("interpolateJson", JSON.stringify(value));
    if (typeof value !== "string") return value;
    return call("interpolate", value);
  };

  const stub = (name) => () => unsupported("bru." + name);
  const deferred = [
    "sleep", "sendRequest", "runRequest", "resetOauth2Credential",
  ];
  // `undefined` leaves the next request alone; `null` ends the run.
  const skipRequest = () => { call("skipRequest"); };
  const stopExecution = () => { call("stopExecution"); };
  const setNextRequest = (name) => {
    if (name !== undefined) call("setNextRequest", name);
  };
  const summarize = (results) => {
    const count = (status) => results.filter((r) => r.status === status).length;
    return { total: results.length, passed: count("pass"), failed: count("fail"), skipped: count("skipped") };
  };
  const cookiesFail = () => unsupported("bru.cookies");

  const bru = {
    cwd: () => call("cwd"),
    getEnvName: () => call("envName"),
    getCollectionName: () => call("collectionName"),
    isSafeMode: () => true,
    getProcessEnv: (name) => call("processEnv", key(name)),
    getOauth2CredentialVar: (name) => call("oauth2Var", key(name)),
    interpolate,

    hasVar: (name) => call("has", "runtime", key(name)),
    getVar: (name) => {
      assertValidVariableName(name);
      return call("get", "runtime", key(name));
    },
    setVar: setter("runtime", noName),
    deleteVar: (name) => call("delete", "runtime", key(name)),
    deleteAllVars: () => call("deleteAll", "runtime"),
    getAllVars: () => call("all", "runtime"),

    hasEnvVar: (name) => call("has", "env", key(name)),
    getEnvVar: (name) => call("get", "env", key(name)),
    setEnvVar: setter("env", noEnvName),
    deleteEnvVar: (name) => call("delete", "env", key(name)),
    deleteAllEnvVars: () => call("deleteAll", "env"),
    getAllEnvVars: () => call("all", "env"),

    hasGlobalEnvVar: (name) => call("has", "global", key(name)),
    getGlobalEnvVar: (name) => call("get", "global", key(name)),
    setGlobalEnvVar: setter("global", noEnvName),
    deleteGlobalEnvVar: (name) => call("delete", "global", key(name)),
    deleteAllGlobalEnvVars: () => call("deleteAll", "global"),
    getAllGlobalEnvVars: () => call("all", "global"),

    hasCollectionVar: (name) => call("has", "collection", key(name)),
    getCollectionVar: (name) => call("get", "collection", key(name)),
    setCollectionVar: setter("collection", noName),
    deleteCollectionVar: (name) => call("delete", "collection", key(name)),
    deleteAllCollectionVars: () => call("deleteAll", "collection"),
    getAllCollectionVars: () => call("all", "collection"),

    getFolderVar: (name) => call("get", "folder", key(name)),
    getRequestVar: (name) => call("get", "request", key(name)),

    runner: { skipRequest, stopExecution, setNextRequest },
    setNextRequest,
    getTestResults: async () => {
      const results = __apinata.testResults();
      return { summary: summarize(results), results };
    },
    getAssertionResults: async () => {
      const results = call("assertions");
      return { summary: summarize(results), results };
    },
    // Serializing `bru` reads `toJSON`, which must not throw.
    cookies: new Proxy(function () {}, {
      apply: cookiesFail,
      construct: cookiesFail,
      get: (_, prop) => (prop === "toJSON" || typeof prop === "symbol" ? undefined : cookiesFail()),
    }),
  };
  for (const name of deferred) bru[name] = stub(name);
  globalThis.bru = bru;
})();
